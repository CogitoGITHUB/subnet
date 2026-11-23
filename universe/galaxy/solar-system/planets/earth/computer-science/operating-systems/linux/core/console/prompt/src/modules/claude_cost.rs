use super::{Context, Module, ModuleConfig};
use crate::configs::claude_cost::ClaudeCostConfig;
use crate::formatter::StringFormatter;
use crate::utils::{humanize_int, render_time};

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("claude_cost");
    let config = ClaudeCostConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    // Read Claude Code data from Context
    let claude_data = context.claude_code_data.as_ref()?;
    let cost_info = claude_data.cost.as_ref()?;
    let total_cost = cost_info.total_cost_usd;

    let display_style = config
        .display
        .iter()
        .filter(|s| total_cost >= f64::from(s.threshold))
        .max_by(|a, b| {
            a.threshold
                .partial_cmp(&b.threshold)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    if display_style.is_some_and(|s| s.hidden) {
        return None;
    }
    if let Some(display_style) = display_style {
        let parsed = StringFormatter::new(config.format).and_then(|formatter| {
            formatter
                .map_meta(|variable, _| match variable {
                    "symbol" => Some(config.symbol),
                    _ => None,
                })
                .map_style(|variable| match variable {
                    "style" => Some(Ok(display_style.style)),
                    _ => None,
                })
                .map(|variable| match variable {
                    "cost" => Some(Ok(format!("{total_cost:.2}"))),
                    "duration" => Some(Ok(render_time(
                        u128::from(cost_info.total_duration_ms),
                        false,
                    ))),
                    "api_duration" => Some(Ok(render_time(
                        u128::from(cost_info.total_api_duration_ms),
                        false,
                    ))),
                    "lines_added" => Some(Ok(humanize_int(cost_info.total_lines_added))),
                    "lines_removed" => Some(Ok(humanize_int(cost_info.total_lines_removed))),
                    _ => None,
                })
                .parse(None, Some(context))
        });

        module.set_segments(match parsed {
            Ok(segments) => segments,
            Err(error) => {
                log::warn!("Error in module `claude_cost`: {error}");
                return None;
            }
        });

        Some(module)
    } else {
        None
    }
}

