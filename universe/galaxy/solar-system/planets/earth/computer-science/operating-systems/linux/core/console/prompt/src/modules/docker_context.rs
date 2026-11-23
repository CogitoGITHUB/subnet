use std::path::PathBuf;

use super::{Context, Module, ModuleConfig};

use crate::configs::docker_context::DockerContextConfig;
use crate::formatter::StringFormatter;
use crate::utils;

/// Creates a module with the currently active Docker context
///
/// Will display the Docker context if the following criteria are met:
///     - There is a non-empty environment variable named `DOCKER_HOST`
///     - Or there is a non-empty environment variable named `DOCKER_CONTEXT`
///     - Or there is a file named `$HOME/.docker/config.json`
///     - Or a file named `$DOCKER_CONFIG/config.json`
///     - The file is JSON and contains a field named `currentContext`
///     - The value of `currentContext` is not `default` or `desktop-linux`
///     - If multiple criteria are met, we use the following order to define the docker context:
///     - `DOCKER_HOST`, `DOCKER_CONTEXT`, $HOME/.docker/config.json, $`DOCKER_CONFIG/config.json`
///     - (This is the same order docker follows, as `DOCKER_HOST` and `DOCKER_CONTEXT` override the
///     config)
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("docker_context");
    let config: DockerContextConfig = DockerContextConfig::try_load(module.config);

    if config.only_with_files
        && !context
            .try_begin_scan()?
            .set_files(&config.detect_files)
            .set_extensions(&config.detect_extensions)
            .set_folders(&config.detect_folders)
            .is_match()
    {
        return None;
    }

    let docker_config = PathBuf::from(
        &context
            .get_env_os("DOCKER_CONFIG")
            .unwrap_or(context.get_home()?.join(".docker").into_os_string()),
    )
    .join("config.json");

    let docker_context_env = ["DOCKER_MACHINE_NAME", "DOCKER_HOST", "DOCKER_CONTEXT"]
        .into_iter()
        .find_map(|env| context.get_env(env));

    let ctx = if let Some(data) = docker_context_env {
        data
    } else {
        if !docker_config.exists() {
            return None;
        }
        let json = utils::read_file(docker_config).ok()?;
        let parsed_json: serde_json::Value = serde_json::from_str(&json).ok()?;
        parsed_json.get("currentContext")?.as_str()?.to_owned()
    };

    let default_contexts = ["default", "desktop-linux"];
    if default_contexts.contains(&ctx.as_str()) || ctx.starts_with("unix://") {
        return None;
    }

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
                "context" => Some(Ok(ctx.as_str())),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `docker_context`:\n{error}");
            return None;
        }
    });
    Some(module)
}

