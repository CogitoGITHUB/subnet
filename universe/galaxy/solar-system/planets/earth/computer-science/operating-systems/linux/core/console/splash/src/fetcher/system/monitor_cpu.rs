//! `system_monitor_cpu` — aggregated CPU usage across all cores.

use std::sync::Mutex;

use sysinfo::System;

use crate::payload::{Body, Payload, RatioData, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::sprites;
use super::{payload, refreshed_system, unsupported_placeholder};

pub struct SystemMonitorCpu {
    state: Mutex<Option<System>>,
}

impl SystemMonitorCpu {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(refreshed_system(System::refresh_cpu_usage)),
        }
    }
}

impl Default for SystemMonitorCpu {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for SystemMonitorCpu {
    fn name(&self) -> &str {
        "system_monitor_cpu"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Aggregated CPU usage across all cores, sampled every frame. Pair with a gauge renderer for a live meter or use the `Text` shape for a plain percentage."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Ratio, Shape::Text, Shape::PixelArt]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Ratio => samples::ratio(0.42, "cpu"),
            Shape::Text => samples::text("42%"),
            Shape::PixelArt => Body::PixelArt(sprites::cpu_sprite(0.42, "42%")),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(sys) = guard.as_mut() else {
            return unsupported_placeholder();
        };
        sys.refresh_cpu_usage();
        let pct = sys.global_cpu_usage();
        let ratio = (f64::from(pct) / 100.0).clamp(0.0, 1.0);
        let label = format!("{pct:.0}%");
        match ctx.shape.unwrap_or(Shape::Ratio) {
            Shape::Text => payload(Body::Text(TextData { value: label })),
            Shape::PixelArt => payload(Body::PixelArt(sprites::cpu_sprite(ratio, &label))),
            _ => payload(Body::Ratio(RatioData {
                value: ratio,
                label: Some(label),
                denominator: None,
            })),
        }
    }
}

