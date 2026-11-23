use super::{Context, Module, ModuleConfig, vcs};

use crate::configs::fossil_branch::FossilBranchConfig;
use crate::formatter::StringFormatter;
use crate::modules::utils::truncate::truncate_text;

/// Creates a module with the Fossil branch of the check-out in the current directory
///
/// Will display the branch name if the current directory is a Fossil check-out
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("fossil_branch");
    let config = FossilBranchConfig::try_load(module.config);

    // As we default to disabled=true, we have to check here after loading our config module,
    // before it was only checking against whatever is in the config starship.toml
    if config.disabled {
        return None;
    }

    vcs::discover_repo_root(context, vcs::Vcs::Fossil)?;

    let len = if config.truncation_length <= 0 {
        log::warn!(
            "\"truncation_length\" should be a positive value, found {}",
            config.truncation_length
        );
        usize::MAX
    } else {
        config.truncation_length as usize
    };

    let truncated_branch_name = {
        let output = context.exec_cmd("fossil", &["branch", "current"])?.stdout;
        truncate_text(output.trim(), len, config.truncation_symbol)
    };

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
                "branch" => Some(Ok(truncated_branch_name.as_str())),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `fossil_branch`:\n{error}");
            return None;
        }
    });

    Some(module)
}

