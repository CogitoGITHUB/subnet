use super::{Context, Module};

use crate::config::ModuleConfig;
use crate::configs::shlvl::ShLvlConfig;
use crate::formatter::StringFormatter;
use std::borrow::Cow;
use std::convert::TryInto;

const SHLVL_ENV_VAR: &str = "SHLVL";

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let props = &context.properties;
    let shlvl = props
        .shlvl
        .or_else(|| context.get_env(SHLVL_ENV_VAR)?.parse::<i64>().ok())?;

    let mut module = context.new_module("shlvl");
    let config: ShLvlConfig = ShLvlConfig::try_load(module.config);

    // As we default to disabled=true, we have to check here after loading our config module,
    // before it was only checking against whatever is in the config starship.toml
    if config.disabled || shlvl < config.threshold {
        return None;
    }

    let shlvl_str = &shlvl.to_string();

    let mut repeat_count: usize = if config.repeat {
        shlvl.try_into().unwrap_or(1)
    } else {
        1
    };

    if config.repeat_offset > 0 {
        repeat_count =
            repeat_count.saturating_sub(config.repeat_offset.try_into().unwrap_or(usize::MAX));
        if repeat_count == 0 {
            return None;
        }
    }

    let symbol = if repeat_count == 1 {
        Cow::Borrowed(config.symbol)
    } else {
        Cow::Owned(config.symbol.repeat(repeat_count))
    };

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "symbol" => Some(symbol.as_ref()),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "shlvl" => Some(Ok(shlvl_str)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `shlvl`:\n{error}");
            return None;
        }
    });

    Some(module)
}

