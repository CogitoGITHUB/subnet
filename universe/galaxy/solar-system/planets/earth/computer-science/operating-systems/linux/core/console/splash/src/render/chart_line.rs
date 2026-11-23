use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    symbols::Marker,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
};

use crate::options::OptionSchema;
use crate::payload::{Body, PointSeries, PointSeriesData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::PALETTE_SERIES, theme::PANEL_BORDER, theme::BG];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "axes",
        type_hint: "bool",
        required: false,
        default: Some("false"),
        description: "Draw a bordered block around the plot so the axes read as framed.",
    },
    OptionSchema {
        name: "legend",
        type_hint: "bool",
        required: false,
        default: Some("false"),
        description: "Placeholder — ratatui's Chart does not render legends yet. Accepted so configs stay forward-compatible.",
    },
];

pub struct ChartLineRenderer;

impl Renderer for ChartLineRenderer {
    fn name(&self) -> &str {
        "chart_line"
    }
    fn description(&self) -> &'static str {
        "Braille line plot of one or more `PointSeries` against shared x/y axes, with each series coloured from the palette. Pick this over `chart_scatter` when the trend between samples matters more than the individual points."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::PointSeries]
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
        if let Body::PointSeries(d) = body {
            render_line_chart(frame, area, d, opts, theme);
        }
    }
}

fn render_line_chart(
    frame: &mut Frame,
    area: Rect,
    data: &PointSeriesData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let datasets: Vec<Dataset> = data
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| to_dataset(s, theme.series_color(i)))
        .collect();
    let (x_bounds, y_bounds) = bounds(&data.series);
    // `Chart` builds an internal `Canvas` per dataset and seeds it with
    // `self.style.bg.unwrap_or(Color::Reset)` (ratatui 0.29 `chart.rs:1023`). Without an
    // explicit `bg` here, the plot area paints `Color::Reset` and erases the surrounding
    // panel surface (`theme.bg` or `bg = "subtle"`). Propagate `theme.bg` so the chart
    // keeps the layout-painted background.
    let mut chart = Chart::new(datasets)
        .style(Style::default().bg(theme.bg))
        .x_axis(Axis::default().bounds(x_bounds))
        .y_axis(Axis::default().bounds(y_bounds));
    if opts.axes.unwrap_or(false) {
        chart = chart.block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.panel_border)),
        );
    }
    frame.render_widget(chart, area);
}

fn to_dataset(series: &PointSeries, color: ratatui::style::Color) -> Dataset<'_> {
    Dataset::default()
        .name(series.name.clone())
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(color))
        .data(&series.points)
}

fn bounds(series: &[PointSeries]) -> ([f64; 2], [f64; 2]) {
    let points: Vec<(f64, f64)> = series
        .iter()
        .flat_map(|s| s.points.iter().copied())
        .collect();
    if points.is_empty() {
        return ([0.0, 1.0], [0.0, 1.0]);
    }
    let (xs, ys): (Vec<f64>, Vec<f64>) = points.into_iter().unzip();
    ([min(&xs), max(&xs)], [min(&ys), max(&ys)])
}

fn min(xs: &[f64]) -> f64 {
    xs.iter().copied().fold(f64::INFINITY, f64::min)
}

fn max(xs: &[f64]) -> f64 {
    xs.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

