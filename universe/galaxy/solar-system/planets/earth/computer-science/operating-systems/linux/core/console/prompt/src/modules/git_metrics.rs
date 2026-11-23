use std::num::Saturating;

use gix::bstr::{BStr, ByteSlice};
use gix::diff::blob::ResourceKind;
use gix::diff::blob::pipeline::WorktreeRoots;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use regex::Regex;

use super::Context;
use crate::configs::git_status::GitStatusConfig;
use crate::{
    config::ModuleConfig, configs::git_metrics::GitMetricsConfig, formatter::StringFormatter,
    formatter::string_formatter::StringFormatterError, module::Module,
};

/// Creates a module with the current added/deleted lines in the git repository at the
/// current directory
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("git_metrics");
    let config: GitMetricsConfig = GitMetricsConfig::try_load(module.config);

    // As we default to disabled=true, we have to check here after loading our config module,
    // before it was only checking against whatever is in the config starship.toml
    if config.disabled {
        return None;
    }

    let repo = context.get_git_repo().ok()?;
    let gix_repo = repo.open();
    gix_repo.workdir()?;
    let status_module = context.new_module("git_status");
    let status_config = GitStatusConfig::try_load(status_module.config);
    let stats = if repo.use_git_executable(status_config.use_git_executable)? {
        let mut git_args = vec!["diff", "--shortstat"];
        if config.ignore_submodules {
            git_args.push("--ignore-submodules");
        }

        let diff = repo.exec_git(context, &git_args)?.stdout;

        GitDiff::parse(&diff)
    } else {
        #[derive(Default)]
        struct Diff {
            added: Saturating<u32>,
            deleted: Saturating<u32>,
        }
        impl Diff {
            fn add(&mut self, c: Option<gix::diff::blob::DiffLineStats>) {
                let Some(c) = c else { return };
                self.added += c.insertions;
                self.deleted += c.removals;
            }
        }
        let status = super::git_status::get_static_repo_status(context, repo, &status_config)?;
        let gix_repo = gix_repo.with_object_memory();
        gix_repo.write_blob([]).ok()?; /* create empty blob */
        let tree_index_cache = prevent_external_diff(
            gix_repo
                .diff_resource_cache(
                    gix::diff::blob::pipeline::Mode::ToGit,
                    WorktreeRoots::default(),
                )
                .ok()?,
        );
        let index_worktree_cache = prevent_external_diff(
            gix_repo
                .diff_resource_cache(
                    gix::diff::blob::pipeline::Mode::ToGit,
                    WorktreeRoots {
                        old_root: None,
                        new_root: gix_repo.workdir().map(ToOwned::to_owned),
                    },
                )
                .ok()?,
        );
        let diff = status
            .changes
            .par_iter()
            .map_init(
                {
                    let repo = gix_repo.into_sync();
                    move || {
                        let repo = repo.to_thread_local();
                        (repo, tree_index_cache.clone(), index_worktree_cache.clone())
                    }
                },
                |(repo, tree_index_cache, index_worktree_cache), change| {
                    use gix::status;
                    let mut diff = Diff::default();
                    match change {
                        status::Item::TreeIndex(change) => {
                            use gix::diff::index::Change;
                            match change {
                                Change::Addition {
                                    entry_mode,
                                    location,
                                    id,
                                    ..
                                } => {
                                    diff.added += count_lines(
                                        location,
                                        id.as_ref().into(),
                                        *entry_mode,
                                        tree_index_cache,
                                        repo,
                                    );
                                }
                                Change::Deletion {
                                    entry_mode,
                                    location,
                                    id,
                                    ..
                                } => {
                                    diff.deleted += count_lines(
                                        location,
                                        id.as_ref().into(),
                                        *entry_mode,
                                        tree_index_cache,
                                        repo,
                                    );
                                }
                                Change::Modification {
                                    location,
                                    previous_entry_mode,
                                    previous_id,
                                    entry_mode,
                                    id,
                                    ..
                                } => {
                                    let location = location.as_ref();
                                    diff.add(diff_two_opt(
                                        location,
                                        previous_id.as_ref().to_owned(),
                                        *previous_entry_mode,
                                        location,
                                        id.as_ref().to_owned(),
                                        *entry_mode,
                                        tree_index_cache,
                                        repo,
                                    ));
                                }
                                Change::Rewrite {
                                    source_location,
                                    source_entry_mode,
                                    source_id,
                                    location,
                                    entry_mode,
                                    id,
                                    copy,
                                    ..
                                } => {
                                    if *copy {
                                        diff.added += count_lines(
                                            location.as_ref(),
                                            id.as_ref().into(),
                                            *entry_mode,
                                            tree_index_cache,
                                            repo,
                                        );
                                    } else {
                                        diff.add(diff_two_opt(
                                            source_location.as_ref(),
                                            source_id.as_ref().to_owned(),
                                            *source_entry_mode,
                                            location,
                                            id.as_ref().to_owned(),
                                            *entry_mode,
                                            tree_index_cache,
                                            repo,
                                        ));
                                    }
                                }
                            }
                        }
                        status::Item::IndexWorktree(change) => {
                            use gix::status::index_worktree::Item;
                            use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
                            match change {
                                Item::Modification {
                                    rela_path,
                                    entry,
                                    status: EntryStatus::Change(Change::Removed),
                                    ..
                                } => {
                                    diff.deleted += count_lines(
                                        rela_path.as_bstr(),
                                        entry.id,
                                        entry.mode,
                                        tree_index_cache,
                                        repo,
                                    );
                                }
                                Item::Modification {
                                    rela_path,
                                    entry,
                                    status:
                                        EntryStatus::Change(Change::Modification {
                                            content_change: Some(()),
                                            ..
                                        }),
                                    ..
                                } => {
                                    let location = rela_path.as_bstr();
                                    diff.add(diff_two_opt(
                                        location,
                                        entry.id,
                                        entry.mode,
                                        location,
                                        repo.object_hash().null(),
                                        entry.mode,
                                        index_worktree_cache,
                                        repo,
                                    ));
                                }
                                Item::Modification {
                                    rela_path,
                                    entry,
                                    status: EntryStatus::IntentToAdd,
                                    ..
                                } => {
                                    diff.added += count_lines(
                                        rela_path.as_bstr(),
                                        repo.object_hash().null(),
                                        entry.mode,
                                        index_worktree_cache,
                                        repo,
                                    );
                                }
                                Item::Rewrite { .. } => {
                                    unreachable!("not activated")
                                }
                                _ => {}
                            }
                        }
                    }
                    diff
                },
            )
            .reduce(Diff::default, |a, b| Diff {
                added: a.added + b.added,
                deleted: a.deleted + b.deleted,
            });

        GitDiff {
            added: diff.added.to_string(),
            deleted: diff.deleted.to_string(),
        }
    };

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "added_style" => Some(Ok(config.added_style)),
                "deleted_style" => Some(Ok(config.deleted_style)),
                _ => None,
            })
            .map(|variable| match variable {
                "added" => GitDiff::get_variable(config.only_nonzero_diffs, &stats.added),
                "deleted" => GitDiff::get_variable(config.only_nonzero_diffs, &stats.deleted),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `git_metrics`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn prevent_external_diff(mut cache: gix::diff::blob::Platform) -> gix::diff::blob::Platform {
    cache.options.skip_internal_diff_if_external_is_configured = false;
    cache
}

#[allow(clippy::too_many_arguments)]
fn diff_two_opt(
    lhs_location: &BStr,
    lhs_id: gix::ObjectId,
    lhs_kind: gix::index::entry::Mode,
    rhs_location: &BStr,
    rhs_id: gix::ObjectId,
    rhs_kind: gix::index::entry::Mode,
    cache: &mut gix::diff::blob::Platform,
    find: &impl gix::objs::FindObjectOrHeader,
) -> Option<gix::diff::blob::DiffLineStats> {
    cache
        .set_resource(
            lhs_id,
            lhs_kind.to_tree_entry_mode()?.kind(),
            lhs_location,
            ResourceKind::OldOrSource,
            find,
        )
        .ok()?;
    cache
        .set_resource(
            rhs_id,
            rhs_kind.to_tree_entry_mode()?.kind(),
            rhs_location,
            ResourceKind::NewOrDestination,
            find,
        )
        .ok()?;
    let mut diff_platform = gix::object::blob::diff::Platform {
        resource_cache: cache,
    };
    diff_platform.line_counts().ok().flatten()
}

fn count_lines(
    location: &BStr,
    id: gix::ObjectId,
    kind: gix::index::entry::Mode,
    cache: &mut gix::diff::blob::Platform,
    find: &impl gix::objs::FindObjectOrHeader,
) -> u32 {
    diff_two_opt(
        location,
        id.kind().null(),
        kind,
        location,
        id,
        kind,
        cache,
        find,
    )
    .map_or(0, |diff| diff.insertions)
}

/// Represents the parsed output from a git diff.
#[derive(Default)]
struct GitDiff {
    added: String,
    deleted: String,
}

impl GitDiff {
    /// Returns the first capture group given a regular expression and a string.
    /// If it fails to get the capture group it will return "0".
    fn get_matched_str<'a>(diff: &'a str, re: &Regex) -> &'a str {
        match re.captures(diff) {
            Some(caps) => caps.get(1).unwrap().as_str(),
            _ => "0",
        }
    }

    /// Parses the result of 'git diff --shortstat' as a `GitDiff` struct.
    pub fn parse(diff: &str) -> Self {
        let added_re = Regex::new(r"(\d+) \w+\(\+\)").unwrap();
        let deleted_re = Regex::new(r"(\d+) \w+\(\-\)").unwrap();

        Self {
            added: Self::get_matched_str(diff, &added_re).to_owned(),
            deleted: Self::get_matched_str(diff, &deleted_re).to_owned(),
        }
    }

    pub fn get_variable(
        only_nonzero_diffs: bool,
        changed: &str,
    ) -> Option<Result<&str, StringFormatterError>> {
        if only_nonzero_diffs {
            match changed {
                "0" => None,
                _ => Some(Ok(changed)),
            }
        } else {
            Some(Ok(changed))
        }
    }
}

