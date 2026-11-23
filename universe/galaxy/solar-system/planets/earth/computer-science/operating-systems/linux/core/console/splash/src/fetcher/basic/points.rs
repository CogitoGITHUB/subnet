//! `basic_points` — `PointSeries` authored inline. Supports multiple series for line / scatter
//! charts where each series carries its own name + (x, y) pairs.

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload, PointSeries, PointSeriesData};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::PointSeries];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "series",
    type_hint: "list of {name, points: [[x, y], ...]}",
    required: false,
    default: Some("[]"),
    description: "One entry per line / cluster. Points are `[x, y]` float pairs. `chart_line` / `chart_scatter` consume this directly.",
}];

pub struct BasicPoints;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub series: Vec<SeriesConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesConfig {
    pub name: String,
    #[serde(default)]
    pub points: Vec<(f64, f64)>,
}

impl RealtimeFetcher for BasicPoints {
    fn name(&self) -> &str {
        "basic_points"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a `PointSeries` from inline `[widget.options].series = [{name, points: [[x, y], ...]}]`. Supports multiple series so a single widget can carry several lines / clusters."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::PointSeries).then(|| {
            Body::PointSeries(PointSeriesData {
                series: vec![PointSeries {
                    name: "demo".into(),
                    points: vec![(0.0, 1.0), (1.0, 2.5), (2.0, 1.8), (3.0, 3.0)],
                }],
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        common::bare(Body::PointSeries(PointSeriesData {
            series: opts
                .series
                .into_iter()
                .map(|s| PointSeries {
                    name: s.name,
                    points: s.points,
                })
                .collect(),
        }))
    }
}

