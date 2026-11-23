//! `basic_ratio` — single 0..=1 progress reading authored inline. Right for manual progress
//! displays (OKR, side-project completion %, "we've finished N of M chapters") where automating
//! the source isn't worth a dedicated fetcher.

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload, RatioData};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Ratio];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "value",
        type_hint: "float (clamped to 0.0..=1.0)",
        required: true,
        default: None,
        description: "Progress as a fraction. Out-of-range values are clamped rather than rejected so a misconfig still renders.",
    },
    OptionSchema {
        name: "label",
        type_hint: "string",
        required: false,
        default: None,
        description: "Optional caption shown beside the gauge.",
    },
    OptionSchema {
        name: "denominator",
        type_hint: "u64",
        required: false,
        default: None,
        description: "Optional total the value is a fraction of (e.g. `denominator = 365` when value tracks day-of-year). Lets renderers print `N of M` instead of percent-only.",
    },
];

pub struct BasicRatio;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub denominator: Option<u64>,
}

impl RealtimeFetcher for BasicRatio {
    fn name(&self) -> &str {
        "basic_ratio"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Single `Ratio` (0..=1) authored inline in config. Pairs with any gauge renderer. Clamps out-of-range values silently — the splash should never be unable to render because of one bad number."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::Ratio).then(|| {
            Body::Ratio(RatioData {
                value: 0.6,
                label: Some("Q2 OKR".into()),
                denominator: None,
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let Some(raw) = opts.value else {
            return common::placeholder("basic_ratio: `value` is required");
        };
        common::bare(Body::Ratio(RatioData {
            value: raw.clamp(0.0, 1.0),
            label: opts.label,
            denominator: opts.denominator,
        }))
    }
}

