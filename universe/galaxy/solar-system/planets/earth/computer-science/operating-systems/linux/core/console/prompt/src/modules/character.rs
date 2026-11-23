use super::{Context, Module, ModuleConfig, Shell};
use crate::configs::character::CharacterConfig;
use crate::formatter::StringFormatter;

/// Creates a module for the prompt character
///
/// The character segment prints an arrow character in a color dependent on the
/// exit-code of the last executed command:
/// - If the exit-code was "0", it will be formatted with `success_symbol`
///   (green arrow by default)
/// - If the exit-code was anything else, it will be formatted with
///   `error_symbol` (red arrow by default)
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    enum ShellEditMode {
        Normal,
        Visual,
        Replace,
        ReplaceOne,
        Insert,
    }
    const ASSUMED_MODE: ShellEditMode = ShellEditMode::Insert;
    // TODO: extend config to more modes

    let mut module = context.new_module("character");
    let config: CharacterConfig = CharacterConfig::try_load(module.config);

    let props = &context.properties;
    let exit_code = props.status_code.as_deref().unwrap_or("0");
    let keymap = props.keymap.as_str();
    let exit_success = exit_code == "0";

    // Match shell "keymap" names to normalized vi modes
    // NOTE: in vi mode, fish reports normal mode as "default".
    // Unfortunately, this is also the name of the non-vi default mode.
    // We do some environment detection in src/init.rs to translate.
    // The result: in non-vi fish, keymap is always reported as "insert"
    let mode = match (&context.shell, keymap) {
        (Shell::Fish, "default")
        | (Shell::Zsh, "vicmd")
        | (Shell::Cmd | Shell::PowerShell | Shell::Pwsh, "vi") => ShellEditMode::Normal,
        (Shell::Fish, "visual") => ShellEditMode::Visual,
        (Shell::Fish, "replace") => ShellEditMode::Replace,
        (Shell::Fish, "replace_one") => ShellEditMode::ReplaceOne,
        _ => ASSUMED_MODE,
    };

    let symbol = match mode {
        ShellEditMode::Normal => config.vimcmd_symbol,
        ShellEditMode::Visual => config.vimcmd_visual_symbol,
        ShellEditMode::Replace => config.vimcmd_replace_symbol,
        ShellEditMode::ReplaceOne => config.vimcmd_replace_one_symbol,
        ShellEditMode::Insert => {
            if exit_success {
                config.success_symbol
            } else {
                config.error_symbol
            }
        }
    };

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "symbol" => Some(symbol),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `character`:\n{error}");
            return None;
        }
    });

    Some(module)
}

