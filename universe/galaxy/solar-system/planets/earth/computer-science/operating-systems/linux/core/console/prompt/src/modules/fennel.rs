use super::{Context, Module, ModuleConfig};

use crate::configs::fennel::FennelConfig;
use crate::formatter::{StringFormatter, VersionFormatter};
use crate::utils::get_command_string_output;

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("fennel");
    let config = FennelConfig::try_load(module.config);

    let is_fnl_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_fnl_project {
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
                    let fennel_version_string =
                        get_command_string_output(context.exec_cmd("fennel", &["--version"])?);
                    let fennel_version = parse_fennel_version(&fennel_version_string)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &fennel_version,
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
            log::warn!("Error in module `fennel`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_fennel_version(fennel_version: &str) -> Option<String> {
    // fennel -v output looks like this:
    // Fennel 1.2.1 on PUC Lua 5.4
    let version = fennel_version
        // split into ["Fennel", "1.2.1", "on", ...]
        .split_whitespace()
        // take "1.2.1"
        .nth(1)?;

    Some(version.to_string())
}

