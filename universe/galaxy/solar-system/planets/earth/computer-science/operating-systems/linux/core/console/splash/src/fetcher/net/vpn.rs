//! `net_vpn` — host VPN connection state.
//!
//! An interface is treated as a VPN tunnel when its `netdev` type is `Tunnel` / `Ppp`, or its
//! name starts with one of the common VPN-driver prefixes (`tun` / `utun` / `wg` / `ipsec` / …).
//! Name-prefix detection is the load-bearing path on Linux because WireGuard and OpenVPN-tun
//! devices report as `Ether`, not `Tunnel`, in `/sys/class/net` and would otherwise slip past
//! the `if_type` check.

use async_trait::async_trait;

use crate::payload::{
    BadgeData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{NetIface, payload, snapshot};

const SHAPES: &[Shape] = &[
    Shape::Badge,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
];

/// Interface-name prefixes that identify a VPN tunnel even when `netdev` doesn't classify the
/// `if_type` as `Tunnel`. Covers the userspace tun drivers (`tun*` / `utun*`) plus the kernel
/// modules from the major commercial / FOSS VPN stacks.
const VPN_NAME_PREFIXES: &[&str] = &[
    "tun", "utun", "wg", "ipsec", "nordlynx", "proton", "tap", "gpd", "cscotun",
];

pub struct NetVpn;

#[async_trait]
impl Fetcher for NetVpn {
    fn name(&self) -> &str {
        "net_vpn"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Whether this host is currently routing through a VPN tunnel. `Badge` (default) is a \
         connected / disconnected pill — `disconnected` is `Warn`, since the widget exists \
         because the user wants to know when the tunnel is *down*. `Text` names the active VPN \
         interface and its address; `TextBlock` / `MarkdownTextBlock` / `Entries` list every \
         active tunnel. Detects `netdev` `Tunnel` / `Ppp` types plus name-prefix fallback for \
         WireGuard, OpenVPN-tun, IPsec, NordLynx, ProtonVPN, Cisco AnyConnect, and GlobalProtect."
    }
    fn refresh_interval(&self) -> u64 {
        60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        body_for_shape(&sample_snapshot(), shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let ifaces = snapshot();
        let shape = ctx.shape.unwrap_or(Shape::Badge);
        Ok(payload(
            body_for_shape(&ifaces, shape).unwrap_or_else(|| Body::Badge(vpn_badge(&[]))),
        ))
    }
}

fn body_for_shape(ifaces: &[NetIface], shape: Shape) -> Option<Body> {
    let vpns = active_vpns(ifaces);
    Some(match shape {
        Shape::Badge => Body::Badge(vpn_badge(&vpns)),
        Shape::Text => Body::Text(TextData {
            value: headline(&vpns),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: vpns.iter().map(|i| row_line(i)).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: vpns
                .iter()
                .map(|i| format!("- **{}** {}", i.name, addr_summary(i)))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vpns
                .iter()
                .map(|i| Entry {
                    key: i.name.clone(),
                    value: Some(addr_summary(i)),
                    status: Some(Status::Ok),
                })
                .collect(),
        }),
        _ => return None,
    })
}

fn active_vpns(ifaces: &[NetIface]) -> Vec<&NetIface> {
    ifaces
        .iter()
        .filter(|i| is_vpn(i) && i.up && (!i.ipv4.is_empty() || !i.ipv6.is_empty()))
        .collect()
}

fn is_vpn(i: &NetIface) -> bool {
    if i.loopback {
        return false;
    }
    if i.kind == "Tunnel" || i.kind == "Ppp" {
        return true;
    }
    VPN_NAME_PREFIXES.iter().any(|p| i.name.starts_with(p))
}

fn headline(vpns: &[&NetIface]) -> String {
    match vpns.first() {
        None => "VPN: off".into(),
        Some(i) => format!("VPN: {} · {}", i.name, addr_summary(i)),
    }
}

fn row_line(i: &NetIface) -> String {
    format!("{}  {}", i.name, addr_summary(i))
}

fn addr_summary(i: &NetIface) -> String {
    [
        i.ipv4.first().map(ToString::to_string),
        i.ipv6.first().map(ToString::to_string),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// `disconnected` is `Warn`, not `Ok`: a user who configured this widget cares about the tunnel
/// being up, so the absence is the noteworthy state worth a glance flag (mirrors `net_proxy`'s
/// "proxied is the surprise" stance, with the direction flipped).
fn vpn_badge(vpns: &[&NetIface]) -> BadgeData {
    if vpns.is_empty() {
        BadgeData {
            status: Status::Warn,
            label: "disconnected".into(),
        }
    } else if vpns.len() == 1 {
        BadgeData {
            status: Status::Ok,
            label: format!("connected · {}", vpns[0].name),
        }
    } else {
        BadgeData {
            status: Status::Ok,
            label: format!("connected · {} tunnels", vpns.len()),
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
            rx_bytes: 0,
            tx_bytes: 0,
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
            rx_bytes: 0,
            tx_bytes: 0,
        },
        NetIface {
            name: "tun0".into(),
            kind: "Tunnel".into(),
            mac: None,
            ipv4: vec![std::net::Ipv4Addr::new(10, 8, 0, 5)],
            ipv6: vec![],
            up: true,
            loopback: false,
            is_default: false,
            mtu: Some(1380),
            rx_bytes: 0,
            tx_bytes: 0,
        },
    ]
}

