use std::borrow::Cow;
use std::path::PathBuf;
use std::str::FromStr;

use super::{Context, Module, ModuleConfig};

use crate::configs::direnv::DirenvConfig;
use crate::formatter::StringFormatter;

use serde::Deserialize;

/// Creates a module with the current direnv rc
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("direnv");
    let config = DirenvConfig::try_load(module.config);
    let has_detected_env_var = context.detect_env_vars(&config.detect_env_vars);

    let direnv_applies = !config.disabled
        && (has_detected_env_var
            || context
                .try_begin_scan()?
                .set_extensions(&config.detect_extensions)
                .set_files(&config.detect_files)
                .set_folders(&config.detect_folders)
                .is_match());

    if !direnv_applies {
        return None;
    }

    // the `--json` flag is silently ignored for direnv versions <2.33.0
    let direnv_status = &context.exec_cmd("direnv", &["status", "--json"])?.stdout;
    let state = serde_json::from_str::<RawDirenvState>(direnv_status).map_or_else(
        |_| {
            DirenvState::from_lines(direnv_status)
                .inspect_err(|e| log::warn!("{e}"))
                .ok()
        },
        |raw| {
            raw.into_direnv_state()
                .inspect_err(|e| log::warn!("{e}"))
                .ok()
                .flatten()
        },
    )?;

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "symbol" => Some(Ok(Cow::from(config.symbol))),
                "rc_path" => Some(Ok(state.rc_path.to_string_lossy())),
                "allowed" => Some(Ok(match state.allowed {
                    AllowStatus::Allowed => Cow::from(config.allowed_msg),
                    AllowStatus::NotAllowed => Cow::from(config.not_allowed_msg),
                    AllowStatus::Denied => Cow::from(config.denied_msg),
                })),
                "loaded" => state
                    .loaded
                    .then_some(config.loaded_msg)
                    .or(Some(config.unloaded_msg))
                    .map(Cow::from)
                    .map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(e) => {
            log::warn!("{e}");

            return None;
        }
    });

    Some(module)
}

struct DirenvState {
    pub rc_path: PathBuf,
    pub allowed: AllowStatus,
    pub loaded: bool,
}

impl FromStr for DirenvState {
    type Err = Cow<'static, str>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match serde_json::from_str::<RawDirenvState>(s) {
            Ok(raw) => raw
                .into_direnv_state()?
                .ok_or_else(|| Cow::from("unknown direnv state")),
            Err(_) => Self::from_lines(s),
        }
    }
}

impl DirenvState {
    fn from_lines(s: &str) -> Result<Self, Cow<'static, str>> {
        let mut rc_path = PathBuf::new();
        let mut allowed = None;
        let mut loaded = true;

        for line in s.lines() {
            if let Some(path) = line.strip_prefix("Found RC path") {
                rc_path = PathBuf::from_str(path.trim()).map_err(|e| Cow::from(e.to_string()))?
            } else if let Some(value) = line.strip_prefix("Found RC allowed") {
                allowed = Some(AllowStatus::from_str(value.trim())?);
            } else if line.contains("No .envrc or .env loaded") {
                loaded = false;
            }
        }

        if rc_path.as_os_str().is_empty() || allowed.is_none() {
            return Err(Cow::from("unknown direnv state"));
        }

        Ok(Self {
            rc_path,
            allowed: allowed.unwrap(),
            loaded,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AllowStatus {
    Allowed,
    NotAllowed,
    Denied,
}

impl FromStr for AllowStatus {
    type Err = Cow<'static, str>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "0" | "true" => Ok(Self::Allowed),
            "1" => Ok(Self::NotAllowed),
            "2" | "false" => Ok(Self::Denied),
            _ => Err(Cow::from("invalid allow status")),
        }
    }
}

impl TryFrom<u8> for AllowStatus {
    type Error = Cow<'static, str>;

    fn try_from(u: u8) -> Result<Self, Self::Error> {
        match u {
            0 => Ok(Self::Allowed),
            1 => Ok(Self::NotAllowed),
            2 => Ok(Self::Denied),
            _ => Err(Cow::from("unknown integer allow status")),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawDirenvState {
    pub state: State,
}

impl RawDirenvState {
    fn into_direnv_state(self) -> Result<Option<DirenvState>, Cow<'static, str>> {
        match (self.state.found_rc, self.state.loaded_rc) {
            (None, None) => Ok(None),
            (Some(found_rc), None) => Ok(Some(DirenvState {
                rc_path: found_rc.path,
                allowed: found_rc.allowed.try_into()?,
                loaded: false,
            })),
            (Some(found_rc), Some(loaded_rc)) => Ok(Some(DirenvState {
                rc_path: found_rc.path,
                allowed: found_rc.allowed.try_into()?,
                loaded: matches!(loaded_rc.allowed.try_into()?, AllowStatus::Allowed),
            })),
            (None, Some(_)) => Err(Cow::from("unknown direnv state")),
        }
    }
}

#[derive(Debug, Deserialize)]
struct State {
    #[serde(rename = "foundRC")]
    pub found_rc: Option<RCStatus>,
    #[serde(rename = "loadedRC")]
    pub loaded_rc: Option<RCStatus>,
}

#[derive(Debug, Deserialize)]
struct RCStatus {
    pub allowed: u8,
    pub path: PathBuf,
}

