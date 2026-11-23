use super::{Context, Module, ModuleConfig};

use crate::configs::nix_shell::NixShellConfig;
use crate::formatter::StringFormatter;

enum NixShellType {
    Pure,
    Impure,
    /// We're in a Nix shell, but we don't know which type.
    /// This can only happen in a `nix shell` shell (not a `nix-shell` one).
    Unknown,
}

impl NixShellType {
    fn detect_shell_type(use_heuristic: bool, context: &Context) -> Option<Self> {
        use NixShellType::{Impure, Pure, Unknown};

        let shell_type = context.get_env("IN_NIX_SHELL");
        match shell_type.as_deref() {
            Some("pure") => return Some(Pure),
            Some("impure") => return Some(Impure),
            _ => {}
        }

        if use_heuristic {
            Self::in_new_nix_shell(context).map(|()| Unknown)
        } else {
            None
        }
    }

    // Hack to detect if we're in a `nix shell` (in contrast to a `nix-shell`).
    // A better way to do this will be enabled by https://github.com/NixOS/nix/issues/6677.
    fn in_new_nix_shell(context: &Context) -> Option<()> {
        let path = context.get_env("PATH")?;

        std::env::split_paths(&path)
            .any(|path| path.starts_with("/nix/store"))
            .then_some(())
    }
}

/// Creates a module showing if inside a nix-shell
///
/// The module will use the `$IN_NIX_SHELL` and `$name` environment variable to
/// determine if it's inside a nix-shell and the name of it.
///
///
/// The following options are available:
///     - `impure_msg` (string)  // change the impure msg
///     - `pure_msg` (string)    // change the pure msg
///     - `unknown_msg` (string) // change the unknown message
///
/// Will display the following:
///     - pure (name)    // $name == "name" in a pure nix-shell
///     - impure (name)  // $name == "name" in an impure nix-shell
///     - pure           // $name == "" in a pure nix-shell
///     - impure         // $name == "" in an impure nix-shell
///     - unknown (name) // $name == "name" in an unknown nix-shell
///     - unknown        // $name == "" in an unknown nix-shell
///
///  When using Lix 2.95+, will also have $level from `$NIX_SHELL_LEVEL`
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("nix_shell");
    let config: NixShellConfig = NixShellConfig::try_load(module.config);

    let shell_type = NixShellType::detect_shell_type(config.heuristic, context)?;
    let shell_type_format = match shell_type {
        NixShellType::Pure => config.pure_msg,
        NixShellType::Impure => config.impure_msg,
        NixShellType::Unknown => config.unknown_msg,
    };

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "symbol" => Some(config.symbol),
                "state" => Some(shell_type_format),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "name" => context.get_env("name").map(Ok),
                "level" => context.get_env("NIX_SHELL_LEVEL").map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `nix_shell`:\n{error}");
            return None;
        }
    });

    Some(module)
}

