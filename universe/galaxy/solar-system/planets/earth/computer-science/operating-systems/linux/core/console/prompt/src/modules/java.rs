use super::{Context, Module, ModuleConfig};
use crate::configs::java::JavaConfig;
use crate::formatter::{StringFormatter, VersionFormatter};
use crate::utils::get_command_string_output;
use std::path::PathBuf;

use regex::Regex;
const JAVA_VERSION_PATTERN: &str =
    "(?:JRE.*\\(|OpenJ9 )(?P<version>\\d+(?:\\.\\d+){0,2}).*, built on";

/// Creates a module with the current Java version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("java");
    let config: JavaConfig = JavaConfig::try_load(module.config);

    let is_java_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_java_project {
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
                    let java_version = get_java_version(context)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &java_version,
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
            log::warn!("Error in module `java`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_java_version(context: &Context) -> Option<String> {
    let java_command = context
        .get_env("JAVA_HOME")
        .map(PathBuf::from)
        .and_then(|path| {
            path.join("bin")
                .join("java")
                .into_os_string()
                .into_string()
                .ok()
        })
        .unwrap_or_else(|| String::from("java"));

    let output = context.exec_cmd(java_command, &["-Xinternalversion"])?;
    let java_version_string = get_command_string_output(output);

    parse_java_version(&java_version_string)
}

fn parse_java_version(java_version_string: &str) -> Option<String> {
    let re = Regex::new(JAVA_VERSION_PATTERN).ok()?;
    let captures = re.captures(java_version_string)?;
    let version = &captures["version"];

    Some(version.to_string())
}

