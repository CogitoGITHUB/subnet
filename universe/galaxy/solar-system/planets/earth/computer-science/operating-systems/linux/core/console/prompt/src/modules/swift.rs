use super::{Context, Module, ModuleConfig};

use crate::configs::swift::SwiftConfig;
use crate::formatter::StringFormatter;
use crate::formatter::VersionFormatter;
use crate::utils::CommandOutput;

/// Creates a module with the current Swift version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("swift");
    let config: SwiftConfig = SwiftConfig::try_load(module.config);

    let is_swift_project = context
        .try_begin_scan()?
        .set_files(&config.detect_files)
        .set_folders(&config.detect_folders)
        .set_extensions(&config.detect_extensions)
        .is_match();

    if !is_swift_project {
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
                "version" => {
                    let swift_version =
                        parse_swift_version(&swift_version_output(context)?.stdout)?;
                    VersionFormatter::format_module_version(
                        module.get_name(),
                        &swift_version,
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
            log::warn!("Error in module `swift`:\n{error}");
            return None;
        }
    });

    Some(module)
}

/// Returns the output of the fastest available `swift` version query.
///
/// `swift --version` goes through the Swift driver, which locates the
/// toolchain on every invocation. On macOS that alone can take seconds,
/// tripping the global command timeout so the module disappears (see #7639).
/// `swift-frontend --version` prints a compatible first line but starts much
/// faster, so prefer it and fall back to the driver.
fn swift_version_output(context: &Context) -> Option<CommandOutput> {
    let output = context.exec_cmd("swift-frontend", &["--version"]);
    if output
        .as_ref()
        .is_some_and(|o| parse_swift_version(&o.stdout).is_some())
    {
        return output;
    }
    #[cfg(target_os = "macos")]
    {
        // `swift-frontend` usually isn't on PATH on macOS; locate it inside
        // the active toolchain instead. `xcrun --find` itself is fast.
        if let Some(found) = context.exec_cmd("xcrun", &["--find", "swift-frontend"]) {
            let output = context.exec_cmd(found.stdout.trim(), &["--version"]);
            if output
                .as_ref()
                .is_some_and(|o| parse_swift_version(&o.stdout).is_some())
            {
                return output;
            }
        }
    }
    context.exec_cmd("swift", &["--version"])
}

fn parse_swift_version(swift_version: &str) -> Option<String> {
    // split into ["Apple", "Swift", "version", "5.2.2", ...] or
    //            ["Swift", "version", "5.3-dev", ...]
    let mut split = swift_version.split_whitespace();
    let _ = split.position(|t| t == "version")?;
    // return "5.2.2" or "5.3-dev"
    let version = split.next()?;

    Some(version.to_string())
}

