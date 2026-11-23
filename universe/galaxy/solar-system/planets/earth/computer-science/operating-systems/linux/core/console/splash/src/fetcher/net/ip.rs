//! `net_ip` — the host's local IP addresses.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{
    NetIface, cache_key, options_placeholder, parse_options, payload, primary_interface, snapshot,
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
    type_hint: "\"primary\" | \"v4\" | \"v6\"",
    required: false,
    default: Some("\"primary\""),
    description: "Which address the `Text` shape shows for the primary interface. Ignored by `TextBlock` / `MarkdownTextBlock` / `Entries` (which always list every interface) and `Badge`.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IpOptions {
    #[serde(default)]
    pub kind: Option<IpKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpKind {
    #[default]
    Primary,
    V4,
    V6,
}

pub struct NetIp;

#[async_trait]
impl Fetcher for NetIp {
    fn name(&self) -> &str {
        "net_ip"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The host's own IPv4 / IPv6 addresses. `Text` (default) shows one address for the primary (default-route) interface — `kind` picks v4 / v6 / first-available; `TextBlock` / `MarkdownTextBlock` / `Entries` list every interface that has an address; `Badge` flags dual-stack / IPv4 / IPv6-only / no-address."
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
        body_for_shape(&sample_snapshot(), shape, IpKind::Primary)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: IpOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(options_placeholder(&msg)),
        };
        let ifaces = snapshot();
        let shape = ctx.shape.unwrap_or(Shape::Text);
        let kind = opts.kind.unwrap_or_default();
        Ok(payload(
            body_for_shape(&ifaces, shape, kind).unwrap_or_else(|| {
                Body::Text(TextData {
                    value: text_value(&ifaces, kind),
                })
            }),
        ))
    }
}

fn body_for_shape(ifaces: &[NetIface], shape: Shape, kind: IpKind) -> Option<Body> {
    Some(match shape {
        Shape::Text => Body::Text(TextData {
            value: text_value(ifaces, kind),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: addressed(ifaces)
                .map(|i| format!("{}  {}", i.name, addr_strings(i).join(", ")))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: addressed(ifaces)
                .map(|i| format!("- **{}** {}", i.name, addr_strings(i).join(" · ")))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: addressed(ifaces)
                .map(|i| Entry {
                    key: i.name.clone(),
                    value: Some(addr_strings(i).join(", ")),
                    status: None,
                })
                .collect(),
        }),
        Shape::Badge => Body::Badge(ip_badge(ifaces)),
        _ => return None,
    })
}

fn text_value(ifaces: &[NetIface], kind: IpKind) -> String {
    let Some(i) = primary_interface(ifaces) else {
        return "no address".into();
    };
    match kind {
        IpKind::V4 => first_v4(i).unwrap_or_else(|| "no IPv4".into()),
        IpKind::V6 => first_v6(i).unwrap_or_else(|| "no IPv6".into()),
        IpKind::Primary => first_v4(i)
            .or_else(|| first_v6(i))
            .unwrap_or_else(|| "no address".into()),
    }
}

fn first_v4(i: &NetIface) -> Option<String> {
    i.ipv4.first().map(ToString::to_string)
}

fn first_v6(i: &NetIface) -> Option<String> {
    i.ipv6.first().map(ToString::to_string)
}

fn addr_strings(i: &NetIface) -> Vec<String> {
    i.ipv4
        .iter()
        .map(ToString::to_string)
        .chain(i.ipv6.iter().map(ToString::to_string))
        .collect()
}

fn addressed(ifaces: &[NetIface]) -> impl Iterator<Item = &NetIface> {
    ifaces
        .iter()
        .filter(|i| !i.ipv4.is_empty() || !i.ipv6.is_empty())
}

fn ip_badge(ifaces: &[NetIface]) -> BadgeData {
    match primary_interface(ifaces) {
        Some(i) if !i.ipv4.is_empty() && !i.ipv6.is_empty() => BadgeData {
            status: Status::Ok,
            label: "dual-stack".into(),
        },
        Some(i) if !i.ipv4.is_empty() => BadgeData {
            status: Status::Ok,
            label: format!("IPv4 {}", i.ipv4[0]),
        },
        Some(i) if !i.ipv6.is_empty() => BadgeData {
            status: Status::Ok,
            label: "IPv6 only".into(),
        },
        _ => BadgeData {
            status: Status::Error,
            label: "no address".into(),
        },
    }
}

fn sample_snapshot() -> Vec<NetIface> {
    vec![
        NetIface {
            name: "lo".into(),
            kind: "Loopback".into(),
            mac: None,
            ipv4: vec![std::net::Ipv4Addr::LOCALHOST],
            ipv6: vec![],
            up: true,
            loopback: true,
            is_default: false,
            mtu: Some(65536),
            rx_bytes: 0,
            tx_bytes: 0,
        },
        NetIface {
            name: "eth0".into(),
            kind: "Ethernet".into(),
            mac: Some("aa:bb:cc:dd:ee:ff".into()),
            ipv4: vec![std::net::Ipv4Addr::new(192, 168, 1, 24)],
            ipv6: vec!["fe80::1ab2".parse().unwrap()],
            up: true,
            loopback: false,
            is_default: true,
            mtu: Some(1500),
            rx_bytes: 0,
            tx_bytes: 0,
        },
    ]
}

