//! `basic_bars` — labeled bars authored inline. Right for ad-hoc comparisons that don't
//! warrant a dedicated fetcher (manual team-by-team counts, "books read per year", etc.).

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Bar, BarsData, Body, Payload};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Bars];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "bars",
    type_hint: "list of {label, value: u64}",
    required: false,
    default: Some("[]"),
    description: "Labeled bars. Order is preserved (sorting / ranking lives in the renderer via `style = \"medal\"` / `max_items`).",
}];

pub struct BasicBars;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub bars: Vec<BarConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarConfig {
    pub label: String,
    pub value: u64,
}

impl RealtimeFetcher for BasicBars {
    fn name(&self) -> &str {
        "basic_bars"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a `Bars` payload from inline `[widget.options].bars = [{label, value}]`. Row order is preserved as authored; sorting / ranking is a renderer concern (`list_ranking`, `chart_bar`'s sort option)."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::Bars).then(|| {
            Body::Bars(BarsData {
                bars: vec![
                    Bar {
                        label: "alice".into(),
                        value: 12,
                        value_label: None,
                    },
                    Bar {
                        label: "bob".into(),
                        value: 7,
                        value_label: None,
                    },
                ],
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        common::bare(Body::Bars(BarsData {
            bars: opts
                .bars
                .into_iter()
                .map(|b| Bar {
                    label: b.label,
                    value: b.value,
                    value_label: None,
                })
                .collect(),
        }))
    }
}

