use regex::Regex;

use super::{Context, Module, ModuleConfig, vcs};

use crate::configs::fossil_metrics::FossilMetricsConfig;
use crate::formatter::StringFormatter;

/// Creates a module with currently added/deleted lines in the Fossil check-out in the current
/// directory.
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("fossil_metrics");
    let config = FossilMetricsConfig::try_load(module.config);

    // As we default to disabled=true, we have to check here after loading our config module,
    // before it was only checking against whatever is in the config starship.toml
    if config.disabled {
        return None;
    }

    vcs::discover_repo_root(context, vcs::Vcs::Fossil)?;

    // Read the total number of added and deleted lines from "fossil diff -i --numstat"
    let output = context
        .exec_cmd("fossil", &["diff", "-i", "--numstat"])?
        .stdout;
    let stats = FossilDiff::parse(&output, config.only_nonzero_diffs);

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "added_style" => Some(Ok(config.added_style)),
                "deleted_style" => Some(Ok(config.deleted_style)),
                _ => None,
            })
            .map(|variable| match variable {
                "added" => Some(Ok(stats.added)),
                "deleted" => Some(Ok(stats.deleted)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `fossil_metrics`:\n{error}");
            return None;
        }
    });

    Some(module)
}

/// Represents the parsed output from a Fossil diff with the -i --numstat option enabled.
#[derive(Debug, PartialEq)]
struct FossilDiff<'a> {
    added: &'a str,
    deleted: &'a str,
}

impl<'a> FossilDiff<'a> {
    /// Parses the output of `fossil diff -i --numstat` as a `FossilDiff` struct.
    pub fn parse(diff_numstat: &'a str, only_nonzero_diffs: bool) -> Self {
        // Fossil formats the last line of the output as "%10d %10d TOTAL over %d changed files\n"
        // where the 1st and 2nd placeholders are the number of added and deleted lines respectively
        let re = Regex::new(r"^\s*(\d+)\s+(\d+) TOTAL over \d+ changed files?$").unwrap();

        let (added, deleted) = diff_numstat
            .lines()
            .last()
            .and_then(|s| re.captures(s))
            .and_then(|caps| {
                let added = match caps.get(1)?.as_str() {
                    "0" if only_nonzero_diffs => "",
                    s => s,
                };

                let deleted = match caps.get(2)?.as_str() {
                    "0" if only_nonzero_diffs => "",
                    s => s,
                };

                Some((added, deleted))
            })
            .unwrap_or_default();

        Self { added, deleted }
    }
}

