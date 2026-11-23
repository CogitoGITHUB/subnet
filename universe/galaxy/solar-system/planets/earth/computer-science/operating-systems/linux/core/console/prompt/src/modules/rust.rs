#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde::Deserialize;
use std::collections::HashMap;

use super::{Context, Module, ModuleConfig};

use crate::configs::rust::RustConfig;
use crate::formatter::{StringFormatter, VersionFormatter};
use crate::utils::{create_command, read_file};
use home::rustup_home;

use std::sync::OnceLock;

use guess_host_triple::guess_host_triple;

type VersionString = String;
type ToolchainString = String;

/// A struct to cache the output of any commands that need to be run.
struct RustToolingEnvironmentInfo {
    /// Rustup settings parsed from $HOME/.rustup/settings.toml
    rustup_settings: OnceLock<RustupSettings>,
    /// Rustc toolchain overrides as contained in the environment or files
    env_toolchain_override: OnceLock<Option<String>>,
    /// The output of `rustup rustc --version` with a fixed toolchain
    rustup_rustc_output: OnceLock<RustupRunRustcVersionOutcome>,
    /// The output of running rustc -vV. Only called if rustup rustc fails or
    /// is unavailable.
    rustc_verbose_output: OnceLock<Option<(VersionString, ToolchainString)>>,
}

impl RustToolingEnvironmentInfo {
    fn new() -> Self {
        Self {
            rustup_settings: OnceLock::new(),
            env_toolchain_override: OnceLock::new(),
            rustup_rustc_output: OnceLock::new(),
            rustc_verbose_output: OnceLock::new(),
        }
    }

    fn get_rustup_settings(&self, context: &Context) -> &RustupSettings {
        self.rustup_settings
            .get_or_init(|| RustupSettings::load(context).unwrap_or_default())
    }

    /// Gets any environmental toolchain overrides without downloading cargo toolchains
    fn get_env_toolchain_override(&self, context: &Context) -> Option<&str> {
        // `$CARGO_HOME/bin/rustc(.exe) --version` may attempt installing a rustup toolchain.
        // https://github.com/starship/starship/issues/417
        //
        // To display appropriate versions preventing `rustc` from downloading toolchains, we have to
        // check
        // 1. `$RUSTUP_TOOLCHAIN`
        // 2. The override list from ~/.rustup/settings.toml (like `rustup override list`)
        // 3. `rust-toolchain` or `rust-toolchain.toml` in `.` or parent directories
        // 4. The `default_toolchain` from ~/.rustup/settings.toml (like `rustup default`)
        // 5. `rustup default` (in addition to the above, this also looks at global fallback config files)
        // as `rustup` does.
        // https://github.com/rust-lang/rustup.rs/tree/eb694fcada7becc5d9d160bf7c623abe84f8971d#override-precedence
        //
        // Probably we have no other way to know whether any toolchain override is specified for the
        // current directory. The following commands also cause toolchain installations.
        // - `rustup show`
        // - `rustup show active-toolchain`
        // - `rustup which`
        self.env_toolchain_override
            .get_or_init(|| {
                let out = env_rustup_toolchain(context)
                    .or_else(|| {
                        self.get_rustup_settings(context)
                            .lookup_override(context.current_dir.as_path())
                    })
                    .or_else(|| find_rust_toolchain_file(context))
                    .or_else(|| {
                        self.get_rustup_settings(context)
                            .default_toolchain()
                            .map(std::string::ToString::to_string)
                    })
                    .or_else(|| execute_rustup_default(context));

                log::debug!("Environmental toolchain override is {out:?}");
                out
            })
            .as_deref()
    }

    /// Gets the output of running `rustup rustc --version` with a toolchain
    /// specified by `self.get_env_toolchain_override()`
    fn get_rustup_rustc_version(&self, context: &Context) -> &RustupRunRustcVersionOutcome {
        self.rustup_rustc_output.get_or_init(|| {
            let out = if let Some(toolchain) = self.get_env_toolchain_override(context) {
                // First try running ~/.rustup/toolchains/<toolchain>/bin/rustc --version
                rustup_home()
                    .map(|rustup_folder| {
                        rustup_folder
                            .join("toolchains")
                            .join(toolchain)
                            .join("bin")
                            .join("rustc")
                    })
                    .and_then(|rustc| {
                        log::trace!("Running rustc --version directly with {rustc:?}");
                        create_command(rustc).map(|mut cmd| {
                            cmd.arg("--version");
                            cmd
                        })
                    })
                    .or_else(|_| {
                        // If that fails, try running rustup rustup run <toolchain> rustc --version
                        // Depending on the source of the toolchain override, it might not have been a full toolchain name ("stable" or "nightly").
                        log::trace!("Running rustup {toolchain} rustc --version");
                        create_command("rustup").map(|mut cmd| {
                            cmd.args(["run", toolchain, "rustc", "--version"]);
                            cmd
                        })
                    })
                    .and_then(|mut cmd| cmd.current_dir(&context.current_dir).output())
                    .map_or(
                        RustupRunRustcVersionOutcome::RustupNotWorking,
                        extract_toolchain_from_rustup_run_rustc_version,
                    )
            } else {
                RustupRunRustcVersionOutcome::ToolchainUnknown
            };

            log::debug!("Rustup rustc version is {out:?}");
            out
        })
    }

    /// Gets the (version, toolchain) string as returned by `rustc -vV`
    fn get_rustc_verbose_version(&self, context: &Context) -> Option<(&str, &str)> {
        let toolchain = self.get_rustup_settings(context).default_toolchain();

        self.rustc_verbose_output
            .get_or_init(|| {
                let Output { status, stdout, .. } = create_command("rustc")
                    .and_then(|mut cmd| {
                        cmd.args(["-Vv"]).current_dir(&context.current_dir).output()
                    })
                    .ok()?;
                if !status.success() {
                    return None;
                }
                let out =
                    format_rustc_version_verbose(std::str::from_utf8(&stdout).ok()?, toolchain);

                log::debug!("Rustup verbose version is {out:?}");
                out
            })
            .as_ref()
            .map(|(x, y)| (&x[..], &y[..]))
    }
}

/// Creates a module with the current Rust version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("rust");
    let config = RustConfig::try_load(module.config);

    let is_rs_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_rs_project {
        return None;
    }

    let rust_env_info = RustToolingEnvironmentInfo::new();

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "version" => get_module_version(context, &config, &rust_env_info).map(Ok),
                "numver" => get_module_numeric_version(context, &config, &rust_env_info).map(Ok),
                "toolchain" => get_toolchain_version(context, &config, &rust_env_info).map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `rust`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_module_version(
    context: &Context,
    config: &RustConfig,
    rust_env_info: &RustToolingEnvironmentInfo,
) -> Option<String> {
    type Outcome = RustupRunRustcVersionOutcome;

    match rust_env_info.get_rustup_rustc_version(context) {
        Outcome::RustcVersion(rustc_version) => {
            format_rustc_version(rustc_version, config.version_format)
        }
        Outcome::RustupNotWorking | Outcome::ToolchainUnknown => {
            // If `rustup` can't be executed, or there is no environmental toolchain, we can
            // execute `rustc --version` without triggering a toolchain download
            format_rustc_version(&execute_rustc_version(context)?, config.version_format)
        }
        Outcome::ToolchainNotInstalled(name) => Some(name.clone()),
        Outcome::Err => None,
    }
}

fn get_module_numeric_version(
    context: &Context,
    _config: &RustConfig,
    rust_env_info: &RustToolingEnvironmentInfo,
) -> Option<String> {
    type Outcome = RustupRunRustcVersionOutcome;

    match rust_env_info.get_rustup_rustc_version(context) {
        Outcome::RustcVersion(version) => {
            let release = version.split_whitespace().nth(1).unwrap_or(version);
            Some(format_semver(release))
        }
        Outcome::RustupNotWorking | Outcome::ToolchainUnknown => {
            let (numver, _toolchain) = rust_env_info.get_rustc_verbose_version(context)?;
            Some(numver.to_string())
        }
        Outcome::ToolchainNotInstalled(_) | RustupRunRustcVersionOutcome::Err => None,
    }
}

fn get_toolchain_version(
    context: &Context,
    _config: &RustConfig,
    rust_env_info: &RustToolingEnvironmentInfo,
) -> Option<String> {
    type Outcome = RustupRunRustcVersionOutcome;

    let settings_host_triple = rust_env_info
        .get_rustup_settings(context)
        .default_host_triple();
    let default_host_triple = if settings_host_triple.is_none() {
        guess_host_triple()
    } else {
        settings_host_triple
    };

    match rust_env_info.get_rustup_rustc_version(context) {
        Outcome::RustcVersion(_) | Outcome::ToolchainNotInstalled(_) => {
            let toolchain_override = rust_env_info
                .get_env_toolchain_override(context)
                // This match arm should only trigger if the toolchain override
                // is not None because of how get_rustup_rustc_version works
                .expect("Toolchain override was None: programming error.");
            Some(format_toolchain(toolchain_override, default_host_triple))
        }
        Outcome::RustupNotWorking | Outcome::ToolchainUnknown => {
            let (_numver, toolchain) = rust_env_info.get_rustc_verbose_version(context)?;
            Some(format_toolchain(toolchain, default_host_triple))
        }
        Outcome::Err => None,
    }
}

fn env_rustup_toolchain(context: &Context) -> Option<String> {
    log::trace!("Searching for rustup toolchain in environment.");
    let val = context.get_env("RUSTUP_TOOLCHAIN")?;
    Some(val.trim().to_owned())
}

fn execute_rustup_default(context: &Context) -> Option<String> {
    log::trace!("Searching for toolchain with rustup default");
    // `rustup default` output is:
    //    stable-x86_64-apple-darwin (default)
    context
        .exec_cmd("rustup", &["default"])?
        .stdout
        .split_whitespace()
        .next()
        .map(str::to_owned)
}

fn find_rust_toolchain_file(context: &Context) -> Option<String> {
    log::trace!("Searching for toolchain in toolchain file");
    // Look for 'rust-toolchain' or 'rust-toolchain.toml' as rustup does.
    // for more information:
    // https://rust-lang.github.io/rustup/overrides.html#the-toolchain-file
    // for the implementation in 'rustup':
    // https://github.com/rust-lang/rustup/blob/a45e4cd21748b04472fce51ba29999ee4b62bdec/src/config.rs#L631

    #[derive(Deserialize)]
    struct OverrideFile {
        toolchain: ToolchainSection,
    }

    #[derive(Deserialize)]
    struct ToolchainSection {
        channel: Option<String>,
    }

    fn read_channel(path: &Path, only_toml: bool) -> Option<String> {
        let contents = read_file(path).ok()?;

        match contents.lines().count() {
            0 => None,
            1 if !only_toml => Some(contents),
            _ => {
                toml::from_str::<OverrideFile>(&contents)
                    .ok()?
                    .toolchain
                    .channel
            }
        }
        .filter(|c| !c.trim().is_empty())
        .map(|c| c.trim().to_owned())
    }

    if context
        .dir_contents()
        .is_ok_and(|dir| dir.has_file("rust-toolchain"))
        && let Some(toolchain) = read_channel(Path::new("rust-toolchain"), false)
    {
        return Some(toolchain);
    }

    if context
        .dir_contents()
        .is_ok_and(|dir| dir.has_file("rust-toolchain.toml"))
        && let Some(toolchain) = read_channel(Path::new("rust-toolchain.toml"), true)
    {
        return Some(toolchain);
    }

    let mut dir = &*context.current_dir;
    loop {
        if let Some(toolchain) = read_channel(&dir.join("rust-toolchain"), false) {
            return Some(toolchain);
        }
        if let Some(toolchain) = read_channel(&dir.join("rust-toolchain.toml"), true) {
            return Some(toolchain);
        }
        dir = dir.parent()?;
    }
}

fn extract_toolchain_from_rustup_run_rustc_version(output: Output) -> RustupRunRustcVersionOutcome {
    if output.status.success() {
        if let Ok(output) = String::from_utf8(output.stdout) {
            return RustupRunRustcVersionOutcome::RustcVersion(output);
        }
    } else if let Ok(stderr) = String::from_utf8(output.stderr)
        && stderr.starts_with("error: toolchain '")
        && stderr.ends_with("' is not installed\n")
    {
        let stderr = stderr
            ["error: toolchain '".len()..stderr.len() - "' is not installed\n".len()]
            .to_owned();
        return RustupRunRustcVersionOutcome::ToolchainNotInstalled(stderr);
    }
    RustupRunRustcVersionOutcome::Err
}

fn execute_rustc_version(context: &Context) -> Option<String> {
    context
        .exec_cmd("rustc", &["--version"])
        .map(|o| o.stdout)
        .filter(|s| !s.is_empty())
}

fn format_rustc_version(rustc_version: &str, version_format: &str) -> Option<String> {
    let version = rustc_version
        // split into ["rustc", "1.34.0", ...]
        .split_whitespace()
        // get down to "1.34.0"
        .nth(1)?;

    match VersionFormatter::format_version(version, version_format) {
        Ok(formatted) => Some(formatted),
        Err(error) => {
            log::warn!("Error formatting `rust` version:\n{error}");
            Some(format!("v{version}"))
        }
    }
}

fn format_toolchain(toolchain: &str, default_host_triple: Option<&str>) -> String {
    default_host_triple
        .map_or(toolchain, |triple| {
            toolchain.trim_end_matches(&format!("-{triple}"))
        })
        .to_owned()
}

fn format_rustc_version_verbose(stdout: &str, toolchain: Option<&str>) -> Option<(String, String)> {
    let (mut release, mut host) = (None, None);
    for line in stdout.lines() {
        if line.starts_with("release: ") {
            release = Some(line.trim_start_matches("release: "));
        }
        if line.starts_with("host: ") {
            host = Some(line.trim_start_matches("host: "));
        }
    }
    let (release, host) = (release?, host?);
    let version = format_semver(release);
    let toolchain = toolchain.map_or_else(|| host.to_string(), ToOwned::to_owned);
    Some((version, toolchain))
}

fn format_semver(semver: &str) -> String {
    format!("v{}", semver.find('-').map_or(semver, |i| &semver[..i]))
}

#[derive(Debug, PartialEq)]
enum RustupRunRustcVersionOutcome {
    RustcVersion(String),
    ToolchainNotInstalled(String),
    ToolchainUnknown,
    RustupNotWorking,
    Err,
}

#[derive(Default, Debug, PartialEq, Deserialize)]
struct RustupSettings {
    default_host_triple: Option<String>,
    default_toolchain: Option<String>,
    overrides: HashMap<PathBuf, String>,
    version: Option<String>,
}

#[inline]
#[cfg(windows)]
fn strip_dos_path(path: PathBuf) -> PathBuf {
    // Use the display version of the path to strip \\?\
    let path = path.to_string_lossy();
    PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(&path))
}

#[inline]
#[cfg(not(windows))]
fn strip_dos_path(path: PathBuf) -> PathBuf {
    path
}

impl RustupSettings {
    fn load(_context: &Context) -> Option<Self> {
        let path = rustup_home().ok()?.join("settings.toml");
        Self::from_toml_str(&read_file(path).ok()?)
    }

    fn from_toml_str(toml_str: &str) -> Option<Self> {
        let settings = toml::from_str::<Self>(toml_str).ok()?;
        if settings.version.as_deref() == Some("12") {
            Some(settings)
        } else {
            log::warn!(
                r#"Rustup settings version is {:?}, expected "12""#,
                settings.version
            );
            None
        }
    }

    fn default_host_triple(&self) -> Option<&str> {
        self.default_host_triple.as_deref()
    }

    fn default_toolchain(&self) -> Option<&str> {
        self.default_toolchain.as_deref()
    }

    fn lookup_override(&self, cwd: &Path) -> Option<String> {
        let cwd = strip_dos_path(cwd.to_owned());
        self.overrides
            .iter()
            .map(|(dir, toolchain)| (strip_dos_path(dir.clone()), toolchain))
            .filter(|(dir, _)| cwd.starts_with(dir))
            .max_by_key(|(dir, _)| dir.components().count())
            .map(|(_, name)| name.clone())
    }
}

