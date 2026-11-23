use ini::Ini;
use std::path::Path;

use super::{Context, Module, ModuleConfig};
use crate::configs::python::PythonConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;
use crate::utils::get_command_string_output;

/// Creates a module with the current Python version and, if active, virtual environment.
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("python");
    let config: PythonConfig = PythonConfig::try_load(module.config);

    let is_py_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    let has_env_vars =
        !config.detect_env_vars.is_empty() && context.detect_env_vars(&config.detect_env_vars);

    if !is_py_project && !has_env_vars {
        return None;
    }

    let pyenv_prefix = if config.pyenv_version_name {
        config.pyenv_prefix
    } else {
        ""
    };

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
                    if config.pyenv_version_name {
                        return get_pyenv_version(context).map(Ok);
                    }
                    let python_version = get_python_version(context, &config)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &python_version,
                        config.version_format,
                    )
                    .map(Ok)
                }
                "virtualenv" => {
                    let virtual_env = get_python_virtual_env(context, &config);
                    virtual_env.as_ref().map(|e| Ok(e.trim().to_string()))
                }
                "pyenv_prefix" => Some(Ok(pyenv_prefix.to_string())),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `python`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_pyenv_version(context: &Context) -> Option<String> {
    let mut version_name = context.get_env("PYENV_VERSION");

    if version_name.is_none() {
        version_name = Some(
            context
                .exec_cmd("pyenv", &["version-name"])?
                .stdout
                .trim()
                .to_string(),
        );
    }

    version_name
}

fn get_python_version(context: &Context, config: &PythonConfig) -> Option<String> {
    config
        .python_binary
        .0
        .iter()
        .find_map(|binary| {
            let command = binary.0.first()?;
            let args: Vec<_> = binary
                .0
                .iter()
                .skip(1)
                .copied()
                .chain(std::iter::once("--version"))
                .collect();

            context.exec_cmd(command, &args)
        })
        .map(get_command_string_output)
        .map(|output| parse_python_version(&output))?
}

fn parse_python_version(python_version_string: &str) -> Option<String> {
    let version = python_version_string
        // split into ["Python", "3.8.6", ...]
        .split_whitespace()
        // get down to "3.8.6"
        .nth(1)?;

    Some(version.to_string())
}

fn get_python_virtual_env(context: &Context, config: &PythonConfig) -> Option<String> {
    context.get_env("VIRTUAL_ENV").and_then(|venv| {
        get_prompt_from_venv(Path::new(&venv))
            .or_else(|| get_venv_from_path(Path::new(&venv)))
            .and_then(|venv_name| {
                if config.generic_venv_names.contains(&venv_name.as_str()) {
                    get_venv_from_path(Path::new(&venv).parent()?)
                } else {
                    Some(venv_name)
                }
            })
    })
}

fn get_prompt_from_venv(venv_path: &Path) -> Option<String> {
    Ini::load_from_file_noescape(venv_path.join("pyvenv.cfg"))
        .ok()?
        .general_section()
        .get("prompt")
        .map(|prompt| String::from(prompt.trim_matches(&['(', ')'] as &[_])))
}

fn get_venv_from_path(venv_path: &Path) -> Option<String> {
    venv_path
        .file_name()?
        .to_str()
        .map(std::string::ToString::to_string)
}

