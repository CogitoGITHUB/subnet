use crate::platform::input::legacy_input::LegacyInput;
use crate::platform::input::reedline_prompt::ReedlinePrompt;
use console_engine::command_prelude::*;
use console_protocol::shell_error::{self, io::IoError};
use reedline::{
    EditCommand, Reedline, Signal,
};

#[derive(Clone)]
pub struct Input;

impl LegacyInput for Input {}

impl Command for Input {
    fn name(&self) -> &str {
        "input"
    }

    fn description(&self) -> &str {
        "Get input from the user via the terminal."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["prompt", "interactive"]
    }

    fn signature(&self) -> Signature {
        Signature::build("input")
            .input_output_types(vec![
                (Type::Nothing, Type::Any),
                (Type::List(Box::new(Type::String)), Type::Any)])
            .allow_variants_without_examples(true)
            .optional("prompt", SyntaxShape::String, "Prompt to show the user.")
            .named(
                "bytes-until-any",
                SyntaxShape::String,
                "Read bytes (not text) until any of the given stop bytes is seen.",
                Some('u'),
            )
            .named(
                "numchar",
                SyntaxShape::Int,
                "Number of characters to read; suppresses output.",
                Some('n'),
            )
            .named(
                "default",
                SyntaxShape::String,
                "Default value if no input is provided.",
                Some('d'),
            )
            .switch(
                "reedline",
                "Use the reedline library, defaults to false.",
                None
            )
            .switch("suppress-output", "Don't print keystroke values.", Some('s'))
            .category(Category::Platform)
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        // Check if we should use the legacy implementation or the reedline implementation
        let use_reedline = [
            // reedline is not set - use legacy implementation
            call.has_flag(engine_state, stack, "reedline")?,
        ]
        .iter()
        .any(|x| *x);

        if !use_reedline {
            // `legacy_input` guards the terminal itself via `RawModeGuard`.
            return self.legacy_input(engine_state, stack, call, input);
        }

        // Reedline grabs the terminal itself, so check the precondition here.
        stack.require_stdin(call.head)?;

        let prompt_str: Option<String> = call.opt(engine_state, stack, 0)?;
        let default_val: Option<String> = call.get_flag(engine_state, stack, "default")?;
        let default_str = match (&prompt_str, &default_val) {
            (Some(_prompt), Some(val)) => format!("(default: {val}) "),
            _ => "".to_string(),
        };

        let prompt = ReedlinePrompt {
            indicator: default_str,
            left_prompt: prompt_str.unwrap_or("".to_string()),
            right_prompt: "".to_string(),
        };

        let mut line_editor = Reedline::create();
        line_editor = line_editor.with_ansi_colors(false);

        // In reedline mode, treat `--default` as the initial editable buffer contents.
        // This keeps options minimal while supporting the "prefilled but editable" UX.
        if let Some(val) = default_val.as_ref() {
            prefill_reedline_buffer(&mut line_editor, val);
        }

        let mut buf = String::new();

        match line_editor.read_line(&prompt) {
            Ok(Signal::Success(buffer) | Signal::HostCommand(buffer)) => {
                buf.push_str(&buffer);
            }
            Ok(Signal::CtrlC(_)) => {
                return Err(IoError::new(
                    shell_error::io::ErrorKind::from_std(std::io::ErrorKind::Interrupted),
                    call.head,
                    None,
                )
                .into());
            }
            Ok(Signal::CtrlD) => {
                // Do nothing on ctrl-d
                return Ok(Value::nothing(call.head).into_pipeline_data());
            }
            // TODO: handle other signals like Signal::ExternalBreak
            Ok(_) => {}
            Err(event_error) => {
                let from_io_error = IoError::factory(call.head, None);
                return Err(from_io_error(event_error).into());
            }
        }
        match default_val {
            Some(val) if buf.is_empty() => Ok(Value::string(val, call.head).into_pipeline_data()),
            _ => Ok(Value::string(buf, call.head).into_pipeline_data()),
        }
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Get input from the user, and assign to a variable.",
                example: "let user_input = (input)",
                result: None,
            },
            Example {
                description: "Get two characters from the user, and assign to a variable.",
                example: "let user_input = (input --numchar 2)",
                result: None,
            },
            Example {
                description: "Get input from the user with default value, and assign to a variable.",
                example: "let user_input = (input --default 10)",
                result: None,
            },
            Example {
                description: "Get multiple lines of input from the user (newlines can be entered using `Alt` + `Enter` or `Ctrl` + `Enter`), and assign to a variable.",
                example: "let multiline_input = (input --reedline)",
                result: None,
            },
        ]
    }
}

fn prefill_reedline_buffer(line_editor: &mut Reedline, default_val: &str) {
    if default_val.is_empty() {
        return;
    }

    // Start with a clean buffer. This also ensures idempotency if this function is ever called
    // more than once.
    line_editor.run_edit_commands(&[EditCommand::Clear]);
    line_editor.run_edit_commands(&[EditCommand::InsertString(default_val.to_string())]);
    // Keep cursor at end (insertion point is naturally advanced by InsertString).
}

