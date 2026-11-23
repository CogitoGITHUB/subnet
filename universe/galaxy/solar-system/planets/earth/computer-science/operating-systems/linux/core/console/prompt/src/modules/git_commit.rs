use super::{Context, Module, ModuleConfig};
use gix::commit::describe::SelectRef::AllTags;

use crate::configs::git_commit::GitCommitConfig;
use crate::context::GitRepo;
use crate::formatter::StringFormatter;

/// Creates a module with the Git commit in the current directory
///
/// Will display the commit hash if the current directory is a git repo
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("git_commit");
    let config: GitCommitConfig = GitCommitConfig::try_load(module.config);

    let repo = context.get_git_repo().ok()?;
    let git_repo = repo.open();
    let git_head = git_repo.head().ok()?;

    let is_detached = git_head.is_detached();
    if (config.only_detached && !is_detached) || git_head.is_unborn() {
        return None;
    }

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "hash" => Some(Ok(git_hash(context.get_git_repo().ok()?, &config)?)),
                "tag" if !config.tag_disabled => Some(Ok(format!(
                    "{}{}",
                    config.tag_symbol,
                    git_tag(context.get_git_repo().ok()?, &config)?
                ))),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `git_commit`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn git_tag(repo: &GitRepo, config: &GitCommitConfig) -> Option<String> {
    let mut git_repo = repo.open();
    // Increase the default object cache size to speed up operation for some repos
    git_repo.object_cache_size_if_unset(4 * 1024 * 1024);
    let head_commit = git_repo.head_commit().ok()?;

    let describe_platform = head_commit
        .describe()
        .names(AllTags)
        .max_candidates(config.tag_max_candidates)
        .traverse_first_parent(true);
    let formatter = describe_platform.try_format().ok()??;

    Some(formatter.name?.to_string())
}

fn git_hash(repo: &GitRepo, config: &GitCommitConfig) -> Option<String> {
    let git_repo = repo.open();
    let head_id = git_repo.head_id().ok()?;

    Some(format!(
        "{}",
        head_id.to_hex_with_len(config.commit_hash_length)
    ))
}

