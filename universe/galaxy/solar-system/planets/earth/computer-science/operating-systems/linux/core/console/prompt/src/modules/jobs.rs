use super::{Context, Module, ModuleConfig};

use crate::configs::jobs::JobsConfig;
use crate::formatter::StringFormatter;

/// Creates a segment to show if there are any active jobs running
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("jobs");
    let config = JobsConfig::try_load(module.config);

    if config.threshold < 0 {
        log::warn!(
            "threshold in [jobs] ({}) was less than zero",
            config.threshold
        );
        return None;
    }

    if config.symbol_threshold < 0 {
        log::warn!(
            "symbol_threshold in [jobs] ({}) was less than zero",
            config.symbol_threshold
        );
        return None;
    }

    if config.number_threshold < 0 {
        log::warn!(
            "number_threshold in [jobs] ({}) was less than zero",
            config.number_threshold
        );
        return None;
    }

    let props = &context.properties;
    let num_of_jobs = props.jobs;

    if num_of_jobs == 0
        && config.threshold > 0
        && config.number_threshold > 0
        && config.symbol_threshold > 0
    {
        return None;
    }

    let default_threshold = 1;
    let mut module_symbol = "";
    let mut module_number = String::new();
    if config.threshold == default_threshold {
        if num_of_jobs >= config.symbol_threshold {
            module_symbol = config.symbol;
        }

        if num_of_jobs >= config.number_threshold {
            module_number = num_of_jobs.to_string();
        }
    } else {
        log::warn!(
            "`threshold` in [jobs] is deprecated. Please remove it and use `symbol_threshold` and `number_threshold`."
        );

        // The symbol should be shown if there are *any* background
        // jobs running.
        if num_of_jobs > 0 {
            module_symbol = config.symbol;
        }

        if num_of_jobs > config.threshold || config.threshold == 0 {
            module_symbol = config.symbol;
            module_number = num_of_jobs.to_string();
        }
    }

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "symbol" => Some(module_symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "number" => Some(Ok(module_number.clone())),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `jobs`:\n{error}");
            return None;
        }
    });

    Some(module)
}

