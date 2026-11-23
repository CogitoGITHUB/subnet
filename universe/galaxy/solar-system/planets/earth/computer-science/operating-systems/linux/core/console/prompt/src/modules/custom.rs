use std::env;
use std::fmt::{self, Debug};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use process_control::{ChildExt, Control, Output};

use super::{Context, Module, ModuleConfig};

use crate::{
    config::Either, configs::custom::CustomConfig, formatter::StringFormatter,
    utils::create_command,
};

/// Creates a custom module with some configuration
///
/// The relevant TOML config will set the files, extensions, and directories needed
/// for the module to be displayed. If none of them match, and optional "when"
/// command can be run -- if its result is 0, the module will be shown.
///
/// Finally, the content of the module itself is also set by a command.
pub fn module<'a>(name: &str, context: &'a Context) -> Option<Module<'a>> {
    let toml_config = get_config(name, context)?;
    let config = CustomConfig::load(toml_config);
    if config.disabled {
        return None;
    }

    if let Some(os) = config.os
        && os != env::consts::OS
        && !(os == "unix" && cfg!(unix))
    {
        return None;
    }

    if config.require_repo && context.get_git_repo().is_err() {
        return None;
    }

    // Note: Forward config if `Module` ends up needing `config`
    let mut module = Module::new(format!("custom.{name}"), config.description, None);

    let mut is_match = context
        .try_begin_scan()?
        .set_extensions(&config.detect_extensions)
        .set_files(&config.detect_files)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_match {
        is_match = match config.when {
            Either::First(b) => b,
            Either::Second(s) => exec_when(s, &config, context),
        };

        if !is_match {
            return None;
        }
    }

    let variables_closure = |variable: &str| match variable {
        "output" => {
            let output = exec_command(config.command, context, &config)?;
            let trimmed = output.trim();

            if trimmed.is_empty() {
                None
            } else {
                Some(Ok(trimmed.to_string()))
            }
        }
        _ => None,
    };

    let parsed = StringFormatter::new(config.format).and_then(|mut formatter| {
        formatter = formatter
            .map_meta(|var, _| match var {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            });

        if config.unsafe_no_escape {
            formatter = formatter.map_no_escaping(variables_closure)
        } else {
            formatter = formatter.map(variables_closure)
        }

        formatter.parse(None, Some(context))
    });

    match parsed {
        Ok(segments) => module.set_segments(segments),
        Err(error) => {
            log::warn!("Error in module `custom.{name}`:\n{error}");
        }
    }
    Some(module)
}

/// Gets the TOML config for the custom module, handling the case where the module is not defined
fn get_config<'a>(module_name: &str, context: &'a Context<'a>) -> Option<&'a toml::Value> {
    struct DebugCustomModules<'tmp>(&'tmp toml::value::Table);

    impl Debug for DebugCustomModules<'_> {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.debug_list().entries(self.0.keys()).finish()
        }
    }

    let config = context.config.get_custom_module_config(module_name);

    if config.is_some() {
        return config;
    } else if let Some(modules) = context.config.get_custom_modules() {
        log::debug!(
            "top level format contains custom module {module_name:?}, but no configuration was provided. Configuration for the following modules were provided: {:?}",
            DebugCustomModules(modules),
        );
    } else {
        log::debug!(
            "top level format contains custom module {module_name:?}, but no configuration was provided.",
        );
    }
    None
}

/// Return the invoking shell, using `shell` and fallbacking in order to `STARSHIP_SHELL` and "sh"/"cmd"
fn get_shell<'a, 'b>(
    shell_args: &'b [&'a str],
    context: &Context,
) -> (std::borrow::Cow<'a, str>, &'b [&'a str]) {
    if !shell_args.is_empty() {
        (shell_args[0].into(), &shell_args[1..])
    } else if let Some(env_shell) = context.get_env("STARSHIP_SHELL") {
        (env_shell.into(), &[] as &[&str])
    } else if cfg!(windows) {
        // `/C` is added by `handle_shell`
        ("cmd".into(), &[] as &[&str])
    } else {
        ("sh".into(), &[] as &[&str])
    }
}

/// Attempt to run the given command in a shell by passing it as either `stdin` or an argument to `get_shell()`,
/// depending on the configuration or by invoking a platform-specific fallback shell if `shell` is empty.
fn shell_command(cmd: &str, config: &CustomConfig, context: &Context) -> Option<Output> {
    let (shell, shell_args) = get_shell(config.shell.0.as_ref(), context);
    let mut use_stdin = config.use_stdin;

    let mut command = match create_command(shell.as_ref()) {
        Ok(command) => command,
        // Don't attempt to use fallback shell if the user specified a shell
        Err(error) if !shell_args.is_empty() => {
            log::debug!(
                "Error creating command with STARSHIP_SHELL, falling back to fallback shell: {error}"
            );

            // Skip `handle_shell` and just set the shell and command
            use_stdin = Some(!cfg!(windows));

            if cfg!(windows) {
                let mut c = create_command("cmd").ok()?;
                c.arg("/C");
                c
            } else {
                let mut c = create_command("/usr/bin/env").ok()?;
                c.arg("sh");
                c
            }
        }
        _ => return None,
    };

    command
        .current_dir(&context.current_dir)
        .args(shell_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let use_stdin = use_stdin.unwrap_or_else(|| handle_shell(&mut command, &shell, shell_args));

    if !use_stdin {
        command.arg(cmd);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            log::debug!(
                "Failed to run command with given shell or STARSHIP_SHELL env variable:: {error}"
            );
            return None;
        }
    };

    if use_stdin {
        child.stdin.as_mut()?.write_all(cmd.as_bytes()).ok()?;
    }

    let mut output = child.controlled_with_output();

    if !config.ignore_timeout {
        output = output
            .time_limit(Duration::from_millis(context.root_config.command_timeout))
            .terminate_for_timeout()
    }

    match output.wait().ok()? {
        None => {
            log::warn!("Executing custom command {cmd:?} timed out.");
            log::warn!(
                "You can set command_timeout in your config to a higher value or set ignore_timeout to true for this module to allow longer-running commands to keep executing."
            );
            None
        }
        Some(status) => Some(status),
    }
}

/// Execute the given command capturing all output, and return whether it return 0
fn exec_when(cmd: &str, config: &CustomConfig, context: &Context) -> bool {
    log::trace!("Running '{cmd}'");

    if let Some(output) = shell_command(cmd, config, context) {
        if !output.status.success() {
            log::trace!("non-zero exit code '{:?}'", output.status.code());
            log::trace!(
                "stdout: {}",
                std::str::from_utf8(&output.stdout).unwrap_or("<invalid utf8>")
            );
            log::trace!(
                "stderr: {}",
                std::str::from_utf8(&output.stderr).unwrap_or("<invalid utf8>")
            );
        }

        output.status.success()
    } else {
        log::debug!("Cannot start command");

        false
    }
}

/// Execute the given command, returning its output on success
fn exec_command(cmd: &str, context: &Context, config: &CustomConfig) -> Option<String> {
    log::trace!("Running '{cmd}'");


    if let Some(output) = shell_command(cmd, config, context) {
        if !output.status.success() {
            log::trace!("Non-zero exit code '{:?}'", output.status.code());
            log::trace!(
                "stdout: {}",
                std::str::from_utf8(&output.stdout).unwrap_or("<invalid utf8>")
            );
            log::trace!(
                "stderr: {}",
                std::str::from_utf8(&output.stderr).unwrap_or("<invalid utf8>")
            );
            return None;
        }

        Some(String::from_utf8_lossy(&output.stdout).into())
    } else {
        None
    }
}

/// If the specified shell refers to `PowerShell`, adds the arguments "-Command -" to the
/// given command.
/// Returns `false` if the shell expects scripts as arguments, `true` if as `stdin`.
fn handle_shell(command: &mut Command, shell: &str, shell_args: &[&str]) -> bool {
    let shell_exe = Path::new(shell).file_stem();
    let no_args = shell_args.is_empty();

    match shell_exe.and_then(std::ffi::OsStr::to_str) {
        Some("pwsh" | "powershell") => {
            if no_args {
                command.arg("-NoProfile").arg("-Command").arg("-");
            }
            true
        }
        Some("cmd") => {
            if no_args {
                command.arg("/C");
            }
            false
        }
        Some("nu") => {
            if no_args {
                command.arg("-c");
            }
            false
        }
        _ => true,
    }
}

