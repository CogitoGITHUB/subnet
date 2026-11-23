use ratatui::{Frame, layout::Rect};
use tui_piechart::{LegendPosition, PieChart, PieSlice};

use crate::options::OptionSchema;
use crate::payload::{BarsData, Body};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub legend: Option<String>,
    #[serde(default)]
    pub donut: Option<bool>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::PALETTE_SERIES];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "legend",
        type_hint: "\"right\" | \"bottom\" | \"top\" | \"left\" | \"none\"",
        required: false,
        default: Some("\"right\""),
        description: "Legend placement. `none` hides the legend; any direction pins it to that side of the chart.",
    },
    OptionSchema {
        name: "donut",
        type_hint: "bool",
        required: false,
        default: Some("false"),
        description: "Render a donut (hollow centre) instead of a filled pie. Implemented by substituting the slice glyph so the centre reads as empty.",
    },
];

/// Pie-chart renderer. Consumes the same `Bars` shape as `chart_bar`, so one fetcher feeds
/// either — `render = "chart_pie"` vs `render = "chart_bar"` is a config choice. Slice colours
/// cycle through the shared `series` palette in input order; labels and percentages come from
/// the widget.
pub struct ChartPieRenderer;

impl Renderer for ChartPieRenderer {
    fn name(&self) -> &str {
        "chart_pie"
    }
    fn description(&self) -> &'static str {
        "Pie chart of `Bars` slices with palette-cycled colours, percentages, and an adjacent legend. Use `donut = true` for a hollow centre, or pick `chart_bar` instead when exact comparisons between slices matter."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Bars]
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        opts: &RenderOptions,
        theme: &Theme,
        _registry: &Registry,
    ) {
        if let Body::Bars(d) = body {
            render_pie(frame, area, d, opts, theme);
        }
    }
}

fn render_pie(frame: &mut Frame, area: Rect, data: &BarsData, opts: &RenderOptions, theme: &Theme) {
    let specific: Options = opts.parse_specific();
    let slices: Vec<PieSlice> = data
        .bars
        .iter()
        .enumerate()
        .map(|(i, b)| PieSlice::new(b.label.as_str(), b.value as f64, theme.series_color(i)))
        .collect();
    let show_legend = !matches!(specific.legend.as_deref(), Some("none"));
    let mut chart = PieChart::new(slices)
        .show_legend(show_legend)
        .show_percentages(true)
        .legend_position(legend_position(specific.legend.as_deref()));
    if specific.donut.unwrap_or(false) {
        chart = chart.pie_char('○');
    }
    frame.render_widget(chart, area);
}

fn legend_position(legend: Option<&str>) -> LegendPosition {
    match legend {
        Some("left") => LegendPosition::Left,
        Some("top") => LegendPosition::Top,
        Some("bottom") => LegendPosition::Bottom,
        _ => LegendPosition::Right,
    }
}

