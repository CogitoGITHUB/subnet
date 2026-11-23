use std::env;

use super::{Context, Module, ModuleConfig};

use crate::configs::sudo::SudoConfig;
use crate::formatter::StringFormatter;

/// Creates a module with sudo credential cache status
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("sudo");
    let config = SudoConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    if !config.allow_windows && env::consts::FAMILY == "windows" {
        return None;
    }

    let sudo_flags = if config.use_legacy_check {
        "-nv"
    } else {
        "-Nnv"
    };

    let is_sudo_cached = context.exec_cmd("sudo", &[sudo_flags]).is_some();

    if !is_sudo_cached {
        return None;
    }

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
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `sudo`:\n{error}");
            return None;
        }
    });

    Some(module)
}

