use crate::configs::scala::ScalaConfig;
use crate::formatter::StringFormatter;

use super::{Context, Module, ModuleConfig};
use crate::formatter::VersionFormatter;
use crate::utils::get_command_string_output;

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("scala");
    let config: ScalaConfig = ScalaConfig::try_load(module.config);

    let is_scala_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_scala_project {
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
                    let scala_version = get_scala_version(context)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &scala_version,
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
            log::warn!("Error in module `scala`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_scala_version(context: &Context) -> Option<String> {
    // try to get the version from scala-cli first as it is faster
    // and return ONLY the version which save us from parsing the version string
    context
        .exec_cmd("scala-cli", &["version", "--scala"])
        .filter(|out| !out.stdout.is_empty())
        .map(|std_out_only| std_out_only.stdout.trim().to_string())
        .or_else(|| {
            let command = context.exec_cmd("scalac", &["-version"])?;
            let scala_version_string = get_command_string_output(command);
            parse_scala_version(&scala_version_string)
        })
}

fn parse_scala_version(scala_version_string: &str) -> Option<String> {
    let version = scala_version_string
        // split into ["Scala", "compiler", "version", "2.13.5", "--", ...]
        .split_whitespace()
        // take "2.13.5"
        .nth(3)?;

    Some(version.to_string())
}

