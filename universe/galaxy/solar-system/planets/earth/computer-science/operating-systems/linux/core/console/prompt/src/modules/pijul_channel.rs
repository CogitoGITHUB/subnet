use super::utils::truncate::truncate_text;
use super::{Context, Module, ModuleConfig, vcs};

use crate::configs::pijul_channel::PijulConfig;
use crate::formatter::StringFormatter;

/// Creates a module with the Pijul channel in the current directory
///
/// Will display the channel lame if the current directory is a pijul repo
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("pijul_channel");
    let config: PijulConfig = PijulConfig::try_load(module.config);

    // We default to disabled=true, so we have to check after loading our config module.
    if config.disabled {
        return None;
    }

    vcs::discover_repo_root(context, vcs::Vcs::Pijul)?;

    let channel_name = get_pijul_current_channel(context)?;

    let truncated_text = truncate_text(
        &channel_name,
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
                "channel" => Some(Ok(&truncated_text)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `pijul_channel`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_pijul_current_channel(ctx: &Context) -> Option<String> {
    let output = ctx.exec_cmd("pijul", &["channel"])?.stdout;

    output
        .lines()
        .find_map(|l| l.strip_prefix("* "))
        .map(str::to_owned)
}

