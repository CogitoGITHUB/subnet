use jiff::{
    Timestamp, Zoned,
    civil::Time,
    tz::{Offset, TimeZone},
};

use super::{Context, Module, ModuleConfig};
use crate::config::Either;
use crate::configs::time::{TimeConfig, TimezoneWrapper};
use crate::formatter::StringFormatter;

/// Outputs the current time
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("time");
    let config: TimeConfig = TimeConfig::try_load(module.config);

    // As we default to disabled=true, we have to check here after loading our config module,
    // before it was only checking against whatever is in the config starship.toml
    if config.disabled {
        return None;
    }

    // Hide prompt if current time is not inside time_range
    let (display_start, display_end) = parse_time_range(config.time_range);
    let time_now = Zoned::now().time();
    if !is_inside_time_range(time_now, display_start, display_end) {
        return None;
    }

    let default_format = if config.use_12hr { "%r" } else { "%T" };
    let time_format = config.time_format.unwrap_or(default_format);

    log::trace!("Timer module is enabled with format string: {time_format}");

    let formatted_time_string = match &config.utc_time_offset {
        Either::First(TimezoneWrapper(tz)) => {
            // Use IANA timezone name
            let target_time = Timestamp::now().to_zoned(tz.clone());
            format_time_fixed_offset(time_format, target_time)
        }
        Either::Second("local") => {
            // Use local timezone
            format_time(time_format, Zoned::now())
        }
        Either::Second(utc_time_offset) => {
            // Use numeric offset
            create_offset_time_string(Timestamp::now(), utc_time_offset, time_format)
                .unwrap_or_else(|_| {
                    log::warn!(
                        "Invalid utc_time_offset configuration provided! Falling back to \"local\"."
                    );
                    format_time(time_format, Zoned::now())
                })
        }
    };

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "time" => Some(Ok(&formatted_time_string)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `time`: \n{error}");
            return None;
        }
    });

    Some(module)
}

fn create_offset_time_string(
    utc_time: Timestamp,
    utc_time_offset_str: &str,
    time_format: &str,
) -> Result<String, String> {
    // Using floats to allow 30/45 minute offsets: https://www.timeanddate.com/time/time-zones-interesting.html
    let utc_time_offset_in_hours = utc_time_offset_str.parse::<f32>().unwrap_or(
        // Passing out of range value to force falling back to "local"
        25_f32,
    );
    if utc_time_offset_in_hours < 24_f32 && utc_time_offset_in_hours > -24_f32 {
        let utc_offset_in_seconds: i32 = (utc_time_offset_in_hours * 3600_f32) as i32;
        let timezone_offset = Offset::from_seconds(utc_offset_in_seconds)
            .map_err(|err| format!("Invalid timezone offset: {err:?}"))?;
        let tz = TimeZone::fixed(timezone_offset);
        log::trace!("Target timezone offset is {timezone_offset}");

        let target_time = utc_time.to_zoned(tz);
        log::trace!("Time in target timezone now is {target_time}");

        Ok(format_time_fixed_offset(time_format, target_time))
    } else {
        Err("Invalid timezone offset.".to_string())
    }
}

/// Format a given time into the given string. This function should be referentially
/// transparent, which makes it easy to test (unlike anything involving the actual time)
fn format_time(time_format: &str, local_time: Zoned) -> String {
    local_time.strftime(time_format).to_string()
}

fn format_time_fixed_offset(time_format: &str, zoned_time: Zoned) -> String {
    zoned_time.strftime(time_format).to_string()
}

/// Returns true if `time_now` is between `time_start` and `time_end`.
/// If one of these values is not given, then it is ignored.
/// It also handles cases where `time_start` and `time_end` have a midnight in between
fn is_inside_time_range(time_now: Time, time_start: Option<Time>, time_end: Option<Time>) -> bool {
    match (time_start, time_end) {
        (None, None) => true,
        (Some(i), None) => time_now > i,
        (None, Some(i)) => time_now < i,
        (Some(i), Some(j)) => {
            if i < j {
                i < time_now && time_now < j
            } else {
                time_now > i || time_now < j
            }
        }
    }
}

/// Parses the config's `time_range` field and returns the starting time and ending time.
/// The range is in the format START_TIME-END_TIME, with `START_TIME` and `END_TIME` being optional.
///
/// If one of the ranges is invalid or not provided, then the corresponding field in the output
/// tuple is None
fn parse_time_range(time_range: &str) -> (Option<Time>, Option<Time>) {
    let value = String::from(time_range);

    // Check if there is exactly one hyphen, and fail otherwise
    if value.matches('-').count() != 1 {
        return (None, None);
    }

    // Split time_range into the two ranges
    let (start, end) = value.split_at(value.find('-').unwrap());
    let end = &end[1..];

    // Parse the ranges
    let start_time = start.parse::<Time>().ok();
    let end_time = end.parse::<Time>().ok();

    (start_time, end_time)
}

/* Because we cannot make acceptance tests for the time module, these unit
tests become extra important */
