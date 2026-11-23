use super::{Context, Module, ModuleConfig};

use crate::configs::elixir::ElixirConfig;
use crate::formatter::StringFormatter;

use crate::formatter::VersionFormatter;
use std::ops::Deref;
use std::sync::LazyLock;

/// Create a module with the current Elixir version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("elixir");
    let config = ElixirConfig::try_load(module.config);

    let is_elixir_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_elixir_project {
        return None;
    }

    let versions = LazyLock::new(|| get_elixir_version(context));

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
                    .map(|(_, elixir_version)| elixir_version)
                    .map(|elixir_version| {
                        VersionFormatter::format_module_version(
                            module.get_name(),
                            elixir_version,
                            config.version_format,
                        )
                    })?
                    .map(Ok),
                "otp_version" => versions
                    .deref()
                    .as_ref()
                    .map(|(otp_version, _)| otp_version.clone())
                    .map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `elixir`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_elixir_version(context: &Context) -> Option<(String, String)> {
    let output = context.exec_cmd("elixir", &["--version"])?.stdout;

    parse_elixir_version(&output)
}

fn parse_elixir_version(version: &str) -> Option<(String, String)> {
    let mut lines = version.lines();
    // split line into ["Erlang/OTP", "22", "[erts-10.5]", ...], take "22"
    let otp_version = lines.next()?.split_whitespace().nth(1)?;
    // skip empty line
    let _ = lines.next()?;
    // split line into ["Elixir", "1.10", "(compiled", ...], take "1.10"
    let elixir_version = lines.next()?.split_whitespace().nth(1)?;

    Some((otp_version.to_string(), elixir_version.to_string()))
}

