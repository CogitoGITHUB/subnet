use super::{Context, Module, ModuleConfig};

use crate::configs::singularity::SingularityConfig;
use crate::formatter::StringFormatter;

/// Creates a module with the current Singularity image
///
/// Will display the Singularity image if `$SINGULARITY_NAME` is set.
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let singularity_env = context.get_env("SINGULARITY_NAME")?;

    let mut module = context.new_module("singularity");
    let config: SingularityConfig = SingularityConfig::try_load(module.config);

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
                "env" => Some(Ok(&singularity_env)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `singularity`: \n{error}");
            return None;
        }
    });

    Some(module)
}

