use regex::Regex;

use super::{Context, Module, ModuleConfig};

use crate::configs::ruby::RubyConfig;
use crate::formatter::{StringFormatter, VersionFormatter};

/// Creates a module with the current Ruby version
///
/// Will display the Ruby version if any of the following criteria are met:
///     - Current directory contains a `.rb` file
///     - Current directory contains a `Gemfile` or `.ruby-version` file
///     - The environment variables `RUBY_VERSION` or `RBENV_VERSION` are set
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("ruby");
    let config = RubyConfig::try_load(module.config);

    let is_rb_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    let is_rb_env = &config
        .detect_variables
        .iter()
        .any(|variable| context.get_env(variable).is_some());

    if !is_rb_project && !is_rb_env {
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
                "version" => format_ruby_version(
                    &context.exec_cmd("ruby", &["-v"])?.stdout,
                    config.version_format,
                )
                .map(Ok),
                "gemset" => {
                    format_rvm_gemset(&context.exec_cmd("rvm", &["current"])?.stdout).map(Ok)
                }
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `ruby`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn format_ruby_version(ruby_version: &str, version_format: &str) -> Option<String> {
    let version = ruby_version
        // split into ["ruby", "2.6.0p0", "linux/amd64"]
        .split_whitespace()
        // return "2.6.0p0"
        .nth(1)?
        // split into ["2.6.0", "0"]
        .split('p')
        // return "2.6.0"
        .next()?;

    match VersionFormatter::format_version(version, version_format) {
        Ok(formatted) => Some(formatted),
        Err(error) => {
            log::warn!("Error formatting `ruby` version:\n{error}");
            Some(format!("v{version}"))
        }
    }
}

fn format_rvm_gemset(current: &str) -> Option<String> {
    let gemset_re = Regex::new(r"@(\S+)").unwrap();
    if let Some(gemset) = gemset_re.captures(current) {
        let gemset_name = gemset.get(1)?.as_str();
        return Some(gemset_name.to_string());
    }

    None
}

