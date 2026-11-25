use super::{Context, Module, ModuleConfig};
use crate::configs::package::PackageConfig;
use crate::formatter::{StringFormatter, VersionFormatter};
use crate::utils::read_file;

use ini::Ini;
use jsonc_parser::ParseOptions;
use quick_xml::Reader as QXReader;
use quick_xml::events::Event as QXEvent;
use regex::Regex;
use serde_json as json;
#[cfg(test)]
use std::fs;
use versions::Version;

/// Creates a module with the current package version
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("package");
    let config: PackageConfig = PackageConfig::try_load(module.config);
    let module_version = get_version(context, &config)?;

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
                "version" => Some(Ok(&module_version)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `package`:\n{error}");
            return None;
        }
    });

    Some(module)
}

fn get_node_package_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("package.json")?;
    let package_json: json::Value = json::from_str(&file_contents).ok()?;

    if !config.display_private
        && package_json.get("private").and_then(json::Value::as_bool) == Some(true)
    {
        return None;
    }

    let raw_version = package_json.get("version")?.as_str()?;
    if raw_version == "null" {
        return None;
    }

    let formatted_version = format_version(raw_version, config.version_format)?;
    if formatted_version == "v0.0.0-development" || formatted_version.starts_with("v0.0.0-semantic")
    {
        return Some("semantic".to_string());
    }

    Some(formatted_version)
}

fn get_jsr_package_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let (filename, contents) = ["deno.json", "deno.jsonc", "jsr.json", "jsr.jsonc"]
        .iter()
        .find_map(|filename| {
            context
                .read_file_from_pwd(filename)
                .map(|contents| (filename, contents))
        })?;

    let json_content: json::Value = if filename.ends_with(".jsonc") {
        jsonc_parser::parse_to_serde_value(&contents, &ParseOptions::default()).ok()?
    } else {
        json::from_str(&contents).ok()?
    };

    let raw_version = json_content.get("version")?.as_str()?;
    format_version(raw_version, config.version_format)
}

fn get_poetry_version(pyproject: &toml::Table) -> Option<String> {
    pyproject
        .get("tool")?
        .get("poetry")?
        .get("version")?
        .as_str()
        .map(std::borrow::ToOwned::to_owned)
}

fn parse_file_version_for_hatchling(context: &Context, path: &str) -> Option<String> {
    let file_contents = read_file(context.current_dir.join(path)).ok()?;
    // https://hatch.pypa.io/latest/version/
    let re = Regex::new(r#"(__version__|VERSION)\s*=\s*["']([^"']+)["']"#).ok()?;
    Some(
        re.captures(&file_contents)
            .and_then(|cap| cap.get(2))?
            .as_str()
            .to_owned(),
    )
}

fn parse_hatchling_dynamic_version(context: &Context, pyproject: &toml::Table) -> Option<String> {
    let version_path = pyproject
        .get("tool")?
        .get("hatch")?
        .get("version")?
        .get("path")?
        .as_str()?;

    parse_file_version_for_hatchling(context, version_path)
        .filter(|s| Version::new(s.as_str()).is_some())
}

fn parse_pep621_dynamic_version(context: &Context, pyproject: &toml::Table) -> Option<String> {
    // TODO: Flit, PDM, Setuptools
    // https://packaging.python.org/en/latest/discussions/single-source-version#build-system-version-handling
    parse_hatchling_dynamic_version(context, pyproject)
}

fn get_pep621_dynamic_version(context: &Context, pyproject: &toml::Table) -> Option<String> {
    pyproject
        .get("project")?
        .get("dynamic")?
        .as_array()?
        .iter()
        .any(|v| v.as_str() == Some("version"))
        .then(|| parse_pep621_dynamic_version(context, pyproject))?
}

fn get_pep621_static_version(pyproject: &toml::Table) -> Option<String> {
    pyproject
        .get("project")?
        .get("version")?
        .as_str()
        .map(std::borrow::ToOwned::to_owned)
}

fn get_pep621_version(context: &Context, pyproject: &toml::Table) -> Option<String> {
    get_pep621_static_version(pyproject).or_else(|| get_pep621_dynamic_version(context, pyproject))
}

fn get_pyproject_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("pyproject.toml")?;
    let pyproject_toml: toml::Table = toml::from_str(&file_contents).ok()?;

    get_pep621_version(context, &pyproject_toml)
        .or_else(|| get_poetry_version(&pyproject_toml))
        .and_then(|raw_version| format_string_version(raw_version, config.version_format))
}

fn get_setup_cfg_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("setup.cfg")?;
    let ini = Ini::load_from_str(&file_contents).ok()?;
    let raw_version = ini.get_from(Some("metadata"), "version")?;

    if raw_version.starts_with("attr:") || raw_version.starts_with("file:") {
        None
    } else {
        format_version(raw_version, config.version_format)
    }
}

fn get_gradle_version(context: &Context, config: &PackageConfig) -> Option<String> {
    context
        .read_file_from_pwd("gradle.properties")
        .and_then(|contents| {
            let re = Regex::new(r"(?m)^\s*version\s*=\s*(?P<version>.*)").unwrap();
            let caps = re.captures(&contents)?;
            format_version(&caps["version"], config.version_format)
        }).or_else(|| {
            let build_file_contents = context
                .read_file_from_pwd("build.gradle")
                .or_else(|| context.read_file_from_pwd("build.gradle.kts"))?;
            let re = Regex::new(r#"(?m)^version( |\s*=\s*)['"](?P<version>[^'"]+)['"]$"#).unwrap(); /*dark magic*/
            let caps = re.captures(&build_file_contents)?;
            format_version(&caps["version"], config.version_format)

        })
}

fn get_composer_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("composer.json")?;
    let composer_json: json::Value = json::from_str(&file_contents).ok()?;
    let raw_version = composer_json.get("version")?.as_str()?;

    format_version(raw_version, config.version_format)
}

fn get_julia_project_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("Project.toml")?;
    let project_toml: toml::Table = toml::from_str(&file_contents).ok()?;
    let raw_version = project_toml.get("version")?.as_str()?;

    format_version(raw_version, config.version_format)
}

fn get_helm_package_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("Chart.yaml")?;
    let yaml = yaml_rust2::YamlLoader::load_from_str(&file_contents).ok()?;
    let version = yaml.first()?["version"].as_str()?;

    format_version(version, config.version_format)
}

fn get_mix_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("mix.exs")?;
    let re = Regex::new(r#"(?m)version: "(?P<version>[^"]+)""#).unwrap();
    let caps = re.captures(&file_contents)?;

    format_version(&caps["version"], config.version_format)
}

fn get_maven_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("pom.xml")?;
    let mut reader = QXReader::from_str(&file_contents);
    reader.config_mut().trim_text(true);

    let mut buf = vec![];
    let mut in_ver = false;
    let mut depth = 0;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(QXEvent::Start(ref e)) => {
                in_ver = depth == 1 && e.name().as_ref() == "version";
                depth += 1;
            }
            Ok(QXEvent::End(_)) => {
                in_ver = false;
                depth -= 1;
            }
            Ok(QXEvent::Text(t)) if in_ver => {
                // Ignore version which is just a property reference
                if !t.as_ref().starts_with('$') {
                    let ver = t.as_ref();
                    return format_version(ver, config.version_format);
                }

                return None;
            }
            Ok(QXEvent::Eof) => break,
            Ok(_) => (),

            Err(err) => {
                log::warn!("Error parsing pom.xml`:\n{err}");
                break;
            }
        }
    }

    None
}

fn get_meson_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context
        .read_file_from_pwd("meson.build")?
        .split_ascii_whitespace()
        .collect::<String>();

    let re = Regex::new(r"project\([^())]*,version:'(?P<version>[^']+)'[^())]*\)").unwrap();
    let caps = re.captures(&file_contents)?;

    format_version(&caps["version"], config.version_format)
}

fn get_vmod_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("v.mod")?;
    let re = Regex::new(r"(?m)^\s*version\s*:\s*'(?P<version>[^']+)'").unwrap();
    let caps = re.captures(&file_contents)?;
    format_version(&caps["version"], config.version_format)
}

fn get_vpkg_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("vpkg.json")?;
    let vpkg_json: json::Value = json::from_str(&file_contents).ok()?;
    let raw_version = vpkg_json.get("version")?.as_str()?;

    format_version(raw_version, config.version_format)
}

fn get_sbt_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("build.sbt")?;
    let re = Regex::new(r"(?m)^(.*/)*\s*version\s*:=\s*.(?P<version>[\d\.]+)").unwrap();
    let caps = re.captures(&file_contents)?;
    format_version(&caps["version"], config.version_format)
}

fn get_cargo_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("Cargo.toml")?;

    let cargo_toml: toml::Table = toml::from_str(&file_contents).ok()?;
    let cargo_version = cargo_toml.get("package").and_then(|p| p.get("version"));
    let ancestor_cargo_toml;
    let raw_version = if let Some(version) = cargo_version.and_then(toml::Value::as_str) {
        // regular version string
        version
    } else if cargo_version
        .and_then(|version| version.get("workspace"))
        .and_then(toml::Value::as_bool)
        .unwrap_or_default()
    {
        // workspace version string (`package.version.workspace = true`)
        // need to read the Cargo.toml file from the workspace root
        let workspace_toml = if cargo_toml.contains_key("workspace") {
            &cargo_toml
        } else {
            // Assume the workspace root is the first ancestor with a readable Cargo.toml.
            let workspace_contents = context
                .current_dir
                .ancestors()
                .skip(1)
                .find_map(|path| read_file(path.join("Cargo.toml")).ok())?;
            ancestor_cargo_toml = toml::from_str(&workspace_contents).ok()?;
            &ancestor_cargo_toml
        };

        workspace_toml
            .get("workspace")?
            .get("package")?
            .get("version")?
            .as_str()?
    } else {
        // This might be a workspace file
        cargo_toml
            .get("workspace")?
            .get("package")?
            .get("version")?
            .as_str()?
    };

    format_version(raw_version, config.version_format)
}

fn get_nimble_version(context: &Context, config: &PackageConfig) -> Option<String> {
    if !context
        .try_begin_scan()?
        .set_extensions(&["nimble"])
        .is_match()
    {
        return None;
    }

    let cmd_output = context.exec_cmd("nimble", &["dump", "--json"])?;
    let nimble_json: json::Value = json::from_str(&cmd_output.stdout).ok()?;

    let raw_version = nimble_json.get("version")?.as_str()?;

    format_version(raw_version, config.version_format)
}

fn get_shard_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("shard.yml")?;

    let data = yaml_rust2::YamlLoader::load_from_str(&file_contents).ok()?;
    let raw_version = data.first()?["version"].as_str()?;

    format_version(raw_version, config.version_format)
}

fn get_daml_project_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("daml.yaml")?;

    let daml_yaml = yaml_rust2::YamlLoader::load_from_str(&file_contents).ok()?;
    let raw_version = daml_yaml.first()?["version"].as_str()?;

    format_version(raw_version, config.version_format)
}

fn get_dart_pub_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("pubspec.yaml")?;

    let data = yaml_rust2::YamlLoader::load_from_str(&file_contents).ok()?;
    let raw_version = data.first()?["version"].as_str()?;

    format_version(raw_version, config.version_format)
}

fn get_rlang_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("DESCRIPTION")?;
    let re = Regex::new(r"(?m)^Version:\s*(?P<version>.*$)").unwrap();
    let caps = re.captures(&file_contents)?;
    format_version(&caps["version"], config.version_format)
}

fn get_galaxy_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("galaxy.yml")?;
    let data = yaml_rust2::YamlLoader::load_from_str(&file_contents).ok()?;
    let raw_version = data.first()?["version"].as_str()?;

    format_version(raw_version, config.version_format)
}

fn get_zig_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let file_contents = context.read_file_from_pwd("build.zig.zon")?;
    let re = Regex::new(r#"(?m)^\s*\.version\s*=\s*"(?P<version>[^"]+)""#).unwrap();
    let caps = re.captures(&file_contents)?;

    format_version(&caps["version"], config.version_format)
}

fn get_version(context: &Context, config: &PackageConfig) -> Option<String> {
    let package_version_fn: Vec<fn(&Context, &PackageConfig) -> Option<String>> = vec![
        get_cargo_version,
        get_nimble_version,
        get_node_package_version,
        get_jsr_package_version,
        get_pyproject_version,
        get_setup_cfg_version,
        get_composer_version,
        get_gradle_version,
        get_julia_project_version,
        get_mix_version,
        get_helm_package_version,
        get_maven_version,
        get_meson_version,
        get_shard_version,
        get_vmod_version,
        get_vpkg_version,
        get_sbt_version,
        get_daml_project_version,
        get_dart_pub_version,
        get_rlang_version,
        get_galaxy_version,
        get_zig_version,
    ];

    package_version_fn.iter().find_map(|f| f(context, config))
}

fn format_string_version(version: String, version_format: &str) -> Option<String> {
    let cleaned = version
        .replace('"', "")
        .trim()
        .trim_start_matches('v')
        .to_string();

    VersionFormatter::format_module_version("package", &cleaned, version_format)
}

fn format_version(version: &str, version_format: &str) -> Option<String> {
    format_string_version(version.to_string(), version_format)
}

