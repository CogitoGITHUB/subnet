use super::{Context, Module, ModuleConfig};

use crate::configs::vagrant::VagrantConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;

/// Creates a module with the current Vagrant version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("vagrant");
    let config = VagrantConfig::try_load(module.config);

    let is_vagrant_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_vagrant_project {
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
                    let vagrant_version = parse_vagrant_version(
                        &context.exec_cmd("vagrant", &["--version"])?.stdout,
                    )?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &vagrant_version,
                        config.version_format,
                    )
                }
                .map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `vagrant`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_vagrant_version(vagrant_stdout: &str) -> Option<String> {
    // `vagrant --version` output looks like this:
    // Vagrant 2.2.10
    let version = vagrant_stdout
        // split into ["Vagrant", "2.2.10"]
        .split_whitespace()
        // return "2.2.10"
        .nth(1)?;

    Some(version.to_string())
}

