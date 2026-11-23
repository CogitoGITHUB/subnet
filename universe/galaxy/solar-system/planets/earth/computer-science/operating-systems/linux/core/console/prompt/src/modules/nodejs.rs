use super::{Context, Module, ModuleConfig};

use crate::configs::nodejs::NodejsConfig;
use crate::formatter::{StringFormatter, VersionFormatter};

use regex::Regex;
use semver::Version;
use semver::VersionReq;
use serde_json as json;
use std::ops::Deref;
use std::sync::LazyLock;

/// Creates a module with the current Node.js version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("nodejs");
    let config = NodejsConfig::try_load(module.config);
    let is_js_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    let is_esy_project = context
        .try_begin_scan()?
        .set_folders(&["esy.lock"])
        .is_match();

    if !is_js_project || is_esy_project {
        return None;
    }

    let nodejs_version = LazyLock::new(|| {
        context
            .exec_cmd("node", &["--version"])
            .map(|cmd| cmd.stdout)
    });
    let engines_version = LazyLock::new(|| get_engines_version(context));

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => {
                    let in_engines_range = check_engines_version(
                        nodejs_version.as_deref(),
                        engines_version.as_deref(),
                    );

                    if in_engines_range {
                        Some(Ok(config.style))
                    } else {
                        Some(Ok(config.not_capable_style))
                    }
                }
                _ => None,
            })
            .map(|variable| match variable {
                "version" => {
                    let node_ver = nodejs_version
                        .deref()
                        .as_ref()?
                        .trim_start_matches('v')
                        .trim();

                    VersionFormatter::format_module_version(
                        module.get_name(),
                        node_ver,
                        config.version_format,
                    )
                    .map(Ok)
                }
                "engines_version" => {
                    let in_engines_range = check_engines_version(
                        nodejs_version.as_deref(),
                        engines_version.as_deref(),
                    );
                    let eng_ver = engines_version.as_deref()?.to_string();

                    (!in_engines_range).then_some(Ok(eng_ver))
                }
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `nodejs`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_engines_version(context: &Context) -> Option<String> {
    let json_str = context.read_file_from_pwd("package.json")?;
    let package_json: json::Value = json::from_str(&json_str).ok()?;
    let raw_version = package_json.get("engines")?.get("node")?.as_str()?;

    Some(raw_version.to_string())
}

fn check_engines_version(nodejs_version: Option<&str>, engines_version: Option<&str>) -> bool {
    let (Some(nodejs_version), Some(engines_version)) = (nodejs_version, engines_version) else {
        return true;
    };

    let Ok(r) = VersionReq::parse(engines_version) else {
        return true;
    };

    let re = Regex::new(r"\d+\.\d+\.\d+").unwrap();
    let version = re
        .captures(nodejs_version)
        .unwrap()
        .get(0)
        .unwrap()
        .as_str();

    let v = match Version::parse(version) {
        Ok(v) => v,
        Err(_e) => return true,
    };
    r.matches(&v)
}

