//! `net_listening_ports` — sockets the host is currently accepting connections on.
//!
//! Per-directory value: after `cd` into a project, the splash answers "is my dev server up?"
//! and "what ports am I exposing right now?" without a separate `lsof -i -P -n | grep LISTEN`.

use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;

use async_trait::async_trait;
use netstat2::{
    AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, SocketInfo, TcpState, get_sockets_info,
};
use serde::Deserialize;
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{cache_key, options_placeholder, parse_options, payload};

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Badge,
];

const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 50;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "protocol",
        type_hint: "\"tcp\" | \"udp\" | \"both\"",
        required: false,
        default: Some("\"tcp\""),
        description: "Which transport protocols to include. Default `tcp` because LISTEN is a TCP-specific state; UDP sockets are simply bound, with no equivalent concept of accepting connections.",
    },
    OptionSchema {
        name: "family",
        type_hint: "\"v4\" | \"v6\" | \"both\"",
        required: false,
        default: Some("\"both\""),
        description: "Restrict the result to IPv4-bound or IPv6-bound sockets.",
    },
    OptionSchema {
        name: "exclude_loopback",
        type_hint: "bool",
        required: false,
        default: Some("false"),
        description: "Drop sockets bound only to `127.0.0.1` / `::1`. Useful when you only care about externally-reachable services.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "1..=50",
        required: false,
        default: Some("10"),
        description: "Cap the number of listed sockets in multi-row shapes (`Entries` / `Text` preview / `TextBlock` / `MarkdownTextBlock`). `Badge` always reports the unclamped total.",
    },
];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortsOptions {
    #[serde(default)]
    pub protocol: Option<Protocol>,
    #[serde(default)]
    pub family: Option<Family>,
    #[serde(default)]
    pub exclude_loopback: bool,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    #[default]
    Tcp,
    Udp,
    Both,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    V4,
    V6,
    #[default]
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SocketProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone)]
pub(crate) struct ListeningSocket {
    pub port: u16,
    pub protocol: SocketProtocol,
    pub addr: IpAddr,
    pub pid: Option<u32>,
    pub process: Option<String>,
}

pub struct NetListeningPorts;

#[async_trait]
impl Fetcher for NetListeningPorts {
    fn name(&self) -> &str {
        "net_listening_ports"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Sockets the host is currently accepting connections on, with the owning process. `Entries` \
         (default) maps port to process; `Text` headlines the count and the leading port list; \
         `TextBlock` / `MarkdownTextBlock` list one row per socket; `Badge` is a count tier. Useful \
         per-directory for confirming a dev server actually came up after `cd`. `protocol` (default \
         `tcp` — LISTEN is TCP-specific), `family`, `exclude_loopback`, and `limit` (1..=50) filter \
         the result."
    }
    fn refresh_interval(&self) -> u64 {
        60
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
        body_for_shape(&sample_sockets(), shape, DEFAULT_LIMIT)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: PortsOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(options_placeholder(&msg)),
        };
        let sockets = match collect_sockets(&opts) {
            Ok(s) => s,
            Err(msg) => return Ok(options_placeholder(&msg)),
        };
        let shape = ctx.shape.unwrap_or(Shape::Entries);
        let limit = resolved_limit(&opts);
        Ok(payload(
            body_for_shape(&sockets, shape, limit).unwrap_or_else(|| entries_body(&sockets, limit)),
        ))
    }
}

fn resolved_limit(opts: &PortsOptions) -> usize {
    opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn collect_sockets(opts: &PortsOptions) -> Result<Vec<ListeningSocket>, String> {
    let af = match opts.family.unwrap_or_default() {
        Family::V4 => AddressFamilyFlags::IPV4,
        Family::V6 => AddressFamilyFlags::IPV6,
        Family::Both => AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
    };
    let proto = match opts.protocol.unwrap_or_default() {
        Protocol::Tcp => ProtocolFlags::TCP,
        Protocol::Udp => ProtocolFlags::UDP,
        Protocol::Both => ProtocolFlags::TCP | ProtocolFlags::UDP,
    };
    let raw = get_sockets_info(af, proto).map_err(|e| format!("netstat: {e}"))?;
    let mut sockets: Vec<ListeningSocket> = raw
        .into_iter()
        .filter_map(|info| from_info(info, opts.exclude_loopback))
        .collect();
    sockets.sort_by(|a, b| a.port.cmp(&b.port).then(a.protocol.cmp(&b.protocol)));
    resolve_process_names(&mut sockets);
    Ok(sockets)
}

fn from_info(info: SocketInfo, exclude_loopback: bool) -> Option<ListeningSocket> {
    let (protocol, addr, port, is_listening) = match info.protocol_socket_info {
        ProtocolSocketInfo::Tcp(tcp) => (
            SocketProtocol::Tcp,
            tcp.local_addr,
            tcp.local_port,
            tcp.state == TcpState::Listen,
        ),
        // UDP has no LISTEN state — a bound UDP socket is always "accepting" datagrams.
        ProtocolSocketInfo::Udp(udp) => (SocketProtocol::Udp, udp.local_addr, udp.local_port, true),
    };
    if !is_listening || (exclude_loopback && is_loopback(&addr)) {
        return None;
    }
    Some(ListeningSocket {
        port,
        protocol,
        addr,
        pid: info.associated_pids.first().copied(),
        process: None,
    })
}

fn resolve_process_names(sockets: &mut [ListeningSocket]) {
    let pid_set: BTreeSet<u32> = sockets.iter().filter_map(|s| s.pid).collect();
    if pid_set.is_empty() {
        return;
    }
    let pids: Vec<Pid> = pid_set.iter().map(|p| Pid::from_u32(*p)).collect();
    let Ok(mut sys) = System::new() else {
        return;
    };
    sys.refresh_processes(ProcessesToUpdate::Some(&pids), true);
    let names: HashMap<u32, String> = pid_set
        .iter()
        .filter_map(|pid| {
            sys.process(Pid::from_u32(*pid))
                .map(|p| (*pid, p.name().to_string_lossy().to_string()))
        })
        .collect();
    sockets.iter_mut().for_each(|s| {
        if let Some(pid) = s.pid {
            s.process = names.get(&pid).cloned();
        }
    });
}

fn is_loopback(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v) => v.is_loopback(),
        IpAddr::V6(v) => v.is_loopback(),
    }
}

fn is_unspecified(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v) => v.is_unspecified(),
        IpAddr::V6(v) => v.is_unspecified(),
    }
}

fn body_for_shape(sockets: &[ListeningSocket], shape: Shape, limit: usize) -> Option<Body> {
    let mixed = has_mixed_protocols(sockets);
    Some(match shape {
        Shape::Entries => entries_body(sockets, limit),
        Shape::Text => Body::Text(TextData {
            value: headline(sockets, limit),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: sockets
                .iter()
                .take(limit)
                .map(|s| row_line(s, mixed))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: sockets
                .iter()
                .take(limit)
                .map(|s| format!("- **{}** {}", port_label(s, mixed), process_label(s)))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Badge => Body::Badge(count_badge(sockets.len())),
        _ => return None,
    })
}

fn entries_body(sockets: &[ListeningSocket], limit: usize) -> Body {
    let mixed = has_mixed_protocols(sockets);
    Body::Entries(EntriesData {
        items: sockets
            .iter()
            .take(limit)
            .map(|s| Entry {
                key: port_label(s, mixed),
                value: Some(process_label(s)),
                status: None,
            })
            .collect(),
    })
}

fn has_mixed_protocols(sockets: &[ListeningSocket]) -> bool {
    let mut tcp = false;
    let mut udp = false;
    for s in sockets {
        match s.protocol {
            SocketProtocol::Tcp => tcp = true,
            SocketProtocol::Udp => udp = true,
        }
        if tcp && udp {
            return true;
        }
    }
    false
}

fn headline(sockets: &[ListeningSocket], limit: usize) -> String {
    if sockets.is_empty() {
        return "no listening ports".into();
    }
    let preview: Vec<_> = sockets
        .iter()
        .take(limit)
        .map(|s| s.port.to_string())
        .collect();
    format!("{} listening · {}", sockets.len(), preview.join(", "))
}

fn row_line(s: &ListeningSocket, mixed: bool) -> String {
    format!("{}  {}", port_label(s, mixed), process_label(s))
}

fn port_label(s: &ListeningSocket, mixed: bool) -> String {
    if mixed {
        format!(
            "{}/{}",
            s.port,
            match s.protocol {
                SocketProtocol::Tcp => "tcp",
                SocketProtocol::Udp => "udp",
            }
        )
    } else {
        s.port.to_string()
    }
}

fn process_label(s: &ListeningSocket) -> String {
    let body = match (&s.process, s.pid) {
        (Some(name), Some(pid)) => format!("{name} (pid {pid})"),
        (Some(name), None) => name.clone(),
        (None, Some(pid)) => format!("pid {pid}"),
        (None, None) => "—".into(),
    };
    if is_loopback(&s.addr) {
        format!("{body} · localhost")
    } else if !is_unspecified(&s.addr) {
        format!("{body} · {}", s.addr)
    } else {
        body
    }
}

fn count_badge(count: usize) -> BadgeData {
    let (status, label) = if count == 0 {
        (Status::Ok, "no ports".to_string())
    } else if count <= 10 {
        (Status::Ok, format!("{count} listening"))
    } else {
        (Status::Warn, format!("{count} listening"))
    };
    BadgeData { status, label }
}

fn sample_sockets() -> Vec<ListeningSocket> {
    vec![
        ListeningSocket {
            port: 22,
            protocol: SocketProtocol::Tcp,
            addr: "0.0.0.0".parse().unwrap(),
            pid: Some(842),
            process: Some("sshd".into()),
        },
        ListeningSocket {
            port: 3000,
            protocol: SocketProtocol::Tcp,
            addr: "127.0.0.1".parse().unwrap(),
            pid: Some(9123),
            process: Some("node".into()),
        },
        ListeningSocket {
            port: 5432,
            protocol: SocketProtocol::Tcp,
            addr: "127.0.0.1".parse().unwrap(),
            pid: Some(2410),
            process: Some("postgres".into()),
        },
    ]
}

