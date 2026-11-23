use super::{Context, Module, ModuleConfig, Shell};

use crate::configs::shell::ShellConfig;
use crate::formatter::StringFormatter;

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("shell");
    let config: ShellConfig = ShellConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    let shell = &context.shell;

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "indicator" => match shell {
                    Shell::Bash => Some(config.bash_indicator),
                    Shell::Fish => Some(config.fish_indicator),
                    Shell::Zsh => Some(config.zsh_indicator),
                    Shell::Pwsh => config.pwsh_indicator.or(Some(config.powershell_indicator)),
                    Shell::PowerShell => Some(config.powershell_indicator),
                    Shell::Ion => Some(config.ion_indicator),
                    Shell::Elvish => Some(config.elvish_indicator),
                    Shell::Tcsh => Some(config.tcsh_indicator),
                    Shell::Nu => Some(config.nu_indicator),
                    Shell::Xonsh => Some(config.xonsh_indicator),
                    Shell::Cmd => Some(config.cmd_indicator),
                    Shell::Unknown => Some(config.unknown_indicator),
                },
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|var| match var {
                "bash_indicator" => Some(Ok(config.bash_indicator)),
                "fish_indicator" => Some(Ok(config.fish_indicator)),
                "zsh_indicator" => Some(Ok(config.zsh_indicator)),
                "powershell_indicator" => Some(Ok(config.powershell_indicator)),
                "pwsh_indicator" => config.pwsh_indicator.map(Ok),
                "ion_indicator" => Some(Ok(config.ion_indicator)),
                "elvish_indicator" => Some(Ok(config.elvish_indicator)),
                "tcsh_indicator" => Some(Ok(config.tcsh_indicator)),
                "xonsh_indicator" => Some(Ok(config.xonsh_indicator)),
                "cmd_indicator" => Some(Ok(config.cmd_indicator)),
                "unknown_indicator" => Some(Ok(config.unknown_indicator)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `shell`: \n{error}");
            return None;
        }
    });

    Some(module)
}

