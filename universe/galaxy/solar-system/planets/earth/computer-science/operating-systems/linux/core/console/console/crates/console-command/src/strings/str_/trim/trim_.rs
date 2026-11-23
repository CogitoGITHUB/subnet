use console_cmd_base::input_handler::{CmdArgument, operate};
use console_engine::command_prelude::*;
use console_protocol::shell_error::generic::GenericError;

#[derive(Clone)]
pub struct StrTrim;

struct Arguments {
    to_trim: Option<char>,
    trim_side: TrimSide,
    cell_paths: Option<Vec<CellPath>>,
    mode: ActionMode,
}

impl CmdArgument for Arguments {
    fn take_cell_paths(&mut self) -> Option<Vec<CellPath>> {
        self.cell_paths.take()
    }
}

pub enum TrimSide {
    Left,
    Right,
    Both,
}

impl Command for StrTrim {
    fn name(&self) -> &str {
        "str trim"
    }

    fn signature(&self) -> Signature {
        Signature::build("str trim")
            .input_output_types(vec![
                (Type::String, Type::String),
                (
                    Type::List(Box::new(Type::String)),
                    Type::List(Box::new(Type::String)),
                ),
                (Type::table(), Type::table()),
                (Type::record(), Type::record()),
            ])
            .allow_variants_without_examples(true)
            .rest(
                "rest",
                SyntaxShape::CellPath,
                "For a data structure input, trim strings at the given cell paths.",
            )
            .named(
                "char",
                SyntaxShape::String,
                "Character to trim (default: whitespace).",
                Some('c'),
            )
            .switch(
                "left",
                "Trims characters only from the beginning of the string.",
                Some('l'),
            )
            .switch(
                "right",
                "Trims characters only from the end of the string.",
                Some('r'),
            )
            .category(Category::Strings)
    }
    fn description(&self) -> &str {
        "Trim whitespace or specific character."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["whitespace", "strip", "lstrip", "rstrip"]
    }

    fn is_const(&self) -> bool {
        true
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let character = call.get_flag::<Spanned<String>>(engine_state, stack, "char")?;
        let cell_paths: Vec<CellPath> = call.rest(engine_state, stack, 0)?;
        let left = call.has_flag(engine_state, stack, "left")?;
        let right = call.has_flag(engine_state, stack, "right")?;
        run(
            character,
            cell_paths,
            (left, right),
            call,
            input,
            engine_state,
        )
    }

    fn run_const(
        &self,
        working_set: &StateWorkingSet,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let character = call.get_flag_const::<Spanned<String>>(working_set, stack, "char")?;
        let cell_paths: Vec<CellPath> = call.rest_const(working_set, stack, 0)?;
        let left = call.has_flag_const(working_set, stack, "left")?;
        let right = call.has_flag_const(working_set, stack, "right")?;
        run(
            character,
            cell_paths,
            (left, right),
            call,
            input,
            working_set.permanent(),
        )
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Trim whitespace.",
                example: "'Nu shell ' | str trim",
                result: Some(Value::test_string("Nu shell")),
            },
            Example {
                description: "Trim a specific character (not the whitespace).",
                example: "'=== Nu shell ===' | str trim --char '='",
                result: Some(Value::test_string(" Nu shell ")),
            },
            Example {
                description: "Trim whitespace from the beginning of string.",
                example: "' Nu shell ' | str trim --left",
                result: Some(Value::test_string("Nu shell ")),
            },
            Example {
                description: "Trim whitespace from the end of string.",
                example: "' Nu shell ' | str trim --right",
                result: Some(Value::test_string(" Nu shell")),
            },
            Example {
                description: "Trim a specific character only from the end of the string.",
                example: "'=== Nu shell ===' | str trim --right --char '='",
                result: Some(Value::test_string("=== Nu shell ")),
            },
        ]
    }
}

fn run(
    character: Option<Spanned<String>>,
    cell_paths: Vec<CellPath>,
    (left, right): (bool, bool),
    call: &Call,
    input: PipelineData,
    engine_state: &EngineState,
) -> Result<PipelineData, ShellError> {
    let to_trim = match character.as_ref() {
        Some(v) => {
            if v.item.chars().count() > 1 {
                return Err(ShellError::Generic(GenericError::new(
                    "Trim only works with single character",
                    "needs single character",
                    v.span,
                )));
            }
            v.item.chars().next()
        }
        None => None,
    };

    let cell_paths = (!cell_paths.is_empty()).then_some(cell_paths);
    let mode = match cell_paths {
        None => ActionMode::Global,
        Some(_) => ActionMode::Local,
    };

    let trim_side = match (left, right) {
        (true, true) => TrimSide::Both,
        (true, false) => TrimSide::Left,
        (false, true) => TrimSide::Right,
        (false, false) => TrimSide::Both,
    };

    let args = Arguments {
        to_trim,
        trim_side,
        cell_paths,
        mode,
    };
    operate(action, args, input, call.head, engine_state.signals())
}

#[derive(Debug, Copy, Clone)]
pub enum ActionMode {
    Local,
    Global,
}

fn action(input: &Value, arg: &Arguments, head: Span) -> Value {
    let char_ = arg.to_trim;
    let trim_side = &arg.trim_side;
    let mode = &arg.mode;
    match input {
        Value::String { val: s, .. } => Value::string(trim(s, char_, trim_side), head),
        // Propagate errors by explicitly matching them before the final case.
        Value::Error { .. } => input.clone(),
        other => {
            let span = other.span();

            match mode {
                ActionMode::Global => match other {
                    Value::Record { val: record, .. } => {
                        let new_record = record
                            .iter()
                            .map(|(k, v)| (k.clone(), action(v, arg, head)))
                            .collect();

                        Value::record(new_record, span)
                    }
                    Value::List { vals, .. } => {
                        let new_vals = vals.iter().map(|v| action(v, arg, head)).collect();

                        Value::list(new_vals, span)
                    }
                    _ => input.clone(),
                },
                ActionMode::Local => Value::error(
                    ShellError::UnsupportedInput {
                        msg: "Only string values are supported".into(),
                        input: format!("input type: {:?}", other.get_type()),
                        msg_span: head,
                        input_span: other.span(),
                    },
                    head,
                ),
            }
        }
    }
}

fn trim(s: &str, char_: Option<char>, trim_side: &TrimSide) -> String {
    let delimiters = match char_ {
        Some(c) => vec![c],
        // Trying to make this trim work like rust default trim()
        // which uses is_whitespace() as a default
        None => vec![
            ' ',    // space
            '\x09', // horizontal tab
            '\x0A', // new line, line feed
            '\x0B', // vertical tab
            '\x0C', // form feed, new page
            '\x0D', // carriage return
        ], //whitespace
    };

    match trim_side {
        TrimSide::Left => s.trim_start_matches(&delimiters[..]).to_string(),
        TrimSide::Right => s.trim_end_matches(&delimiters[..]).to_string(),
        TrimSide::Both => s.trim_matches(&delimiters[..]).to_string(),
    }
}

