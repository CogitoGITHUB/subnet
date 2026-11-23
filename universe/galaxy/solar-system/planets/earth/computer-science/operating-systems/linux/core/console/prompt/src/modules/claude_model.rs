use super::{Context, Module, ModuleConfig};
use crate::configs::claude_model::ClaudeModelConfig;
use crate::formatter::StringFormatter;

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("claude_model");
    let config = ClaudeModelConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    // Read Claude Code data from Context
    let claude_data = context.claude_code_data.as_ref()?;

    // Check if there's an alias for this model
    let model_display = config
        .model_aliases
        .get(&claude_data.model.id)
        .or_else(|| {
            config
                .model_aliases
                .get(claude_data.model.display_name.as_str())
        })
        .copied()
        .unwrap_or(&claude_data.model.display_name);

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
                "model" => Some(Ok(model_display)),
                "model_id" => Some(Ok(claude_data.model.id.as_str())),
                "effort" => claude_data.effort.as_ref().map(|e| Ok(e.level.as_str())),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `claude_model`: {error}");
            return None;
        }
    });

    Some(module)
}

