use super::{Context, Module, ModuleConfig};

use crate::configs::solidity::SolidityConfig;
use crate::formatter::{StringFormatter, VersionFormatter};
use crate::utils::get_command_string_output;

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("solidity");
    let config = SolidityConfig::try_load(module.config);

    let is_solidity_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_solidity_project {
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
                "version" => {
                    let version = get_solidity_version(context, &config)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &version,
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
            log::warn!("Error in module 'solidity'\n {error}");
            return None;
        }
    });
    Some(module)
}

fn get_solidity_version(context: &Context, config: &SolidityConfig) -> Option<String> {
    let version = config
        .compiler
        .0
        .iter()
        .find_map(|compiler_name| context.exec_cmd(compiler_name, &["--version"]))
        .map(get_command_string_output)?;

    parse_solidity_version(&version)
}

fn parse_solidity_version(version: &str) -> Option<String> {
    /*solc --version output looks like "solc, the solidity compiler commandline interface Version: 0.8.16+commit.07a7930e.Linux.g++"
    solcjs --version out looks like 0.8.15+commit.e14f2714.Emscripten.clang */
    let version_var = match version.split_whitespace().nth(7) {
        // Will return Some(x) for solc --version and None for solcjs --version
        Some(c) => c.split_terminator('+').next()?, //Isolates the versioning number e.g "0.8.16"
        None => version.split_terminator('+').next()?, //Isolates the version number e.g "0.8.15"
    };

    Some(version_var.to_string())
}

