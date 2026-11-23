use super::{Context, Module};

use crate::config::ModuleConfig;
use crate::configs::hostname::HostnameConfig;
use crate::formatter::StringFormatter;

use whoami::hostname;

/// Creates a module with the system hostname
///
/// Will display the hostname if all of the following criteria are met:
///     - `hostname.disabled` is absent or false
///     - `hostname.ssh_only` is false OR the user is currently connected as an SSH session (`$SSH_CONNECTION`, `$SSH_CLIENT`, `$SSH_TTY`)
///     - `hostname.ssh_only` is false AND `hostname.detect_env_vars` is either empty or contains a defined environment variable
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("hostname");
    let config: HostnameConfig = HostnameConfig::try_load(module.config);

    let is_ssh = context.is_ssh_session();
    if (config.ssh_only && !is_ssh) || !context.detect_env_vars(&config.detect_env_vars) {
        return None;
    }

    let host = hostname()
        .inspect_err(|e| log::warn!("Failed to get hostname: {e}"))
        .ok()?;

    let mut host = if !config.trim_at.is_empty()
        && let Some(index) = host.find(config.trim_at)
    {
        host.split_at(index).0
    } else {
        host.as_ref()
    };

    if let Some(&alias) = config.aliases.get(host) {
        host = alias;
    }

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|var, _| match var {
                "ssh_symbol" => {
                    if is_ssh {
                        Some(config.ssh_symbol)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "hostname" => Some(Ok(host)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `hostname`:\n{error}");
            return None;
        }
    });

    Some(module)
}

