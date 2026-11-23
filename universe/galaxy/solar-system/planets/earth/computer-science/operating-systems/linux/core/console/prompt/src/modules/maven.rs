use std::path::Path;

use crate::{
    config::ModuleConfig,
    configs::maven::MavenConfig,
    context::Context,
    formatter::{StringFormatter, VersionFormatter},
    module::Module,
    utils,
};

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("maven");
    let config = MavenConfig::try_load(module.config);
    let wrapper_properties = get_wrapper_properties_file(context, config.recursive);
    let is_maven_project = wrapper_properties.is_some()
        || context
            .try_begin_scan()?
            .set_files(&config.detect_files)
            .set_extensions(&config.detect_extensions)
            .set_folders(&config.detect_folders)
            .is_match();

    if !is_maven_project {
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
                    let properties = wrapper_properties.as_deref()?;
                    let maven_version = parse_maven_version_from_properties(properties)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &maven_version,
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
            log::warn!("Error in module `maven`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_maven_version_from_properties(wrapper_properties: &str) -> Option<String> {
    // Example `maven-wrapper.properties` content
    /*
        wrapperVersion=3.3.4
        distributionType=only-script
        distributionUrl=https://repo.maven.apache.org/maven2/org/apache/maven/apache-maven/3.9.12/apache-maven-3.9.12-bin.zip
    */
    let version = wrapper_properties
        .lines()
        .find(|line| line.starts_with("distributionUrl="))?
        .rsplit_once('/')?
        .1
        .strip_prefix("apache-maven-")?
        .rsplit_once('-')?
        .0;
    Some(version.to_string())
}

/// Tries to find the maven-wrapper.properties file.
fn get_wrapper_properties_file(context: &Context, recursive: bool) -> Option<String> {
    let read_wrapper_properties = |base_dir: &Path| {
        utils::read_file(base_dir.join(".mvn/wrapper/maven-wrapper.properties")).ok()
    };

    // Try current directory first
    if context.try_begin_scan()?.set_folders(&[".mvn"]).is_match()
        && let Some(properties) = read_wrapper_properties(&context.current_dir)
    {
        return Some(properties);
    }

    // Try parent directories if recursive
    if recursive
        && let Some(base_dir) = context.begin_ancestor_scan().set_folders(&[".mvn"]).scan()
        && let Some(properties) = read_wrapper_properties(&base_dir)
    {
        return Some(properties);
    }

    None
}

