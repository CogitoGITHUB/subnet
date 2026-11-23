use std::time::SystemTime;

use chrono::{DateTime, FixedOffset, Offset, Utc};
use console_plugin::{EngineInterface, EvaluatedCall, PluginCommand, SimplePluginCommand};
use console_protocol::{
    Category, Example, LabeledError, Record, Signature, Span, Value as NuValue, record,
};
use plist::{Date as PlistDate, Dictionary, Value as PlistValue};

use crate::FormatCmdsPlugin;

pub struct FromPlist;

impl SimplePluginCommand for FromPlist {
    type Plugin = FormatCmdsPlugin;

    fn name(&self) -> &str {
        "from plist"
    }

    fn description(&self) -> &str {
        "Convert plist to Nushell values"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            example: r#"'<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
        <key>a</key>
        <integer>3</integer>
</dict>
</plist>' | from plist"#,
            description: "Convert a table into a plist file",
            result: Some(NuValue::test_record(record!( "a" => NuValue::test_int(3)))),
        }]
    }

    fn signature(&self) -> Signature {
        Signature::build(PluginCommand::name(self)).category(Category::Formats)
    }

    fn run(
        &self,
        _plugin: &FormatCmdsPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        input: &NuValue,
    ) -> Result<NuValue, LabeledError> {
        match input {
            NuValue::String { val, .. } => {
                let plist = plist::from_bytes(val.as_bytes())
                    .map_err(|e| build_label_error(format!("{e}"), input.span()))?;
                let converted = convert_plist_value(&plist, call.head)?;
                Ok(converted)
            }
            NuValue::Binary { val, .. } => {
                let plist = plist::from_bytes(val)
                    .map_err(|e| build_label_error(format!("{e}"), input.span()))?;
                let converted = convert_plist_value(&plist, call.head)?;
                Ok(converted)
            }
            _ => Err(build_label_error(
                format!("Invalid input, must be string not: {input:?}"),
                call.head,
            )),
        }
    }
}

fn build_label_error(msg: impl Into<String>, span: Span) -> LabeledError {
    LabeledError::new("Could not load plist").with_label(msg, span)
}

fn convert_plist_value(plist_val: &PlistValue, span: Span) -> Result<NuValue, LabeledError> {
    match plist_val {
        PlistValue::String(s) => Ok(NuValue::string(s.to_owned(), span)),
        PlistValue::Boolean(b) => Ok(NuValue::bool(*b, span)),
        PlistValue::Real(r) => Ok(NuValue::float(*r, span)),
        PlistValue::Date(d) => Ok(NuValue::date(convert_date(d), span)),
        PlistValue::Integer(i) => {
            let signed = i
                .as_signed()
                .ok_or_else(|| build_label_error(format!("Cannot convert {i} to i64"), span))?;
            Ok(NuValue::int(signed, span))
        }
        PlistValue::Uid(uid) => Ok(NuValue::float(uid.get() as f64, span)),
        PlistValue::Data(data) => Ok(NuValue::binary(data.to_owned(), span)),
        PlistValue::Array(arr) => Ok(NuValue::list(convert_array(arr, span)?, span)),
        PlistValue::Dictionary(dict) => Ok(convert_dict(dict, span)?),
        _ => Ok(NuValue::nothing(span)),
    }
}

fn convert_dict(dict: &Dictionary, span: Span) -> Result<NuValue, LabeledError> {
    let cols: Vec<String> = dict.keys().cloned().collect();
    let vals: Result<Vec<NuValue>, LabeledError> = dict
        .values()
        .map(|v| convert_plist_value(v, span))
        .collect();
    Ok(NuValue::record(
        Record::from_raw_cols_vals(cols, vals?, span, span)?,
        span,
    ))
}

fn convert_array(plist_array: &[PlistValue], span: Span) -> Result<Vec<NuValue>, LabeledError> {
    plist_array
        .iter()
        .map(|v| convert_plist_value(v, span))
        .collect()
}

pub fn convert_date(plist_date: &PlistDate) -> DateTime<FixedOffset> {
    // In the docs the plist date object is listed as a utc timestamp, so this
    // conversion should be fine
    let plist_sys_time: SystemTime = plist_date.to_owned().into();
    let utc_date: DateTime<Utc> = plist_sys_time.into();
    let utc_offset = utc_date.offset().fix();
    utc_date.with_timezone(&utc_offset)
}

