use super::{Context, Module, ModuleConfig};

use crate::configs::os::OSConfig;
use crate::formatter::StringFormatter;

/// Creates a module with the current operating system
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("os");
    let config: OSConfig = OSConfig::try_load(module.config);

    if config.disabled {
        return None;
    }

    #[cfg(not(test))]
    let os = os_info::get();


    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "symbol" => get_symbol(&config, os.os_type()),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "codename" => get_codename(&os).map(Ok),
                "edition" => get_edition(&os).map(Ok),
                "name" => get_name(&os).map(Ok),
                "type" => get_type(&os).map(Ok),
                "version" => get_version(&os).map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });
    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `os`:\n{error}");
            return None;
        }
    });

    Some(module)
}

// Get the operating system symbol from user config, or else default config
// when user has not defined a symbol for the operating system.
fn get_symbol<'a>(config: &'a OSConfig, os_type: os_info::Type) -> Option<&'a str> {
    config
        .get_symbol(os_type)
        .or_else(|| OSConfig::default().get_symbol(os_type))
}

fn get_codename(os: &os_info::Info) -> Option<String> {
    os.codename().map(String::from)
}

fn get_edition(os: &os_info::Info) -> Option<String> {
    os.edition().map(String::from)
}

fn get_name(os: &os_info::Info) -> Option<String> {
    Some(os.os_type().to_string())
}

fn get_type(os: &os_info::Info) -> Option<String> {
    // String from os_info::Type
    Some(format!("{:?}", os.os_type()))
}

fn get_version(os: &os_info::Info) -> Option<String> {
    Some(os.version())
        .filter(|&x| x != &os_info::Version::Unknown)
        .map(os_info::Version::to_string)
}

