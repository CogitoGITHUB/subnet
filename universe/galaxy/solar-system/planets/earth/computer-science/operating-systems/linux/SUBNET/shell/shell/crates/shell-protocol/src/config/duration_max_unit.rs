use super::{config_update_string_enum, prelude::*};

use crate::{self as shell_protocol};

/// The largest time unit used when formatting durations for display.
///
/// When set to a smaller unit, durations that would previously show weeks
/// will instead show the equivalent number of days, hours, etc.
#[derive(
    Clone, Copy, Default, Debug, IntoValue, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum DurationMaxUnit {
    #[default]
    #[shell_value(rename = "wk")]
    Week,
    #[shell_value(rename = "day")]
    Day,
    #[shell_value(rename = "hr")]
    Hour,
    #[shell_value(rename = "min")]
    Minute,
    #[shell_value(rename = "sec")]
    Second,
    #[shell_value(rename = "ms")]
    Millisecond,
    #[shell_value(rename = "us")]
    Microsecond,
    #[shell_value(rename = "ns")]
    Nanosecond,
}

impl FromStr for DurationMaxUnit {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "wk" => Ok(Self::Week),
            "day" => Ok(Self::Day),
            "hr" => Ok(Self::Hour),
            "min" => Ok(Self::Minute),
            "sec" => Ok(Self::Second),
            "ms" => Ok(Self::Millisecond),
            "us" | "µs" => Ok(Self::Microsecond),
            "ns" => Ok(Self::Nanosecond),
            _ => Err("'wk', 'day', 'hr', 'min', 'sec', 'ms', 'us', 'µs', or 'ns'"),
        }
    }
}

impl UpdateFromValue for DurationMaxUnit {
    fn update(&mut self, value: &Value, path: &mut ConfigPath, errors: &mut ConfigErrors) {
        config_update_string_enum(self, value, path, errors)
    }
}
