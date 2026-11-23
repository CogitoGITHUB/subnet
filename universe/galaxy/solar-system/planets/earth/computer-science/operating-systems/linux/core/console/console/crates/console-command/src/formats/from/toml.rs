use console_engine::command_prelude::*;
use toml::value::{Datetime, Offset};

#[derive(Clone)]
pub struct FromToml;

impl Command for FromToml {
    fn name(&self) -> &str {
        "from toml"
    }

    fn signature(&self) -> Signature {
        Signature::build("from toml")
            .input_output_types(vec![(Type::String, Type::record())])
            .category(Category::Formats)
    }

    fn description(&self) -> &str {
        "Parse text as .toml and create record."
    }

    fn run(
        &self,
        _engine_state: &EngineState,
        _stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let span = call.head;
        let (mut string_input, span, metadata) = input.collect_string_strict(span)?;
        string_input.push('\n');
        Ok(convert_string_to_value(string_input, span)?
            .into_pipeline_data_with_metadata(metadata.map(|md| md.with_content_type(None))))
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "'a = 1' | from toml",
                description: "Converts toml formatted string to record.",
                result: Some(Value::test_record(record! {
                    "a" => Value::test_int(1),
                })),
            },
            Example {
                example: "'a = 1
b = [1, 2]' | from toml",
                description: "Converts toml formatted string to record.",
                result: Some(Value::test_record(record! {
                    "a" =>  Value::test_int(1),
                    "b" =>  Value::test_list(vec![
                        Value::test_int(1),
                        Value::test_int(2)],),
                })),
            },
        ]
    }
}

pub(crate) fn convert_toml_datetime_to_value(dt: &Datetime, span: Span) -> Value {
    match &dt.clone() {
        toml::value::Datetime {
            date: Some(_),
            time: _,
            offset: _,
        } => (),
        _ => return Value::string(dt.to_string(), span),
    }

    let date = match dt.date {
        Some(date) => {
            chrono::NaiveDate::from_ymd_opt(date.year.into(), date.month.into(), date.day.into())
        }
        None => Some(chrono::NaiveDate::default()),
    };

    let time = match dt.time {
        Some(time) => chrono::NaiveTime::from_hms_nano_opt(
            time.hour.into(),
            time.minute.into(),
            time.second.unwrap_or_default().into(),
            time.nanosecond.unwrap_or_default(),
        ),
        None => Some(chrono::NaiveTime::default()),
    };

    let tz = match dt.offset {
        Some(offset) => match offset {
            Offset::Z => chrono::FixedOffset::east_opt(0),
            Offset::Custom { minutes: min } => chrono::FixedOffset::east_opt(min as i32 * 60),
        },
        None => chrono::FixedOffset::east_opt(0),
    };

    let datetime = match (date, time, tz) {
        (Some(date), Some(time), Some(tz)) => chrono::NaiveDateTime::new(date, time)
            .and_local_timezone(tz)
            .earliest(),
        _ => None,
    };

    match datetime {
        Some(datetime) => Value::date(datetime, span),
        None => Value::string(dt.to_string(), span),
    }
}

fn convert_toml_to_value(value: &toml::Value, span: Span) -> Value {
    match value {
        toml::Value::Array(array) => {
            let v: Vec<Value> = array
                .iter()
                .map(|x| convert_toml_to_value(x, span))
                .collect();

            Value::list(v, span)
        }
        toml::Value::Boolean(b) => Value::bool(*b, span),
        toml::Value::Float(f) => Value::float(*f, span),
        toml::Value::Integer(i) => Value::int(*i, span),
        toml::Value::Table(k) => Value::record(
            k.iter()
                .map(|(k, v)| (k.clone(), convert_toml_to_value(v, span)))
                .collect(),
            span,
        ),
        toml::Value::String(s) => Value::string(s.clone(), span),
        toml::Value::Datetime(dt) => convert_toml_datetime_to_value(dt, span),
    }
}

pub(crate) fn convert_string_to_value(
    string_input: String,
    span: Span,
) -> Result<Value, ShellError> {
    let result: Result<toml::Value, toml::de::Error> = toml::from_str(&string_input);
    match result {
        Ok(value) => Ok(convert_toml_to_value(&value, span)),

        Err(err) => Err(ShellError::CantConvert {
            to_type: "structured toml data".into(),
            from_type: "string".into(),
            span,
            help: Some(err.to_string()),
        }),
    }
}

