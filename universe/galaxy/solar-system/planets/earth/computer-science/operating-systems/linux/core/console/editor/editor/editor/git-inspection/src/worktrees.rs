// One job: the worktrees — the working trees of one repository, and the
// branches or commits they stand on.
//
// A linked worktree shares the repository's objects: it is not a repository
// of its own, and the model says so. The porcelain list is read here, and
// nothing else is: no worktree is created, moved, locked or pruned.

use crate::model::{Divergence, Freshness, Head, Worktree};

/// parse reads the porcelain list of the worktrees. The list is read in
/// either shape it comes in: the one with lines, and the one with NULs.
pub fn parse(output: &str) -> Vec<Worktree> {
    let text = output.replace('\0', "\n");
    let mut worktrees: Vec<Worktree> = Vec::new();
    let mut path = String::new();
    let mut commit = String::new();
    let mut branch = String::new();
    let mut detached = false;
    let mut locked = false;
    let mut prunable = false;
    let mut bare = false;
    let mut seen = false;
    let flush = |worktrees: &mut Vec<Worktree>,
                    path: &mut String,
                    commit: &mut String,
                    branch: &mut String,
                    detached: &mut bool,
                    locked: &mut bool,
                    prunable: &mut bool,
                    bare: &mut bool,
                    seen: &mut bool| {
        if !*seen {
            return;
        }
        let head = if *detached {
            Head::Detached(commit.clone())
        } else {
            Head::OnBranch(branch.clone())
        };
        worktrees.push(Worktree {
            path: path.clone(),
            head,
            commit: commit.clone(),
            locked: *locked,
            prunable: *prunable,
            bare: *bare,
            counts: Default::default(),
            divergence: Divergence::Unknown,
            freshness: Freshness::Fresh,
        });
        *path = String::new();
        *commit = String::new();
        *branch = String::new();
        *detached = false;
        *locked = false;
        *prunable = false;
        *bare = false;
        *seen = false;
    };
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(
                &mut worktrees,
                &mut path,
                &mut commit,
                &mut branch,
                &mut detached,
                &mut locked,
                &mut prunable,
                &mut bare,
                &mut seen,
            );
            path = rest.to_string();
            seen = true;
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            commit = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = rest.trim_start_matches("refs/heads/").to_string();
        } else if line == "detached" {
            detached = true;
        } else if line == "locked" {
            locked = true;
        } else if line.starts_with("prunable") {
            prunable = true;
        } else if line == "bare" {
            bare = true;
        }
    }
    flush(
        &mut worktrees,
        &mut path,
        &mut commit,
        &mut branch,
        &mut detached,
        &mut locked,
        &mut prunable,
        &mut bare,
        &mut seen,
    );
    worktrees
}

