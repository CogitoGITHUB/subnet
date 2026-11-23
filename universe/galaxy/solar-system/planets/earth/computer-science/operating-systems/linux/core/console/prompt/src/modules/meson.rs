use super::{Context, Module, ModuleConfig};

use super::utils::truncate::truncate_text;
use crate::configs::meson::MesonConfig;
use crate::formatter::StringFormatter;

/// Creates a module with the current Meson dev environment
///
/// Will display the Meson environment if `$MESON_DEVENV` and `MESON_PROJECT_NAME` are set.
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let meson_env = context.get_env("MESON_DEVENV")?;
    let project_env = context.get_env("MESON_PROJECT_NAME")?;
    if meson_env != "1" || project_env.trim().is_empty() {
        return None;
    }

    let mut module = context.new_module("meson");
    let config: MesonConfig = MesonConfig::try_load(module.config);

    let truncated_text = truncate_text(
        &project_env,
        config.truncation_length as usize,
        config.truncation_symbol,
    );

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
                "project" => Some(Ok(&truncated_text)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `meson`:\n{error}");
            return None;
        }
    });

    Some(module)
}

