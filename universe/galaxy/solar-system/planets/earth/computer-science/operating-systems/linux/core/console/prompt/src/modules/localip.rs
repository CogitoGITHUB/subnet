use super::{Context, Module};

use crate::config::ModuleConfig;
use crate::configs::localip::LocalipConfig;
use crate::formatter::StringFormatter;

use std::io::Error;
use std::net::UdpSocket;

fn get_local_ipv4() -> Result<String, Error> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("192.0.2.0:80")?;

    let addr = socket.local_addr()?;

    Ok(addr.ip().to_string())
}

/// Creates a module with the ipv4 address of the local machine.
///
/// The IP address is gathered from the local endpoint of an UDP socket
/// connected to a reserved IPv4 remote address, which is an accurate and fast
/// way, especially if there are multiple IP addresses available.
/// There should be no actual packets send over the wire.
///
/// Will display the ip if all of the following criteria are met:
///     - `localip.disabled` is false
///     - `localip.ssh_only` is false OR the user is currently connected as an SSH session (`$SSH_CONNECTION`)
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("localip");
    let config: LocalipConfig = LocalipConfig::try_load(module.config);

    // As we default to disabled=true, we have to check here after loading our config module,
    // before it was only checking against whatever is in the config starship.toml
    if config.disabled {
        return None;
    }

    let ssh_connection = context.get_env("SSH_CONNECTION");
    if config.ssh_only && ssh_connection.is_none() {
        return None;
    }

    let localip = match get_local_ipv4() {
        Ok(ip) => ip,
        Err(e) => {
            // ErrorKind::NetworkUnreachable is unstable
            if cfg!(target_os = "linux") && e.raw_os_error() == Some(101) {
                "NetworkUnreachable".to_string()
            } else {
                log::warn!("unable to determine local ipv4 address: {e}");
                return None;
            }
        }
    };

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "localipv4" => Some(Ok(&localip)),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `localip`:\n{error}");
            return None;
        }
    });

    Some(module)
}

