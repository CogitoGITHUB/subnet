use super::{Context, Module, ModuleConfig};

use crate::configs::perl::PerlConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;

/// Creates a module with the current perl version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("perl");
    let config: PerlConfig = PerlConfig::try_load(module.config);
    let is_perl_project = context
        .try_begin_scan()?
        .set_extensions(&config.detect_extensions)
        .set_files(&config.detect_files)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_perl_project {
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
                    let perl_version = context
                        .exec_cmd("perl", &["-e", "printf q#%vd#,$^V;"])?
                        .stdout;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &perl_version,
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
            log::warn!("Error in module `perl`:\n{error}");
            return None;
        }
    });

    Some(module)
}

