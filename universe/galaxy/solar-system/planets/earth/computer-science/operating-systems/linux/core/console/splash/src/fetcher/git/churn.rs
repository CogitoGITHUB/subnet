use std::collections::HashMap;
use std::convert::Infallible;
use std::ops::ControlFlow;

use async_trait::async_trait;
use gix::object::tree::diff::Change;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::payload::{Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_block_body, text_body};

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Bars,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Text,
];
const DEFAULT_DAYS: u64 = 30;
const TOP_FILES: usize = 10;
const MAX_COMMITS: usize = 500;

/// Top files by change count over the last N days (default 30, configurable via `format = "N"`).
/// `Entries` is the default (`path: count` rows); `Bars` ranks visually; `TextBlock` emits one
/// `"path (count)"` per line; `MarkdownTextBlock` outputs the same ranking as a numbered markdown
/// list; `Text` collapses everything into a single-line basename summary
/// `"main.rs (42), lib.rs (31), …"`. Pairs with `code_largest_files` for "where's the action vs
/// where's the mass" framing.
pub struct GitChurn;

#[async_trait]
impl Fetcher for GitChurn {
    fn name(&self) -> &str {
        "git_churn"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top files by change count over the last N days (default 30, override with `format = \"N\"`), capped at ten. Pairs with `code_largest_files` for 'where's the action vs where's the mass' framing."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 30
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => samples::entries(&[
                ("src/main.rs", "42"),
                ("src/lib.rs", "31"),
                ("src/render/mod.rs", "18"),
                ("src/fetcher/mod.rs", "12"),
            ]),
            Shape::Bars => samples::bars(&[
                ("src/main.rs", 42),
                ("src/lib.rs", 31),
                ("src/render/mod.rs", 18),
                ("src/fetcher/mod.rs", 12),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "src/main.rs (42)",
                "src/lib.rs (31)",
                "src/render/mod.rs (18)",
                "src/fetcher/mod.rs (12)",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "1. `src/main.rs` — 42\n2. `src/lib.rs` — 31\n3. `src/render/mod.rs` — 18\n4. `src/fetcher/mod.rs` — 12",
            ),
            Shape::Text => samples::text("main.rs (42), lib.rs (31), mod.rs (18)"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let days = parse_days(ctx.format.as_deref());
        let ranked = churn(&repo, days, ctx.timezone.as_deref())?;
        Ok(payload(render_body(
            ranked,
            ctx.shape.unwrap_or(Shape::Entries),
        )))
    }
}

fn churn(
    repo: &gix::Repository,
    days: u64,
    timezone: Option<&str>,
) -> Result<Vec<(String, u64)>, FetchError> {
    let Ok(head_id) = repo.head_id() else {
        return Ok(Vec::new());
    };
    let cutoff = crate::time::now_in(timezone).timestamp() - (days as i64) * 86_400;
    let walker = repo
        .rev_walk([head_id.detach()])
        .sorting(Sorting::ByCommitTimeCutoff {
            seconds: cutoff,
            order: CommitTimeOrder::NewestFirst,
        })
        .all()
        .map_err(fail)?;

    let empty_tree = repo.empty_tree();
    let mut totals: HashMap<String, u64> = HashMap::new();

    for info in walker.take(MAX_COMMITS) {
        let Ok(info) = info else { continue };
        let Ok(commit) = repo.find_commit(info.id) else {
            continue;
        };
        let Ok(tree) = commit.tree() else { continue };
        let parent_tree = commit
            .parent_ids()
            .next()
            .and_then(|id| repo.find_commit(id.detach()).ok())
            .and_then(|c| c.tree().ok())
            .unwrap_or_else(|| empty_tree.clone());

        let Ok(mut changes) = parent_tree.changes() else {
            continue;
        };
        let _ = changes.for_each_to_obtain_tree(&tree, |change| {
            if let Some(path) = changed_path(&change) {
                *totals.entry(path).or_insert(0) += 1;
            }
            Ok::<_, Infallible>(ControlFlow::Continue(()))
        });
    }

    let mut ranked: Vec<_> = totals.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(TOP_FILES);
    Ok(ranked)
}

fn changed_path(change: &Change<'_, '_, '_>) -> Option<String> {
    let (loc, mode) = match change {
        Change::Addition {
            location,
            entry_mode,
            ..
        }
        | Change::Deletion {
            location,
            entry_mode,
            ..
        }
        | Change::Modification {
            location,
            entry_mode,
            ..
        }
        | Change::Rewrite {
            location,
            entry_mode,
            ..
        } => (*location, entry_mode),
    };
    if loc.is_empty() || mode.is_tree() {
        None
    } else {
        Some(loc.to_string())
    }
}

fn parse_days(format: Option<&str>) -> u64 {
    format
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_DAYS)
}

fn render_body(ranked: Vec<(String, u64)>, shape: Shape) -> Body {
    match shape {
        Shape::Bars => Body::Bars(BarsData {
            bars: ranked
                .into_iter()
                .map(|(path, count)| Bar {
                    label: path,
                    value: count,
                    value_label: None,
                })
                .collect(),
        }),
        Shape::TextBlock => text_block_body(
            ranked
                .into_iter()
                .map(|(path, count)| format!("{path} ({count})"))
                .collect(),
        ),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: ranked
                .into_iter()
                .enumerate()
                .map(|(i, (path, count))| format!("{}. `{path}` — {count}", i + 1))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Text => {
            if ranked.is_empty() {
                text_body("")
            } else {
                let line = ranked
                    .into_iter()
                    .map(|(path, count)| format!("{} ({count})", short_path(&path)))
                    .collect::<Vec<_>>()
                    .join(", ");
                text_body(line)
            }
        }
        _ => Body::Entries(EntriesData {
            items: ranked
                .into_iter()
                .map(|(path, count)| Entry {
                    key: path,
                    value: Some(count.to_string()),
                    status: None,
                })
                .collect(),
        }),
    }
}

fn short_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

