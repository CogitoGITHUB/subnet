use super::{Context, Module, ModuleConfig};

use crate::configs::red::RedConfig;
use crate::formatter::{StringFormatter, VersionFormatter};

/// Creates a module with the current  Red version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("red");
    let config = RedConfig::try_load(module.config);
    let is_red_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_red_project {
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
                "version" => context
                    .exec_cmd("red", &["--version"])
                    .map(|output| {
                        VersionFormatter::format_module_version(
                            module.get_name(),
                            output.stdout.trim(),
                            config.version_format,
                        )
                    })?
                    .map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `red`:\n{error}");
            return None;
        }
    });

    Some(module)
}

