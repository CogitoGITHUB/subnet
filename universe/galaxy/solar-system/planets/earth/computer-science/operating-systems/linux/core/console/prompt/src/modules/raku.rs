use super::{Context, Module, ModuleConfig};

use crate::configs::raku::RakuConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;
use std::ops::Deref;
use std::sync::LazyLock;

/// Creates a module with the current raku version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("raku");
    let config: RakuConfig = RakuConfig::try_load(module.config);
    let is_raku_project = context
        .try_begin_scan()?
        .set_extensions(&config.detect_extensions)
        .set_files(&config.detect_files)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_raku_project {
        return None;
    }

    let versions = LazyLock::new(|| get_raku_version(context));

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
                "version" => versions
                    .deref()
                    .as_ref()
                    .map(|(raku_version, _)| raku_version)
                    .map(|raku_version| {
                        VersionFormatter::format_module_version(
                            module.get_name(),
                            raku_version,
                            config.version_format,
                        )
                    })?
                    .map(Ok),
                "vm_version" => versions
                    .deref()
                    .as_ref()
                    .map(|(_, vm_version)| vm_version.clone())
                    .map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `raku`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_raku_version(context: &Context) -> Option<(String, String)> {
    let output = context.exec_cmd("raku", &["--version"])?.stdout;

    parse_raku_version(&output)
}

fn parse_raku_version(version: &str) -> Option<(String, String)> {
    let mut lines = version.lines();
    // skip 1st line
    let _ = lines.next()?;
    // split 2nd line into ["Implement", "the", "Raku®", ..., "v6.d."], take "v6.d."
    // get rid of the trailing "."
    let raku_version = lines
        .next()?
        .split_whitespace()
        .nth(5)?
        .strip_suffix('.')?
        .to_string();

    // split line into ["Built", "on", "MoarVM", ...], take "MoarVM"
    // and change MoarVM to Moar (community's preference), leave other VMs as they are
    let vm_version = lines
        .next()?
        .split_whitespace()
        .nth(2)?
        .replace("MoarVM", "Moar");

    Some((raku_version.to_lowercase(), vm_version.to_lowercase()))
}

