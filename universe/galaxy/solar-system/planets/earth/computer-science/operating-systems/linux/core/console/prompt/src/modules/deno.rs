use super::{Context, Module, ModuleConfig};

use crate::configs::deno::DenoConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;

/// Creates a module with the current Deno version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("deno");
    let config = DenoConfig::try_load(module.config);
    let is_deno_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_deno_project {
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
                    let deno_version =
                        parse_deno_version(&context.exec_cmd("deno", &["-V"])?.stdout)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &deno_version,
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
            log::warn!("Error in module `deno`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_deno_version(deno_version: &str) -> Option<String> {
    Some(
        deno_version
            // split into ["deno", "1.8.3"]
            .split_whitespace()
            // return "1.8.3"
            .nth(1)?
            .to_string(),
    )
}

