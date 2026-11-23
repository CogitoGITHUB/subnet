use gix::state::InProgress;
use std::path::PathBuf;

use super::{Context, Module, ModuleConfig};
use crate::configs::git_state::GitStateConfig;
use crate::context::GitRepo;
use crate::formatter::StringFormatter;

/// Creates a module with the state of the git repository at the current directory
///
/// During a git operation it will show: REBASING, BISECTING, MERGING, etc.
/// If the progress information is available (e.g. rebasing 3/10), it will show that too.
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("git_state");
    let config: GitStateConfig = GitStateConfig::try_load(module.config);

    let repo = context.get_git_repo().ok()?;

    let state_description = get_state_description(repo, &config)?;

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "state" => Some(state_description.label),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "progress_current" => state_description.current.as_ref().map(Ok),
                "progress_total" => state_description.total.as_ref().map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `git_state`:\n{error}");
            return None;
        }
    });

    Some(module)
}

/// Returns the state of the current repository
///
/// During a git operation it will show: REBASING, BISECTING, MERGING, etc.
fn get_state_description<'a>(
    repo: &'a GitRepo,
    config: &GitStateConfig<'a>,
) -> Option<StateDescription<'a>> {
    match repo.state.as_ref()? {
        InProgress::Merge => Some(StateDescription {
            label: config.merge,
            current: None,
            total: None,
        }),
        InProgress::Revert | InProgress::RevertSequence => Some(StateDescription {
            label: config.revert,
            current: None,
            total: None,
        }),
        InProgress::CherryPick | InProgress::CherryPickSequence => Some(StateDescription {
            label: config.cherry_pick,
            current: None,
            total: None,
        }),
        InProgress::Bisect => Some(StateDescription {
            label: config.bisect,
            current: None,
            total: None,
        }),
        InProgress::ApplyMailbox => Some(describe_rebase_apply(repo, config.am)),
        InProgress::ApplyMailboxRebase => Some(describe_rebase_apply(repo, config.am_or_rebase)),
        InProgress::Rebase | InProgress::RebaseInteractive => {
            Some(describe_rebase_apply(repo, config.rebase))
        }
    }
}

// TODO: Use future gitoxide API to get the state of the rebase
fn describe_rebase_apply<'a>(repo: &'a GitRepo, state_label: &'a str) -> StateDescription<'a> {
    /*
     *  Sadly, libgit2 seems to have some issues with reading the state of
     *  interactive rebases. So, instead, we'll poke a few of the .git files
     *  ourselves. This might be worth re-visiting this in the future...
     *
     *  The following is based heavily on: https://github.com/magicmonty/bash-git-prompt
     */

    let has_path = |relative_path: &str| {
        let path = repo.path.join(PathBuf::from(relative_path));
        path.exists()
    };

    let file_to_usize = |relative_path: &str| {
        let path = repo.path.join(PathBuf::from(relative_path));
        let contents = crate::utils::read_file(path).ok()?;
        let quantity = contents.trim().parse::<usize>().ok()?;
        Some(quantity)
    };

    let paths_to_progress = |current_path: &str, total_path: &str| {
        let current = file_to_usize(current_path)?;
        let total = file_to_usize(total_path)?;
        Some((current, total))
    };

    let progress = if has_path("rebase-merge/msgnum") {
        paths_to_progress("rebase-merge/msgnum", "rebase-merge/end")
    } else if has_path("rebase-apply") {
        paths_to_progress("rebase-apply/next", "rebase-apply/last")
    } else {
        None
    };

    let (current, total) = if let Some((c, t)) = progress {
        (Some(format!("{c}")), Some(format!("{t}")))
    } else {
        (None, None)
    };

    StateDescription {
        label: state_label,
        current,
        total,
    }
}

struct StateDescription<'a> {
    label: &'a str,
    current: Option<String>,
    total: Option<String>,
}

