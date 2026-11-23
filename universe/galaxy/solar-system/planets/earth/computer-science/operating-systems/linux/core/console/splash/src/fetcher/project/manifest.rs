//! `project_manifest` — reads version, description, name, and license from the nearest
//! project manifest file (Cargo.toml › package.json › pyproject.toml › go.mod › composer.json)
//! walking up from the process CWD. `Text` emits the version string; `Entries` delivers all
//! available fields as key/value rows; `TextBlock` lists them as human-readable lines;
//! `MarkdownTextBlock` formats name / description / version / license as a styled block;
//! `Badge` shows the version with a tone derived from license permissiveness.

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::payload::{
    BadgeData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::detect::{ManifestData, cwd_cache_component, detect_manifest};

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::Entries,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Badge,
];

pub struct ProjectManifest;

#[async_trait]
impl Fetcher for ProjectManifest {
    fn name(&self) -> &str {
        "project_manifest"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Reads project metadata from the nearest manifest file found by walking up from the \
         process CWD — Cargo.toml, package.json, pyproject.toml, go.mod, or composer.json. \
         `Text` emits the version string (useful as a hero subtitle); `Entries` delivers \
         name / version / description / license as key/value rows; `TextBlock` lists the \
         same fields as human-readable lines; `MarkdownTextBlock` formats them as a styled \
         block; `Badge` shows the version with a tone derived from license permissiveness \
         (permissive → ok, copyleft → warn, unknown → warn)."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 5
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("default");
        let raw = format!("{}|{}|{}", self.name(), shape, cwd_cache_component());
        let digest = Sha256::digest(raw.as_bytes());
        let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        format!("{}-{hex}", self.name())
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("1.4.2"),
            Shape::Entries => samples::entries(&[
                ("name", "splashboard"),
                ("version", "1.4.2"),
                ("description", "Terminal splash dashboard"),
                ("license", "MIT"),
            ]),
            Shape::TextBlock => {
                samples::text_block(&["splashboard 1.4.2", "Terminal splash dashboard", "MIT"])
            }
            Shape::MarkdownTextBlock => {
                samples::markdown("# splashboard\n\n_Terminal splash dashboard_\n\n**1.4.2** · MIT")
            }
            Shape::Badge => samples::badge(Status::Ok, "v1.4.2"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let data = detect_manifest().unwrap_or_default();
        let shape = ctx.shape.unwrap_or(Shape::Entries);
        Ok(Payload {
            icon: None,
            status: None,
            format: None,
            body: render_body(&data, shape),
        })
    }
}

fn render_body(data: &ManifestData, shape: Shape) -> Body {
    match shape {
        Shape::Text => text_body(data),
        Shape::TextBlock => text_block_body(data),
        Shape::MarkdownTextBlock => markdown_body(data),
        Shape::Badge => badge_body(data),
        _ => entries_body(data),
    }
}

fn text_body(data: &ManifestData) -> Body {
    Body::Text(TextData {
        value: data.version.clone().unwrap_or_default(),
    })
}

fn text_block_body(data: &ManifestData) -> Body {
    let mut lines = Vec::new();
    if let Some(name) = &data.name {
        let header = match &data.version {
            Some(v) => format!("{name} {v}"),
            None => name.clone(),
        };
        lines.push(header);
    }
    if let Some(desc) = &data.description {
        lines.push(desc.clone());
    }
    if let Some(lic) = &data.license {
        lines.push(lic.clone());
    }
    Body::TextBlock(TextBlockData { lines })
}

fn markdown_body(data: &ManifestData) -> Body {
    let mut sections: Vec<String> = Vec::new();
    if let Some(name) = &data.name {
        sections.push(format!("# {name}"));
    }
    if let Some(desc) = &data.description {
        sections.push(format!("_{desc}_"));
    }
    let footer = match (&data.version, &data.license) {
        (Some(v), Some(l)) => Some(format!("**{v}** · {l}")),
        (Some(v), None) => Some(format!("**{v}**")),
        (None, Some(l)) => Some(l.clone()),
        (None, None) => None,
    };
    if let Some(f) = footer {
        sections.push(f);
    }
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: sections.join("\n\n"),
    })
}

fn badge_body(data: &ManifestData) -> Body {
    match (&data.version, &data.name) {
        (Some(v), _) => Body::Badge(BadgeData {
            status: license_status(data.license.as_deref()),
            label: format!("v{v}"),
        }),
        (None, Some(n)) => Body::Badge(BadgeData {
            status: Status::Warn,
            label: n.clone(),
        }),
        (None, None) => Body::Badge(BadgeData {
            status: Status::Warn,
            label: String::new(),
        }),
    }
}

fn license_status(license: Option<&str>) -> Status {
    match license {
        None => Status::Warn,
        Some(s) => {
            let lower = s.to_ascii_lowercase();
            if lower.contains("agpl") || lower.contains("gpl") {
                Status::Warn
            } else {
                Status::Ok
            }
        }
    }
}

fn entries_body(data: &ManifestData) -> Body {
    let fields: &[(&str, &Option<String>)] = &[
        ("name", &data.name),
        ("version", &data.version),
        ("description", &data.description),
        ("license", &data.license),
    ];
    Body::Entries(EntriesData {
        items: fields
            .iter()
            .filter_map(|(key, val)| {
                val.as_ref().map(|v| Entry {
                    key: key.to_string(),
                    value: Some(v.clone()),
                    status: None,
                })
            })
            .collect(),
    })
}

