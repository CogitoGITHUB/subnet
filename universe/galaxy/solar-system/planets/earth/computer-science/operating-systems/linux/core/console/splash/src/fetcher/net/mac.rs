//! `net_mac` — link-layer (MAC) addresses per interface.

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
    name: "interface",
    type_hint: "string (interface name)",
    required: false,
    default: Some("primary interface"),
    description: "Interface the `Text` / `Badge` shapes speak for. Defaults to the primary (default-route) interface. Ignored by `TextBlock` / `MarkdownTextBlock` / `Entries`, which always list every interface with a MAC.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacOptions {
    #[serde(default)]
    pub interface: Option<String>,
}

pub struct NetMac;

#[async_trait]
impl Fetcher for NetMac {
    fn name(&self) -> &str {
        "net_mac"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Link-layer (MAC) addresses. `Text` (default) shows the MAC of the `interface` option's target — the primary (default-route) interface by default; `TextBlock` / `MarkdownTextBlock` / `Entries` list every interface that has a MAC; `Badge` flags whether the selected MAC is universally administered or locally administered (i.e. randomized / spoofed)."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
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
        body_for_shape(&sample_snapshot(), shape, None)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: MacOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(options_placeholder(&msg)),
        };
        let ifaces = snapshot();
        let shape = ctx.shape.unwrap_or(Shape::Text);
        let interface = opts.interface.as_deref();
        Ok(payload(
            body_for_shape(&ifaces, shape, interface).unwrap_or_else(|| {
                Body::Text(TextData {
                    value: text_value(&ifaces, interface),
                })
            }),
        ))
    }
}

fn body_for_shape(ifaces: &[NetIface], shape: Shape, interface: Option<&str>) -> Option<Body> {
    Some(match shape {
        Shape::Text => Body::Text(TextData {
            value: text_value(ifaces, interface),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: with_mac(ifaces)
                .map(|(name, mac)| format!("{name}  {mac}"))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: with_mac(ifaces)
                .map(|(name, mac)| format!("- **{name}** {mac}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: with_mac(ifaces)
                .map(|(name, mac)| Entry {
                    key: name.to_string(),
                    value: Some(mac.to_string()),
                    status: None,
                })
                .collect(),
        }),
        Shape::Badge => Body::Badge(mac_badge(ifaces, interface)),
        _ => return None,
    })
}

fn text_value(ifaces: &[NetIface], interface: Option<&str>) -> String {
    match interface {
        Some(name) => match ifaces.iter().find(|i| i.name == name) {
            Some(i) => i.mac.clone().unwrap_or_else(|| "n/a".into()),
            None => format!("no interface {name}"),
        },
        None => primary_interface(ifaces)
            .and_then(|i| i.mac.clone())
            .unwrap_or_else(|| "n/a".into()),
    }
}

fn with_mac(ifaces: &[NetIface]) -> impl Iterator<Item = (&str, &str)> {
    ifaces
        .iter()
        .filter_map(|i| i.mac.as_deref().map(|m| (i.name.as_str(), m)))
}

fn mac_badge(ifaces: &[NetIface], interface: Option<&str>) -> BadgeData {
    let selected = match interface {
        Some(name) => ifaces.iter().find(|i| i.name == name),
        None => primary_interface(ifaces),
    };
    match selected.and_then(|i| i.mac.as_deref()) {
        Some(mac) => match is_locally_administered(mac) {
            Some(true) => BadgeData {
                status: Status::Warn,
                label: "randomized".into(),
            },
            Some(false) => BadgeData {
                status: Status::Ok,
                label: "universal".into(),
            },
            None => BadgeData {
                status: Status::Warn,
                label: mac.to_string(),
            },
        },
        None => BadgeData {
            status: Status::Warn,
            label: "no MAC".into(),
        },
    }
}

/// The U/L bit (bit `0x02` of the first octet) distinguishes a vendor-burned-in address from a
/// locally administered one — the latter is what randomized / spoofed MACs use.
fn is_locally_administered(mac: &str) -> Option<bool> {
    let first = mac.split([':', '-']).next()?;
    let octet = u8::from_str_radix(first, 16).ok()?;
    Some(octet & 0x02 != 0)
}

fn sample_snapshot() -> Vec<NetIface> {
    vec![
        NetIface {
            name: "eth0".into(),
            kind: "Ethernet".into(),
            mac: Some("3c:22:fb:8a:1d:04".into()),
            ipv4: vec![std::net::Ipv4Addr::new(192, 168, 1, 24)],
            ipv6: vec![],
            up: true,
            loopback: false,
            is_default: true,
            mtu: Some(1500),
            rx_bytes: 0,
            tx_bytes: 0,
        },
        NetIface {
            name: "wlan0".into(),
            kind: "Wireless IEEE 802.11".into(),
            mac: Some("aa:bb:cc:dd:ee:ff".into()),
            ipv4: vec![],
            ipv6: vec![],
            up: false,
            loopback: false,
            is_default: false,
            mtu: Some(1500),
            rx_bytes: 0,
            tx_bytes: 0,
        },
    ]
}

