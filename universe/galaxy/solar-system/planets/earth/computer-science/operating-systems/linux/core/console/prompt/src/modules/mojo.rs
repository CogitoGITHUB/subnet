use super::{Context, Module, ModuleConfig};

use crate::configs::mojo::MojoConfig;
use crate::formatter::StringFormatter;

use std::sync::LazyLock;

/// Creates a module with the current Mojo version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("mojo");
    let config = MojoConfig::try_load(module.config);

    let is_mojo_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_mojo_project {
        return None;
    }

    let version_hash = LazyLock::new(|| get_mojo_version(context));

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "version" => match &*version_hash {
                    Some((version, _)) => Some(Ok(version)),
                    _ => None,
                },
                "hash" => match &*version_hash {
                    Some((_, Some(hash))) => Some(Ok(hash)),
                    _ => None,
                },
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `mojo`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_mojo_version(context: &Context) -> Option<(String, Option<String>)> {
    let mojo_version_output = context.exec_cmd("mojo", &["--version"])?.stdout;

    let version_items = mojo_version_output
        .split_ascii_whitespace()
        .collect::<Vec<&str>>();

    let (version, hash) = match version_items[..] {
        [_, version] => (version.trim().to_string(), None),
        [_, version, hash, ..] => (version.trim().to_string(), Some(hash.trim().to_string())),
        _ => {
            log::debug!("Unexpected `mojo --version` output: {mojo_version_output}");
            return None;
        }
    };

    Some((version, hash))
}

