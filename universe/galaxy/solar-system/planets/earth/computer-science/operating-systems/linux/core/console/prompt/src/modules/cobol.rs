use super::{Context, Module, ModuleConfig};

use crate::configs::cobol::CobolConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;

/// Creates a module with the current COBOL version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("cobol");
    let config = CobolConfig::try_load(module.config);
    let is_cobol_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_cobol_project {
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
                    let cobol_version =
                        get_cobol_version(&context.exec_cmd("cobc", &["-version"])?.stdout)?;

                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &cobol_version,
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
            log::warn!("Error in module `cobol`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_cobol_version(cobol_stdout: &str) -> Option<String> {
    // cobol output looks like this:
    // cobc (GnuCOBOL) 3.1.2.0
    // ...

    Some(
        cobol_stdout
            // split into ["cobc", "(GNUCOBOL)", "3.1.2.0"...]
            .split_whitespace()
            // return "3.1.2.0"
            .nth(2)?
            .to_string(),
    )
}

