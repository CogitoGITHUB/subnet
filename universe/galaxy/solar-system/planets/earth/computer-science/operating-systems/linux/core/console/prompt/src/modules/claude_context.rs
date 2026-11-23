use super::{Context, Module, ModuleConfig};
use crate::configs::claude_context::ClaudeContextConfig;
use crate::formatter::StringFormatter;
use crate::utils::humanize_int;

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("claude_context");
    let config = ClaudeContextConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    // Read Claude Code data from Context
    let claude_data = context.claude_code_data.as_ref()?;

    let total_tokens = claude_data.context_window.context_window_size;
    let percentage_float = claude_data.context_window.used_percentage.clamp(0.0, 100.0);
    let percentage = percentage_float.round() as u8;

    // Determine style based on percentage
    let display_style = config
        .display
        .iter()
        .filter(|s| percentage_float >= s.threshold)
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
                    "gauge" => {
                        let filled_float =
                            (f64::from(percentage) / 100.0) * f64::from(config.gauge_width);
                        let (filled_count, partial) = if config.gauge_partial_symbol.is_empty() {
                            (filled_float.round() as usize, false)
                        } else {
                            let full = filled_float.floor() as usize;
                            let rem = filled_float - full as f64;
                            // Show partial block if remainder is significant enough (> 0.25)
                            // but if it's very close to 1 (> 0.75), just round up to full (handled by empty_count check)
                            if rem >= 0.25 {
                                (full, true)
                            } else {
                                (full, false)
                            }
                        };

                        let filled_count = filled_count.min(config.gauge_width as usize);
                        let partial_count =
                            usize::from(partial && filled_count < config.gauge_width as usize);
                        let empty_count = (config.gauge_width as usize)
                            .saturating_sub(filled_count + partial_count);

                        let gauge = config.gauge_full_symbol.repeat(filled_count)
                            + &config.gauge_partial_symbol.repeat(partial_count)
                            + &config.gauge_empty_symbol.repeat(empty_count);
                        Some(Ok(gauge))
                    }
                    "percentage" => Some(Ok(format!("{percentage}%"))),
                    "input_tokens" => Some(Ok(humanize_int(
                        claude_data.context_window.total_input_tokens,
                    ))),
                    "output_tokens" => Some(Ok(humanize_int(
                        claude_data.context_window.total_output_tokens,
                    ))),
                    "curr_input_tokens" => Some(Ok(humanize_int(
                        claude_data.context_window.current_usage.input_tokens,
                    ))),
                    "curr_output_tokens" => Some(Ok(humanize_int(
                        claude_data.context_window.current_usage.output_tokens,
                    ))),
                    "curr_cache_creation_tokens" => Some(Ok(humanize_int(
                        claude_data
                            .context_window
                            .current_usage
                            .cache_creation_input_tokens,
                    ))),
                    "curr_cache_read_tokens" => Some(Ok(humanize_int(
                        claude_data
                            .context_window
                            .current_usage
                            .cache_read_input_tokens,
                    ))),
                    "total_tokens" => Some(Ok(humanize_int(total_tokens))),
                    _ => None,
                })
                .parse(None, Some(context))
        });

        module.set_segments(match parsed {
            Ok(segments) => segments,
            Err(error) => {
                log::warn!("Error in module `claude_context`: {error}");
                return None;
            }
        });

        Some(module)
    } else {
        None
    }
}

