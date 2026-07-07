//! Real network readers — the first non-fixture host behaviour (chumby-pi
//! "Info & Licenses + live WLAN signal" milestone, I3).
//!
//! `RealNetHost` wraps a [`FixtureHost`] and answers the network `exec`
//! touchpoints (`network_status.sh`, `signal_strength`, `macgen.sh`) from live
//! kernel state; everything else delegates to the inner host. It is **always
//! active** — when there is no connected interface (or on a non-Linux build),
//! the reads return `None` and the call falls back to the inner fixture, so a
//! desktop/CI run with no usable network still behaves as before.
//!
//! Reads are `std` + `getifaddrs`: the interface's IPv4 address and netmask
//! come straight from `getifaddrs` (the same source `ip`/`ifconfig` use); the
//! default-route interface + gateway from `/proc/net/route`, DNS from
//! `/etc/resolv.conf`, MAC from `/sys/class/net`. No shell, per the project's
//! Rust-over-shell design principle. Assumption (user 2026-07-08): exactly one
//! interface is connected, so the default route names it.

use super::fixture::FixtureHost;
use super::host::{ChumbyFs, ChumbyHost, HostError, HostValue};

pub struct RealNetHost {
    inner: FixtureHost,
}

struct NetInfo {
    ip: String,
    netmask: String,
    gateway: String,
    dns1: String,
    dns2: String,
}

impl RealNetHost {
    pub fn new(inner: FixtureHost) -> Self {
        Self { inner }
    }

    /// `<network>` XML the panel parses (frame_2 `gotNetworkStatus`).
    /// `type="lan"` makes the Info screen show the honest Ethernet page.
    /// `None` when no interface is connected → caller falls back to the fixture.
    fn network_status_xml(&self) -> Option<String> {
        read_primary_interface().map(|n| {
            format!(
                "<network><configuration type=\"lan\" ssid=\"\" auth=\"\" encryption=\"\"/>\
                 <interface ip=\"{}\" netmask=\"{}\" gateway=\"{}\" nameserver1=\"{}\" nameserver2=\"{}\"/></network>",
                n.ip, n.netmask, n.gateway, n.dns1, n.dns2
            )
        })
    }

    /// Drives the dashboard `WifiIndicator`. `connected="1"` + full linkquality
    /// shows all five bars, which the ui-policy `tint` rule recolours blue =
    /// "wired, link up". `None` (no interface) → caller falls back to fixture.
    fn signal_strength_xml(&self) -> Option<String> {
        read_primary_interface()
            .map(|_| "<wifi connected=\"1\" linkquality=\"100\" signalstrength=\"100\"/>".to_owned())
    }

    fn mac(&self) -> Option<String> {
        let iface = default_route()?.0;
        Some(std::fs::read_to_string(format!("/sys/class/net/{iface}/address")).ok()?.trim().to_owned())
    }
}

impl ChumbyHost for RealNetHost {
    fn native(&self, index: u16, name: &str, args: &[HostValue]) -> HostValue {
        self.inner.native(index, name, args)
    }

    fn exec(&self, command: &str) -> Result<Vec<u8>, HostError> {
        let real = if command.starts_with("network_status.sh") {
            self.network_status_xml()
        } else if command.starts_with("signal_strength") {
            self.signal_strength_xml()
        } else if command.starts_with("macgen.sh") {
            self.mac().map(|m| format!("{m}\n"))
        } else {
            None
        };
        // Not one of ours, or no interface connected → the inner fixture host.
        match real {
            Some(body) => Ok(body.into_bytes()),
            None => self.inner.exec(command),
        }
    }

    fn fetch(&self, url: &str) -> Option<Result<Vec<u8>, HostError>> {
        self.inner.fetch(url)
    }

    fn fs(&self) -> &dyn ChumbyFs {
        self.inner.fs()
    }
}

fn read_primary_interface() -> Option<NetInfo> {
    let (iface, gateway) = default_route()?;
    let (ip, netmask) = if_ipv4(&iface)?;
    let (dns1, dns2) = read_dns();
    Some(NetInfo { ip, netmask, gateway, dns1, dns2 })
}

/// The default-route interface and its gateway (dotted), from the kernel
/// routing table. `None` when there is no default route.
fn default_route() -> Option<(String, String)> {
    let table = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in table.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        // Iface Destination Gateway Flags Refcnt Use Metric Mask ...
        if f.len() >= 8 && f[1] == "00000000" {
            return Some((f[0].to_owned(), hex_le_to_ip(f[2])));
        }
    }
    None
}

fn read_dns() -> (String, String) {
    let mut ns = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/etc/resolv.conf") {
        for line in text.lines() {
            if let Some(addr) = line.trim().strip_prefix("nameserver") {
                let addr = addr.trim();
                if !addr.is_empty() {
                    ns.push(addr.to_owned());
                }
            }
        }
    }
    (ns.first().cloned().unwrap_or_default(), ns.get(1).cloned().unwrap_or_default())
}

/// `/proc/net/route` stores an address as the hex of a little-endian `__le32`,
/// so its octets are the value's little-endian bytes in order.
fn hex_le_to_ip(hex: &str) -> String {
    let v = u32::from_str_radix(hex, 16).unwrap_or(0);
    let b = v.to_le_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

/// IPv4 address + netmask of `iface`, straight from `getifaddrs` (what
/// `ip`/`ifconfig` read). `None` if the interface has no IPv4 address.
#[cfg(unix)]
fn if_ipv4(iface: &str) -> Option<(String, String)> {
    use std::ffi::CStr;
    use std::net::Ipv4Addr;

    // SAFETY: textbook getifaddrs walk — every early return goes through
    // freeifaddrs, pointers are null-checked, and sockaddrs are only read
    // after confirming `sa_family == AF_INET`.
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return None;
        }
        let mut out = None;
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() || (*ifa.ifa_addr).sa_family as i32 != libc::AF_INET {
                continue;
            }
            if CStr::from_ptr(ifa.ifa_name).to_string_lossy() != iface {
                continue;
            }
            let addr = &*(ifa.ifa_addr as *const libc::sockaddr_in);
            let ip = Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()).to_string();
            let netmask = if ifa.ifa_netmask.is_null() {
                String::new()
            } else {
                let nm = &*(ifa.ifa_netmask as *const libc::sockaddr_in);
                Ipv4Addr::from(nm.sin_addr.s_addr.to_ne_bytes()).to_string()
            };
            out = Some((ip, netmask));
            break;
        }
        libc::freeifaddrs(ifap);
        out
    }
}

#[cfg(not(unix))]
fn if_ipv4(_iface: &str) -> Option<(String, String)> {
    None
}
