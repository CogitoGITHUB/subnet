//! `basic_numbers` — `NumberSeries` authored inline. Right for tiny static series the user
//! wants to sparkline / histogram (manual weekly counts, learning curve "1 chapter / day").

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, NumberSeriesData, Payload};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::NumberSeries];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "values",
    type_hint: "list of u64",
    required: false,
    default: Some("[]"),
    description: "The series values in chronological / sequential order. Renderers like `chart_sparkline` consume this directly.",
}];

pub struct BasicNumbers;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub values: Vec<u64>,
}

impl RealtimeFetcher for BasicNumbers {
    fn name(&self) -> &str {
        "basic_numbers"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a `NumberSeries` from inline `[widget.options].values`. Right for tiny static series the user wants to sparkline or histogram without a dedicated fetcher (weekly tallies, manual progress curves)."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::NumberSeries).then(|| {
            Body::NumberSeries(NumberSeriesData {
                values: vec![3, 5, 8, 13, 21, 34],
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        common::bare(Body::NumberSeries(NumberSeriesData {
            values: opts.values,
        }))
    }
}

