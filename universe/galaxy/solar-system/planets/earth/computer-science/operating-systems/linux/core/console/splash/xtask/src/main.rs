//! Workspace tooling for splashboard. Run via `cargo xtask`.
//!
//! Reads the built-in fetcher / renderer registries and writes Markdown reference pages (one
//! per fetcher, one per renderer, plus an overview). Also renders full dashboard TOMLs to
//! inline HTML using each fetcher's `sample_body`, so the docs site can show real splash
//! output without ever touching the network.

use std::path::{Path, PathBuf};
use std::{fs, io::Write};

use anyhow::{Context, Result};
use clap::Parser;

mod dashboard_snapshot;
mod gallery;
mod gen_matrix;
mod html_snapshot;
mod preset_index;
mod snapshots;

#[derive(Parser)]
#[command(
    version,
    about = "Generate splashboard's fetcher/renderer reference pages"
)]
struct Cli {
    /// Destination directory. Writes `<out>/matrix.md`, `<out>/fetchers/<name>.md`,
    /// `<out>/renderers/<name>.md`. Default lands inside the Starlight content tree so
    /// `astro build` picks the pages up without further wiring.
    #[arg(long, default_value = "docs-site/src/content/docs/reference")]
    out: PathBuf,

    /// Destination directory for rendered dashboard HTML snippets, imported via `?raw` by
    /// `index.mdx` and other landing surfaces. One file per entry in `DASHBOARD_SNAPSHOTS`.
    #[arg(long, default_value = "docs-site/src/assets/rendered")]
    rendered_out: PathBuf,

    /// Source directory of `examples/usecases/*.toml` — maintainer-curated environment-specific
    /// dashboards. Each file's `[showcase]` table supplies title / description / context.
    #[arg(long, default_value = "examples/usecases")]
    usecases_src: PathBuf,

    /// Source directory of `examples/community/*.toml` — user-submitted dashboards. Same TOML
    /// shape as `usecases_src`, looser curation tier.
    #[arg(long, default_value = "examples/community")]
    community_src: PathBuf,
}

/// Every preset renders at the same 120 × 42 cell canvas so the embedded snapshots read as a
/// uniform gallery under `.splash-landing`. 42 = project_github's natural height; shorter
/// presets get blank theme-bg rows below their content. `html_snapshot::buffer_to_html_dashboard`
/// then top-aligns the rendered content (rotating any leading blank rows to the bottom) and
/// prepends a fixed `PADDING_TOP_ROWS` breathing-room band — so the final HTML lands at a
/// consistent height with consistent top padding regardless of how dense the preset is.
const SNAPSHOT_WIDTH: u16 = 120;
const SNAPSHOT_HEIGHT: u16 = 42;

const DASHBOARD_SNAPSHOTS: &[(&str, &str)] = &[
    ("src/templates/home_splash.toml", "home_splash.html"),
    ("src/templates/home_daily.toml", "home_daily.html"),
    ("src/templates/home_github.toml", "home_github.html"),
    ("src/templates/home_minimal.toml", "home_minimal.html"),
    ("src/templates/home_feed.toml", "home_feed.html"),
    ("src/templates/project_splash.toml", "project_splash.html"),
    ("src/templates/project_github.toml", "project_github.html"),
    ("src/templates/project_minimal.toml", "project_minimal.html"),
    (
        "src/templates/project_codebase.toml",
        "project_codebase.html",
    ),
];

fn main() -> Result<()> {
    let cli = Cli::parse();
    gen_matrix::run(&cli.out)?;
    render_dashboards(&cli.rendered_out)?;
    preset_index::run(&cli.rendered_out)?;
    gallery::run(
        &cli.usecases_src,
        &cli.rendered_out.join("usecases"),
        SNAPSHOT_WIDTH,
        SNAPSHOT_HEIGHT,
    )?;
    gallery::run(
        &cli.community_src,
        &cli.rendered_out.join("community"),
        SNAPSHOT_WIDTH,
        SNAPSHOT_HEIGHT,
    )?;
    Ok(())
}

fn render_dashboards(out_dir: &Path) -> Result<()> {
    // Realtime fetchers (invoked by dashboard_snapshot::sample_payloads) peek at env vars to
    // guess the terminal / shell / host. Scrub the signals that vary by dev machine so the
    // committed snapshot is deterministic — "terminal" / "shell" fallbacks, empty hostname.
    scrub_host_env();
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    for (config, output_name) in DASHBOARD_SNAPSHOTS {
        let html = dashboard_snapshot::render_config_html(
            Path::new(config),
            SNAPSHOT_WIDTH,
            SNAPSHOT_HEIGHT,
        )?;
        let dest = out_dir.join(output_name);
        let mut f =
            fs::File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        f.write_all(html.as_bytes())
            .with_context(|| format!("write {}", dest.display()))?;
        println!("wrote {}", dest.display());
    }
    Ok(())
}

fn scrub_host_env() {
    for key in [
        "WT_SESSION",
        "GHOSTTY_RESOURCES_DIR",
        "KITTY_WINDOW_ID",
        "ALACRITTY_WINDOW_ID",
        "ALACRITTY_LOG",
        "WEZTERM_PANE",
        "TERM_PROGRAM",
        // `media_image`'s picker treats `LC_TERMINAL=iTerm2` as a hint to switch from the
        // halfblocks protocol to iTerm2's OSC 1337 inline-image escape; that escape leaks
        // into the test buffer when xtask runs on a Mac iTerm host. Scrub it so the
        // committed snapshots stay halfblocks-rendered regardless of dev machine.
        "LC_TERMINAL",
        "LC_TERMINAL_VERSION",
    ] {
        // SAFETY: xtask is a single-threaded CLI entry point; no other threads can race this.
        unsafe { std::env::remove_var(key) };
    }
}

