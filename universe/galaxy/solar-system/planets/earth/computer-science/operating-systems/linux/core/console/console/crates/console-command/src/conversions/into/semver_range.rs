use crate::semver::parse;
use crate::semver::range::SemverRangeValue;
use console_engine::command_prelude::*;
use console_protocol::shell_error::generic::GenericError;

#[derive(Clone)]
pub struct IntoSemverRange;

impl Command for IntoSemverRange {
    fn name(&self) -> &str {
        "into semver-range"
    }

    fn signature(&self) -> Signature {
        Signature::build("into semver-range")
            .input_output_types(vec![
                (Type::String, Type::Custom("semver-range".into())),
                (
                    Type::Custom("semver-range".into()),
                    Type::Custom("semver-range".into()),
                ),
            ])
            .switch(
                "loose",
                "Allow common non-strict version prefixes such as v1.2.3, v.1.2.3, v:1.2.3, v-1.2.3, or v_1.2.3 inside the range",
                Some('l'),
            )
            .category(Category::Conversions)
    }

    fn description(&self) -> &str {
        "Convert a string to a semver range."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["version", "requirement", "semantic"]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let head = call.head;
        let loose = call.has_flag(engine_state, stack, "loose")?;

        input.map(
            move |value| into_semver_range(&value, head, loose),
            engine_state.signals(),
        )
    }

    fn examples(&self) -> Vec<Example<'static>> {
        vec![
            Example {
                description: "Convert a string to a semver range",
                example: "'>=1.0.0' | into semver-range",
                result: None,
            },
            Example {
                description: "Convert a complex range",
                example: "'^1.2.3' | into semver-range",
                result: None,
            },
            Example {
                description: "Parse a range with a leading v on the version",
                example: "'>=v1.0.0' | into semver-range --loose",
                result: None,
            },
        ]
    }
}

fn into_semver_range(input: &Value, head: Span, loose: bool) -> Value {
    match input {
        Value::Custom { val, .. } if val.type_name() == "semver-range" => input.clone(),
        Value::String { val, .. } => match parse::parse_version_req(val, loose) {
            Ok(requirement) => Value::custom(Box::new(SemverRangeValue::new(requirement)), head),
            Err(_) => {
                let help = if loose {
                    "expected format: >=1.0.0, ^1.2.3, ~1.2, etc.; version numbers may use a v/V, v., v:, v-, or v_ prefix"
                } else {
                    "expected format: >=1.0.0, ^1.2.3, ~1.2, etc.; use --loose for prefixes like v1.2.3"
                };
                Value::error(
                    ShellError::Generic(
                        GenericError::new(
                            format!("Cannot convert \"{val}\" to a semver range"),
                            "the given string is not a valid semver requirement",
                            head,
                        )
                        .with_help(help),
                    ),
                    head,
                )
            }
        },
        _ => Value::error(
            ShellError::Generic(GenericError::new(
                format!("Cannot convert {} to semver range", input.get_type()),
                "expected a string or semver-range value",
                head,
            )),
            head,
        ),
    }
}


