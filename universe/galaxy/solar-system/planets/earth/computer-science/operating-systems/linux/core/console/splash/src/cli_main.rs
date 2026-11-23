use std::future::Future;
use std::io::{self, BufRead, IsTerminal, Write, stdin, stdout};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use clap::{Parser, Subcommand};

use crate::cache::{self, CacheEntry};
use crate::catalog;
use crate::config::{
    self, Config, DashboardConfig, DashboardSource, SettingsConfig, WidgetConfig,
};
use crate::daemon::{self, DashboardKind};
use crate::fetcher::{FetchContext, Registry, Safety};
use crate::install::{self, InstallOptions};
use crate::logging;
use crate::paths;
use crate::render::Registry as RenderRegistry;
use crate::runtime;
use crate::secrets::SecretsConfig;
use crate::shell::{self, Shell};
use crate::trust::{TrustStore, load_dashboard_and_hash};

const OPT_OUT_ENV_VARS: &[&str] = &["CI", "SPLASH_SILENT", "NO_SPLASH"];
const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 16;

#[derive(Parser)]
#[command(version, about = "A customizable terminal splash screen")]
struct Cli {
    /// Render only if the current directory directly resolves to a dashboard (per-dir file or
    /// git repo root); otherwise exit silently. Intended for cd-hook invocations so the splash
    /// shows exactly once per project entry instead of on every subdirectory navigation.
    #[arg(long)]
    on_cd: bool,

    /// Wait for fresh data before drawing (skips the cache-first fast path). Slower startup,
    /// guarantees the frame reflects current values. Equivalent to `general.wait_for_fresh`.
    #[arg(long)]
    wait: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Emit a shell init snippet; source it from your rc file to render on new shells and on cd.
    Init {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// One-shot onboarding: pick a home/project template pair, write them to
    /// `$HOME/.splash/`, and wire your shell rc so the splash renders on every new shell.
    /// Runs an interactive gallery picker in a TTY; non-TTY callers must pass
    /// `--home-template` and `--project-template`. Existing files are always backed up to
    /// `.bak` sidecars before being overwritten, so re-running is safe.
    Install {
        /// Shell to wire. Omit to auto-detect from `$SHELL`.
        #[arg(long, value_enum)]
        shell: Option<Shell>,
        /// Home template name (e.g. `home_splash`). Required in non-TTY mode.
        #[arg(long)]
        home_template: Option<String>,
        /// Project template name (e.g. `project_github`). Required in non-TTY mode.
        #[arg(long)]
        project_template: Option<String>,
        /// Block the first render until every widget has fetched at least once. Writes
        /// `wait_for_fresh = true` into `settings.toml`.
        #[arg(long)]
        wait: bool,
    },
    /// Grant this project-local dashboard permission to run Network widgets. Safe widgets
    /// always run regardless; this is the consent step for anything that talks to the outside
    /// world. Defaults to the nearest `.splash.toml` walking up from the current directory.
    Trust {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
    /// Remove trust for a project-local dashboard. Network widgets in it will render the
    /// "🔒 requires trust" placeholder until re-trusted.
    Revoke {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
    /// Print the currently trusted local dashboards.
    ListTrusted,
    /// Browse the built-in fetcher and renderer catalog — the same info the docs site exposes,
    /// rendered for the terminal. Run without a target for an overview; use
    /// `catalog fetcher [NAME]` or `catalog renderer [NAME]` to narrow.
    Catalog {
        #[command(subcommand)]
        target: Option<CatalogTarget>,
    },
    /// Print embedded license text. Defaults to the third-party dependency bundle
    /// (`THIRDPARTY-LICENSES.md`); pass `--own` for splash's own ISC LICENSE.
    License {
        /// Print splash's own ISC LICENSE instead of the third-party bundle.
        #[arg(long)]
        own: bool,
    },
    /// Internal: run fetchers and update the cache. Spawned as a detached child by the main
    /// splash invocation; not intended to be run directly.
    #[command(hide = true)]
    FetchOnly {
        #[arg(long, value_enum)]
        kind: DashboardKind,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Inspect and manage the disk cache (`$HOME/.splash/cache/`). Useful when a widget
    /// shows stale data, you need to force-refresh a single entry, or you want to know how much
    /// disk the cache is using.
    Cache {
        #[command(subcommand)]
        subcommand: CacheSubcommand,
    },
    /// Persistent dashboard mode: render the splash full-screen and keep it live — realtime
    /// widgets tick, cached widgets refresh on their TTL, and the screen repaints as data
    /// lands. Press `q` or Ctrl-C to exit. Unlike the shell-hook splash this is a deliberate
    /// foreground invocation and ignores `auto_home` / `auto_on_cd`.
    Watch,
    /// Print the dashboard as plain text, no terminal needed.
    Dump {
        which: String,
        #[arg(default_value_t = 70)]
        width: u16,
    },
}

#[derive(Subcommand)]
enum CacheSubcommand {
    /// Print the resolved cache directory.
    Path,
    /// List every cache entry: key, age, TTL, fresh/stale, payload size, kind. Oldest first
    /// (stale / orphan entries surface at the top). Pass `--json` for one JSON object per line.
    List {
        /// Emit one JSON object per entry instead of the human-readable table.
        #[arg(long)]
        json: bool,
    },
    /// Remove cache entries. Pass a widget id to remove only that widget's entry (the widget must
    /// still be configured); otherwise removes every entry plus any leftover `.lock` files.
    ///
    /// Single-widget clear computes the cache key from whichever dashboard would render in the
    /// current directory (project-local if you're inside a configured project, otherwise the
    /// home dashboard). If your widget only lives in the other dashboard, run this from a
    /// directory that resolves to that dashboard, or use `splash cache list` + delete the
    /// file directly.
    ///
    /// Cache key is computed using the widget's default shape and no locale/timezone
    /// overrides. If your config overrides any of those at runtime, the on-disk filename will
    /// differ and this command won't find the entry — fall back to `cache list` + manual
    /// delete in that case.
    ///
    /// May race against an active daemon refresh: if the daemon is mid-write when you clear,
    /// the next refresh re-creates the entry; rerun if that's not what you want. (A future
    /// release may add lock-aware skipping.)
    ///
    /// When clearing all entries, partial removal failures exit non-zero while still reporting
    /// the count that was removed — JSON consumers should read both `removed` and the exit
    /// status.
    Clear {
        /// Widget id to clear. If omitted, clears every entry.
        #[arg(value_name = "WIDGET_ID")]
        widget_id: Option<String>,
        /// Skip the confirmation prompt when clearing all entries. Has no effect when a widget
        /// id is given (single-widget clear is non-interactive).
        #[arg(long)]
        yes: bool,
        /// Emit a JSON summary instead of the human-readable lines.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum CatalogTarget {
    /// List all fetchers, or show details for one when NAME is given.
    #[command(alias = "fetchers")]
    Fetcher { name: Option<String> },
    /// List all renderers, or show details for one when NAME is given.
    #[command(alias = "renderers")]
    Renderer { name: Option<String> },
}

pub fn run<I, T>(argv: I) -> io::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(argv);
    match cli.command {
        Some(Command::Init { shell }) => {
            print!("{}", shell::init_snippet(shell));
            Ok(())
        }
        Some(Command::Install {
            shell,
            home_template,
            project_template,
            wait,
        }) => install::run(InstallOptions {
            shell,
            home_template,
            project_template,
            // Flag absence = "leave it to the picker / default"; flag presence = force the
            // opposite of the default. `--wait` enables wait.
            wait: wait.then_some(true),
            ..Default::default()
        }),
        Some(Command::FetchOnly { kind, path }) => run_async({
            logging::init();
            apply_secrets();
            daemon::run_fetch_only(kind, path.as_deref())
        }),
        Some(Command::Trust { path }) => run_trust(path),
        Some(Command::Dump { which, width }) => run_async({
            logging::init();
            apply_secrets();
            run_dump(which, width)
        }),
        Some(Command::Revoke { path }) => run_revoke(path),
        Some(Command::ListTrusted) => run_list_trusted(),
        Some(Command::Catalog { target }) => run_catalog(target),
        Some(Command::License { own }) => run_license(own),
        Some(Command::Cache { subcommand }) => run_cache(subcommand),
        Some(Command::Watch) => run_async({
            logging::init();
            apply_secrets();
            run_watch()
        }),
        None => {
            if !should_render() {
                return Ok(());
            }
            logging::init();
            apply_secrets();
            // Swallow render errors at the shell-facing boundary so a broken splash never breaks
            // the user's prompt. Internal paths (FetchOnly above) still propagate errors.
            let _ = if cli.on_cd {
                run_async(render_for_cd(cli.wait))
            } else {
                run_async(render_splash(cli.wait))
            };
            Ok(())
        }
    }
}

fn run_async<F>(future: F) -> io::Result<()>
where
    F: Future<Output = io::Result<()>>,
{
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(future)
}

async fn render_splash(wait: bool) -> io::Result<()> {
    if !should_render() {
        return Ok(());
    }
    let source = config::resolve_dashboard_source();
    let (config, ident) = load_full_config(&source)?;
    if matches!(source, DashboardSource::Home) && !config.general.auto_home {
        return Ok(());
    }
    let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
    runtime::run(&config, &source, ident_ref, wait).await
}

async fn render_for_cd(wait: bool) -> io::Result<()> {
    if !should_render() {
        return Ok(());
    }
    let Some(source) = config::resolve_on_cd_dashboard_source() else {
        return Ok(());
    };
    let (config, ident) = load_full_config(&source)?;
    if !config.general.auto_on_cd {
        return Ok(());
    }
    let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
    runtime::run(&config, &source, ident_ref, wait).await
}

/// Entry point for `splash watch`. A deliberate foreground invocation, so it skips the
/// `auto_home` / `auto_on_cd` opt-out gates the shell-hook splash honours — but still needs an
/// interactive terminal for raw mode and the alternate screen.
async fn run_watch() -> io::Result<()> {
    if !stdout().is_terminal() || !stdin().is_terminal() {
        eprintln!("splash watch requires an interactive terminal");
        return Ok(());
    }
    let source = config::resolve_dashboard_source();
    let (config, ident) = load_full_config(&source)?;
    let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
    runtime::watch(&config, ident_ref).await
}

fn should_render() -> bool {
    stdout().is_terminal()
        && stdin().is_terminal()
        && allow_render(|k| std::env::var(k).ok())
        && meets_minimum_size()
}

fn allow_render(env: impl Fn(&str) -> Option<String>) -> bool {
    if OPT_OUT_ENV_VARS.iter().any(|k| env(k).is_some()) {
        return false;
    }
    !matches!(env("TERM").as_deref(), Some("dumb"))
}

fn meets_minimum_size() -> bool {
    ratatui::crossterm::terminal::size()
        .map(|(w, h)| is_large_enough(w, h))
        .unwrap_or(false)
}

fn is_large_enough(width: u16, height: u16) -> bool {
    width >= MIN_WIDTH && height >= MIN_HEIGHT
}

/// Loads settings + the resolved dashboard and composes them into a `Config`. The optional
/// `(path, hash)` identifies a local dashboard for trust gating; HOME-backed sources return
/// `None` so they're treated as implicitly trusted.
fn load_full_config(source: &DashboardSource) -> io::Result<(Config, Option<(PathBuf, String)>)> {
    let settings = load_settings()?;
    let (dashboard, ident) = match source {
        DashboardSource::Local(p) => {
            let (d, h) = load_dashboard_and_hash(p).map_err(io::Error::other)?;
            (d, Some((p.clone(), h)))
        }
        DashboardSource::Home => (load_home_dashboard_or_baked()?, None),
        DashboardSource::Project => (load_project_dashboard_or_baked()?, None),
    };
    Ok((Config::from_parts(settings, dashboard), ident))
}

/// Loads `$HOME/.splash/secrets.toml` and exports each entry as a process env var, but
/// only when the env doesn't already define it. Runs once at startup so every fetcher
/// (`GH_TOKEN`, `TODOIST_TOKEN`, etc.) sees the same view regardless of who reads first.
/// Failures (parse error, unreadable file) log and continue — a stale token shouldn't break
/// the splash, and the user's shell env is still authoritative.
fn apply_secrets() {
    let Some(path) = paths::secrets_path() else {
        return;
    };
    match SecretsConfig::load_or_default(&path) {
        Ok(secrets) => {
            secrets.apply_to_env(
                |k| std::env::var(k).ok(),
                // SAFETY: called once from `main` before any user code reads env. The Tokio
                // worker threads exist but are idle — none of our fetchers / runtime tasks
                // have been spawned yet, so no thread is racing on `getenv`/`setenv`.
                |k, v| unsafe { std::env::set_var(k, v) },
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to load secrets.toml; continuing without it");
        }
    }
}

fn load_settings() -> io::Result<SettingsConfig> {
    match paths::settings_path() {
        Some(p) => SettingsConfig::load_or_default(&p).map_err(io::Error::other),
        None => Ok(SettingsConfig::default_baked()),
    }
}

fn load_home_dashboard_or_baked() -> io::Result<DashboardConfig> {
    load_dashboard_file_or(paths::home_dashboard_path(), DashboardConfig::default_home)
}

fn load_project_dashboard_or_baked() -> io::Result<DashboardConfig> {
    load_dashboard_file_or(
        paths::project_dashboard_path(),
        DashboardConfig::default_project,
    )
}

fn load_dashboard_file_or(
    path: Option<PathBuf>,
    baked: impl FnOnce() -> DashboardConfig,
) -> io::Result<DashboardConfig> {
    let Some(path) = path else {
        return Ok(baked());
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => DashboardConfig::parse(&s)
            .map_err(|e| io::Error::other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(baked()),
        Err(e) => Err(e),
    }
}

fn run_trust(path: Option<PathBuf>) -> io::Result<()> {
    let Some(target) = resolve_trust_target(path) else {
        eprintln!(
            "no project-local dashboard found (run from inside a directory with .splash.toml)"
        );
        return Ok(());
    };
    // Read the bytes once so the hash we show the user matches the hash we store — and so an
    // attacker can't swap the file between "here's what it asks for" and "ok I trust it".
    let (dashboard, hash) = load_dashboard_and_hash(&target).map_err(io::Error::other)?;
    let registry = Registry::with_builtins();
    print_trust_summary(&target, &hash, &dashboard.widgets, &registry)?;
    if !prompt_yes_no("Trust this dashboard?")? {
        println!("not trusted");
        return Ok(());
    }
    let mut store = TrustStore::load();
    store.trust(&target, hash)?;
    println!("trusted: {}", target.display());
    Ok(())
}

fn run_revoke(path: Option<PathBuf>) -> io::Result<()> {
    let Some(target) = resolve_trust_target(path) else {
        eprintln!("no project-local dashboard found");
        return Ok(());
    };
    let mut store = TrustStore::load();
    let display = sanitize_for_display(&target.display().to_string());
    if store.revoke(&target)? {
        println!("revoked: {display}");
    } else {
        println!("not trusted: {display}");
    }
    Ok(())
}

fn run_list_trusted() -> io::Result<()> {
    let store = TrustStore::load();
    for entry in store.list() {
        println!(
            "{}  {}",
            entry.sha256,
            sanitize_for_display(&entry.path.display().to_string())
        );
    }
    Ok(())
}

fn run_cache(subcommand: CacheSubcommand) -> io::Result<()> {
    let Some(dir) = paths::cache_dir() else {
        return Err(io::Error::other("no cache dir resolved (is $HOME set?)"));
    };
    match subcommand {
        CacheSubcommand::Path => {
            println!("{}", dir.display());
            Ok(())
        }
        CacheSubcommand::List { json } => run_cache_list(&dir, json),
        CacheSubcommand::Clear {
            widget_id,
            yes,
            json,
        } => run_cache_clear(&dir, widget_id, yes, json),
    }
}

/// One row of the `cache list` output. Also serialized as a JSON object when `--json` is used.
#[derive(serde::Serialize)]
struct CacheListRow {
    /// Cache key — the filename stem, e.g. `clock-3f2a1c8b`.
    key: String,
    /// `entry` for `.json` payloads, `lock` for orphan `.lock` files.
    kind: &'static str,
    /// Seconds since the file was last written; for lock files this is the lock age.
    age_seconds: u64,
    /// TTL declared by the entry, in seconds. Zero for lock files.
    ttl_seconds: u64,
    /// `fresh` when `age_seconds < ttl_seconds`, otherwise `stale`. Lock files report `n/a`.
    freshness: &'static str,
    /// Size of the on-disk file, in bytes.
    size_bytes: u64,
    /// Outcome of the fetch that produced this entry — `ok` / `err` / `timeout`. `n/a` for locks.
    outcome: &'static str,
}

fn run_cache_list(dir: &Path, json: bool) -> io::Result<()> {
    let mut rows = collect_cache_rows(dir)?;
    // Oldest first by default — surfaces stale and orphan files at the top where they're easier
    // to spot when scrolling a long list.
    rows.sort_by_key(|r| std::cmp::Reverse(r.age_seconds));

    if json {
        for row in &rows {
            let line = serde_json::to_string(row).map_err(io::Error::other)?;
            println!("{line}");
        }
        return Ok(());
    }

    if rows.is_empty() {
        println!("(empty)");
        return Ok(());
    }

    println!(
        "{:<40}  {:<6}  {:>10}  {:>10}  {:<8}  {:>10}  OUTCOME",
        "KEY", "KIND", "AGE(s)", "TTL(s)", "STATE", "SIZE(B)"
    );
    for row in &rows {
        println!(
            "{:<40}  {:<6}  {:>10}  {:>10}  {:<8}  {:>10}  {}",
            truncate(&row.key, 40),
            row.kind,
            row.age_seconds,
            row.ttl_seconds,
            row.freshness,
            row.size_bytes,
            row.outcome,
        );
    }
    Ok(())
}

fn collect_cache_rows(dir: &Path) -> io::Result<Vec<CacheListRow>> {
    let mut rows = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // Treat a missing cache dir the same as an empty one — operator hasn't run splash
        // yet, no cached data, nothing to report.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(rows),
        Err(e) => return Err(e),
    };
    let now = SystemTime::now();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let age_seconds = metadata
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size_bytes = metadata.len();
        match ext {
            "json" => {
                let entry = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<CacheEntry>(&s).ok());
                let (ttl, freshness, outcome) = entry
                    .as_ref()
                    .map(|e| {
                        let ttl = e.ttl_seconds;
                        let freshness = if e.is_fresh() { "fresh" } else { "stale" };
                        let outcome = match e.kind {
                            cache::CacheEntryKind::Ok => "ok",
                            cache::CacheEntryKind::Err => "err",
                            cache::CacheEntryKind::Timeout => "timeout",
                        };
                        (ttl, freshness, outcome)
                    })
                    .unwrap_or((0, "unreadable", "unreadable"));
                rows.push(CacheListRow {
                    key: stem.to_string(),
                    kind: "entry",
                    age_seconds,
                    ttl_seconds: ttl,
                    freshness,
                    size_bytes,
                    outcome,
                });
            }
            "lock" => {
                rows.push(CacheListRow {
                    key: stem.to_string(),
                    kind: "lock",
                    age_seconds,
                    ttl_seconds: 0,
                    freshness: "n/a",
                    size_bytes,
                    outcome: "n/a",
                });
            }
            // Anything else (`.tmp` leftovers, future formats) is skipped silently to keep the
            // output focused on the things users can act on.
            _ => continue,
        }
    }
    Ok(rows)
}

fn run_cache_clear(dir: &Path, widget_id: Option<String>, yes: bool, json: bool) -> io::Result<()> {
    match widget_id {
        Some(id) => clear_one(dir, &id, json),
        None => clear_all(dir, yes, json),
    }
}

fn clear_all(dir: &Path, yes: bool, json: bool) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e.collect::<Result<Vec<_>, _>>()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let targets: Vec<PathBuf> = entries
        .into_iter()
        .filter(|e| e.metadata().map(|m| m.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("json") | Some("lock")
            )
        })
        .collect();

    if targets.is_empty() {
        if json {
            println!(r#"{{"removed":0}}"#);
        } else {
            println!("(empty)");
        }
        return Ok(());
    }

    if !yes
        && !prompt_yes_no(&format!(
            "Remove all {} cache file(s) from {}?",
            targets.len(),
            dir.display()
        ))?
    {
        println!("cancelled");
        return Ok(());
    }

    let mut removed = 0usize;
    let mut errors = Vec::new();
    for path in &targets {
        match std::fs::remove_file(path) {
            Ok(()) => removed += 1,
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }

    if json {
        let payload = serde_json::json!({
            "removed": removed,
            "errors": errors,
        });
        println!("{payload}");
    } else {
        println!("removed {removed} file(s)");
        for err in &errors {
            eprintln!("error: {err}");
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} file(s) could not be removed",
            errors.len()
        )))
    }
}

fn clear_one(dir: &Path, widget_id: &str, json: bool) -> io::Result<()> {
    // Use option 1 from the issue: compute the key from the current config. Falls back to the
    // home dashboard so the command works outside any project-local context (common case for
    // ad-hoc debugging on $HOME/.splash/cache/).
    let source = config::resolve_dashboard_source();
    let (config, _ident) = load_full_config(&source)?;
    let widget = config
        .widgets
        .iter()
        .find(|w| w.id == widget_id)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no widget '{widget_id}' in current config ({:?}). \
Cache key can only be computed for widgets that are still configured; \
to remove an orphaned entry run `splash cache list` and delete \
the file directly.",
                    source
                ),
            )
        })?;
    let registry = Registry::with_builtins();
    let fetcher = registry.get_cached(&widget.fetcher).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "widget '{}' uses fetcher '{}' which is not in the registry \
(realtime widgets don't have disk-cached entries)",
                widget.id, widget.fetcher,
            ),
        )
    })?;
    let key = fetcher.cache_key(&FetchContext {
        widget_id: widget.id.clone(),
        format: widget.format.clone(),
        // Cache key derivation does not actually look at timeout; pass zero to be explicit.
        timeout: std::time::Duration::from_secs(0),
        file_format: widget.file_format.clone(),
        // Shape is part of the key but we don't know which shape will be used at fetch time
        // without doing a layout pass. The default-shape path matches what `runtime::fetch_all`
        // uses when no widget-specific shape override is in play.
        shape: Some(fetcher.default_shape()),
        options: widget.options.clone(),
        // Locale and timezone do NOT participate in `default_cache_key` (see
        // `fetcher/mod.rs::default_cache_key`); a None pair here is correct and intentional,
        // and any fetcher that overrides `cache_key` to consume these would derive a key that
        // won't match a runtime-produced entry anyway.
        timezone: None,
        locale: None,
    });

    // Same sanitization function that `cache.rs` uses to write entries — keeping a second copy
    // would silently desync if the rule ever tightened.
    let sanitized = cache::sanitize(&key);
    let entry_path = dir.join(format!("{sanitized}.json"));
    let lock_path = dir.join(format!("{sanitized}.lock"));
    let mut removed = Vec::new();
    let mut missing = Vec::new();
    for path in [&entry_path, &lock_path] {
        match std::fs::remove_file(path) {
            Ok(()) => removed.push(path.display().to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                missing.push(path.display().to_string());
            }
            Err(e) => {
                return Err(io::Error::other(format!(
                    "removing {}: {e}",
                    path.display()
                )));
            }
        }
    }

    if json {
        let payload = serde_json::json!({
            "widget_id": widget_id,
            "key": key,
            "removed": removed,
            "missing": missing,
        });
        println!("{payload}");
    } else if removed.is_empty() {
        // Defensive: if `cache list` shows the widget on disk but `clear` reports
        // nothing removed, the on-disk entry was written with a non-default shape /
        // locale / timezone (overridden at runtime). The key we computed here uses
        // `fetcher.default_shape()` with locale/timezone = None — that match the
        // common case but not overrides. `cache list` + delete-by-key is the
        // fallback path in that scenario.
        println!(
            "widget '{widget_id}' had no cache entry under key {key}\n\
             (computed using default shape + no locale/timezone overrides; \
              if your config overrides those, run `splash cache list` and \
              delete the matching file directly)"
        );
    } else {
        println!(
            "removed {} file(s) for widget '{widget_id}' (key {key})",
            removed.len()
        );
        for path in &removed {
            println!("  {path}");
        }
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    // `chars().count()` walks the string once but is O(n) — fine here since `max` is at most
    // a column width (e.g. 40). The earlier byte-slice version (`&s[..max-1]`) would panic on a
    // multi-byte codepoint straddling the boundary; cache keys are alnum + dash today, but the
    // table renders arbitrary user content (widget ids in --help output, fetcher names, ...).
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let prefix: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{prefix}…")
    }
}

fn run_catalog(target: Option<CatalogTarget>) -> io::Result<()> {
    let fetchers = Registry::with_builtins();
    let renderers = RenderRegistry::with_builtins();
    let output = match target {
        None => Ok(catalog::overview(&fetchers, &renderers)),
        Some(CatalogTarget::Fetcher { name: None }) => Ok(catalog::fetcher_list(&fetchers)),
        Some(CatalogTarget::Fetcher { name: Some(n) }) => {
            catalog::fetcher_detail(&n, &fetchers, &renderers)
        }
        Some(CatalogTarget::Renderer { name: None }) => Ok(catalog::renderer_list(&renderers)),
        Some(CatalogTarget::Renderer { name: Some(n) }) => {
            catalog::renderer_detail(&n, &renderers, &fetchers)
        }
    };
    match output {
        Ok(s) => {
            print!("{s}");
            Ok(())
        }
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    }
}

fn run_license(own: bool) -> io::Result<()> {
    let text = if own {
        include_str!("../LICENSE")
    } else {
        ""
    };
    print!("{text}");
    Ok(())
}

fn resolve_trust_target(override_path: Option<PathBuf>) -> Option<PathBuf> {
    override_path.or_else(config::resolve_local_dashboard_path)
}

fn print_trust_summary(
    path: &Path,
    hash: &str,
    widgets: &[WidgetConfig],
    registry: &Registry,
) -> io::Result<()> {
    // Paths and widget ids/fetchers flow into the terminal unmodified; sanitize control chars so
    // a malicious config can't spoof the prompt with ANSI escape sequences.
    println!(
        "Dashboard: {}",
        sanitize_for_display(&path.display().to_string())
    );
    println!("sha256: {hash}");
    println!();

    let mut declared = 0usize;
    for w in widgets {
        let Some(fetcher) = registry.get(&w.fetcher) else {
            continue;
        };
        let label = match fetcher.safety() {
            Safety::Safe => continue,
            Safety::Network => "network",
            Safety::Exec => "exec",
        };
        if declared == 0 {
            println!("This dashboard requests:");
        }
        println!(
            "  - {label:<7}: {} ({})",
            sanitize_for_display(&w.id),
            sanitize_for_display(&w.fetcher)
        );
        declared += 1;
    }
    if declared == 0 {
        println!("(no Network or Exec widgets — nothing to trust)");
    }
    println!();
    Ok(())
}

/// Replaces control characters (including ANSI escape initiators) with U+FFFD so a hostile
/// config can't draw over the trust prompt to make it look like something else.
fn sanitize_for_display(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() {
                char::REPLACEMENT_CHARACTER
            } else {
                c
            }
        })
        .collect()
}

fn prompt_yes_no(question: &str) -> io::Result<bool> {
    print!("{question} [y/N] ");
    stdout().flush()?;
    let mut line = String::new();
    stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
}


async fn run_dump(which: String, width: u16) -> io::Result<()> {
    let source = if which == "project" {
        DashboardSource::Project
    } else {
        DashboardSource::Home
    };
    let (config, ident) = load_full_config(&source)?;
    let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
    let text = runtime::dump(&config, &source, ident_ref, width)?;
    println!("{}", text);
    Ok(())
}
