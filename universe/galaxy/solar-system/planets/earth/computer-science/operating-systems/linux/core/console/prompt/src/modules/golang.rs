use super::{Context, Module, ModuleConfig};

use crate::configs::go::GoConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;

use regex::Regex;
use semver::Version;
use semver::VersionReq;
use std::ops::Deref;
use std::sync::LazyLock;

/// Creates a module with the current Go version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("golang");
    let config = GoConfig::try_load(module.config);
    let is_go_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_go_project {
        return None;
    }

    let golang_version =
        LazyLock::new(|| parse_go_version(&context.exec_cmd("go", &["version"])?.stdout));
    let mod_version = LazyLock::new(|| get_go_mod_version(context));

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => {
                    let in_mod_range =
                        check_go_version(golang_version.as_deref(), mod_version.as_deref());

                    if in_mod_range {
                        Some(Ok(config.style))
                    } else {
                        Some(Ok(config.not_capable_style))
                    }
                }
                _ => None,
            })
            .map(|variable| match variable {
                "version" => {
                    let go_ver = golang_version.deref().as_ref()?;

                    VersionFormatter::format_module_version(
                        module.get_name(),
                        go_ver,
                        config.version_format,
                    )
                    .map(Ok)
                }
                "mod_version" => {
                    let in_mod_range =
                        check_go_version(golang_version.as_deref(), mod_version.as_deref());
                    let mod_ver = mod_version.as_deref()?.to_string();

                    (!in_mod_range).then_some(Ok(mod_ver))
                }
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `golang`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_go_version(go_stdout: &str) -> Option<String> {
    // go version output looks like this:
    // go version go1.13.3 linux/amd64

    let version = go_stdout
        // split into ("", "1.12.4 linux/amd64")
        .split_once("go version go")?
        // return "1.12.4 linux/amd64"
        .1
        // split into ["1.12.4", "linux/amd64"]
        .split_whitespace()
        // return "1.12.4"
        .next()?;

    Some(version.to_string())
}

fn get_go_mod_version(context: &Context) -> Option<String> {
    let mod_str = context.read_file_from_pwd("go.mod")?;
    let re = Regex::new(r"(?:go\s)(\d+(\.\d+)+)").unwrap();

    if let Some(cap) = re.captures(&mod_str) {
        let mod_ver = cap.get(1)?.as_str();
        Some(mod_ver.to_string())
    } else {
        None
    }
}

fn check_go_version(go_version: Option<&str>, mod_version: Option<&str>) -> bool {
    let (Some(go_version), Some(mod_version)) = (go_version, mod_version) else {
        return true;
    };
    let Ok(r) = VersionReq::parse(mod_version) else {
        return true;
    };
    let Ok(v) = Version::parse(go_version) else {
        return true;
    };

    r.matches(&v)
}

