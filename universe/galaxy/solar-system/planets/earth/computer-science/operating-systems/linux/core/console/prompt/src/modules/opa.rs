/// Creates a module with the current Open Policy Agent version
use super::{Context, Module, ModuleConfig};

use crate::configs::opa::OpaConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;
use crate::utils::get_command_string_output;

/// Creates a module with the current Open Policy Agent version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("opa");
    let config = OpaConfig::try_load(module.config);

    let is_opa_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_opa_project {
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
                    let opa_version = get_opa_version(context)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &opa_version,
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
            log::warn!("Error in module `opa`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_opa_version(context: &Context) -> Option<String> {
    let version_output: String = context
        .exec_cmd("opa", &["version"])
        .map(get_command_string_output)?;
    parse_opa_version(&version_output)
}

fn parse_opa_version(version_output: &str) -> Option<String> {
    Some(version_output.split_whitespace().nth(1)?.to_string())
}

