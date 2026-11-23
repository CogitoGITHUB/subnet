//! `net_interfaces` — every host network interface with its state, address, MTU and MAC.

use async_trait::async_trait;

use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, RatioData,
    Status, TextBlockData, TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{NetIface, payload, primary_interface, snapshot};

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Ratio,
    Shape::Bars,
    Shape::Badge,
];

pub struct NetInterfaces;

#[async_trait]
impl Fetcher for NetInterfaces {
    fn name(&self) -> &str {
        "net_interfaces"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Every host network interface with its up/down state, primary IP, MTU and MAC. `Entries` (default) / `TextBlock` / `MarkdownTextBlock` list one row per interface; `Text` headlines the primary (default-route) interface; `Ratio` is the up-of-total fraction; `Bars` ranks interfaces by total bytes transferred; `Badge` is an online / offline pill."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 10
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        body_for_shape(&sample_snapshot(), shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let ifaces = snapshot();
        let shape = ctx.shape.unwrap_or(Shape::Entries);
        Ok(payload(
            body_for_shape(&ifaces, shape).unwrap_or_else(|| entries_body(&ifaces)),
        ))
    }
}

fn body_for_shape(ifaces: &[NetIface], shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::Entries => entries_body(ifaces),
        Shape::Text => Body::Text(TextData {
            value: headline(ifaces),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: ifaces.iter().map(row_line).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: ifaces
                .iter()
                .map(|i| format!("- **{}** {}", i.name, iface_summary(i)))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Ratio => Body::Ratio(up_ratio(ifaces)),
        Shape::Bars => Body::Bars(BarsData {
            bars: ifaces
                .iter()
                .map(|i| Bar {
                    label: i.name.clone(),
                    value: i.rx_bytes.saturating_add(i.tx_bytes),
                    value_label: None,
                })
                .collect(),
        }),
        Shape::Badge => Body::Badge(connectivity_badge(ifaces)),
        _ => return None,
    })
}

fn entries_body(ifaces: &[NetIface]) -> Body {
    Body::Entries(EntriesData {
        items: ifaces
            .iter()
            .map(|i| Entry {
                key: i.name.clone(),
                value: Some(iface_summary(i)),
                status: Some(if i.up { Status::Ok } else { Status::Warn }),
            })
            .collect(),
    })
}

/// Speaks for the primary interface ("what's my connection") rather than a bare count — the
/// count is still the fallback when no interface is up.
fn headline(ifaces: &[NetIface]) -> String {
    match primary_interface(ifaces) {
        Some(i) => format!("{} · {}", i.name, iface_summary(i)),
        None => format!("{} interfaces", ifaces.len()),
    }
}

fn row_line(i: &NetIface) -> String {
    format!("{}  {}", i.name, iface_summary(i))
}

fn iface_summary(i: &NetIface) -> String {
    let mut parts = vec![if i.up { "up" } else { "down" }.to_string()];
    if let Some(addr) = primary_addr(i) {
        parts.push(addr);
    }
    if let Some(mtu) = i.mtu {
        parts.push(format!("mtu {mtu}"));
    }
    parts.join(" · ")
}

fn primary_addr(i: &NetIface) -> Option<String> {
    i.ipv4
        .first()
        .map(ToString::to_string)
        .or_else(|| i.ipv6.first().map(ToString::to_string))
}

fn up_ratio(ifaces: &[NetIface]) -> RatioData {
    let total = ifaces.len() as u64;
    let up = ifaces.iter().filter(|i| i.up).count() as u64;
    RatioData {
        value: if total == 0 {
            0.0
        } else {
            up as f64 / total as f64
        },
        label: Some(format!("{up}/{total} up")),
        denominator: Some(total),
    }
}

/// Online when at least one non-loopback interface is up *and* carries an address — an `up`
/// interface with no IP isn't actually reachable.
fn connectivity_badge(ifaces: &[NetIface]) -> BadgeData {
    let online = ifaces
        .iter()
        .any(|i| !i.loopback && i.up && (!i.ipv4.is_empty() || !i.ipv6.is_empty()));
    if online {
        BadgeData {
            status: Status::Ok,
            label: "online".into(),
        }
    } else {
        BadgeData {
            status: Status::Error,
            label: "offline".into(),
        }
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
            rx_bytes: 4_096,
            tx_bytes: 4_096,
        },
        NetIface {
            name: "eth0".into(),
            kind: "Ethernet".into(),
            mac: Some("aa:bb:cc:dd:ee:ff".into()),
            ipv4: vec![std::net::Ipv4Addr::new(192, 168, 1, 24)],
            ipv6: vec![],
            up: true,
            loopback: false,
            is_default: true,
            mtu: Some(1500),
            rx_bytes: 8_400_000,
            tx_bytes: 1_200_000,
        },
        NetIface {
            name: "wlan0".into(),
            kind: "Wireless IEEE 802.11".into(),
            mac: Some("11:22:33:44:55:66".into()),
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

