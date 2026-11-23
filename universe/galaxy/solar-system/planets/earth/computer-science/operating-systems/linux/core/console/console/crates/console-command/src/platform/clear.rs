use crossterm::{
    QueueableCommand,
    cursor::MoveTo,
    terminal::{Clear as ClearCommand, ClearType},
};
use console_engine::command_prelude::*;
use console_protocol::shell_error::io::IoError;

use std::io::Write;

#[derive(Clone)]
pub struct Clear;

impl Command for Clear {
    fn name(&self) -> &str {
        "clear"
    }

    fn description(&self) -> &str {
        "Clear the terminal screen."
    }

    fn extra_description(&self) -> &str {
        "By default clears the current screen and the off-screen scrollback buffer."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["cls"]
    }

    fn signature(&self) -> Signature {
        Signature::build("clear")
            .category(Category::Platform)
            .input_output_types(vec![(Type::Nothing, Type::Nothing)])
            .switch(
                "keep-scrollback",
                "Do not clear the scrollback history.",
                Some('k'),
            )
    }

    #[allow(unreachable_code, unused_variables)]
    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        _input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let from_io_error = IoError::factory(call.head, None);
        let keep = call.has_flag(engine_state, stack, "keep-scrollback")?;
        #[cfg(target_os = "linux")]
        {
            let mut cfg = super::clear_anim::config::Config::default();
            if let nu_ansi_term::Color::Rgb(r, g, b) = crate::roles::text() {
                cfg.color = super::clear_anim::config::Rgb(r, g, b);
            }
            cfg.clear_scrollback = !keep;
            super::clear_anim::run(&cfg, None);
            return Ok(PipelineData::empty());
        }
        match call.has_flag(engine_state, stack, "keep-scrollback")? {
            true => {
                std::io::stdout()
                    .queue(MoveTo(0, 0))
                    .map_err(&from_io_error)?
                    .queue(ClearCommand(ClearType::All))
                    .map_err(&from_io_error)?
                    .flush()
                    .map_err(&from_io_error)?;
            }
            _ => {
                std::io::stdout()
                    .queue(MoveTo(0, 0))
                    .map_err(&from_io_error)?
                    .queue(ClearCommand(ClearType::All))
                    .map_err(&from_io_error)?
                    .queue(ClearCommand(ClearType::Purge))
                    .map_err(&from_io_error)?
                    .flush()
                    .map_err(&from_io_error)?;
            }
        };

        Ok(PipelineData::empty())
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Clear the terminal.",
                example: "clear",
                result: None,
            },
            Example {
                description: "Clear the terminal but not its scrollback history.",
                example: "clear --keep-scrollback",
                result: None,
            },
        ]
    }
}
