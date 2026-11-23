use super::{Context, Module, ModuleConfig};

use crate::configs::mise::MiseConfig;
use crate::formatter::StringFormatter;

/// Creates a module with the current mise config
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("mise");
    let config = MiseConfig::try_load(module.config);

    let mise_applies = !config.disabled
        && context
            .try_begin_scan()?
            .set_extensions(&config.detect_extensions)
            .set_files(&config.detect_files)
            .set_folders(&config.detect_folders)
            .is_match();

    if !mise_applies {
        return None;
    }

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "symbol" => Some(Ok(config.symbol)),
                "health" => match context.exec_cmd("mise", &["doctor"]) {
                    Some(_) => Some(Ok(config.healthy_symbol)),
                    None => Some(Ok(config.unhealthy_symbol)),
                },
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(e) => {
            log::warn!("{e}");

            return None;
        }
    });

    Some(module)
}

