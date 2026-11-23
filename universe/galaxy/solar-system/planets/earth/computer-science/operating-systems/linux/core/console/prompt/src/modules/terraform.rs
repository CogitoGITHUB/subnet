use super::{Context, Module, ModuleConfig};

use crate::configs::terraform::TerraformConfig;
use crate::formatter::StringFormatter;
use crate::utils;

use crate::formatter::VersionFormatter;
use std::io;
use std::path::PathBuf;

/// Creates a module with the current Terraform version and workspace
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("terraform");
    let config: TerraformConfig = TerraformConfig::try_load(module.config);

    let is_terraform_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_folders(&config.detect_folders)
        .set_extensions(&config.detect_extensions)
        .is_match();

    if !is_terraform_project {
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
                    let terraform_version = parse_terraform_version(
                        context
                            .exec_cmds_return_first(&config.commands)?
                            .stdout
                            .as_str(),
                    )?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &terraform_version,
                        config.version_format,
                    )
                }
                .map(Ok),
                "workspace" => get_terraform_workspace(context).map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `terraform`:\n{error}");
            return None;
        }
    });

    Some(module)
}

// Determines the currently selected workspace (see https://github.com/hashicorp/terraform/blob/master/command/meta.go for the original implementation)
fn get_terraform_workspace(context: &Context) -> Option<String> {
    // Workspace can be explicitly overwritten by an env var
    let workspace_override = context.get_env("TF_WORKSPACE");
    if workspace_override.is_some() {
        return workspace_override;
    }

    // Data directory containing current workspace can be overwritten by an env var
    let datadir = match context.get_env("TF_DATA_DIR") {
        Some(s) => PathBuf::from(s),
        None => context.current_dir.join(".terraform"),
    };
    match utils::read_file(datadir.join("environment")) {
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => Some("default".to_string()),
        Ok(s) => Some(s),
        _ => None,
    }
}

fn parse_terraform_version(version: &str) -> Option<String> {
    // `terraform version` or `tofu version` output looks like this
    //   Terraform v0.12.14/OpenTofu v1.7.2
    // with potential extra output if it detects you are not running the latest version

    let version = version
        .lines()
        .next()?
        .trim_start_matches("Terraform ")
        .trim_start_matches("OpenTofu ")
        .trim()
        .trim_start_matches('v');

    Some(version.to_string())
}

