use std::{borrow::Cow, ops::Deref, sync::LazyLock};

use semver::Version;

use crate::{
    config::ModuleConfig,
    configs::fortran::FortranConfig,
    formatter::{StringFormatter, VersionFormatter},
};

use super::{Context, Module};

pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("fortran");
    let config = FortranConfig::try_load(module.config);

    let is_fortran_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_extensions(&config.detect_extensions)
        .set_folders(&config.detect_folders)
        .is_match();

    if !is_fortran_project {
        return None;
    }

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        let compiler_info = LazyLock::new(|| context.exec_cmds_return_first(&config.commands));

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
                "name" => {
                    let compiler_info = &compiler_info.deref().as_ref()?.stdout;

                    let compiler = if compiler_info.contains("GNU") {
                        "gfortran"
                    } else if compiler_info.contains("flang-new") || compiler_info.contains("flang")
                    {
                        "flang"
                    } else {
                        return None;
                    };
                    Some(Ok(Cow::Borrowed(compiler)))
                }
                "version" => {
                    let compiler_info = &compiler_info.deref().as_ref()?.stdout;

                    VersionFormatter::format_module_version(
                        module.get_name(),
                        compiler_info
                            .split_whitespace()
                            .find(|word| Version::parse(word).is_ok())?,
                        config.version_format,
                    )
                    .map(Cow::Owned)
                    .map(Ok)
                }
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `fortran`:\n{error}");
            return None;
        }
    });

    Some(module)
}

