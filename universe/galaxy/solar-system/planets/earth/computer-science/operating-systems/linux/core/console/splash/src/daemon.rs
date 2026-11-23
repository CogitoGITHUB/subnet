use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use clap::ValueEnum;
use tokio::process::{Child, Command};

use crate::config::{Config, DashboardConfig, DashboardSource, SettingsConfig};
use crate::paths;
use crate::runtime;

/// Which dashboard the parent resolved. Passed to the daemon subprocess so it loads the same
/// source without having to re-resolve from scratch (avoids drift if CWD changed between
/// parent spawn and daemon start).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DashboardKind {
    Local,
    Home,
    Project,
}

impl DashboardSource {
    pub fn kind(&self) -> DashboardKind {
        match self {
            Self::Local(_) => DashboardKind::Local,
            Self::Home => DashboardKind::Home,
            Self::Project => DashboardKind::Project,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Local(p) => Some(p),
            _ => None,
        }
    }
}

/// Entry point for the `fetch-only` subcommand. Run by the detached child process: resolves the
/// dashboard, drives the fetchers, writes the cache, and exits. The parent splashboard invocation
/// either detaches from this child and exits, or (in `--wait` mode) blocks on it with a deadline.
pub async fn run_fetch_only(kind: DashboardKind, path: Option<&Path>) -> io::Result<()> {
    let (dashboard, ident) = load_dashboard(kind, path)?;
    let settings = load_settings_or_default();
    let config = Config::from_parts(settings, dashboard);
    let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
    let cache = crate::cache::Cache::open_default();
    let backend = cache.as_ref().map(|c| c as &dyn crate::cache::CacheBackend);
    runtime::fetch_and_persist(&config, ident_ref, backend).await;
    Ok(())
}

pub fn spawn_fetch_daemon(source: &DashboardSource) -> io::Result<Child> {
    let exe = std::env::current_exe()?;
    let args = fetch_daemon_args(source);
    let mut cmd = Command::new(exe);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
    detach(cmd.as_std_mut());
    cmd.spawn()
}

pub(crate) fn fetch_daemon_args(source: &DashboardSource) -> Vec<std::ffi::OsString> {
    let kind = match source.kind() {
        DashboardKind::Local => "local",
        DashboardKind::Home => "home",
        DashboardKind::Project => "project",
    };
    let mut args: Vec<std::ffi::OsString> = vec!["fetch-only".into(), "--kind".into(), kind.into()];
    if let Some(p) = source.path() {
        args.push("--path".into());
        args.push(p.as_os_str().to_os_string());
    }
    args
}

fn load_dashboard(
    kind: DashboardKind,
    path: Option<&Path>,
) -> io::Result<(DashboardConfig, Option<(PathBuf, String)>)> {
    match kind {
        DashboardKind::Local => {
            let p =
                path.ok_or_else(|| io::Error::other("fetch-only --kind local requires --path"))?;
            let (d, h) = crate::trust::load_dashboard_and_hash(p)?;
            Ok((d, Some((p.to_path_buf(), h))))
        }
        DashboardKind::Home => Ok((load_home_dashboard_or_baked(), None)),
        DashboardKind::Project => Ok((load_project_dashboard_or_baked(), None)),
    }
}

fn load_home_dashboard_or_baked() -> DashboardConfig {
    paths::home_dashboard_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| DashboardConfig::parse(&s).ok())
        .unwrap_or_else(DashboardConfig::default_home)
}

fn load_project_dashboard_or_baked() -> DashboardConfig {
    paths::project_dashboard_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| DashboardConfig::parse(&s).ok())
        .unwrap_or_else(DashboardConfig::default_project)
}

fn load_settings_or_default() -> SettingsConfig {
    paths::settings_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| SettingsConfig::parse(&s).ok())
        .unwrap_or_else(SettingsConfig::default_baked)
}

#[cfg(unix)]
fn detach(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // New process group so SIGINT to the shell doesn't tear the daemon down mid-fetch.
    cmd.process_group(0);
}

#[cfg(windows)]
fn detach(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach(_cmd: &mut std::process::Command) {}

