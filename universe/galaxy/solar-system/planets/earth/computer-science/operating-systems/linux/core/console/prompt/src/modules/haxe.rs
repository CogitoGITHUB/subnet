use super::{Context, Module, ModuleConfig};

use crate::configs::haxe::HaxeConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;
use serde_json as json;

use regex::Regex;
const HAXERC_VERSION_PATTERN: &str = "(?:[0-9a-zA-Z][-+0-9.a-zA-Z]+)";

/// Creates a module with the current Haxe version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("haxe");
    let config = HaxeConfig::try_load(module.config);

    let is_haxe_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_haxe_project {
        return None;
    }

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
                "version" => {
                    let haxe_version = get_haxe_version(context)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &haxe_version,
                        config.version_format,
                    )
                    .map(Ok)
                }
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `haxe`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_haxe_version(context: &Context) -> Option<String> {
    get_haxerc_version(context).or_else(|| {
        let cmd_output = context.exec_cmd("haxe", &["--version"])?;
        parse_haxe_version(cmd_output.stdout.as_str())
    })
}

fn get_haxerc_version(context: &Context) -> Option<String> {
    let raw_json = context.read_file_from_pwd(".haxerc")?;
    let package_json: json::Value = json::from_str(&raw_json).ok()?;

    let raw_version = package_json.get("version")?.as_str()?;
    if raw_version.contains('/') || raw_version.contains('\\') {
        return None;
    }
    Some(raw_version.to_string())
}

fn parse_haxe_version(raw_version: &str) -> Option<String> {
    let re = Regex::new(HAXERC_VERSION_PATTERN).ok()?;
    if !re.is_match(raw_version) {
        return None;
    }
    Some(raw_version.trim().to_string())
}

