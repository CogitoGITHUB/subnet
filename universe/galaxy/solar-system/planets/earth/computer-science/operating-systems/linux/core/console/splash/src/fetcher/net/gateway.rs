//! `net_gateway` — the host's default-route gateway.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, MarkdownTextBlockData, Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{
    GatewayInfo, cache_key, default_gateway, entry, options_placeholder, parse_options, payload,
};

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"gateway\" | \"interface\"",
    required: false,
    default: Some("\"gateway\""),
    description: "Which field the `Text` shape shows — the gateway address or the interface it routes through. Ignored by the other shapes. (Route metric isn't exposed by the underlying platform APIs, so it isn't a `kind`.)",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayOptions {
    #[serde(default)]
    pub kind: Option<GatewayKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayKind {
    #[default]
    Gateway,
    Interface,
}

pub struct NetGateway;

#[async_trait]
impl Fetcher for NetGateway {
    fn name(&self) -> &str {
        "net_gateway"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The host's default-route gateway. `Text` (default) shows the gateway address — `kind = \"interface\"` shows the interface it routes through instead; `TextBlock` / `MarkdownTextBlock` / `Entries` roll up gateway + interface; `Badge` is a has-default-route / no-route pill."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 10
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        body_for_shape(&sample_gateway(), shape, GatewayKind::Gateway)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: GatewayOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(options_placeholder(&msg)),
        };
        let gw = default_gateway();
        let shape = ctx.shape.unwrap_or(Shape::Text);
        let kind = opts.kind.unwrap_or_default();
        Ok(payload(body_for_shape(&gw, shape, kind).unwrap_or_else(
            || {
                Body::Text(TextData {
                    value: text_value(&gw, kind),
                })
            },
        )))
    }
}

fn body_for_shape(gw: &GatewayInfo, shape: Shape, kind: GatewayKind) -> Option<Body> {
    Some(match shape {
        Shape::Text => Body::Text(TextData {
            value: text_value(gw, kind),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: vec![
                format!("gateway  {}", gateway_field(gw)),
                format!("interface  {}", interface_field(gw)),
            ],
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format!(
                "- **gateway** {}\n- **interface** {}",
                gateway_field(gw),
                interface_field(gw),
            ),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("gateway", &gateway_field(gw)),
                entry("interface", &interface_field(gw)),
            ],
        }),
        Shape::Badge => Body::Badge(gateway_badge(gw)),
        _ => return None,
    })
}

fn text_value(gw: &GatewayInfo, kind: GatewayKind) -> String {
    match kind {
        GatewayKind::Gateway => gw
            .ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "no default route".into()),
        GatewayKind::Interface => interface_field(gw),
    }
}

fn gateway_field(gw: &GatewayInfo) -> String {
    gw.ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "n/a".into())
}

fn interface_field(gw: &GatewayInfo) -> String {
    gw.interface.clone().unwrap_or_else(|| "n/a".into())
}

fn gateway_badge(gw: &GatewayInfo) -> BadgeData {
    match gw.ip {
        Some(ip) => BadgeData {
            status: Status::Ok,
            label: ip.to_string(),
        },
        None => BadgeData {
            status: Status::Error,
            label: "no default route".into(),
        },
    }
}

fn sample_gateway() -> GatewayInfo {
    GatewayInfo {
        ip: Some(std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            192, 168, 1, 1,
        ))),
        interface: Some("eth0".into()),
    }
}

