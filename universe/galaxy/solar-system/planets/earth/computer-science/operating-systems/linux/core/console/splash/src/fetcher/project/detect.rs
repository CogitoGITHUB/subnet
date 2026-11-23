//! Walk-up manifest detection — tries each known manifest format in priority order, returning
//! the first one found by ascending from the process CWD.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Default)]
pub struct ManifestData {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub ecosystem: &'static str,
}

pub fn detect_manifest() -> Option<ManifestData> {
    let cwd = std::env::current_dir().ok()?;
    walk_up(&cwd)
}

pub fn cwd_cache_component() -> String {
    std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

fn walk_up(start: &Path) -> Option<ManifestData> {
    let mut cur: &Path = start;
    loop {
        if let Some(d) = try_in_dir(cur) {
            return Some(d);
        }
        cur = cur.parent()?;
    }
}

fn try_in_dir(dir: &Path) -> Option<ManifestData> {
    try_cargo(dir)
        .or_else(|| try_package_json(dir))
        .or_else(|| try_pyproject(dir))
        .or_else(|| try_go_mod(dir))
        .or_else(|| try_composer(dir))
}

// ── Cargo.toml ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CargoToml {
    package: Option<CargoPackage>,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: Option<String>,
    version: Option<toml::Value>,
    description: Option<String>,
    license: Option<String>,
}

fn try_cargo(dir: &Path) -> Option<ManifestData> {
    let text = read_file(dir.join("Cargo.toml"))?;
    let t: CargoToml = toml::from_str(&text).ok()?;
    let pkg = t.package?;
    Some(ManifestData {
        name: pkg.name,
        version: toml_string(pkg.version),
        description: pkg.description,
        license: pkg.license,
        ecosystem: "cargo",
    })
}

fn toml_string(v: Option<toml::Value>) -> Option<String> {
    match v? {
        toml::Value::String(s) => Some(s),
        toml::Value::Table(t) => t
            .get("version")
            .and_then(|v| v.as_str().map(str::to_string)),
        _ => None,
    }
}

// ── package.json ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PackageJson {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    license: Option<serde_json::Value>,
}

fn try_package_json(dir: &Path) -> Option<ManifestData> {
    let text = read_file(dir.join("package.json"))?;
    let p: PackageJson = serde_json::from_str(&text).ok()?;
    Some(ManifestData {
        name: p.name,
        version: p.version,
        description: p.description,
        license: p.license.and_then(json_license),
        ecosystem: "npm",
    })
}

fn json_license(v: serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Object(m) => m.get("type").and_then(|v| v.as_str().map(str::to_string)),
        _ => None,
    }
}

// ── pyproject.toml ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PyprojectToml {
    project: Option<PyprojectProject>,
    tool: Option<PyprojectTool>,
}

#[derive(Deserialize)]
struct PyprojectProject {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    license: Option<toml::Value>,
}

#[derive(Deserialize)]
struct PyprojectTool {
    poetry: Option<PoetryProject>,
}

#[derive(Deserialize)]
struct PoetryProject {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    license: Option<String>,
}

fn try_pyproject(dir: &Path) -> Option<ManifestData> {
    let text = read_file(dir.join("pyproject.toml"))?;
    let t: PyprojectToml = toml::from_str(&text).ok()?;
    if let Some(p) = t.project {
        return Some(ManifestData {
            name: p.name,
            version: p.version,
            description: p.description,
            license: pyproject_license(p.license),
            ecosystem: "python",
        });
    }
    if let Some(poetry) = t.tool.and_then(|t| t.poetry) {
        return Some(ManifestData {
            name: poetry.name,
            version: poetry.version,
            description: poetry.description,
            license: poetry.license,
            ecosystem: "python",
        });
    }
    None
}

fn pyproject_license(v: Option<toml::Value>) -> Option<String> {
    match v? {
        toml::Value::String(s) => Some(s),
        toml::Value::Table(t) => t
            .get("text")
            .or_else(|| t.get("expression"))
            .and_then(|v| v.as_str().map(str::to_string)),
        _ => None,
    }
}

// ── go.mod ───────────────────────────────────────────────────────────────────

fn try_go_mod(dir: &Path) -> Option<ManifestData> {
    let text = read_file(dir.join("go.mod"))?;
    let name = text
        .lines()
        .find(|l| l.starts_with("module "))?
        .strip_prefix("module ")?
        .trim()
        .to_string();
    Some(ManifestData {
        name: Some(name),
        version: None,
        description: None,
        license: None,
        ecosystem: "go",
    })
}

// ── composer.json ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ComposerJson {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    license: Option<serde_json::Value>,
}

fn try_composer(dir: &Path) -> Option<ManifestData> {
    let text = read_file(dir.join("composer.json"))?;
    let c: ComposerJson = serde_json::from_str(&text).ok()?;
    Some(ManifestData {
        name: c.name,
        version: c.version,
        description: c.description,
        license: c.license.and_then(|v| match v {
            serde_json::Value::String(s) => Some(s),
            serde_json::Value::Array(arr) => arr
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .reduce(|a, b| format!("{a}/{b}")),
            _ => None,
        }),
        ecosystem: "php",
    })
}

// ── I/O helper ───────────────────────────────────────────────────────────────

fn read_file(path: PathBuf) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

