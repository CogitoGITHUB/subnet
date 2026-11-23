use super::{Context, Module, ModuleConfig};

use crate::configs::typst::TypstConfig;
use crate::formatter::{StringFormatter, VersionFormatter};

/// Creates a module with the current Typst version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("typst");
    let config = TypstConfig::try_load(module.config);

    let is_typst_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_typst_project {
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
                    let version = get_typst_config(context)?;
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
            log::warn!("Error in module `typst`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_typst_config(context: &Context) -> Option<String> {
    context
        .exec_cmd("typst", &["--version"])?
        .stdout
        .trim()
        .strip_prefix("typst ")
        .and_then(|version| version.split_whitespace().next().map(ToOwned::to_owned))
}

