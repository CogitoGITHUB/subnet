use std::io::Error;
use std::path::Path;

use super::utils::truncate::truncate_text;
use super::{Context, Module, ModuleConfig, vcs};

use crate::configs::hg_branch::HgBranchConfig;
use crate::formatter::StringFormatter;
use crate::utils::read_file;

/// Creates a module with the Hg bookmark or branch in the current directory
///
/// Will display the bookmark or branch name if the current directory is an hg repo
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("hg_branch");
    let config: HgBranchConfig = HgBranchConfig::try_load(module.config);

    // As we default to disabled=true, we have to check here after loading our config module,
    // before it was only checking against whatever is in the config starship.toml
    if config.disabled {
        return None;
    }

    let len = if config.truncation_length <= 0 {
        log::warn!(
            "\"truncation_length\" should be a positive value, found {}",
            config.truncation_length
        );
        usize::MAX
    } else {
        config.truncation_length as usize
    };

    let repo_root = vcs::discover_repo_root(context, vcs::Vcs::Hg)?;
    let branch_name = get_hg_current_bookmark(&repo_root).unwrap_or_else(|_| {
        get_hg_branch_name(&repo_root).unwrap_or_else(|_| String::from("default"))
    });

    let branch_graphemes = truncate_text(&branch_name, len, config.truncation_symbol);
    let topic_graphemes = if let Ok(topic) = get_hg_topic_name(&repo_root) {
        truncate_text(&topic, len, config.truncation_symbol)
    } else {
        String::new()
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
                "branch" => Some(Ok(branch_graphemes.as_str())),
                "topic" => Some(Ok(topic_graphemes.as_str())),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `hg_branch`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_hg_branch_name(hg_root: &Path) -> Result<String, Error> {
    match read_file(hg_root.join(".hg").join("branch")) {
        Ok(b) => Ok(b.trim().to_string()),
        Err(e) => Err(e),
    }
}

fn get_hg_current_bookmark(hg_root: &Path) -> Result<String, Error> {
    read_file(hg_root.join(".hg").join("bookmarks.current"))
}

fn get_hg_topic_name(hg_root: &Path) -> Result<String, Error> {
    match read_file(hg_root.join(".hg").join("topic")) {
        Ok(b) => Ok(b.trim().to_string()),
        Err(e) => Err(e),
    }
}

