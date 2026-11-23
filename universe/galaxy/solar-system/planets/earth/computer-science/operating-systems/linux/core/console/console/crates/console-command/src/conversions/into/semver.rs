use crate::semver::value::SemverValue;
use console_cmd_base::input_handler::{CmdArgument, operate};
use console_engine::command_prelude::*;
use console_protocol::shell_error::generic::GenericError;

#[derive(Clone)]
pub struct IntoSemver;

impl Command for IntoSemver {
    fn name(&self) -> &str {
        "into semver"
    }

    fn signature(&self) -> Signature {
        Signature::build("into semver")
            .input_output_types(vec![
                (Type::String, Type::custom("semver")),
                (Type::custom("semver"), Type::custom("semver")),
                (Type::record(), Type::custom("semver")),
                (Type::record(), Type::record()),
                (Type::table(), Type::table()),
                (Type::list(Type::String), Type::list(Type::custom("semver"))),
                (
                    Type::list(Type::custom("semver")),
                    Type::list(Type::custom("semver")),
                ),
                (Type::table(), Type::list(Type::custom("semver"))),
                // Relaxed case to support heterogeneous lists
                (Type::Any, Type::custom("semver")),
            ])
            .switch(
                "loose",
                "Allow common non-strict prefixes such as v1.2.3, v.1.2.3, v:1.2.3, v-1.2.3, or v_1.2.3",
                Some('l'),
            )
            .rest(
                "rest",
                SyntaxShape::CellPath,
                "For a data structure input, convert data at the given cell paths.",
            )
            .category(Category::Conversions)
    }

    fn description(&self) -> &str {
        "Convert a value (string, record, or semver) to a semantic version."
    }

    fn extra_description(&self) -> &str {
        "From a record: major/minor/patch are required; pre, build, and prefix are optional. \
         prefix is display-only metadata (e.g. \"v\"); re-parsing from text with --loose only \
         accepts recognized loose prefixes (v/V with optional . : - _)."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["version", "convert", "semantic"]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let head = call.head;
        let cell_paths = call.rest(engine_state, stack, 0)?;
        let loose = call.has_flag(engine_state, stack, "loose")?;
        let args = Arguments {
            cell_paths: (!cell_paths.is_empty()).then_some(cell_paths),
            loose,
        };
        operate(into_semver, args, input, head, engine_state.signals())
    }

    fn examples(&self) -> Vec<Example<'static>> {
        vec![
            Example {
                description: "Convert a string to a semver value",
                example: "'1.2.3' | into semver",
                result: None,
            },
            Example {
                description: "Convert a string with prerelease",
                example: "'1.2.3-alpha.1+build.2' | into semver",
                result: None,
            },
            Example {
                description: "Convert a record to a semver value",
                example: "{major: 1, minor: 2, patch: 3} | into semver",
                result: None,
            },
            Example {
                description: "Parse a version with a common leading v prefix",
                example: "'v1.2.3' | into semver --loose",
                result: None,
            },
            Example {
                description: "Parse versions like v.1.2.3, v:1.2.3, v-1.2.3, or v_1.2.3",
                example: "['v.1.2.3' 'v:2.0.0' 'v-3.0.0' 'v_4.0.0'] | into semver --loose",
                result: None,
            },
        ]
    }
}

struct Arguments {
    cell_paths: Option<Vec<CellPath>>,
    loose: bool,
}

impl CmdArgument for Arguments {
    fn take_cell_paths(&mut self) -> Option<Vec<CellPath>> {
        self.cell_paths.take()
    }
}

fn into_semver(input: &Value, args: &Arguments, head: Span) -> Value {
    match input {
        Value::Custom { val, .. } if val.type_name() == "semver" => input.clone(),
        Value::String { val, .. } => match SemverValue::parse(val, args.loose) {
            Ok(version) => Value::custom(Box::new(version), head),
            Err(_) => {
                let help = if args.loose {
                    "expected format: major.minor.patch (e.g. 1.2.3), optionally with a v/V, v., v:, v-, or v_ prefix"
                } else {
                    "expected format: major.minor.patch (e.g. 1.2.3); use --loose for prefixes like v1.2.3"
                };
                Value::error(
                    ShellError::Generic(
                        GenericError::new(
                            format!("Cannot convert \"{val}\" to a semver"),
                            "the given string is not a valid semver version",
                            head,
                        )
                        .with_help(help),
                    ),
                    head,
                )
            }
        },
        Value::Record { val, .. } => parse_record_to_semver(val, head),
        _ => Value::error(
            ShellError::Generic(GenericError::new(
                format!("Cannot convert {} to semver", input.get_type()),
                "expected a string, record, or semver value",
                head,
            )),
            head,
        ),
    }
}

fn parse_record_to_semver(record: &console_protocol::Record, head: Span) -> Value {
    let major = record.get("major").and_then(|v| v.as_int().ok());
    let minor = record.get("minor").and_then(|v| v.as_int().ok());
    let patch = record.get("patch").and_then(|v| v.as_int().ok());

    let major = match major {
        Some(v) if v >= 0 => v as u64,
        _ => {
            return Value::error(
                ShellError::Generic(
                    GenericError::new(
                        "Cannot convert record to semver",
                        "missing or invalid 'major' field",
                        head,
                    )
                    .with_help("expected a non-negative integer"),
                ),
                head,
            );
        }
    };

    let minor = match minor {
        Some(v) if v >= 0 => v as u64,
        _ => {
            return Value::error(
                ShellError::Generic(
                    GenericError::new(
                        "Cannot convert record to semver",
                        "missing or invalid 'minor' field",
                        head,
                    )
                    .with_help("expected a non-negative integer"),
                ),
                head,
            );
        }
    };

    let patch = match patch {
        Some(v) if v >= 0 => v as u64,
        _ => {
            return Value::error(
                ShellError::Generic(
                    GenericError::new(
                        "Cannot convert record to semver",
                        "missing or invalid 'patch' field",
                        head,
                    )
                    .with_help("expected a non-negative integer"),
                ),
                head,
            );
        }
    };

    let pre = record
        .get("pre")
        .and_then(|v| v.as_str().ok())
        .unwrap_or("");

    let build = record
        .get("build")
        .and_then(|v| v.as_str().ok())
        .unwrap_or("");

    // Display-only; any string is accepted so `into record` → `into semver` round-trips.
    // Text re-parse via `into semver --loose` only recognizes known loose prefixes.
    let prefix = record
        .get("prefix")
        .and_then(|v| v.as_str().ok())
        .unwrap_or("")
        .to_string();

    let pre = match semver::Prerelease::new(pre) {
        Ok(p) => p,
        Err(e) => {
            return Value::error(
                ShellError::Generic(GenericError::new(
                    "Cannot convert record to semver",
                    format!("invalid prerelease: {e}"),
                    head,
                )),
                head,
            );
        }
    };

    let build = match semver::BuildMetadata::new(build) {
        Ok(b) => b,
        Err(e) => {
            return Value::error(
                ShellError::Generic(GenericError::new(
                    "Cannot convert record to semver",
                    format!("invalid build metadata: {e}"),
                    head,
                )),
                head,
            );
        }
    };

    let version = semver::Version {
        major,
        minor,
        patch,
        pre,
        build,
    };

    Value::custom(Box::new(SemverValue::with_prefix(version, prefix)), head)
}


