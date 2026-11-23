//! One-shot onboarding: `splash install` wires the shell rc + writes a starter
//! home/project dashboard pair from the template catalog. In a TTY we open a small
//! gallery picker so users preview each preset before committing; non-TTY callers must
//! supply `--home-template` / `--project-template` for a deterministic scripted flow.

mod picker;
mod preview;
mod rc;

use std::io::{self, IsTerminal, stdout};
use std::path::{Path, PathBuf};

use crate::paths;
use crate::shell::{self, Shell};
use crate::templates::{self, Template, TemplateContext};

/// Inputs parsed from the CLI. All optional — the installer fills gaps interactively
/// when a TTY is available, or fails with an actionable message otherwise.
#[derive(Debug, Default, Clone)]
pub struct InstallOptions {
    pub shell: Option<Shell>,
    pub home_template: Option<String>,
    pub project_template: Option<String>,
    /// Whether to block the initial paint until every widget has fetched fresh data.
    /// `Some(true)` writes `wait_for_fresh = true`. `None` inherits `false`.
    pub wait: Option<bool>,
    /// Overrides both the home and project dashboard destinations with this explicit
    /// directory. Primarily for tests; users should lean on `SPLASH_HOME`.
    pub config_dir: Option<PathBuf>,
    pub rc_path: Option<PathBuf>,
}

/// Resolved toggle values used during install. Separated from [`InstallOptions`] so the
/// post-picker flow can carry concrete values without re-threading the `Option`s.
#[derive(Debug, Clone, Copy)]
struct SettingsChoices {
    wait: bool,
}

impl SettingsChoices {
    const DEFAULT: Self = Self {
        wait: false,
    };
}

pub fn run(opts: InstallOptions) -> io::Result<()> {
    let shell = resolve_shell(opts.shell)?;
    let home_dir = resolve_home_dir(opts.config_dir.clone())?;
    let destinations = Destinations::for_home_dir(&home_dir);

    // Decide up front what the picker needs to ask. Each axis is independent so a user
    // who passes `--home-template` / `--project-template` but not `--wait` still gets
    // the toggle screens. The settings picker runs whenever the user isn't being
    // explicit about wait (via flags) — each file write is inherently safe
    // because the old content lands in a `.bak` sidecar before the new content is written.
    let need_templates = opts.home_template.is_none() || opts.project_template.is_none();
    let need_settings_picker = opts.wait.is_none();

    let picker_ran = is_tty() && (need_templates || need_settings_picker);
    let (home, project, settings_choices) = if picker_ran {
        let needs = picker::Needs {
            templates: need_templates,
            settings: need_settings_picker,
        };
        let Some(selection) = picker::run(needs)? else {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "install cancelled",
            ));
        };
        let home = match selection.templates.as_ref() {
            Some(p) => p.home,
            None => select_template(opts.home_template.as_deref(), TemplateContext::Home)?,
        };
        let project = match selection.templates.as_ref() {
            Some(p) => p.project,
            None => select_template(opts.project_template.as_deref(), TemplateContext::Project)?,
        };
        let settings = selection
            .settings
            .unwrap_or_else(|| merge_settings_flags(&opts));
        (home, project, settings)
    } else {
        let home = select_template(opts.home_template.as_deref(), TemplateContext::Home)?;
        let project = select_template(opts.project_template.as_deref(), TemplateContext::Project)?;
        let settings = merge_settings_flags(&opts);
        (home, project, settings)
    };

    // Confirmation page — only shown when the user went through the picker. A scripted
    // invocation with explicit flags is its own confirmation; forcing a TTY prompt would
    // break dotfile bootstrap.
    if picker_ran {
        let rc_path = opts
            .rc_path
            .clone()
            .or_else(|| shell::default_rc_path(shell));
        let summary = picker::Summary {
            shell: shell.as_str().to_string(),
            home_template: home.name.to_string(),
            project_template: project.name.to_string(),
            wait: settings_choices.wait,
            files: vec![
                destinations.home_dashboard.clone(),
                destinations.project_dashboard.clone(),
                destinations.settings.clone(),
            ],
            rc_path,
        };
        if !picker::confirm(summary)? {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "install cancelled",
            ));
        }
    }

    ensure_parent_dirs(&destinations)?;
    ensure_secrets_gitignore(&home_dir)?;
    let mut write_report = write_dashboards(&destinations, home, project)?;
    let settings_report = write_settings(
        &destinations.settings,
        settings_choices,
        &mut write_report.backups,
    )?;
    let rc_report = rc::wire_shell_rc(shell, opts.rc_path.clone())?;

    print_summary(SummaryContext {
        shell,
        home,
        project,
        dest: &destinations,
        write: &write_report,
        settings: &settings_report,
        rc: &rc_report,
        settings_choices,
    });
    Ok(())
}

fn resolve_shell(override_shell: Option<Shell>) -> io::Result<Shell> {
    if let Some(shell) = override_shell {
        return Ok(shell);
    }
    if let Some(shell) = shell::detect_shell(|k| std::env::var(k).ok()) {
        return Ok(shell);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "could not detect your shell — pass --shell <bash|zsh|fish|powershell|nushell>",
    ))
}

/// Combines explicit flags with defaults for the scripted / non-TTY flow.
fn merge_settings_flags(opts: &InstallOptions) -> SettingsChoices {
    SettingsChoices {
        wait: opts.wait.unwrap_or(SettingsChoices::DEFAULT.wait),
    }
}

fn select_template(name: Option<&str>, context: TemplateContext) -> io::Result<&'static Template> {
    let Some(name) = name else {
        let label = match context {
            TemplateContext::Home => "--home-template",
            TemplateContext::Project => "--project-template",
        };
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{label} is required in non-interactive mode ({} available)",
                catalog_hint(context)
            ),
        ));
    };
    match templates::find(name) {
        Some(t) if t.context == context => Ok(t),
        Some(t) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "template `{name}` is a {:?} template; expected a {:?} template",
                t.context, context
            ),
        )),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unknown template `{name}` (available: {})",
                catalog_hint(context)
            ),
        )),
    }
}

fn catalog_hint(context: TemplateContext) -> String {
    templates::for_context(context)
        .map(|t| t.name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn resolve_home_dir(override_dir: Option<PathBuf>) -> io::Result<PathBuf> {
    if let Some(d) = override_dir {
        return Ok(d);
    }
    paths::splashboard_home().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not locate $HOME — set $HOME or $SPLASH_HOME",
        )
    })
}

fn is_tty() -> bool {
    stdout().is_terminal() && io::stdin().is_terminal()
}

struct Destinations {
    settings: PathBuf,
    home_dashboard: PathBuf,
    project_dashboard: PathBuf,
}

impl Destinations {
    fn for_home_dir(dir: &Path) -> Self {
        Self {
            settings: dir.join("settings.toml"),
            home_dashboard: dir.join("home.dashboard.toml"),
            project_dashboard: dir.join("project.dashboard.toml"),
        }
    }
}

/// Drops `secrets.toml` into `$HOME/.splash/.gitignore` so dotfiles-in-git users who
/// commit the rest of `.splash/` don't leak their tokens by accident. Idempotent:
/// appends the line only when missing, leaves the rest of the file alone.
fn ensure_secrets_gitignore(home_dir: &Path) -> io::Result<()> {
    let path = home_dir.join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing
        .lines()
        .any(|l| l.trim() == "secrets.toml" || l.trim() == "/secrets.toml")
    {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    if next.is_empty() {
        next.push_str("# Written by `splash install` — keeps tokens out of dotfiles repos.\n");
    }
    next.push_str("secrets.toml\n");
    std::fs::write(&path, next)
}

fn ensure_parent_dirs(dest: &Destinations) -> io::Result<()> {
    for path in [
        &dest.settings,
        &dest.home_dashboard,
        &dest.project_dashboard,
    ] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteOutcome {
    Created,
    Overwrote,
    /// Existing file already matches the new content — nothing written, nothing backed up.
    /// Lets a fresh re-run (e.g. `splash install` with the same flags as last time)
    /// stay noise-free in the summary instead of churning a `.bak` sidecar needlessly.
    Unchanged,
}

struct DashboardWriteReport {
    home: WriteOutcome,
    project: WriteOutcome,
    backups: Vec<PathBuf>,
}

fn write_dashboards(
    dest: &Destinations,
    home: &Template,
    project: &Template,
) -> io::Result<DashboardWriteReport> {
    let mut backups = Vec::new();
    let home_outcome = write_template(&dest.home_dashboard, home.body, &mut backups)?;
    let project_outcome = write_template(&dest.project_dashboard, project.body, &mut backups)?;
    Ok(DashboardWriteReport {
        home: home_outcome,
        project: project_outcome,
        backups,
    })
}

/// Writes `body` to `path`, renaming any existing file to a `.bak` sidecar first. Skips
/// the rename / write entirely when the existing content already matches `body` — that
/// keeps re-running the installer with the same flags from churning noise into `.bak`.
/// The old content is always recoverable from the sidecar, so the installer never asks
/// "are you sure?" before overwriting — `.bak` _is_ the confirmation step.
fn write_template(path: &Path, body: &str, backups: &mut Vec<PathBuf>) -> io::Result<WriteOutcome> {
    if path.exists() {
        if std::fs::read_to_string(path).ok().as_deref() == Some(body) {
            return Ok(WriteOutcome::Unchanged);
        }
        let backup = path.with_extension(backup_extension(path));
        std::fs::rename(path, &backup)?;
        backups.push(backup);
        std::fs::write(path, body)?;
        return Ok(WriteOutcome::Overwrote);
    }
    std::fs::write(path, body)?;
    Ok(WriteOutcome::Created)
}

fn backup_extension(path: &Path) -> String {
    // `home.dashboard.toml` → `home.dashboard.toml.bak`. We can't just call
    // `set_extension("bak")` because that replaces `.toml` and we'd end up with
    // `home.dashboard.bak`, which collides with a different conceptual file.
    format!(
        "{}.bak",
        path.extension().and_then(|e| e.to_str()).unwrap_or("bak")
    )
}

/// Writes / refreshes `settings.toml`. Follows the same `.bak`-sidecar model as
/// dashboards — existing files are renamed out of the way before the new content lands,
/// and a no-op re-run (same flags as last time) short-circuits to `Unchanged` so the
/// summary and `.bak` stash don't churn.
fn write_settings(
    path: &Path,
    choices: SettingsChoices,
    backups: &mut Vec<PathBuf>,
) -> io::Result<WriteOutcome> {
    let body = build_settings_toml(choices);
    write_template(path, &body, backups)
}

/// Emits a settings.toml body from the chosen toggles. Omits every key that matches the
/// built-in default so a user who picks the defaults ends up with a near-empty file they
/// can extend rather than one full of no-op overrides.
fn build_settings_toml(choices: SettingsChoices) -> String {
    let mut out = String::new();
    out.push_str(
        "# Written by `splash install`. User preferences that apply to every dashboard.\n\
         # Safe to edit by hand. Re-running `splash install` leaves this file alone\n\
         # unless you pass `--yes`, which backs it up as `settings.toml.bak` before rewriting.\n\n",
    );

    if choices.wait != SettingsChoices::DEFAULT.wait {
        out.push_str("[general]\n");
        out.push_str(&format!("wait_for_fresh = {}\n\n", choices.wait));
    }
    out
}

struct SummaryContext<'a> {
    shell: Shell,
    home: &'a Template,
    project: &'a Template,
    dest: &'a Destinations,
    write: &'a DashboardWriteReport,
    settings: &'a WriteOutcome,
    rc: &'a rc::RcReport,
    settings_choices: SettingsChoices,
}

fn print_summary(ctx: SummaryContext<'_>) {
    let SummaryContext {
        shell,
        home,
        project,
        dest,
        write,
        settings,
        rc,
        settings_choices,
    } = ctx;
    println!("splash install — done.");
    println!();
    println!("Shell:     {}", shell.as_str());
    println!(
        "Templates: {} (home) · {} (project)",
        home.name, project.name
    );
    println!(
        "Settings:  wait_for_fresh = {}",
        settings_choices.wait,
    );
    println!();
    println!("Files:");
    println!(
        "  {:8} {}",
        label_for_outcome(write.home),
        dest.home_dashboard.display()
    );
    println!(
        "  {:8} {}",
        label_for_outcome(write.project),
        dest.project_dashboard.display()
    );
    println!(
        "  {:8} {}",
        label_for_outcome(*settings),
        dest.settings.display()
    );
    for b in &write.backups {
        println!("  backup   {}", b.display());
    }
    rc.describe();
    println!();
    println!("Next steps:");
    println!(
        "  • Start a new shell (or `source {}`) to see the splash.",
        rc.rc_path_display()
    );
    println!("  • Browse widgets: `splash catalog`.");
    println!(
        "  • Re-run with a different preset: `splash install --home-template <name> --project-template <name>` (old files land in `.bak`)."
    );
}

fn label_for_outcome(outcome: WriteOutcome) -> &'static str {
    match outcome {
        WriteOutcome::Created => "created",
        WriteOutcome::Overwrote => "updated",
        WriteOutcome::Unchanged => "unchanged",
    }
}

