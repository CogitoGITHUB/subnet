#![warn(missing_docs)]
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use yaml_rust2::{Yaml, YamlLoader};

use super::{Context, Module, ModuleConfig};
use crate::configs::pulumi::PulumiConfig;
use crate::formatter::{StringFormatter, VersionFormatter};
use crate::utils::read_file;

static PULUMI_HOME: &str = "PULUMI_HOME";
static PULUMI_BACKEND_URL: &str = "PULUMI_BACKEND_URL";

#[derive(Deserialize)]
struct Credentials {
    current: Option<String>,
    accounts: Option<HashMap<String, Account>>,
}

#[derive(Deserialize)]
struct Account {
    username: Option<String>,
}

/// Since v3.256.0 Pulumi records a stack per backend in `stacks`.
/// It still reads the older `stack`, and so do we, but it no longer writes it.
#[derive(Debug, Deserialize)]
struct Workspace {
    stack: Option<String>,
    #[serde(default)]
    stacks: HashMap<String, String>,
}

/// Creates a module with the current Pulumi version and stack name.
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("pulumi");
    let config = PulumiConfig::try_load(module.config);

    let project_file_in_cwd = context
        .try_begin_scan()?
        .set_files(&["Pulumi.yaml", "Pulumi.yml"])
        .is_match();

    if !project_file_in_cwd && !config.search_upwards {
        return None;
    }

    let project_file = find_package_file(&context.logical_dir)?;

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "version" => {
                    let stdout = context.exec_cmd("pulumi", &["version"])?.stdout;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        parse_version(&stdout),
                        config.version_format,
                    )
                }
                .map(Ok),
                "username" => get_pulumi_username(context).map(Ok),
                "stack" => stack_name(&project_file, context).map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    match parsed {
        Ok(x) => {
            module.set_segments(x);
            Some(module)
        }
        Err(e) => {
            log::warn!("Error in module `pulumi`:\n{e}");
            None
        }
    }
}

/// Parse and sanitize the output of `pulumi version` into just the version string.
///
/// Normally, this just means returning it. When Pulumi is being developed, it
/// can return results like `3.12.0-alpha.1630554544+f89e9a29.dirty`, which we
/// don't want to see. Instead we display that as `3.12.0-alpha`.
fn parse_version(version: &str) -> &str {
    let new_version = version.strip_prefix('v').unwrap_or(version);

    let sanitized_version = new_version.trim_end();

    let mut periods = 0;
    for (i, c) in sanitized_version.as_bytes().iter().enumerate() {
        if *c == b'.' {
            if periods == 2 {
                return &sanitized_version[0..i];
            }
            periods += 1;
        }
    }
    // We didn't hit 3 periods, so we just return the whole string.
    sanitized_version
}

/// Find a file describing a Pulumi package in the current directory (or any parent directory).
fn find_package_file(path: &Path) -> Option<PathBuf> {
    for path in path.ancestors() {
        log::trace!("Looking for package file in {path:?}");
        let dir = std::fs::read_dir(path).ok()?;
        let goal = dir.filter_map(Result::ok).find(|path| {
            path.file_name() == OsStr::new("Pulumi.yaml")
                || path.file_name() == OsStr::new("Pulumi.yml")
        });
        if let Some(goal) = goal {
            return Some(goal.path());
        }
    }
    log::trace!("Did not find a Pulumi package file");
    None
}

/// We get the name of the current stack.
///
/// Pulumi has no CLI option that is fast enough to get this for us, but finding
/// the location is simple. We get it ourselves.
fn stack_name(project_file: &Path, context: &Context) -> Option<String> {
    let contents = read_file(project_file).ok()?;
    let mut project = YamlLoader::load_from_str(&contents)
        .ok()
        .and_then(|yaml| yaml.into_iter().next())
        .and_then(Yaml::into_hash)?;
    log::trace!("Parsed {project_file:?} into yaml");

    let name = project
        .remove(&Yaml::String("name".to_string()))?
        .into_string()?;
    log::trace!("Found project name: {name:?}");

    let project_backend = project
        .remove(&Yaml::String("backend".to_string()))
        .and_then(Yaml::into_hash)
        .and_then(|mut backend| backend.remove(&Yaml::String("url".to_string())))
        .and_then(Yaml::into_string);

    let workspace_file = get_pulumi_workspace(context, &name, project_file)
        .map(File::open)?
        .ok()?;
    log::trace!("Trying to read workspace_file: {workspace_file:?}");
    let workspace: Workspace = match serde_json::from_reader(workspace_file) {
        Ok(k) => k,
        Err(e) => {
            log::debug!("Failed to parse workspace file: {e}");
            return None;
        }
    };
    log::trace!("Read workspace_file: {workspace:?}");

    let backend = current_backend_url(context, project_backend);
    log::trace!("Looking for a stack selected for backend {backend:?}");

    selected_stack(workspace, backend.as_deref())
}

/// Pick the stack Pulumi considers selected, newest file format first.
fn selected_stack(workspace: Workspace, backend: Option<&str>) -> Option<String> {
    let Workspace { stack, mut stacks } = workspace;

    if let Some(stack) = backend.and_then(|backend| stacks.remove(backend)) {
        return Some(stack);
    }

    if stack.is_some() {
        return stack;
    }

    // Only when the backend could not be determined at all: a lone selection
    // is the one Pulumi would pick too. With a known backend, a selection for
    // a different one is not ours to report.
    if backend.is_none() && stacks.len() == 1 {
        return stacks.into_values().next();
    }

    None
}

/// The backend Pulumi is pointed at, in the order the CLI resolves it.
fn current_backend_url(context: &Context, project_backend: Option<String>) -> Option<String> {
    if let Some(url) = context.get_env(PULUMI_BACKEND_URL) {
        return Some(url);
    }

    project_backend.or_else(|| read_credentials(context)?.current)
}

/// Calculates the path of the workspace settings file for a given pulumi stack.
fn get_pulumi_workspace(context: &Context, name: &str, project_file: &Path) -> Option<PathBuf> {
    let project_file = if cfg!(test) {
        // Because this depends on the absolute path of the file, it changes in
        // each test run. We thus mock it.
        "test".to_string()
    } else {
        let mut hasher = Sha1::new();
        hasher.update(project_file.to_str()?.as_bytes());
        crate::utils::encode_to_hex(&hasher.finalize())
    };
    let unique_file_name = format!("{name}-{project_file}-workspace.json");
    let mut path = pulumi_home_dir(context)?;
    path.push("workspaces");
    path.push(unique_file_name);
    Some(path)
}

/// Get the Pulumi home directory. We first check `PULUMI_HOME`. If that isn't
/// set, we return `$HOME/.pulumi`.
fn pulumi_home_dir(context: &Context) -> Option<PathBuf> {
    if let Some(k) = context.get_env(PULUMI_HOME) {
        std::path::PathBuf::from_str(&k).ok()
    } else {
        context.get_home().map(|p| p.join(".pulumi"))
    }
}

fn read_credentials(context: &Context) -> Option<Credentials> {
    let home_dir = pulumi_home_dir(context)?;
    let creds_path = home_dir.join("credentials.json");

    let file = File::open(creds_path).ok()?;
    let reader = BufReader::new(file);

    serde_json::from_reader(reader).ok()
}

fn get_pulumi_username(context: &Context) -> Option<String> {
    let creds = read_credentials(context)?;

    let current_api_provider = creds.current?;

    creds.accounts?.remove(&current_api_provider)?.username
}

