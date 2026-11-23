use super::{Context, Module};

use crate::config::{ModuleConfig, parse_style_string};
use crate::configs::fill::FillConfig;
use crate::segment::Segment;

/// Creates a module that fills the any extra space on the line.
///
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("fill");
    let config: FillConfig = FillConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    let style = parse_style_string(config.style, Some(context));

    module.set_segments(vec![Segment::fill(style, config.symbol)]);

    Some(module)
}

