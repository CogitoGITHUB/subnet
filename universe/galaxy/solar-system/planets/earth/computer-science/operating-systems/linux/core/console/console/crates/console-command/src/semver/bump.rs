use super::value::SemverValue;
use console_engine::command_prelude::*;
use console_protocol::{Parameter, shell_error::generic::GenericError};

#[derive(Clone)]
pub struct SemverBump;

impl Command for SemverBump {
    fn name(&self) -> &str {
        "semver bump"
    }

    fn signature(&self) -> Signature {
        Signature::build("semver bump")
            .input_output_types(vec![
                (Type::Custom("semver".into()), Type::Custom("semver".into())),
                (Type::String, Type::Custom("semver".into())),
            ])
            .switch(
                "ignore-errors",
                "If the input is not a valid semver version, return the original input unchanged",
                Some('i'),
            )
            .switch(
                "preserve-build-metadata",
                "Preserve the existing build metadata from the input version",
                Some('p'),
            )
            .switch(
                "loose",
                "Allow common non-strict prefixes such as v1.2.3, v.1.2.3, v:1.2.3, v-1.2.3, or v_1.2.3 when parsing string input; the prefix is preserved on the result",
                Some('l'),
            )
            .named(
                "build-metadata",
                SyntaxShape::String,
                "Additionally set the build metadata. Takes precedence over --preserve-build-metadata",
                Some('b'),
            )
            .param(Parameter::Required(
                PositionalArg::new("level", SyntaxShape::String)
                    .desc("The level to bump: major, minor, patch, alpha, beta, rc, release.")
                    .completion(Completion::new_list(&[
                        "major", "minor", "patch", "alpha", "beta", "rc", "release",
                    ])),
            ))
            .category(Category::Filters)
    }

    fn description(&self) -> &str {
        "Bump a semantic version to the next level."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["version", "increment", "major", "minor", "patch"]
    }

    fn examples(&self) -> Vec<Example<'static>> {
        vec![
            Example {
                description: "Bump major version",
                example: "'1.2.3' | into semver | semver bump major",
                result: Some(SemverValue::test_value("2.0.0")),
            },
            Example {
                description: "Bump minor version",
                example: "'1.2.3' | into semver | semver bump minor",
                result: Some(SemverValue::test_value("1.3.0")),
            },
            Example {
                description: "Bump patch version",
                example: "'1.2.3' | into semver | semver bump patch",
                result: Some(SemverValue::test_value("1.2.4")),
            },
            Example {
                description: "Bump patch version with string input",
                example: "'1.2.3' | semver bump patch",
                result: Some(SemverValue::test_value("1.2.4")),
            },
            Example {
                description: "Add alpha prerelease",
                example: "'1.2.3' | into semver | semver bump alpha",
                result: Some(SemverValue::test_value("1.2.3-alpha.1")),
            },
            Example {
                description: "Remove prerelease",
                example: "'1.2.3-alpha' | into semver | semver bump release",
                result: Some(SemverValue::test_value("1.2.3")),
            },
            Example {
                description: "Bump with preserved build metadata",
                example: "'1.2.3+build.5' | into semver | semver bump patch --preserve-build-metadata",
                result: Some(SemverValue::test_value("1.2.4+build.5")),
            },
            Example {
                // Assert via `to text` so the example locks the display prefix;
                // SemverValue equality ignores prefix by design.
                description: "Bump a loosely-prefixed version string",
                example: "'v1.2.3' | semver bump patch --loose | to text",
                result: Some(Value::test_string("v1.2.4")),
            },
            Example {
                // Assert via `to text` so the example locks the display prefix;
                // SemverValue equality ignores prefix by design.
                description: "Bump after converting with --loose (prefix is preserved)",
                example: "'v1.2.3' | into semver --loose | semver bump major | to text",
                result: Some(Value::test_string("v2.0.0")),
            },
        ]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let level: String = call.req(engine_state, stack, 0)?;
        let ignore_errors = call.has_flag(engine_state, stack, "ignore-errors")?;
        let build_metadata: Option<String> =
            call.get_flag(engine_state, stack, "build-metadata")?;
        let preserve_build_metadata =
            call.has_flag(engine_state, stack, "preserve-build-metadata")?;
        let loose = call.has_flag(engine_state, stack, "loose")?;
        let head = call.head;

        input.map(
            move |value| {
                bump_value_with_options(
                    &value,
                    &level,
                    head,
                    ignore_errors,
                    build_metadata.as_deref(),
                    preserve_build_metadata,
                    loose,
                )
                .unwrap_or_else(|err| Value::error(err, head))
            },
            engine_state.signals(),
        )
    }
}

fn bump_value_with_options(
    input: &Value,
    level: &str,
    head: Span,
    ignore_errors: bool,
    build_metadata: Option<&str>,
    preserve_build_metadata: bool,
    loose: bool,
) -> Result<Value, ShellError> {
    let semver_val = match SemverValue::try_from_value(input, loose) {
        Ok(semver) => semver,
        Err(err) => {
            if ignore_errors {
                return Ok(input.clone());
            }
            return Err(err);
        }
    };

    let original_build = semver_val.version.build.clone();

    let result = match level {
        "major" => semver_val.bump_major(),
        "minor" => semver_val.bump_minor(),
        "patch" => semver_val.bump_patch(),
        "alpha" | "beta" | "rc" => semver_val.bump_prerelease(level)?,
        "release" => semver_val.bump_release(),
        _ => {
            return Err(ShellError::Generic(
                GenericError::new(
                    "Invalid bump level",
                    format!("'{}' is not a valid bump level", level),
                    head,
                )
                .with_help("valid levels: major, minor, patch, alpha, beta, rc, release"),
            ));
        }
    };

    let result = match (build_metadata, preserve_build_metadata) {
        (Some(metadata), _) => result.set_build_metadata(metadata)?,
        (None, true) => SemverValue::with_prefix(
            semver::Version {
                build: original_build,
                ..result.version
            },
            result.prefix,
        ),
        (None, false) => result,
    };

    Ok(Value::custom(Box::new(result), head))
}


