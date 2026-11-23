use super::{Context, Module, ModuleConfig};

use crate::configs::quarto::QuartoConfig;
use crate::formatter::{StringFormatter, VersionFormatter};

/// Creates a module with the current Quarto version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("quarto");
    let config = QuartoConfig::try_load(module.config);

    let is_quarto_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_quarto_project {
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
                    let version = context
                        .exec_cmd("quarto", &["--version"])?
                        .stdout
                        .trim_end()
                        .to_owned();
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &version,
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
            log::warn!("Error in module `quarto`:\n{error}");
            return None;
        }
    });

    Some(module)
}

