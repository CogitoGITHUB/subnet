use super::{Context, Detected, Module, ModuleConfig};

use crate::configs::username::UsernameConfig;
use crate::formatter::StringFormatter;

#[cfg(not(target_os = "windows"))]
const USERNAME_ENV_VAR: &str = "USER";

#[cfg(target_os = "windows")]
const USERNAME_ENV_VAR: &str = "USERNAME";

/// Creates a module with the current user's username
///
/// Will display the username if any of the following criteria are met:
///     - The current user is root (UID = 0) [1]
///     - The current user isn't the same as the one that is logged in (`$LOGNAME` != `$USER`) [2]
///     - The user is currently connected as an SSH session (`$SSH_CONNECTION`) [3]
///     - The option `username.detect_env_vars` is set with a not negated environment variable [4]
/// Does not display the username:
///     - If the option `username.detect_env_vars` is set with a negated environment variable [A]
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    #[cfg(not(any(test, target_os = "android")))]
    let mut username = whoami::username()
        .inspect_err(|e| log::debug!("Failed to get username {e:?}"))
        .ok()
        .or_else(|| context.get_env(USERNAME_ENV_VAR))?;

    #[cfg(any(test, target_os = "android"))]
    let mut username = context.get_env(USERNAME_ENV_VAR)?;

    let mut module = context.new_module("username");
    let config: UsernameConfig = UsernameConfig::try_load(module.config);
    let has_detected_env_var = context.detect_env_vars2(&config.detect_env_vars);

    let is_root = is_root_user();
    if cfg!(target_os = "windows") && is_root {
        username = "Administrator".to_string();
    }

    let show_username = config.show_always
        || is_root // [1]
        || !is_login_user(context, &username) // [2]
        || context.is_ssh_session() // [3]
        || has_detected_env_var == Detected::Yes; // [4]

    if !show_username || has_detected_env_var == Detected::Negated {
        return None; // [A]
    }

    if let Some(&alias) = config.aliases.get(&username) {
        username = alias.to_string();
    }

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "style" => {
                    let module_style = if is_root {
                        config.style_root
                    } else {
                        config.style_user
                    };
                    Some(Ok(module_style))
                }
                _ => None,
            })
            .map(|variable| match variable {
                "user" => Some(Ok(&username)),
                _ => None,
            })
            .parse(None, Some(context))
    });
    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `username`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn is_login_user(context: &Context, username: &str) -> bool {
    context
        .get_env("LOGNAME")
        .is_none_or(|logname| logname == username)
}

#[cfg(all(target_os = "windows", not(test)))]
fn is_root_user() -> bool {
    use deelevate::{PrivilegeLevel, Token};
    let token = match Token::with_current_process() {
        Ok(token) => token,
        Err(e) => {
            log::warn!("Failed to get process token: {e:?}");
            return false;
        }
    };
    matches!(
        match token.privilege_level() {
            Ok(level) => level,
            Err(e) => {
                log::warn!("Failed to get privilege level: {e:?}");
                return false;
            }
        },
        PrivilegeLevel::Elevated | PrivilegeLevel::HighIntegrityAdmin
    )
}

#[cfg(test)]
fn is_root_user() -> bool {
    false
}

#[cfg(all(not(target_os = "windows"), not(test)))]
fn is_root_user() -> bool {
    nix::unistd::geteuid() == nix::unistd::ROOT
}

