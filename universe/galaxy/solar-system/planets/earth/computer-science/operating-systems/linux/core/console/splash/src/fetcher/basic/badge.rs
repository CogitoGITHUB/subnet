//! `basic_badge` — fixed status pill emitted from inline config. Common use: tagging a
//! per-directory dashboard with the environment it represents (`staging`, `prod`, `WIP`).

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{BadgeData, Body, Payload, Status};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Badge];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "status",
        type_hint: "\"ok\" | \"warn\" | \"error\"",
        required: false,
        default: Some("\"ok\""),
        description: "Tone of the pill. Drives the renderer's colour pick (green / yellow / red).",
    },
    OptionSchema {
        name: "label",
        type_hint: "string",
        required: true,
        default: None,
        description: "Short text shown inside the pill.",
    },
];

pub struct BasicBadge;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub status: Option<Status>,
    #[serde(default)]
    pub label: Option<String>,
}

impl RealtimeFetcher for BasicBadge {
    fn name(&self) -> &str {
        "basic_badge"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a single status pill from `[widget.options]`. `status` picks the tone (`ok` / `warn` / `error`), `label` the visible text. Right for tagging a per-directory splash with its environment (`staging`, `prod`), or pinning a stale-data warning."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::Badge).then(|| {
            Body::Badge(BadgeData {
                status: Status::Warn,
                label: "staging".into(),
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let Some(label) = opts.label.filter(|l| !l.trim().is_empty()) else {
            return common::placeholder("basic_badge: `label` is required");
        };
        common::bare(Body::Badge(BadgeData {
            status: opts.status.unwrap_or(Status::Ok),
            label,
        }))
    }
}

