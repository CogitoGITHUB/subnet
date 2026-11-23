use crate::env_access::EnvAccess;
use crate::errors::{ConfigError, ConfigWarning};
use crate::overrides::CliOverrides;
use crate::paths::{ConfigPath, NushellConfigDirs};
#[cfg(unix)]
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// One-shot entry point for resolving all Nushell configuration paths.
///
/// Call this once at startup and store the result in `EngineState.config_dirs`.
/// Every downstream consumer (`create_nu_constant`, config-file loading,
/// history backend, etc.) reads from that struct instead of re-resolving.
///
/// # Resolution order
///
/// For each path, the priority is:
/// 1. CLI override ([`CliOverrides`])
/// 2. Corresponding XDG environment variable
/// 3. Platform default via the [`EnvAccess`] seam
///
/// # Errors
///
/// Returns [`ConfigError`] if no config directory can be determined.
/// Non-fatal warnings (e.g. an empty XDG config dir) are returned in the
/// [`ConfigWarning`] vec.
pub fn resolve_paths(
    env: &impl EnvAccess,
    cli: &CliOverrides,
) -> Result<(NushellConfigDirs, Vec<ConfigWarning>), ConfigError> {
    let mut warnings = Vec::new();

    // ── config_home ──────────────────────────────────────────────────────
    let (config_home, config_home_source) = resolve_config_home_with_source(env, cli)?;

    // ── config_file / env_file ───────────────────────────────────────────
    let config_file = config_path_from_cli(cli.config_file.as_ref(), config_home.join("config.nu"));
    let env_file = config_path_from_cli(cli.env_file.as_ref(), config_home.join("env.nu"));

    // ── data_home ────────────────────────────────────────────────────────
    let data_home = resolve_xdg_base(env, "XDG_DATA_HOME", |e| e.data_dir())
        .unwrap_or_else(|| config_home.join("data"))
        .join("console");

    // ── cache_home ───────────────────────────────────────────────────────
    let cache_home = resolve_xdg_base(env, "XDG_CACHE_HOME", |e| e.cache_dir())
        .unwrap_or_else(|| config_home.join("cache"))
        .join("console");

    // ── home_dir ─────────────────────────────────────────────────────────
    // A missing home is not fatal: a user without one can still keep config
    // under XDG_CONFIG_HOME, so leave it empty and let `$nu.home-dir` report it.
    let home_dir = env.home_dir().unwrap_or_default();

    // ── vendor_autoload_dirs ─────────────────────────────────────────────
    let vendor_autoload_dirs = resolve_vendor_autoload_dirs(env);

    // ── user_autoload_dirs ───────────────────────────────────────────────
    let user_autoload_dirs = vec![config_home.join("autoload")];

    // ── plugin_file ──────────────────────────────────────────────────────
    #[cfg(feature = "plugin")]
    let plugin_file = config_path_from_cli(
        cli.plugin_file.as_ref(),
        config_home.join("plugin.msgpackz"),
    );

    // ── XDG validation (moved from main.rs) ──────────────────────────────
    // Warn only when we fell back to the platform default because XDG was
    // set but unusable (relative / non-absolute). Do not warn when CLI
    // `--config-home` simply took priority over a valid XDG value.
    if matches!(config_home_source, ConfigHomeSource::PlatformDefault)
        && let Some(xdg_raw) = env.var("XDG_CONFIG_HOME")
        && !xdg_raw.is_empty()
    {
        warnings.push(ConfigWarning::XdgConfigIgnored {
            xdg: xdg_raw,
            resolved: config_home.clone(),
        });
    }

    // If XDG_CONFIG_HOME was used but the old dir still has files while
    // the new one is empty, warn about the migration.
    if matches!(config_home_source, ConfigHomeSource::Xdg)
        && let Some(old_config) = env.config_dir().map(|p| p.join("shell"))
    {
        let new_config_empty = config_home
            .read_dir()
            .map_or(true, |mut dir| dir.next().is_none());
        let old_config_empty = old_config
            .read_dir()
            .map_or(true, |mut dir| dir.next().is_none());
        if !old_config_empty && new_config_empty {
            warnings.push(ConfigWarning::OldConfigDirHasFiles {
                old: old_config,
                new: config_home.clone(),
            });
        }
    }

    Ok((
        NushellConfigDirs {
            config_home: config_home.clone(),
            config_file,
            env_file,
            data_home,
            cache_home,
            home_dir,
            vendor_autoload_dirs,
            user_autoload_dirs,
            #[cfg(feature = "plugin")]
            plugin_file,
        },
        warnings,
    ))
}

// ─── Internal helpers ─────────────────────────────────────────────────────

/// Build a [`ConfigPath`] from an optional CLI path and a default path.
///
/// Centralizes the Default vs Override decision so resolve sites stay DRY.
fn config_path_from_cli(cli: Option<&PathBuf>, default: PathBuf) -> ConfigPath {
    match cli {
        Some(path) => ConfigPath::Override(path.clone()),
        None => ConfigPath::Default(default),
    }
}

/// Where the resolved config home came from.
///
/// Used to decide which non-fatal XDG warnings apply. CLI override must not
/// be treated as “XDG was invalid.”
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigHomeSource {
    /// `--config-home` CLI flag.
    Cli,
    /// Absolute non-empty `$XDG_CONFIG_HOME`.
    Xdg,
    /// Platform config dir fallback (dirs crate / no usable XDG).
    PlatformDefault,
}

/// Resolve the shell config directory and record which source won.
fn resolve_config_home_with_source(
    env: &impl EnvAccess,
    cli: &CliOverrides,
) -> Result<(PathBuf, ConfigHomeSource), ConfigError> {
    // 1. CLI override takes highest priority.
    if let Some(ref home) = cli.config_home {
        return Ok((home.clone(), ConfigHomeSource::Cli));
    }

    // 2. XDG_CONFIG_HOME env var.
    if let Some(xdg) = env.var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
        && Path::new(&xdg).is_absolute()
    {
        let mut home = PathBuf::from(xdg);
        home.push("console");
        return Ok((home, ConfigHomeSource::Xdg));
    }

    // 3. Platform default.
    let base = env.config_dir().ok_or(ConfigError::ConfigDirNotFound)?;
    Ok((base.join("console"), ConfigHomeSource::PlatformDefault))
}

/// Resolve an XDG base directory (CONFIG, DATA, CACHE) with fallback.
fn resolve_xdg_base<E: EnvAccess>(
    env: &E,
    var_name: &str,
    fallback: impl FnOnce(&E) -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(val) = env.var(var_name)
        && !val.is_empty()
        && Path::new(&val).is_absolute()
    {
        return Some(PathBuf::from(val));
    }
    fallback(env)
}

/// Resolve vendor autoload directories.
///
/// Order is load order: earlier entries run first, later entries can override.
/// On Unix, `XDG_DATA_DIRS` entries are **reversed** so that earlier entries
/// in the env var have higher precedence (standard XDG behavior).
///
/// This logic was moved from `console-protocol/src/eval_const.rs` /
/// `get_vendor_autoload_dirs()` so that all path resolution is in one place.
fn resolve_vendor_autoload_dirs(env: &impl EnvAccess) -> Vec<PathBuf> {
    let into_autoload_path = |mut path: PathBuf| {
        path.push("console");
        path.push("vendor");
        path.push("autoload");
        path
    };

    let mut dirs = Vec::new();

    let mut append = |path: PathBuf| {
        if !dirs.contains(&path) {
            dirs.push(path);
        }
    };

    #[cfg(target_os = "macos")]
    std::iter::once("/Library/Application Support")
        .map(PathBuf::from)
        .map(&into_autoload_path)
        .for_each(&mut append);

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let data_dirs: OsString = env
            .var_os("XDG_DATA_DIRS")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                option_env!("PREFIX").map_or_else(
                    || OsString::from("/usr/local/share/:/usr/share/"),
                    |prefix| {
                        if prefix.ends_with("local") {
                            OsString::from(format!("{prefix}/share"))
                        } else {
                            OsString::from(format!("{prefix}/local/share:{prefix}/share"))
                        }
                    },
                )
            });

        // The XDG Base Directory Specification requires every entry to be
        // absolute. Ignore invalid entries rather than interpreting them
        // relative to the current working directory.
        //
        // Reverse so earlier XDG_DATA_DIRS entries load later and win.
        data_dirs
            .as_encoded_bytes()
            .split(|b| *b == b':')
            .map(|split| PathBuf::from(std::ffi::OsStr::from_bytes(split)))
            .filter(|path| path.is_absolute())
            .map(&into_autoload_path)
            .rev()
            .for_each(&mut append);
    }

    #[cfg(windows)]
    {
        if let Some(path) = env.program_data_dir() {
            append(into_autoload_path(path));
        }
    }

    // Compile-time NU_VENDOR_AUTOLOAD_DIR: baked into the binary when `nu` is
    // built (e.g. distro packaging). Inserted before the data-home path so
    // packaged vendor scripts load early in the autoload order.
    if let Some(path) = option_env!("NU_VENDOR_AUTOLOAD_DIR").map(PathBuf::from)
        && path.is_absolute()
    {
        append(path);
    }

    if let Some(data_dir) = resolve_xdg_base(env, "XDG_DATA_HOME", |e| e.data_dir()) {
        append(into_autoload_path(data_dir));
    }

    // Runtime NU_VENDOR_AUTOLOAD_DIR: process environment when `nu` starts.
    // Inserted last so a user/admin can override compile-time and data-home
    // vendor dirs without rebuilding. Same path as compile-time is deduped by
    // `append`. Not the same source as `option_env!` above.
    if let Some(path) = env.var_os("NU_VENDOR_AUTOLOAD_DIR").map(PathBuf::from)
        && path.is_absolute()
    {
        append(path);
    }

    // Cover platform fallbacks (including Windows ProgramData), compile-time
    // PREFIX defaults, and any future source before exposing these paths via $nu.
    dirs.retain(|path| path.is_absolute());
    dirs
}

