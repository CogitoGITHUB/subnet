use super::{Context, Module, ModuleConfig};
use crate::formatter::VersionFormatter;

use crate::configs::rlang::RLangConfig;
use crate::formatter::StringFormatter;
use crate::utils::get_command_string_output;

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("rlang");
    let config: RLangConfig = RLangConfig::try_load(module.config);

    let is_r_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();
    if !is_r_project {
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
                    let r_version_string =
                        get_command_string_output(context.exec_cmd("R", &["--version"])?);
                    let r_version = parse_r_version(&r_version_string)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &r_version,
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
            log::warn!("Error in module `rlang`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_r_version(r_version: &str) -> Option<String> {
    r_version
        .lines()
        // take first line
        .next()?
        // split into ["R", "version", "3.6.3", "(2020-02-29)", ...]
        .split_whitespace()
        // and pick version entry at index 2, i.e. "3.6.3".
        .nth(2)
        .map(ToString::to_string)
}

