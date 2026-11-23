use std::string::ToString;

use super::{Context, Module, ModuleConfig};

use crate::configs::status::StatusConfig;
use crate::formatter::{StringFormatter, string_formatter::StringFormatterError};
use crate::segment::Segment;

type ExitCode = i32;
type SignalNumber = u32;
#[derive(PartialEq)]
enum PipeStatusStatus<'a> {
    Disabled,
    NoPipe,
    Pipe(&'a Vec<std::string::String>),
}

/// Creates a module with the status of the last command
///
/// Will display the status
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("status");
    let config = StatusConfig::try_load(module.config);

    // As we default to disabled=true, we have to check here after loading our config module,
    // before it was only checking against whatever is in the config starship.toml
    if config.disabled {
        return None;
    }

    let exit_code = context.properties.status_code.as_deref().unwrap_or("0");

    let pipestatus_status = match &context.properties.pipestatus {
        None => PipeStatusStatus::Disabled,
        Some(ps) => {
            if ps.len() > 1 {
                PipeStatusStatus::Pipe(ps)
            } else {
                PipeStatusStatus::NoPipe
            }
        }
    };

    let pipestatus_status = if config.pipestatus {
        pipestatus_status
    } else {
        PipeStatusStatus::Disabled
    };

    // Exit code is zero while success_symbol and pipestatus are all zero or disabled/missing
    if exit_code == "0"
        && config.success_symbol.is_empty()
        && (match pipestatus_status {
            PipeStatusStatus::Pipe(ps) => ps.iter().all(|s| s == "0"),
            _ => true,
        })
    {
        return None;
    }

    let segment_format = config.pipestatus_segment_format.unwrap_or(config.format);
    let segment_format_with_separator = [segment_format, config.pipestatus_separator].join("");

    // Create pipestatus segments
    let pipestatus = match pipestatus_status {
        PipeStatusStatus::Pipe(ps) => ps
            .iter()
            .enumerate()
            .filter_map(|(i, ec)| {
                let formatted = format_exit_code(
                    ec.as_str(),
                    if i == ps.len() - 1 {
                        segment_format
                    } else {
                        &segment_format_with_separator
                    },
                    None,
                    &config,
                    context,
                );
                match formatted {
                    Ok(segments) => Some(segments),
                    Err(e) => {
                        log::warn!("Error parsing format string in `status.pipestatus_segment_format`: {e:?}");
                        None
                    }
                }
            })
            .flatten()
            .collect(),
        _ => Vec::new(),
    };

    let main_format = match pipestatus_status {
        PipeStatusStatus::Pipe(_) => config.pipestatus_format,
        _ => config.format,
    };
    let parsed = format_exit_code(exit_code, main_format, Some(&pipestatus), &config, context);

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(_error) => {
            log::warn!("Error parsing format string in `status.format`");
            return None;
        }
    });
    Some(module)
}

fn format_exit_code<'a>(
    exit_code: &'a str,
    format: &'a str,
    pipestatus: Option<&Vec<Segment>>,
    config: &'a StatusConfig,
    context: &'a Context,
) -> Result<Vec<Segment>, StringFormatterError> {
    // First, parse as i64 to accept both i32 or u32, then normalize to i32.
    let exit_code_int = if let Ok(i) = exit_code.parse::<i64>() {
        i as ExitCode
    } else {
        log::warn!("Error parsing exit_code string to int");
        return Ok(Vec::new());
    };

    let hex_status = format!("0x{exit_code_int:X}");

    let common_meaning = status_common_meaning(exit_code_int);

    let raw_signal_number = if config.recognize_signal_code {
        status_to_signal(exit_code_int)
    } else {
        None
    };
    let signal_number = raw_signal_number.map(|sn| sn.to_string());
    let signal_name =
        raw_signal_number.and_then(|sn| status_signal_name(sn).or(signal_number.as_deref()));

    // If not a signal and not a common meaning, it should at least print the raw exit code number
    let maybe_exit_code_number = if common_meaning.is_none() && signal_name.is_none() {
        Some(exit_code)
    } else {
        None
    };

    StringFormatter::new(format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "symbol" => match exit_code_int {
                    0 => Some(config.success_symbol),
                    126 if config.map_symbol => Some(config.not_executable_symbol),
                    127 if config.map_symbol => Some(config.not_found_symbol),
                    130 if config.recognize_signal_code && config.map_symbol => {
                        Some(config.sigint_symbol)
                    }
                    x if (129..256).contains(&x)
                        && config.recognize_signal_code
                        && config.map_symbol =>
                    {
                        Some(config.signal_symbol)
                    }
                    _ => Some(config.symbol),
                },
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(if exit_code_int == 0 {
                    config.success_style
                } else {
                    config.failure_style
                }
                .unwrap_or(config.style))),
                _ => None,
            })
            .map(|variable| match variable {
                "status" => Some(Ok(exit_code)),
                "hex_status" => Some(Ok(hex_status.as_ref())),
                "int" => Some(Ok(exit_code)),
                "maybe_int" => Ok(maybe_exit_code_number).transpose(),
                "common_meaning" => Ok(common_meaning).transpose(),
                "signal_number" => Ok(signal_number.as_deref()).transpose(),
                "signal_name" => Ok(signal_name).transpose(),
                _ => None,
            })
            .map_variables_to_segments(|variable| match variable {
                "pipestatus" if pipestatus.is_none() => {
                    // We might enter this case if pipestatus hasn't
                    // been processed yet, which means that it has been
                    // set in format
                    log::warn!("pipestatus variable is only available in pipestatus_format");
                    None
                }
                "pipestatus" => pipestatus.cloned().map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    })
}

fn status_common_meaning(ex: ExitCode) -> Option<&'static str> {
    // Over 128 are Signal exit code
    if ex > 128 {
        return None;
    }
    match ex {
        0 => Some(""), // SUCCESS can be defined by $success_symbol if the user wishes too.
        1 => Some("ERROR"),
        2 => Some("USAGE"),

        // status codes 64-78 from libc
        64 => Some("USAGE"),
        65 => Some("DATAERR"),
        66 => Some("NOINPUT"),
        67 => Some("NOUSER"),
        68 => Some("NOHOST"),
        69 => Some("UNAVAILABLE"),
        70 => Some("SOFTWARE"),
        71 => Some("OSERR"),
        72 => Some("OSFILE"),
        73 => Some("CANTCREAT"),
        74 => Some("IOERR"),
        75 => Some("TEMPFAIL"),
        76 => Some("PROTOCOL"),
        77 => Some("NOPERM"),
        78 => Some("CONFIG"),

        126 => Some("NOPERM"),
        127 => Some("NOTFOUND"),
        _ => None,
    }
}

fn status_to_signal(ex: ExitCode) -> Option<SignalNumber> {
    if ex < 129 {
        return None;
    }
    let sn = ex - 128;
    Some(sn as u32)
}

fn status_signal_name(signal: SignalNumber) -> Option<&'static str> {
    match signal {
        1 => Some("HUP"),     // 128 + 1
        2 => Some("INT"),     // 128 + 2
        3 => Some("QUIT"),    // 128 + 3
        4 => Some("ILL"),     // 128 + 4
        5 => Some("TRAP"),    // 128 + 5
        6 => Some("IOT"),     // 128 + 6
        7 => Some("BUS"),     // 128 + 7
        8 => Some("FPE"),     // 128 + 8
        9 => Some("KILL"),    // 128 + 9
        10 => Some("USR1"),   // 128 + 10
        11 => Some("SEGV"),   // 128 + 11
        12 => Some("USR2"),   // 128 + 12
        13 => Some("PIPE"),   // 128 + 13
        14 => Some("ALRM"),   // 128 + 14
        15 => Some("TERM"),   // 128 + 15
        16 => Some("STKFLT"), // 128 + 16
        17 => Some("CHLD"),   // 128 + 17
        18 => Some("CONT"),   // 128 + 18
        19 => Some("STOP"),   // 128 + 19
        20 => Some("TSTP"),   // 128 + 20
        21 => Some("TTIN"),   // 128 + 21
        22 => Some("TTOU"),   // 128 + 22
        _ => None,
    }
}

