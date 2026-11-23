//! Cross-platform system fetchers backed by `sysinfo`.
//!
//! All are `Safety::Safe` — local kernel counters only, no network or exec. Realtime fetchers
//! cache a `Mutex<Option<System>>` (`None` only on targets `sysinfo` doesn't support) and refresh only the fields they need per frame, so the `<1ms
//! infallible` contract holds even as many widgets sample the same source.

use std::sync::{Arc, OnceLock};

use sysinfo::{Disk, System};

use crate::payload::{Bar, Body, Entry, ErrorData, ErrorKind, Payload, TextData};

use super::{Fetcher, RealtimeFetcher};

pub mod dmi;

#[cfg(test)]
pub(crate) mod test_helpers;

mod info_bios;
mod info_board;
mod info_cpu;
mod info_desktop;
mod info_env;
mod info_host;
mod info_kernel;
mod info_locale;
mod info_machine;
mod info_memory;
mod info_timezone;
mod monitor_battery;
mod monitor_cpu;
mod monitor_disk;
mod monitor_host;
mod monitor_load;
mod monitor_memory;
mod monitor_processes;
mod monitor_uptime;
pub(crate) mod sprites;

pub use info_bios::SystemInfoBios;
pub use info_board::SystemInfoBoard;
pub use info_cpu::SystemInfoCpu;
pub use info_desktop::SystemInfoDesktop;
pub use info_env::SystemInfoEnv;
pub use info_host::SystemInfoHost;
pub use info_kernel::SystemInfoKernel;
pub use info_locale::SystemInfoLocale;
pub use info_machine::SystemInfoMachine;
pub use info_memory::SystemInfoMemory;
pub use info_timezone::SystemInfoTimezone;
pub use monitor_battery::SystemMonitorBattery;
pub use monitor_cpu::SystemMonitorCpu;
pub use monitor_disk::SystemMonitorDisk;
pub use monitor_host::SystemMonitorHost;
pub use monitor_load::SystemMonitorLoad;
pub use monitor_memory::SystemMonitorMemory;
pub use monitor_processes::SystemMonitorProcesses;
pub use monitor_uptime::SystemMonitorUptime;

pub fn realtime_fetchers() -> Vec<Arc<dyn RealtimeFetcher>> {
    vec![
        Arc::new(SystemMonitorHost::new()),
        Arc::new(SystemInfoHost),
        Arc::new(SystemInfoCpu),
        Arc::new(SystemInfoMemory),
        Arc::new(SystemInfoKernel),
        Arc::new(SystemInfoMachine),
        Arc::new(SystemInfoBoard),
        Arc::new(SystemInfoBios),
        Arc::new(SystemInfoLocale),
        Arc::new(SystemInfoTimezone),
        Arc::new(SystemInfoEnv),
        Arc::new(SystemInfoDesktop),
        Arc::new(SystemMonitorCpu::new()),
        Arc::new(SystemMonitorMemory::new()),
        Arc::new(SystemMonitorUptime),
        Arc::new(SystemMonitorLoad),
        Arc::new(SystemMonitorProcesses::new()),
        Arc::new(SystemMonitorBattery::new()),
    ]
}

pub fn cached_fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(SystemMonitorDisk)]
}

#[derive(Debug, Clone)]
struct CpuInfo {
    model: String,
    vendor: String,
    frequency_mhz: u64,
}

fn cached_cpu_info() -> &'static CpuInfo {
    static CACHE: OnceLock<CpuInfo> = OnceLock::new();
    CACHE.get_or_init(|| {
        let sys = refreshed_system(System::refresh_cpu_usage);
        let cpu = sys.as_ref().and_then(|s| s.cpus().first());
        let frequency_mhz = cpu
            .map(|c| c.frequency())
            .filter(|f| *f > 0)
            .or_else(cpu_mhz_from_proc_cpuinfo)
            .unwrap_or(0);
        CpuInfo {
            model: cpu
                .map(|c| c.brand().trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".into()),
            vendor: cpu
                .map(|c| c.vendor_id().trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".into()),
            frequency_mhz,
        }
    })
}

/// Fallback for environments where sysinfo's `Cpu::frequency()` returns 0 (WSL, some
/// containers). Reads the first `cpu MHz` line from `/proc/cpuinfo` and rounds to MHz.
fn cpu_mhz_from_proc_cpuinfo() -> Option<u64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    text.lines()
        .find_map(|line| {
            line.strip_prefix("cpu MHz")
                .and_then(|rest| rest.split(':').nth(1))
        })
        .and_then(|val| val.trim().parse::<f64>().ok())
        .map(|mhz| mhz.round() as u64)
        .filter(|m| *m > 0)
}

fn format_cpu_cores() -> String {
    let physical = System::physical_core_count().unwrap_or(0);
    let logical = refreshed_system(System::refresh_cpu_usage).map_or(0, |s| s.cpus().len());
    match (physical, logical) {
        (0, 0) => "unknown".into(),
        (0, l) => format!("{l} threads"),
        (p, l) if p == l => format!("{p} cores"),
        (p, l) => format!("{p} cores / {l} threads"),
    }
}

fn format_cpu_frequency(mhz: u64) -> String {
    match mhz {
        0 => "unknown".into(),
        m if m >= 1000 => format!("{:.2} GHz", m as f64 / 1000.0),
        m => format!("{m} MHz"),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MemoryTotals {
    memory: u64,
    swap: u64,
}

fn cached_memory_totals() -> MemoryTotals {
    static CACHE: OnceLock<MemoryTotals> = OnceLock::new();
    *CACHE.get_or_init(|| {
        refreshed_system(System::refresh_memory)
            .map(|sys| MemoryTotals {
                memory: sys.total_memory(),
                swap: sys.total_swap(),
            })
            .unwrap_or_default()
    })
}

fn kernel_name() -> String {
    match std::env::consts::OS {
        "linux" => "Linux".into(),
        "macos" => "Darwin".into(),
        "windows" => "Windows NT".into(),
        "freebsd" => "FreeBSD".into(),
        "openbsd" => "OpenBSD".into(),
        "netbsd" => "NetBSD".into(),
        other if !other.is_empty() => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_else(|| other.into())
        }
        _ => "unknown".into(),
    }
}

fn dmi_or_na(value: Option<String>) -> String {
    value.unwrap_or_else(|| "n/a".into())
}

fn detect_locale() -> String {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "C".into())
}

fn detect_timezone() -> String {
    if let Some(tz) = std::env::var("TZ").ok().filter(|v| !v.is_empty()) {
        return tz;
    }
    if let Ok(path) = std::fs::read_link("/etc/localtime") {
        let s = path.to_string_lossy();
        if let Some(idx) = s.find("/zoneinfo/") {
            return s[idx + "/zoneinfo/".len()..].to_string();
        }
        if let Some(name) = path.file_name() {
            return name.to_string_lossy().into_owned();
        }
    }
    "unknown".into()
}

fn env_basename(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .and_then(|s| {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| {
                std::path::Path::new(trimmed)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| trimmed.to_string())
            })
        })
        .unwrap_or_else(|| fallback.into())
}

fn detect_desktop_environment() -> String {
    if cfg!(target_os = "macos") {
        return "Aqua".into();
    }
    if cfg!(target_os = "windows") {
        return "Windows".into();
    }
    ["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION", "GDMSESSION"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .map(|s| s.split(':').next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "tty".into())
}

fn detect_window_manager() -> String {
    if cfg!(target_os = "macos") {
        return "Quartz".into();
    }
    if cfg!(target_os = "windows") {
        return "DWM".into();
    }
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        return std::env::var("XDG_SESSION_DESKTOP")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "wayland".into());
    }
    if std::env::var("DISPLAY").is_ok() {
        return std::env::var("XDG_SESSION_DESKTOP")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "x11".into());
    }
    "tty".into()
}

fn detect_init_system() -> String {
    if cfg!(target_os = "macos") {
        return "launchd".into();
    }
    if cfg!(target_os = "windows") {
        return "wininit".into();
    }
    if cfg!(target_os = "linux")
        && let Ok(comm) = std::fs::read_to_string("/proc/1/comm")
    {
        let name = comm.trim();
        if !name.is_empty() {
            return name.into();
        }
    }
    "unknown".into()
}

/// Detect the terminal emulator via well-known env vars. Env vars are the only signal available
/// without spawning a subprocess, so this is a best-effort match with a generic fallback.
fn detect_terminal() -> String {
    let env = |k: &str| std::env::var(k).ok();
    if env("WT_SESSION").is_some() {
        return "Windows Terminal".into();
    }
    if env("GHOSTTY_RESOURCES_DIR").is_some() {
        return "Ghostty".into();
    }
    if env("KITTY_WINDOW_ID").is_some() || env("TERM").as_deref() == Some("xterm-kitty") {
        return "Kitty".into();
    }
    if env("ALACRITTY_WINDOW_ID").is_some() || env("ALACRITTY_LOG").is_some() {
        return "Alacritty".into();
    }
    if env("WEZTERM_PANE").is_some() {
        return "WezTerm".into();
    }
    match env("TERM_PROGRAM").as_deref() {
        Some("iTerm.app") => "iTerm2".into(),
        Some("Apple_Terminal") => "Terminal".into(),
        Some("ghostty") => "Ghostty".into(),
        Some("WezTerm") => "WezTerm".into(),
        Some("Hyper") => "Hyper".into(),
        Some("vscode") => "VS Code".into(),
        Some(other) if !other.is_empty() => other.into(),
        _ => "(unset)".into(),
    }
}

fn detect_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| {
            std::path::Path::new(&s)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "(unset)".into())
}

fn parse_options<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&toml::Value>,
) -> Result<T, String> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

fn options_placeholder(msg: &str) -> Payload {
    payload(Body::Text(TextData {
        value: format!("⚠ {msg}"),
    }))
}

/// Shown by realtime fetchers when `sysinfo` can't build a `System` (unsupported target).
/// `Body::Error` bypasses renderer shape validation, so the warning survives any `render`.
fn unsupported_placeholder() -> Payload {
    payload(Body::Error(ErrorData {
        kind: ErrorKind::Fetch,
        message: "⚠ sysinfo: unsupported platform".into(),
        detail: None,
    }))
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

/// A fresh `System` with `refresh` applied, or `None` on targets `sysinfo` doesn't support.
fn refreshed_system(refresh: impl FnOnce(&mut System)) -> Option<System> {
    System::new().ok().map(|mut sys| {
        refresh(&mut sys);
        sys
    })
}

fn os_label() -> String {
    System::long_os_version()
        .or_else(|_| System::name())
        .unwrap_or_else(|_| "unknown".into())
}

fn memory_ratio(sys: &System) -> f64 {
    ratio_of(sys.used_memory(), sys.total_memory())
}

fn ratio_of(numer: u64, denom: u64) -> f64 {
    if denom == 0 {
        0.0
    } else {
        (numer as f64 / denom as f64).clamp(0.0, 1.0)
    }
}

fn top_processes(sys: &System, count: usize) -> Vec<(String, f32)> {
    let mut rows: Vec<(String, f32)> = sys
        .processes()
        .values()
        .map(|p| (p.name().to_string_lossy().into_owned(), p.cpu_usage()))
        .collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    rows.truncate(count);
    rows
}

fn primary_disk(disks: &[Disk]) -> Option<(u64, u64)> {
    disks
        .iter()
        .filter(|d| d.total_space() > 0)
        .max_by_key(|d| d.total_space())
        .map(|d| (d.total_space(), d.available_space()))
}

fn disk_bars(disks: &[Disk]) -> Vec<Bar> {
    disks
        .iter()
        .filter(|d| d.total_space() > 0)
        .map(|d| Bar {
            label: d.mount_point().to_string_lossy().into_owned(),
            value: d.total_space().saturating_sub(d.available_space()),
            value_label: None,
        })
        .collect()
}

fn disk_label(total: u64, available: u64) -> String {
    let used = total.saturating_sub(available);
    format!(
        "{:.0}% of {}",
        ratio_of(used, total) * 100.0,
        format_bytes(total)
    )
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3600;
    let minutes = (secs % 3600) / 60;
    match (days, hours, minutes) {
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

fn format_load(v: f64) -> String {
    if cfg!(windows) {
        "n/a".into()
    } else {
        format!("{v:.2}")
    }
}

#[cfg(windows)]
fn load_line(_: f64, _: f64, _: f64) -> String {
    "n/a (windows)".into()
}

#[cfg(not(windows))]
fn load_line(one: f64, five: f64, fifteen: f64) -> String {
    format!("{one:.2} {five:.2} {fifteen:.2}")
}

const KB: u64 = 1024;
const MB: u64 = 1024 * KB;
const GB: u64 = 1024 * MB;
const TB: u64 = 1024 * GB;

fn format_bytes(bytes: u64) -> String {
    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

