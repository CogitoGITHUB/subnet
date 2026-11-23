use crate::formats::{preserve_toml_document, read_toml_source_from_metadata};
use chrono::{DateTime, Datelike, FixedOffset, Timelike};
use console_engine::command_prelude::*;
use console_protocol::{PipelineMetadata, ast::PathMember};

#[derive(Clone)]
pub struct ToToml;

impl Command for ToToml {
    fn name(&self) -> &str {
        "to toml"
    }

    fn signature(&self) -> Signature {
        Signature::build("to toml")
            .input_output_types(vec![(Type::record(), Type::String)])
            .switch(
                "serialize",
                "Serialize shell types that cannot be deserialized.",
                Some('s'),
            )
            .category(Category::Formats)
    }

    fn description(&self) -> &str {
        "Convert record into .toml text."
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            description: "Outputs an TOML string representing the contents of this record.",
            example: "{foo: 1 bar: 'qwe'} | to toml",
            result: Some(Value::test_string("foo = 1\nbar = \"qwe\"\n")),
        }]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let head = call.head;
        let serialize_types = call.has_flag(engine_state, stack, "serialize")?;

        to_toml(engine_state, input, head, serialize_types)
    }
}

// Helper method to recursively convert console_protocol::Value -> toml::Value
// This shouldn't be called at the top-level
pub(crate) fn shell_value_to_toml_value(
    engine_state: &EngineState,
    v: &Value,
    serialize_types: bool,
) -> Result<toml::Value, ShellError> {
    Ok(match &v {
        Value::Bool { val, .. } => toml::Value::Boolean(*val),
        Value::Int { val, .. } => toml::Value::Integer(*val),
        Value::Filesize { val, .. } => toml::Value::Integer(val.get()),
        Value::Duration { val, .. } => toml::Value::String(val.to_string()),
        Value::Date { val, .. } => toml::Value::Datetime(to_toml_datetime(val)),
        Value::Range { .. } => toml::Value::String("<Range>".to_string()),
        Value::Float { val, .. } => toml::Value::Float(*val),
        Value::String { val, .. } | Value::Glob { val, .. } => toml::Value::String(val.clone()),
        Value::Record { val, .. } => {
            let mut m = toml::map::Map::new();
            for (k, v) in &**val {
                m.insert(
                    k.clone(),
                    shell_value_to_toml_value(engine_state, v, serialize_types)?,
                );
            }
            toml::Value::Table(m)
        }
        Value::List { vals, .. } => {
            toml::Value::Array(toml_list(engine_state, vals, serialize_types)?)
        }
        Value::Closure { val, .. } => {
            if serialize_types {
                let block = engine_state.try_get_block(val.block_id);
                if let Some(span) = block.and_then(|block| block.span) {
                    let contents_bytes = engine_state.get_span_contents(span);
                    let contents_string = String::from_utf8_lossy(contents_bytes);
                    toml::Value::String(contents_string.to_string())
                } else {
                    toml::Value::String(format!(
                        "unable to retrieve block contents for toml block_id {}",
                        val.block_id.get()
                    ))
                }
            } else {
                toml::Value::String(format!("closure_{}", val.block_id.get()))
            }
        }
        Value::Nothing { .. } => toml::Value::String("<Nothing>".to_string()),
        Value::Error { error, .. } => return Err(*error.clone()),
        Value::Binary { val, .. } => toml::Value::Array(
            val.iter()
                .map(|x| toml::Value::Integer(*x as i64))
                .collect(),
        ),
        Value::CellPath { val, .. } => toml::Value::Array(
            val.members
                .iter()
                .map(|x| match &x {
                    PathMember::String { val, .. } => Ok(toml::Value::String(val.clone())),
                    PathMember::Int { val, .. } => Ok(toml::Value::Integer(*val as i64)),
                })
                .collect::<Result<Vec<toml::Value>, ShellError>>()?,
        ),
        Value::Custom { .. } => toml::Value::String("<Custom Value>".to_string()),
    })
}

fn toml_list(
    engine_state: &EngineState,
    input: &[Value],
    serialize_types: bool,
) -> Result<Vec<toml::Value>, ShellError> {
    let mut out = vec![];

    for value in input {
        out.push(shell_value_to_toml_value(
            engine_state,
            value,
            serialize_types,
        )?);
    }

    Ok(out)
}

fn toml_into_pipeline_data(
    toml_value: &toml::Value,
    value_type: Type,
    span: Span,
    metadata: Option<PipelineMetadata>,
) -> Result<PipelineData, ShellError> {
    let new_md = Some(
        metadata
            .unwrap_or_default()
            .with_content_type(Some("text/x-toml".into())),
    );

    match toml::to_string_pretty(&toml_value) {
        Ok(serde_toml_string) => {
            Ok(Value::string(serde_toml_string, span).into_pipeline_data_with_metadata(new_md))
        }
        _ => Ok(Value::error(
            ShellError::CantConvert {
                to_type: "TOML".into(),
                from_type: value_type.to_string(),
                span,
                help: None,
            },
            span,
        )
        .into_pipeline_data_with_metadata(new_md)),
    }
}

fn value_to_toml_value(
    engine_state: &EngineState,
    v: &Value,
    head: Span,
    serialize_types: bool,
) -> Result<toml::Value, ShellError> {
    match v {
        Value::Record { .. } | Value::Closure { .. } => {
            shell_value_to_toml_value(engine_state, v, serialize_types)
        }
        // Propagate existing errors
        Value::Error { error, .. } => Err(*error.clone()),
        _ => Err(ShellError::UnsupportedInput {
            msg: format!("{:?} is not valid top-level TOML", v.get_type()),
            input: "value originates from here".into(),
            msg_span: head,
            input_span: v.span(),
        }),
    }
}

fn to_toml(
    engine_state: &EngineState,
    mut input: PipelineData,
    span: Span,
    serialize_types: bool,
) -> Result<PipelineData, ShellError> {
    let metadata = input.take_metadata();
    let value = input.into_value(span)?;

    if !serialize_types
        && let Some(original_source) = read_toml_source_from_metadata(metadata.as_ref())
        && let Value::Record { .. } = &value
        && let Ok(preserved) = preserve_toml_document(engine_state, &value, &original_source, span)
    {
        let new_md = Some(
            metadata
                .unwrap_or_default()
                .with_content_type(Some("text/x-toml".into())),
        );
        return Ok(Value::string(preserved, span).into_pipeline_data_with_metadata(new_md));
    }

    let toml_value = value_to_toml_value(engine_state, &value, span, serialize_types)?;
    match toml_value {
        toml::Value::Array(ref vec) => match vec[..] {
            [toml::Value::Table(_)] => toml_into_pipeline_data(
                vec.iter().next().expect("this should never trigger"),
                value.get_type(),
                span,
                metadata,
            ),
            _ => toml_into_pipeline_data(&toml_value, value.get_type(), span, metadata),
        },
        _ => toml_into_pipeline_data(&toml_value, value.get_type(), span, metadata),
    }
}

/// Convert chrono datetime into a toml::Value datetime.  The latter uses its
/// own ad-hoc datetime types, which makes this somewhat convoluted.
fn to_toml_datetime(datetime: &DateTime<FixedOffset>) -> toml::value::Datetime {
    let date = toml::value::Date {
        // TODO: figure out what to do with BC dates, because the toml
        // crate doesn't support them.  Same for large years, which
        // don't fit in u16.
        year: datetime.year_ce().1 as u16,
        // Panic: this is safe, because chrono guarantees that the month
        // value will be between 1 and 12 and the day will be between 1
        // and 31
        month: datetime.month() as u8,
        day: datetime.day() as u8,
    };

    let time = toml::value::Time {
        // Panic: same as before, chorono guarantees that all of the following 3
        // methods return values less than 65'000
        hour: datetime.hour() as u8,
        minute: datetime.minute() as u8,
        second: Some(datetime.second() as u8),
        nanosecond: Some(datetime.nanosecond()),
    };

    let offset = toml::value::Offset::Custom {
        // Panic: minute timezone offset fits into i16 (that's more than
        // 1000 hours)
        minutes: (-datetime.timezone().utc_minus_local() / 60) as i16,
    };

    toml::value::Datetime {
        date: Some(date),
        time: Some(time),
        offset: Some(offset),
    }
}

