use super::{Context, Module, ModuleConfig};

use crate::configs::dart::DartConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;
use crate::utils::get_command_string_output;

/// Creates a module with the current Dart version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("dart");
    let config: DartConfig = DartConfig::try_load(module.config);

    let is_dart_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_dart_project {
        return None;
    }

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "version" => {
                    let command = context.exec_cmd("dart", &["--version"])?;
                    let dart_version = parse_dart_version(&get_command_string_output(command))?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &dart_version,
                        config.version_format,
                    )
                    .map(Ok)
                }
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `dart`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_dart_version(dart_version: &str) -> Option<String> {
    Some(
        dart_version
            // split into ["Dart", "VM", "version:", "2.8.4", "(stable)", ...]
            .split_whitespace()
            // return "2.8.4"
            .nth(3)?
            .to_string(),
    )
}

