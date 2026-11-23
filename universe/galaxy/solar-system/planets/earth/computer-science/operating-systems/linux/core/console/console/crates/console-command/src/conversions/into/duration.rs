use fancy_regex::Regex;
use itertools::Itertools;
use console_cmd_base::input_handler::{CmdArgument, operate};
use console_engine::command_prelude::*;
use console_parser::{DURATION_UNIT_GROUPS, parse_unit_value};
use console_protocol::{
    ParseError, SUPPORTED_DURATION_UNITS, Unit, ast::Expr, shell_error::generic::GenericError,
};
use std::str::FromStr;
use std::sync::LazyLock;

const NS_PER_MS: i64 = 1_000_000;
const NS_PER_SEC: i64 = 1_000_000_000;
const NS_PER_MINUTE: i64 = 60 * NS_PER_SEC;
const NS_PER_HOUR: i64 = 60 * NS_PER_MINUTE;
const NS_PER_DAY: i64 = 24 * NS_PER_HOUR;

const ALLOWED_COLUMNS: [&str; 9] = [
    "week",
    "day",
    "hour",
    "minute",
    "second",
    "millisecond",
    "microsecond",
    "nanosecond",
    "sign",
];

#[derive(Clone, Debug)]
struct Arguments {
    unit: Option<Spanned<Unit>>,
    cell_paths: Option<Vec<CellPath>>,
}

impl CmdArgument for Arguments {
    fn take_cell_paths(&mut self) -> Option<Vec<CellPath>> {
        self.cell_paths.take()
    }
}

#[derive(Clone)]
pub struct IntoDuration;

impl Command for IntoDuration {
    fn name(&self) -> &str {
        "into duration"
    }

    fn signature(&self) -> Signature {
        Signature::build("into duration")
            .input_output_types(vec![
                (Type::Int, Type::Duration),
                (Type::Float, Type::Duration),
                (Type::String, Type::Duration),
                (Type::Duration, Type::Duration),
                (Type::record(), Type::record()),
                (Type::record(), Type::Duration),
                (Type::table(), Type::table()),
            ])
            .allow_variants_without_examples(true)
            .param(
                Flag::new("unit")
                    .short('u')
                    .arg(SyntaxShape::String)
                    .desc(
                        "Unit to convert number into (will have an effect only with integer input)",
                    )
                    .completion(Completion::new_list(SUPPORTED_DURATION_UNITS.as_slice())),
            )
            .rest(
                "rest",
                SyntaxShape::CellPath,
                "For a data structure input, convert data at the given cell paths.",
            )
            .category(Category::Conversions)
    }

    fn description(&self) -> &str {
        "Convert value to a duration."
    }

    fn extra_description(&self) -> &str {
        "Max duration value is i64::MAX nanoseconds; max duration time unit is wk (weeks)."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["convert", "time", "period"]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let unit = call
            .get_flag::<Spanned<String>>(engine_state, stack, "unit")?
            .map(|Spanned { item, span }| match Unit::from_str(&item) {
                Err(_) | Ok(Unit::Filesize(_)) => Err(ShellError::InvalidUnit {
                    span,
                    supported_units: SUPPORTED_DURATION_UNITS.join(", "),
                }),
                Ok(u) => Ok(u.into_spanned(span)),
            })
            .transpose()?;

        let cell_paths = Some(call.rest(engine_state, stack, 0)?).filter(|x| !x.is_empty());

        let args = Arguments { unit, cell_paths };
        operate(action, args, input, call.head, engine_state.signals())
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Convert duration string to duration value.",
                example: "'7min' | into duration",
                result: Some(Value::test_duration(7 * 60 * NS_PER_SEC)),
            },
            Example {
                description: "Convert compound duration string to duration value.",
                example: "'1day 2hr 3min 4sec' | into duration",
                result: Some(Value::test_duration(
                    (((((/* 1 * */24) + 2) * 60) + 3) * 60 + 4) * NS_PER_SEC,
                )),
            },
            Example {
                description: "Convert table of duration strings to table of duration values.",
                example: "[[value]; ['1sec'] ['2min'] ['3hr'] ['4day'] ['5wk']] | into duration value",
                result: Some(Value::test_list(vec![
                    Value::test_record(record! {
                        "value" => Value::test_duration(NS_PER_SEC),
                    }),
                    Value::test_record(record! {
                        "value" => Value::test_duration(2 * 60 * NS_PER_SEC),
                    }),
                    Value::test_record(record! {
                        "value" => Value::test_duration(3 * 60 * 60 * NS_PER_SEC),
                    }),
                    Value::test_record(record! {
                        "value" => Value::test_duration(4 * 24 * 60 * 60 * NS_PER_SEC),
                    }),
                    Value::test_record(record! {
                        "value" => Value::test_duration(5 * 7 * 24 * 60 * 60 * NS_PER_SEC),
                    }),
                ])),
            },
            Example {
                description: "Convert duration to duration.",
                example: "420sec | into duration",
                result: Some(Value::test_duration(7 * 60 * NS_PER_SEC)),
            },
            Example {
                description: "Convert `hh:mm:ss`-style string to duration",
                example: "'3:34:00' | into duration",
                result: Some(Value::test_duration(3 * NS_PER_HOUR + 34 * NS_PER_MINUTE)),
            },
            Example {
                description: "Convert `hh:mm:ss.f`-style string to duration",
                example: "'2:45:31.2' | into duration",
                result: Some(Value::test_duration(
                    2 * NS_PER_HOUR + 45 * NS_PER_MINUTE + 31 * NS_PER_SEC + 200 * NS_PER_MS,
                )),
            },
            Example {
                description: "Convert a number of ns to duration.",
                example: "1_234_567 | into duration",
                result: Some(Value::test_duration(1_234_567)),
            },
            Example {
                description: "Convert a number of an arbitrary unit to duration.",
                example: "1_234 | into duration --unit ms",
                result: Some(Value::test_duration(1_234 * 1_000_000)),
            },
            Example {
                description: "Convert a floating point number of an arbitrary unit to duration.",
                example: "1.234 | into duration --unit sec",
                result: Some(Value::test_duration(1_234 * 1_000_000)),
            },
            Example {
                description: "Convert a record to a duration.",
                example: "{day: 10, hour: 2, minute: 6, second: 50, sign: '+'} | into duration",
                result: Some(Value::duration(
                    10 * NS_PER_DAY + 2 * NS_PER_HOUR + 6 * NS_PER_MINUTE + 50 * NS_PER_SEC,
                    Span::test_data(),
                )),
            },
        ]
    }
}

fn split_whitespace_indices(s: &str, span: Span) -> impl Iterator<Item = (&str, Span)> {
    s.split_whitespace().map(move |sub| {
        // Gets the offset of the `sub` substring inside the string `s`.
        // `wrapping_` operations are necessary because the pointers can
        // overflow on 32-bit platforms.  The result will not overflow, because
        // `sub` is within `s`, and the end of `s` has to be a valid memory
        // address.
        //
        // XXX: this should be replaced with `str::substr_range` from the
        // standard library when it's stabilized.
        let start_offset = span
            .start
            .wrapping_add(sub.as_ptr() as usize)
            .wrapping_sub(s.as_ptr() as usize);
        (sub, Span::new(start_offset, start_offset + sub.len()))
    })
}

type DurationPart = (Unit, i64, Span);

fn sum_durations(
    mut durations: impl Iterator<Item = DurationPart>,
    span: Span,
) -> Result<Value, ShellError> {
    durations.try_fold(
        Value::duration(0, span),
        |accum, (unit, count, sub_span)| {
            unit.build_value(count, sub_span)
                .and_then(|val| accum.add(sub_span, &val, sub_span))
        },
    )
}

fn parse_string_duration(s: &str, span: Span) -> Result<Value, ShellError> {
    if let Some(parsed) = parse_clock_duration(s.trim(), span)? {
        Ok(parsed)
    } else {
        compound_to_duration(s, span)
    }
}

pub(crate) fn compound_to_duration(s: &str, span: Span) -> Result<Value, ShellError> {
    let mut parts = split_whitespace_indices(s, span)
        .map(|(substring, substring_span)| string_to_duration(substring, substring_span))
        .peekable();

    if parts.peek().is_none() {
        Err(ShellError::IncorrectValue {
            msg: "empty duration string".to_string(),
            val_span: span,
            call_span: span,
        })
    } else {
        parts
            .process_results(|duration_parts| sum_durations(duration_parts, span))
            .and_then(|r| r)
    }
}

const CLOCKTIME_PATTERN: &str = r#"(?x)
        ^
        (?<hours>\d+)
        :                           
        (?<minutes>\d{2})
        :
        (?<seconds>\d{2})
        (
            \.
            (?<fractional>\d+)
        )?
        $
"#;

static CLOCKTIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(CLOCKTIME_PATTERN).expect("clocktime_pattern should be valid"));

// Try to parse a string formatted as `hh:mm:ss` with an optional fractional
// seconds component using 1 to 9 digits of sub-second precision.
fn parse_clock_duration(s: &str, span: Span) -> Result<Option<Value>, ShellError> {
    if !s.contains(':') {
        return Ok(None);
    }

    // helper for consistent error messaging
    fn clock_format_error(span: Span) -> ShellError {
        ShellError::IncorrectValue {
            msg: "invalid clock-style duration; please use hh:mm:ss with optional .f up to .fffffffff"
                .to_string(),
            val_span: span,
            call_span: span,
        }
    }

    fn clock_range_error(span: Span) -> ShellError {
        ShellError::IncorrectValue {
            msg: "invalid clock-style duration; hours must be >= 0 and minutes/seconds must be >= 0 and < 60"
                .to_string(),
            val_span: span,
            call_span: span,
        }
    }

    let caps = CLOCKTIME_RE
        .captures(s)
        .map_err(|_| clock_format_error(span))?
        .ok_or_else(|| clock_format_error(span))?;

    let (hours_part, minutes_part, seconds_part, fractional_part) = (
        caps.name("hours")
            .ok_or_else(|| clock_format_error(span))?
            .as_str(),
        caps.name("minutes")
            .ok_or_else(|| clock_format_error(span))?
            .as_str(),
        caps.name("seconds")
            .ok_or_else(|| clock_format_error(span))?
            .as_str(),
        caps.name("fractional").map(|s| s.as_str()),
    );

    let hours = hours_part
        .parse::<i64>()
        .map_err(|_| clock_format_error(span))?;
    let minutes = minutes_part
        .parse::<i64>()
        .map_err(|_| clock_format_error(span))?;

    let seconds = seconds_part
        .parse::<i64>()
        .map_err(|_| clock_format_error(span))?;

    let fractional_ns = match fractional_part {
        Some(fractional) if fractional.chars().all(|c| c.is_ascii_digit()) => {
            if fractional.is_empty() || fractional.len() > 9 {
                return Err(clock_format_error(span));
            }

            let scale = 10_i64.pow((9 - fractional.len()) as u32);
            fractional
                .parse::<i64>()
                .map(|value| value * scale)
                .map_err(|_| clock_format_error(span))?
        }
        Some(_) => return Err(clock_format_error(span)),
        None => 0,
    };

    if hours < 0 || minutes >= 60 || seconds >= 60 || minutes < 0 || seconds < 0 {
        return Err(clock_range_error(span));
    }

    sum_durations(
        [
            (Unit::Hour, hours, span),
            (Unit::Minute, minutes, span),
            (Unit::Second, seconds, span),
            (Unit::Nanosecond, fractional_ns, span),
        ]
        .into_iter(),
        span,
    )
    .map(Some)
}

fn string_to_duration(s: &str, span: Span) -> Result<DurationPart, ShellError> {
    match parse_unit_value(
        s.as_bytes(),
        span,
        DURATION_UNIT_GROUPS,
        Type::Duration,
        |x| x,
    ) {
        Some(Ok(expression)) => {
            if let Expr::ValueWithUnit(value) = expression.expr
                && let Expr::Int(x) = value.expr.expr
            {
                return Ok((value.unit.item, x, span));
            }
        }
        Some(Err(mk_err)) if matches!(mk_err("duration"), ParseError::LiteralTooLarge(..)) => {
            return Err(ShellError::Generic(GenericError::new(
                "duration too large",
                "duration too large",
                span,
            )));
        }
        _ => {}
    }

    Err(ShellError::InvalidUnit {
        span,
        supported_units: SUPPORTED_DURATION_UNITS.join(", "),
    })
}

fn action(input: &Value, args: &Arguments, head: Span) -> Value {
    let value_span = input.span();
    let unit_option = &args.unit;

    if let Value::Record { .. } | Value::Duration { .. } = input
        && let Some(unit) = unit_option
    {
        return Value::error(
            ShellError::IncompatibleParameters {
                left_message: "got a record as input".into(),
                left_span: head,
                right_message: "the units should be included in the record".into(),
                right_span: unit.span,
            },
            head,
        );
    }

    let unit = match unit_option {
        Some(unit) => &unit.item,
        None => &Unit::Nanosecond,
    };

    let float_to_duration = |f: f64| {
        unit.build_value(1, head)
            .and_then(|val| val.mul(head, &Value::float(f, head), head))
            .unwrap_or_else(|e| Value::error(e, head))
    };

    match input {
        Value::Duration { .. } => input.clone(),
        Value::Record { val, .. } => {
            merge_record(val, head, value_span).unwrap_or_else(|e| Value::error(e, value_span))
        }
        Value::String { val, .. } => {
            if let Ok(num) = val.parse::<f64>() {
                float_to_duration(num)
            } else {
                parse_string_duration(val, value_span)
                    .map(|val| val.with_span(head))
                    .unwrap_or_else(|e| Value::error(e, head))
            }
        }
        Value::Float { val, .. } => float_to_duration(*val),
        Value::Int { val, .. } => unit
            .build_value(*val, head)
            .unwrap_or_else(|e| Value::error(e, head)),
        // Propagate errors by explicitly matching them before the final case.
        Value::Error { .. } => input.clone(),
        other => Value::error(
            ShellError::OnlySupportsThisInputType {
                exp_input_type: "string or duration".into(),
                wrong_type: other.get_type().to_string(),
                dst_span: head,
                src_span: other.span(),
            },
            head,
        ),
    }
}

fn merge_record(record: &Record, head: Span, span: Span) -> Result<Value, ShellError> {
    if let Some(invalid_col) = record
        .columns()
        .find(|key| !ALLOWED_COLUMNS.contains(&key.as_str()))
    {
        let allowed_cols = ALLOWED_COLUMNS.join(", ");
        return Err(ShellError::UnsupportedInput {
            msg: format!(
                "Column '{invalid_col}' is not valid for a structured duration. Allowed columns are: {allowed_cols}"
            ),
            input: "value originates from here".into(),
            msg_span: head,
            input_span: span,
        });
    };

    let sign = if let Some(sign_value) = record.get("sign") {
        match sign_value {
            Value::String { val, .. } => match val.as_str() {
                "+" => 1,
                "-" => -1,
                other => {
                    return Err(ShellError::IncorrectValue {
                        msg: format!("Invalid sign '{other}'. Allowed signs are +, -"),
                        val_span: sign_value.span(),
                        call_span: head,
                    });
                }
            },
            other => {
                return Err(ShellError::OnlySupportsThisInputType {
                    exp_input_type: "string".to_string(),
                    wrong_type: other.get_type().to_string(),
                    dst_span: head,
                    src_span: other.span(),
                });
            }
        }
    } else {
        1
    };

    [
        ("week", Unit::Week),
        ("day", Unit::Day),
        ("hour", Unit::Hour),
        ("minute", Unit::Minute),
        ("second", Unit::Second),
        ("millisecond", Unit::Millisecond),
        ("microsecond", Unit::Microsecond),
        ("nanosecond", Unit::Nanosecond),
    ]
    .iter()
    .try_fold(Value::duration(0, span), |accum, (column_name, unit)| {
        if let Some(col_val) = record.get(column_name) {
            parse_number_from_record(col_val, &head)
                .map(|dur| dur * sign)
                .and_then(|dur_count| unit.build_value(dur_count, head))
                .and_then(|dur| accum.add(span, &dur, span))
        } else {
            Ok(accum)
        }
    })
}

fn parse_number_from_record(col_val: &Value, head: &Span) -> Result<i64, ShellError> {
    let value = match col_val {
        Value::Int { val, .. } => {
            if *val < 0 {
                return Err(ShellError::IncorrectValue {
                    msg: "number should be positive".to_string(),
                    val_span: col_val.span(),
                    call_span: *head,
                });
            }
            *val
        }
        other => {
            return Err(ShellError::OnlySupportsThisInputType {
                exp_input_type: "int".to_string(),
                wrong_type: other.get_type().to_string(),
                dst_span: *head,
                src_span: other.span(),
            });
        }
    };
    Ok(value)
}

