use super::{Context, Module, ModuleConfig};
use std::ops::Deref;
use std::path::Path;
use std::sync::LazyLock;

use crate::configs::ocaml::OCamlConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;

#[derive(Debug, PartialEq)]
enum SwitchType {
    Global,
    Local,
}
type OpamSwitch = (SwitchType, String);

/// Creates a module with the current OCaml version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("ocaml");
    let config: OCamlConfig = OCamlConfig::try_load(module.config);
    let is_ocaml_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_folders(&config.detect_folders)
        .set_extensions(&config.detect_extensions)
        .is_match();

    if !is_ocaml_project {
        return None;
    }

    let opam_switch: LazyLock<Option<OpamSwitch>, _> = LazyLock::new(|| get_opam_switch(context));

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "symbol" => Some(config.symbol),
                "switch_indicator" => {
                    let (switch_type, _) = &opam_switch.deref().as_ref()?;
                    match switch_type {
                        SwitchType::Global => Some(config.global_switch_indicator),
                        SwitchType::Local => Some(config.local_switch_indicator),
                    }
                }
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "switch_name" => {
                    let (_, name) = opam_switch.deref().as_ref()?;
                    Some(Ok(name.clone()))
                }
                "version" => {
                    let is_esy_project = context
                        .try_begin_scan()?
                        .set_folders(&["esy.lock"])
                        .is_match();

                    let ocaml_version = if is_esy_project {
                        context.exec_cmd("esy", &["ocaml", "-vnum"])?.stdout
                    } else {
                        context.exec_cmd("ocaml", &["-vnum"])?.stdout
                    };
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        ocaml_version.trim(),
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
            log::warn!("Error in module `ocaml`: \n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_opam_switch(context: &Context) -> Option<OpamSwitch> {
    let opam_switch = context
        .exec_cmd("opam", &["switch", "show", "--safe"])?
        .stdout;

    parse_opam_switch(opam_switch.trim())
}

fn parse_opam_switch(opam_switch: &str) -> Option<OpamSwitch> {
    if opam_switch.is_empty() {
        return None;
    }

    let path = Path::new(opam_switch);
    if path.has_root() {
        Some((SwitchType::Local, path.file_name()?.to_str()?.to_string()))
    } else {
        Some((SwitchType::Global, opam_switch.to_string()))
    }
}

