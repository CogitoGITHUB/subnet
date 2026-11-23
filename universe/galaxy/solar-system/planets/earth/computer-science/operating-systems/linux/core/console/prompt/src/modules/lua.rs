use super::{Context, Module, ModuleConfig};

use crate::configs::lua::LuaConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;
use crate::utils::get_command_string_output;

/// Creates a module with the current Lua version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("lua");
    let config = LuaConfig::try_load(module.config);

    let is_lua_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_folders(&config.detect_folders)
        .set_extensions(&config.detect_extensions)
        .is_match();

    if !is_lua_project {
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
                    let lua_version_string =
                        get_command_string_output(context.exec_cmd(config.lua_binary, &["-v"])?);
                    let lua_version = parse_lua_version(&lua_version_string)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &lua_version,
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
            log::warn!("Error in module `lua`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn parse_lua_version(lua_version: &str) -> Option<String> {
    // lua -v output looks like this:
    // Lua 5.4.0  Copyright (C) 1994-2020 Lua.org, PUC-Rio

    // luajit -v output looks like this:
    // LuaJIT 2.0.5 -- Copyright (C) 2005-2017 Mike Pall. http://luajit.org/
    let version = lua_version
        // Lua: split into ["Lua", "5.4.0", "Copyright", ...]
        // LuaJIT: split into ["LuaJIT", "2.0.5", "--", ...]
        .split_whitespace()
        // Lua: take "5.4.0"
        // LuaJIT: take "2.0.5"
        .nth(1)?;

    Some(version.to_string())
}

