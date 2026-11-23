use std::collections::HashMap;

use async_trait::async_trait;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, TextBlockData,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_body};

const SHAPES: &[Shape] = &[
    Shape::Bars,
    Shape::Entries,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Text,
];
const DEFAULT_DAYS: u64 = 30;
const MAX_ENTRIES: usize = 10;

/// Commit authors over the last N days (default 30, configurable via `format = "N"`). Ranked
/// by commit count, truncated to the top 10 so a busy repo doesn't blow up the widget. `Bars`
/// is the default (visual ranking); `Entries` emits `name: count` rows; `TextBlock` lists
/// `"alice  42"` per line; `MarkdownTextBlock` renders the same ranking as a markdown list;
/// `Text` collapses to `"alice (23), bob (11)"` style.
pub struct GitContributors;

#[async_trait]
impl Fetcher for GitContributors {
    fn name(&self) -> &str {
        "git_contributors"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top commit authors over the last N days (default 30, override with `format = \"N\"`), ranked by commit count and capped at ten. Bars/Entries/TextBlock/MarkdownTextBlock all carry the ranking; Text is the headline summary."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 30
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Bars
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Bars => {
                samples::bars(&[("alice", 42), ("bob", 28), ("charlie", 17), ("dave", 9)])
            }
            Shape::Entries => samples::entries(&[
                ("alice", "42"),
                ("bob", "28"),
                ("charlie", "17"),
                ("dave", "9"),
            ]),
            Shape::TextBlock => {
                samples::text_block(&["alice  42", "bob  28", "charlie  17", "dave  9"])
            }
            Shape::MarkdownTextBlock => samples::markdown(
                "1. **alice** — 42\n2. **bob** — 28\n3. **charlie** — 17\n4. **dave** — 9",
            ),
            Shape::Text => samples::text("alice (42), bob (28), charlie (17), dave (9)"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let days = parse_days(ctx.format.as_deref());
        let ranked = contributors(&repo, days, ctx.timezone.as_deref())?;
        Ok(payload(render_body(
            ranked,
            ctx.shape.unwrap_or(Shape::Bars),
        )))
    }
}

fn contributors(
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
    let mut counts: HashMap<String, u64> = HashMap::new();
    for info in walker {
        let Ok(info) = info else { continue };
        let Ok(commit) = repo.find_commit(info.id) else {
            continue;
        };
        let Ok(author) = commit.author() else {
            continue;
        };
        let name = author.name.to_string();
        *counts.entry(name).or_insert(0) += 1;
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(MAX_ENTRIES);
    Ok(ranked)
}

fn parse_days(format: Option<&str>) -> u64 {
    format
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_DAYS)
}

fn render_body(ranked: Vec<(String, u64)>, shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: ranked
                .into_iter()
                .map(|(name, count)| Entry {
                    key: name,
                    value: Some(count.to_string()),
                    status: None,
                })
                .collect(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: ranked
                .into_iter()
                .map(|(name, count)| format!("{name}  {count}"))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: ranked
                .into_iter()
                .enumerate()
                .map(|(i, (name, count))| format!("{}. **{name}** — {count}", i + 1))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Text => {
            if ranked.is_empty() {
                text_body("")
            } else {
                let line = ranked
                    .into_iter()
                    .map(|(name, count)| format!("{name} ({count})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                text_body(line)
            }
        }
        _ => Body::Bars(BarsData {
            bars: ranked
                .into_iter()
                .map(|(name, count)| Bar {
                    label: name,
                    value: count,
                    value_label: None,
                })
                .collect(),
        }),
    }
}

