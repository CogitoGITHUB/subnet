//! Shared helpers for the `claude_*` family — formatting helpers and the JSONL discovery
//! roots used by `claude_code_usage`.
//!
//! Pricing has moved to the shared [`crate::fetcher::llm_pricing`] module so claude and codex
//! consume the same snapshot. See its module doc for the fetch / fallback story.

use std::path::PathBuf;

/// "$12.34" / "$0.05" / "$1.2k" — compact enough to fit a Badge or a Text headline.
pub fn format_cost(usd: f64) -> String {
    if usd >= 1_000.0 {
        format!("${:.1}k", usd / 1_000.0)
    } else if usd >= 10.0 {
        format!("${:.2}", usd)
    } else if usd > 0.0 {
        format!("${:.3}", usd)
    } else {
        "$0.00".into()
    }
}

/// "3.4M" / "812k" / "42" — same compact strategy for raw token counts.
pub fn format_tokens(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{}k", count / 1_000)
    } else {
        count.to_string()
    }
}

/// Claude Code's per-CWD project directories. The CLI uses `~/.claude/projects/<encoded-cwd>/`
/// by default; the older `~/.config/claude/projects/` is still seen on Linux hosts upgraded from
/// pre-3.x. `CLAUDE_CONFIG_DIR` accepts a comma-separated list so power users with multiple
/// installs can point at every root.
pub fn discover_jsonl_dirs() -> Vec<PathBuf> {
    if let Ok(raw) = std::env::var("CLAUDE_CONFIG_DIR") {
        return raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(s).join("projects"))
            .collect();
    }
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [home.join(".claude"), home.join(".config").join("claude")]
        .into_iter()
        .map(|p| p.join("projects"))
        .filter(|p| p.is_dir())
        .collect()
}

/// Last path component of a Claude `cwd` field — the bare project name without filesystem noise.
/// Empty cwd falls back to `"(unknown)"` so renderers always get a non-empty key.
pub fn project_name_from_cwd(cwd: &str) -> String {
    PathBuf::from(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(unknown)".into())
}

