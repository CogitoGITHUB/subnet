use gix::bstr::ByteSlice;
use unicode_segmentation::UnicodeSegmentation;

use super::{Context, Module, ModuleConfig};

use crate::configs::git_branch::GitBranchConfig;
use crate::context::GitRepo;
use crate::formatter::StringFormatter;

/// Creates a module with the Git branch in the current directory
///
/// Will display the branch name if the current directory is a git repo
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("git_branch");
    let config = GitBranchConfig::try_load(module.config);

    let truncation_symbol = get_first_grapheme(config.truncation_symbol);

    let len = if config.truncation_length <= 0 {
        log::warn!(
            "\"truncation_length\" should be a positive value, found {}",
            config.truncation_length
        );
        usize::MAX
    } else {
        config.truncation_length as usize
    };

    let repo = context.get_git_repo().ok()?;

    let gix_repo = repo.open();
    if config.ignore_bare_repo && gix_repo.workdir().is_none() {
        return None;
    }

    // Get branch and remote information
    let (branch_name, remote_branch, remote_name) = if repo.uses_reftables() {
        // Use git executable for branch information
        if let Some((branch, remote_branch)) = get_branch_info_from_git(context, repo) {
            // Successfully got branch name, parse upstream into remote_name and remote_branch
            let remote_name = remote_branch
                .as_ref()
                .map(|remote_branch| {
                    find_longest_matching_remote_name(
                        remote_branch.as_ref(),
                        &gix_repo.remote_names(),
                    )
                })
                .unwrap_or_default();
            (
                branch.shorten().to_string(),
                remote_branch
                    .zip(remote_name.as_deref())
                    .map(|(remote_branch, remote_name)| {
                        remote_branch
                            .shorten()
                            .to_str_lossy()
                            .strip_prefix(remote_name)
                            .expect("remote name determined by finding it as prefix before")
                            .trim_start_matches('/')
                            .to_string()
                    }),
                remote_name,
            )
        } else {
            // get_branch_info_from_git returns None when:
            // 1. HEAD is detached (git symbolic-ref fails)
            // 2. git command fails for other reasons (corrupted repo, missing git, etc.)
            // In both cases, if only_attached is set, we should return None
            if config.only_attached {
                return None;
            }
            // Fallback to HEAD for detached state or when branch can't be determined
            ("HEAD".to_string(), None, None)
        }
    } else {
        if config.only_attached && gix_repo.head().ok()?.is_detached() {
            return None;
        }

        let branch = repo.branch.clone().unwrap_or_else(|| "HEAD".to_string());
        let (remote_branch, remote_name) = if let Some(remote) = repo.remote.as_ref() {
            (remote.branch.clone(), remote.name.clone())
        } else {
            (None, None)
        };
        (branch, remote_branch, remote_name)
    };

    if config
        .ignore_branches
        .iter()
        .any(|&ignored| branch_name.eq(ignored))
    {
        return None;
    }

    let is_ignored_remote = config
        .ignore_remotes
        .iter()
        .any(|ignored| remote_name.as_deref() == Some(ignored));

    let mut graphemes: Vec<&str> = branch_name.graphemes(true).collect();

    let remote_branch_string = remote_branch.unwrap_or_default();
    let mut remote_branch_graphemes: Vec<&str> = remote_branch_string.graphemes(true).collect();

    let remote_name_string = remote_name.unwrap_or_default();
    let mut remote_name_graphemes: Vec<&str> = remote_name_string.graphemes(true).collect();

    // Truncate fields if need be
    for e in &mut [
        &mut graphemes,
        &mut remote_branch_graphemes,
        &mut remote_name_graphemes,
    ] {
        let e = &mut **e;
        let trunc_len = len.min(e.len());
        if trunc_len < e.len() {
            // The truncation symbol should only be added if we truncate
            e[trunc_len] = truncation_symbol;
            e.truncate(trunc_len + 1);
        }
    }

    let show_remote = config.always_show_remote
        || (!graphemes.eq(&remote_branch_graphemes) && !remote_branch_graphemes.is_empty());

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "branch" => Some(Ok(graphemes.concat())),
                "remote_branch" => {
                    if show_remote && !remote_branch_graphemes.is_empty() {
                        Some(Ok(remote_branch_graphemes.concat()))
                    } else {
                        None
                    }
                }
                "remote_name" => {
                    if show_remote && !is_ignored_remote && !remote_name_graphemes.is_empty() {
                        Some(Ok(remote_name_graphemes.concat()))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `git_branch`: \n{error}");
            return None;
        }
    });

    Some(module)
}

/// Given `remote_names`, find the longest matching remote name in `remote_ref_name` and return it.
fn find_longest_matching_remote_name(
    remote_ref_name: &gix::refs::FullNameRef,
    remote_names: &gix::remote::Names,
) -> Option<String> {
    let (category, shorthand_name) = remote_ref_name.category_and_short_name()?;
    if !matches!(category, gix::refs::Category::RemoteBranch) {
        return None;
    }

    let longest_remote = remote_names
        .iter()
        .rfind(|reference_name| shorthand_name.starts_with(reference_name))?;
    Some(longest_remote.to_string())
}

/// Get branch information using the git executable.
/// Returns `None` if not on a branch (detached HEAD) or if the git command fails.
fn get_branch_info_from_git(
    context: &Context,
    repo: &GitRepo,
) -> Option<(gix::refs::FullName, Option<gix::refs::FullName>)> {
    // Get current branch name using git symbolic-ref
    // These fixed ref queries do not run worktree filters or hooks.
    let branch_output = repo.exec_git_with_required_trust(
        context,
        gix::sec::Trust::Reduced,
        ["symbolic-ref", "HEAD"],
    )?;

    // Check if the command was successful (exit code 0)
    // symbolic-ref returns non-zero exit code when HEAD is detached
    let branch_name = branch_output.stdout.trim();
    if branch_name.is_empty() {
        return None;
    }

    let branch_name: gix::refs::FullName = branch_name.try_into().ok()?;
    // Get upstream tracking branch using git for-each-ref
    let upstream_output = repo.exec_git_with_required_trust(
        context,
        gix::sec::Trust::Reduced,
        [
            "for-each-ref",
            "--format=%(upstream)",
            branch_name.as_bstr().to_str_lossy().as_ref(),
        ],
    )?;
    let upstream = upstream_output.stdout.trim();
    let remote_info = if upstream.is_empty() {
        None
    } else {
        upstream.try_into().ok()
    };

    Some((branch_name, remote_info))
}

fn get_first_grapheme(text: &str) -> &str {
    UnicodeSegmentation::graphemes(text, true)
        .next()
        .unwrap_or("")
}

