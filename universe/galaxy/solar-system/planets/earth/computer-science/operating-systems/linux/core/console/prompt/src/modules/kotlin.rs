use super::{Context, Module, ModuleConfig};

use crate::configs::kotlin::KotlinConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;
use crate::utils::get_command_string_output;

use regex::Regex;
const KOTLIN_VERSION_PATTERN: &str = "(?P<version>[\\d\\.]+[\\d\\.]+[\\d\\.]+)";

/// Creates a module with the current Kotlin version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("kotlin");
    let config = KotlinConfig::try_load(module.config);

    let is_kotlin_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_kotlin_project {
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
                    let kotlin_version = get_kotlin_version(context, config.kotlin_binary)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &kotlin_version,
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
            log::warn!("Error in module `kotlin`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_kotlin_version(context: &Context, kotlin_binary: &str) -> Option<String> {
    let command = context.exec_cmd(kotlin_binary, &["-version"])?;
    let kotlin_version_string = get_command_string_output(command);

    parse_kotlin_version(&kotlin_version_string)
}

fn parse_kotlin_version(kotlin_stdout: &str) -> Option<String> {
    // kotlin -version output looks like this:
    // Kotlin version 1.4.21-release-411 (JRE 14.0.1+7)
    // kotlinc -version output looks like this:
    // info: kotlinc-jvm 1.4.21 (JRE 14.0.1+7)
    let re = Regex::new(KOTLIN_VERSION_PATTERN).ok()?;
    let captures = re.captures(kotlin_stdout)?;
    let version = &captures["version"];
    Some(version.to_string())
}

