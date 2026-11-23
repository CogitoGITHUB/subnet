use std::path::Path;

use crate::{
    config::ModuleConfig,
    configs::gradle::GradleConfig,
    context::Context,
    formatter::{StringFormatter, VersionFormatter},
    module::Module,
    utils,
};

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("gradle");
    let config = GradleConfig::try_load(module.config);
    let is_gradle_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_gradle_project {
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
                    let gradle_version = {
                        let properties = get_wrapper_properties_file(context, config.recursive)?;
                        parse_gradle_version_from_properties(&properties)?
                    };
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &gradle_version,
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
            log::warn!("Error in module `gradle`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_gradle_version_from_properties(wrapper_properties: &str) -> Option<String> {
    // example gradle.properties content
    /*
        distributionBase=GRADLE_USER_HOME
        distributionPath=wrapper/dists
        distributionUrl=https\://services.gradle.org/distributions/gradle-7.5.1-bin.zip
        zipStoreBase=GRADLE_USER_HOME
        zipStorePath=wrapper/dists
    */
    let version = wrapper_properties
        .lines()
        .find(|line| line.starts_with("distributionUrl="))?
        .rsplit_once('/')?
        .1
        .strip_prefix("gradle-")?
        .rsplit_once('-')?
        .0;
    Some(version.to_string())
}

/// Tries to find the gradle-wrapper.properties file.
fn get_wrapper_properties_file(context: &Context, recursive: bool) -> Option<String> {
    let read_wrapper_properties = |base_dir: &Path| {
        utils::read_file(base_dir.join("gradle/wrapper/gradle-wrapper.properties")).ok()
    };

    // Try current directory first
    if context
        .try_begin_scan()?
        .set_folders(&["gradle"])
        .is_match()
        && let Some(properties) = read_wrapper_properties(&context.current_dir)
    {
        return Some(properties);
    }

    // Try parent directories if recursive
    if recursive {
        for dir in context.current_dir.ancestors().skip(1) {
            if let Some(properties) = read_wrapper_properties(dir) {
                return Some(properties);
            }
        }
    }

    None
}

