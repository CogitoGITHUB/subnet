//! `net_*` family — local network interface / address state, plus a connection speed test.
//!
//! All `Safety::Safe`. The interface / address / gateway / proxy fetchers read only host-local
//! state (the `netdev` interface table, the routing table, `$*_PROXY` env). `net_speedtest` does
//! talk to the network, but only ever to the hardcoded `speed.cloudflare.com` — config can't
//! redirect it, so it stays `Safe` under the host-fixed rule (same as `weather` → Open-Meteo).
//!
//! The four `netdev`-backed fetchers and `net_speedtest` are cache-backed — `get_interfaces()`
//! walks every interface and isn't a `<1ms` read, and `net_speedtest` does multi-second network
//! I/O. `net_proxy` is the one realtime fetcher: pure `std::env` reads, infallible, no I/O.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::payload::{Body, Entry, Payload, TextData};

use super::{FetchContext, Fetcher, RealtimeFetcher};

mod gateway;
mod interfaces;
mod ip;
mod listening_ports;
mod mac;
mod proxy;
mod speedtest;
mod vpn;

pub use gateway::NetGateway;
pub use interfaces::NetInterfaces;
pub use ip::NetIp;
pub use listening_ports::NetListeningPorts;
pub use mac::NetMac;
pub use proxy::NetProxy;
pub use speedtest::NetSpeedtest;
pub use vpn::NetVpn;

pub fn cached_fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(NetInterfaces),
        Arc::new(NetIp),
        Arc::new(NetMac),
        Arc::new(NetGateway),
        Arc::new(NetSpeedtest),
        Arc::new(NetVpn),
        Arc::new(NetListeningPorts),
    ]
}

pub fn realtime_fetchers() -> Vec<Arc<dyn RealtimeFetcher>> {
    vec![Arc::new(NetProxy)]
}

/// Normalised view of one network interface, decoupled from `netdev`'s types so the per-fetcher
/// modules — and, more importantly, their tests — never depend on the crate directly.
#[derive(Debug, Clone)]
pub(crate) struct NetIface {
    pub name: String,
    pub kind: String,
    pub mac: Option<String>,
    pub ipv4: Vec<Ipv4Addr>,
    pub ipv6: Vec<Ipv6Addr>,
    pub up: bool,
    pub loopback: bool,
    pub is_default: bool,
    pub mtu: Option<u32>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Resolved default-route gateway. `ip` / `interface` are independently optional because
/// `netdev` can resolve a route without naming its interface (and vice versa).
#[derive(Debug, Clone, Default)]
pub(crate) struct GatewayInfo {
    pub ip: Option<IpAddr>,
    pub interface: Option<String>,
}

pub(crate) fn snapshot() -> Vec<NetIface> {
    netdev::get_interfaces()
        .into_iter()
        .map(|mut i| {
            // `get_interfaces()` leaves `stats` unpopulated on Linux — it builds interfaces with
            // `stats: None` and expects an explicit refresh. Without this, the byte counters are
            // always zero and `net_interfaces`' byte-count `Bars` would be flat.
            let _ = i.update_stats();
            NetIface {
                loopback: i.is_loopback(),
                up: i.is_up(),
                is_default: i.default,
                kind: i.if_type.name(),
                mac: i.mac_addr.map(|m| m.to_string()),
                ipv4: i.ipv4.iter().map(|n| n.addr()).collect(),
                ipv6: i.ipv6.iter().map(|n| n.addr()).collect(),
                mtu: i.mtu,
                rx_bytes: i.stats.as_ref().map(|s| s.rx_bytes).unwrap_or(0),
                tx_bytes: i.stats.as_ref().map(|s| s.tx_bytes).unwrap_or(0),
                name: i.name,
            }
        })
        .collect()
}

pub(crate) fn default_gateway() -> GatewayInfo {
    // The interface `netdev` marks as the default route carries the resolved next-hop device,
    // so we get the gateway IP *and* its interface name in one pass.
    for iface in netdev::get_interfaces() {
        if iface.default {
            return GatewayInfo {
                ip: iface.gateway.as_ref().and_then(gateway_ip),
                interface: Some(iface.name),
            };
        }
    }
    // Fallback: the standalone route lookup has no interface name to offer.
    GatewayInfo {
        ip: netdev::get_default_gateway()
            .ok()
            .as_ref()
            .and_then(gateway_ip),
        interface: None,
    }
}

fn gateway_ip(dev: &netdev::NetworkDevice) -> Option<IpAddr> {
    dev.ipv4
        .first()
        .copied()
        .map(IpAddr::V4)
        .or_else(|| dev.ipv6.first().copied().map(IpAddr::V6))
}

/// The interface a single-value fetcher should speak for: the default-route interface, falling
/// back to the first non-loopback interface that's up.
pub(crate) fn primary_interface(ifaces: &[NetIface]) -> Option<&NetIface> {
    ifaces
        .iter()
        .find(|i| i.is_default)
        .or_else(|| ifaces.iter().find(|i| i.up && !i.loopback))
}

pub(crate) fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

pub(crate) fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

pub(crate) fn parse_options<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&toml::Value>,
) -> Result<T, String> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

pub(crate) fn options_placeholder(msg: &str) -> Payload {
    payload(Body::Text(TextData {
        value: format!("⚠ {msg}"),
    }))
}

/// Cache key that mixes in `ctx.options` on top of name + shape — the option-bearing `net_*`
/// fetchers (`kind` / `interface`) branch their body on it, so two option sets must not collide
/// on one cache entry. Mirrors [`crate::fetcher::default_cache_key`] otherwise.
pub(crate) fn cache_key(name: &str, ctx: &FetchContext) -> String {
    let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("");
    let opts = ctx
        .options
        .as_ref()
        .and_then(|v| toml::to_string(v).ok())
        .unwrap_or_default();
    let raw = format!("{name}|{shape}|{opts}");
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

