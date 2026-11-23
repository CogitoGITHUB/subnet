use super::{Context, Module, ModuleConfig};
use serde_json as json;

use crate::configs::nats::NatsConfig;
use crate::formatter::StringFormatter;

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("nats");
    let config = NatsConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    let ctx_str = context
        .exec_cmd("nats", &["context", "info", "--json"])?
        .stdout;
    let nats_context: json::Value = json::from_str(&ctx_str)
        .map_err(|e| {
            log::warn!("Error parsing nats context JSON: {e}\n");
            drop(e);
        })
        .ok()?;

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
                "name" => Some(Ok(nats_context.get("name")?.as_str()?)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `nats`:\n{error}");
            return None;
        }
    });

    Some(module)
}

