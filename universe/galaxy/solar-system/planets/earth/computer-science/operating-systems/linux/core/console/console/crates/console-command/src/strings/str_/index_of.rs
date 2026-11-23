use std::ops::Bound;

use crate::{grapheme_flags, grapheme_flags_const};
use console_cmd_base::input_handler::{CmdArgument, operate};
use console_engine::command_prelude::*;
use console_protocol::{IntRange, engine::StateWorkingSet};
use unicode_segmentation::UnicodeSegmentation;

struct Arguments {
    end: bool,
    substring: String,
    range: Option<Spanned<IntRange>>,
    cell_paths: Option<Vec<CellPath>>,
    graphemes: bool,
}

impl CmdArgument for Arguments {
    fn take_cell_paths(&mut self) -> Option<Vec<CellPath>> {
        self.cell_paths.take()
    }
}

#[derive(Clone)]
pub struct StrIndexOf;

impl Command for StrIndexOf {
    fn name(&self) -> &str {
        "str index-of"
    }

    fn signature(&self) -> Signature {
        Signature::build("str index-of")
            .input_output_types(vec![
                (Type::String, Type::Int),
                (Type::List(Box::new(Type::String)), Type::List(Box::new(Type::Int))),
                (Type::table(), Type::table()),
                (Type::record(), Type::record()),
            ])
            .allow_variants_without_examples(true)
            .required("string", SyntaxShape::String, "The string to find in the input.")
            .switch(
                "grapheme-clusters",
                "Count indexes using grapheme clusters (all visible chars have length 1).",
                Some('g'),
            )
            .switch(
                "utf-8-bytes",
                "Count indexes using UTF-8 bytes (default; non-ASCII chars have length 2+).",
                Some('b'),
            )
            .rest(
                "rest",
                SyntaxShape::CellPath,
                "For a data structure input, search strings at the given cell paths, and replace with result.",
            )
            .named(
                "range",
                SyntaxShape::Range,
                "Optional start and/or end index.",
                Some('r'),
            )
            .switch("end", "Search from the end of the input.", Some('e'))
            .category(Category::Strings)
    }

    fn description(&self) -> &str {
        "Returns start index of first occurrence of string in input, or -1 if no match."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["match", "find", "search"]
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
        let substring: Spanned<String> = call.req(engine_state, stack, 0)?;
        let cell_paths: Vec<CellPath> = call.rest(engine_state, stack, 1)?;
        let cell_paths = (!cell_paths.is_empty()).then_some(cell_paths);
        let args = Arguments {
            substring: substring.item,
            range: call.get_flag(engine_state, stack, "range")?,
            end: call.has_flag(engine_state, stack, "end")?,
            cell_paths,
            graphemes: grapheme_flags(engine_state, stack, call)?,
        };
        operate(action, args, input, call.head, engine_state.signals())
    }

    fn run_const(
        &self,
        working_set: &StateWorkingSet,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let substring: Spanned<String> = call.req_const(working_set, stack, 0)?;
        let cell_paths: Vec<CellPath> = call.rest_const(working_set, stack, 1)?;
        let cell_paths = (!cell_paths.is_empty()).then_some(cell_paths);
        let args = Arguments {
            substring: substring.item,
            range: call.get_flag_const(working_set, stack, "range")?,
            end: call.has_flag_const(working_set, stack, "end")?,
            cell_paths,
            graphemes: grapheme_flags_const(working_set, stack, call)?,
        };
        operate(
            action,
            args,
            input,
            call.head,
            working_set.permanent().signals(),
        )
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Returns index of string in input.",
                example: " 'my_library.rb' | str index-of '.rb'",
                result: Some(Value::test_int(10)),
            },
            Example {
                description: "Count length using grapheme clusters.",
                example: "'🇯🇵ほげ ふが ぴよ' | str index-of --grapheme-clusters 'ふが'",
                result: Some(Value::test_int(4)),
            },
            Example {
                description: "A match that falls inside a grapheme cluster is reported as not found.",
                example: "'🇯🇵' | str index-of --grapheme-clusters '🇵'",
                result: Some(Value::test_int(-1)),
            },
            Example {
                description: "Returns index of string in input within a`rhs open range`.",
                example: " '.rb.rb' | str index-of '.rb' --range 1..",
                result: Some(Value::test_int(3)),
            },
            Example {
                description: "Returns index of string in input within a lhs open range.",
                example: " '123456' | str index-of '6' --range ..4",
                result: Some(Value::test_int(-1)),
            },
            Example {
                description: "Returns index of string in input within a range.",
                example: " '123456' | str index-of '3' --range 1..4",
                result: Some(Value::test_int(2)),
            },
            Example {
                description: "Returns index of string in input.",
                example: " '/this/is/some/path/file.txt' | str index-of '/' -e",
                result: Some(Value::test_int(18)),
            },
        ]
    }
}

fn action(
    input: &Value,
    Arguments {
        substring,
        range,
        end,
        graphemes,
        ..
    }: &Arguments,
    head: Span,
) -> Value {
    match input {
        Value::String { val: s, .. } => {
            let (search_str, start_index) = if let Some(spanned_range) = range {
                let range_span = spanned_range.span;
                let range = &spanned_range.item;

                let (start, end) = range.absolute_bounds(s.len());
                let s = match end {
                    Bound::Excluded(end) => s.get(start..end),
                    Bound::Included(end) => s.get(start..=end),
                    Bound::Unbounded => s.get(start..),
                };

                let s = match s {
                    Some(s) => s,
                    None => {
                        return Value::error(
                            ShellError::OutOfBounds {
                                left_flank: start.to_string(),
                                right_flank: match range.end() {
                                    Bound::Unbounded => "".to_string(),
                                    Bound::Included(end) => format!("={end}"),
                                    Bound::Excluded(end) => format!("<{end}"),
                                },
                                span: range_span,
                            },
                            head,
                        );
                    }
                };
                (s, start)
            } else {
                (s.as_str(), 0)
            };

            // When the -e flag is present, search using rfind instead of find.s
            if let Some(result) = if *end {
                search_str.rfind(&**substring)
            } else {
                search_str.find(&**substring)
            } {
                let result = result + start_index;
                Value::int(
                    if *graphemes {
                        // Having found the substring's byte index, convert to grapheme index.
                        // grapheme_indices iterates graphemes alongside their UTF-8 byte indices, so .enumerate()
                        // is used to get the grapheme index alongside it.
                        s.grapheme_indices(true)
                            .enumerate()
                            .find(|e| e.1.0 >= result)
                            .map_or(-1, |e| e.0 as i64)
                    } else {
                        result as i64
                    },
                    head,
                )
            } else {
                Value::int(-1, head)
            }
        }
        Value::Error { .. } => input.clone(),
        _ => Value::error(
            ShellError::OnlySupportsThisInputType {
                exp_input_type: "string".into(),
                wrong_type: input.get_type().to_string(),
                dst_span: head,
                src_span: input.span(),
            },
            head,
        ),
    }
}

