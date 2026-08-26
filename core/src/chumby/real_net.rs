//! Real network readers — the first non-fixture host behaviour (chumby-pi
//! "Info & Licenses + live WLAN signal" milestone, I3).
//!
//! `RealNetHost` wraps a [`FixtureHost`] and answers the network `exec`
//! touchpoints (`network_status.sh`, `signal_strength`, `macgen.sh`) from live
//! kernel state, plus the device-identity ones (`guidgen.sh`,
//! `chumby_version -n`, `md5sum` — see `real_ident.rs`); everything else
//! delegates to the inner host. It is **always active** — when there is no
//! default route (or on a non-Linux build), the reads return `None` and the
//! call falls back to the inner fixture, so a desktop/CI run with no usable
//! network still behaves as before.
//!
//! Reads are `std` + two thin libc calls: the interface's IPv4 address and
//! netmask come from `getifaddrs` (the same source `ip`/`ifconfig` use), the
//! SSID from the `SIOCGIWESSID` wireless-extensions ioctl (served by
//! cfg80211's wext compat — the same layer that populates
//! `/proc/net/wireless`, so wherever the signal is readable the SSID is too).
//! The default-route interface + gateway come from `/proc/net/route`, link
//! quality from `/proc/net/wireless`, DNS from `/etc/resolv.conf`, MAC from
//! `/sys/class/net`. No shell, per the project's Rust-over-shell design
//! principle. Assumption (user 2026-07-08): exactly one interface is
//! connected, so the default route names it.
//!
//! Attribute orientation: the original firmware's `signal_strength` script
//! emitted `linkquality` = dBm and `signalstrength` = percent — swapped — and
//! the panel un-swaps whenever `linkquality` is negative. We emit the sane
//! orientation (`linkquality` = percent, `signalstrength` = dBm), which the
//! panel uses directly on both surfaces.

use super::fixture::FixtureHost;
use super::host::{ChumbyFs, ChumbyHost, HostError, HostValue};

pub struct RealNetHost {
    inner: FixtureHost,
}

struct NetInfo {
    iface: String,
    wireless: bool,
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
    /// `type="lan"` → the Info screen's Ethernet page; `type="wlan"` → the
    /// ssid line plus a live `signal_strength` readout. `auth`/`encryption`
    /// stay empty: the panel renders a bare ssid line for any auth value it
    /// does not recognize, and reading the security mode would need nl80211
    /// for a purely decorative suffix.
    /// With no interface connected the answer is `NO_NETWORK`, never a
    /// fixture: a fabricated LAN page is a lie the owner reads as fact
    /// (Jan, 2026-08-26).
    fn network_status_xml(&self) -> String {
        match read_primary_interface() {
            Some(n) => network_xml(&n, essid(&n.iface).unwrap_or_default()),
            None => NO_NETWORK.to_owned(),
        }
    }

    /// Drives the dashboard `WifiIndicator` (bars = `(linkquality − 50) × 2`,
    /// hidden when `connected != 1`) and the Info screen's "link quality"
    /// line. On a wired link the answer is `connected="0"`: the meter is a
    /// wifi meter — the SWF has no ethernet vocabulary — so it hides, and the
    /// wired diagnostics live on the Info screen (user 2026-07-10, replacing
    /// the I3 blue-tint repurposing). No route, or a wired one, answers
    /// `connected="0"` — again never a fixture.
    fn signal_strength_xml(&self) -> String {
        let wifi = default_route().map(|(iface, _)| iface).filter(|i| is_wireless(i)).and_then(|iface| {
            std::fs::read_to_string("/proc/net/wireless")
                .ok()
                .and_then(|t| parse_proc_wireless(&t, &iface))
        });
        match wifi {
            Some((quality, dbm, noise)) => {
                // −256 is the "no noise data" sentinel (brcmfmac on the Pi);
                // the Info screen would print it as a nonsense dBm number.
                let noise = if noise == -256 {
                    "n/a".to_owned()
                } else {
                    noise.to_string()
                };
                format!(
                    "<wifi connected=\"1\" linkquality=\"{quality}\" signalstrength=\"{dbm}\" noiselevel=\"{noise}\"/>"
                )
            }
            None => NO_WIFI.to_owned(),
        }
    }
}

impl ChumbyHost for RealNetHost {
    fn native(&self, index: u16, name: &str, args: &[HostValue]) -> HostValue {
        self.inner.native(index, name, args)
    }

    fn exec(&self, command: &str) -> Result<Vec<u8>, HostError> {
        // These three never fall through: their fixtures were deleted with
        // this change, because a canned network page or MAC reads as fact.
        if command.starts_with("network_status.sh") {
            return Ok(self.network_status_xml().into_bytes());
        }
        if command.starts_with("signal_strength") {
            return Ok(self.signal_strength_xml().into_bytes());
        }
        if command.starts_with("macgen.sh") {
            return Ok(format!("{}\n", mac()).into_bytes());
        }
        let real = if command.starts_with("guidgen.sh") {
            // Device identity (real_ident.rs): the crypto processor's job.
            // Priority: player.toml device_guid, else serial-derived, else a
            // per-box random GUID persisted as /psp/guid.
            super::real_ident::resolve_guid(self.config(), self.inner.fs())
                .map(|g| format!("{g}\n"))
        } else if command.starts_with("chumby_version -n") {
            super::real_ident::hw_serial().map(|s| format!("{s}\n"))
        } else if let Some(path) = command.strip_prefix("md5sum ") {
            let path = path.trim();
            self.inner
                .fs()
                .get_file(path)
                .map(|content| super::real_ident::md5sum_line(&content, path))
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

    fn config(&self) -> &super::config::PlayerConfig {
        self.inner.config()
    }

    fn brightness_available(&self) -> bool {
        self.inner.brightness_available()
    }
}

/// The panel's own "no network" protocol: an `<error/>` child inside
/// `<interface>` clears `Object._chumby.hasNetwork` (frame_2
/// `gotNetworkStatus`), and the Info screen then prints its translated
/// "network: not connected" line instead of any interface field.
const NO_NETWORK: &str = "<network><interface><error/></interface></network>";
/// `connected="0"` hides the dashboard's wifi meter and the link-quality line.
const NO_WIFI: &str = "<wifi connected=\"0\"/>";

/// The `<network>` XML for a connected interface.
fn network_xml(n: &NetInfo, ssid: String) -> String {
    let (net_type, ssid) = if n.wireless { ("wlan", ssid) } else { ("lan", String::new()) };
    format!(
        "<network><configuration type=\"{}\" ssid=\"{}\" auth=\"\" encryption=\"\"/>\
         <interface ip=\"{}\" netmask=\"{}\" gateway=\"{}\" nameserver1=\"{}\" nameserver2=\"{}\"/></network>",
        net_type, xml_escape(&ssid), n.ip, n.netmask, n.gateway, n.dns1, n.dns2
    )
}

/// The Info screen prints the MAC outside its has-network branch, so this
/// must answer without a route: the default-route interface if there is one,
/// else the first non-loopback interface carrying a MAC. Empty string when
/// the machine has none — an empty line beats an invented address.
fn mac() -> String {
    if let Some((iface, _)) = default_route() {
        if let Some(m) = read_mac(&iface) {
            return m;
        }
    }
    let mut candidates: Vec<String> = std::fs::read_dir("/sys/class/net")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "lo")
        .collect();
    candidates.sort();
    candidates.iter().find_map(|iface| read_mac(iface)).unwrap_or_default()
}

/// A non-empty, non-all-zero MAC for `iface`.
fn read_mac(iface: &str) -> Option<String> {
    let addr = std::fs::read_to_string(format!("/sys/class/net/{iface}/address")).ok()?;
    let addr = addr.trim().to_owned();
    (!addr.is_empty() && addr != "00:00:00:00:00:00").then_some(addr)
}

fn read_primary_interface() -> Option<NetInfo> {
    let (iface, gateway) = default_route()?;
    let (ip, netmask) = if_ipv4(&iface)?;
    let (dns1, dns2) = read_dns();
    let wireless = is_wireless(&iface);
    Some(NetInfo { iface, wireless, ip, netmask, gateway, dns1, dns2 })
}

/// Wireless ⟺ the kernel exposes a `wireless/` dir for the interface (FR10).
fn is_wireless(iface: &str) -> bool {
    std::path::Path::new(&format!("/sys/class/net/{iface}/wireless")).exists()
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

/// `/proc/net/wireless` stats for `iface`: (link quality %, signal dBm,
/// noise dBm). Values carry a trailing '.' ("updated" flag). brcmfmac — the
/// Pi's driver — reports quality on a 0–70 scale; the panel wants percent.
/// Noise passes through raw, −256 "unknown" sentinel included; the XML
/// formatting in `signal_strength_xml` turns the sentinel into "n/a".
fn parse_proc_wireless(text: &str, iface: &str) -> Option<(i32, i32, i32)> {
    for line in text.lines().skip(2) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 || f[0].strip_suffix(':') != Some(iface) {
            continue;
        }
        let num = |s: &str| s.trim_end_matches('.').parse::<i32>().ok();
        let (link, level, noise) = (num(f[2])?, num(f[3])?, num(f[4])?);
        return Some((link.clamp(0, 70) * 100 / 70, level, noise));
    }
    None
}

/// `/proc/net/route` stores an address as the hex of a little-endian `__le32`,
/// so its octets are the value's little-endian bytes in order.
fn hex_le_to_ip(hex: &str) -> String {
    let v = u32::from_str_radix(hex, 16).unwrap_or(0);
    let b = v.to_le_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

/// Minimal escape for XML attribute values (SSIDs are arbitrary bytes).
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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

/// The connected SSID via the `SIOCGIWESSID` wext ioctl. `None` when the
/// kernel refuses (not associated, no wext compat) — the panel then shows an
/// empty ssid line rather than a wrong one. Verified against brcmfmac on the
/// Pi 3B+ (2026-07-10).
#[cfg(target_os = "linux")]
fn essid(iface: &str) -> Option<String> {
    const SIOCGIWESSID: libc::c_ulong = 0x8B1B;
    const IW_ESSID_MAX_SIZE: usize = 32;

    // struct iwreq, specialized to the union arm used by ESSID requests
    // (a struct iw_point payload). Layouts from linux/wireless.h.
    #[repr(C)]
    struct IwPoint {
        pointer: *mut libc::c_void,
        length: u16,
        flags: u16,
    }
    #[repr(C)]
    struct IwReqEssid {
        ifr_name: [u8; libc::IFNAMSIZ],
        data: IwPoint,
    }

    let name = iface.as_bytes();
    if name.len() >= libc::IFNAMSIZ {
        return None;
    }
    // SAFETY: the kernel writes at most `length` bytes into `buf`, which
    // outlives the ioctl; the fd is closed on every path.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            return None;
        }
        let mut buf = [0u8; IW_ESSID_MAX_SIZE];
        let mut req: IwReqEssid = std::mem::zeroed();
        req.ifr_name[..name.len()].copy_from_slice(name);
        req.data.pointer = buf.as_mut_ptr().cast();
        req.data.length = IW_ESSID_MAX_SIZE as u16;
        let rc = libc::ioctl(fd, SIOCGIWESSID as _, &mut req);
        let len = (req.data.length as usize).min(IW_ESSID_MAX_SIZE);
        libc::close(fd);
        if rc != 0 || len == 0 {
            return None;
        }
        Some(String::from_utf8_lossy(&buf[..len]).trim_end_matches('\0').to_owned())
    }
}

#[cfg(not(target_os = "linux"))]
fn essid(_iface: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from the Pi 3B+ (brcmfmac, 2026-07-10): quality 63 of 70,
    /// −47 dBm signal, −256 = noise unknown.
    const PI_SAMPLE: &str = "\
Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE
 face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22
 wlan0: 0000   63.  -47.  -256        0      0      0      0      0        0";

    /// A wireless-less machine (the dev VM): header only.
    const EMPTY_SAMPLE: &str = "\
Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE
 face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22";

    #[test]
    fn test_parse_proc_wireless_pi_sample() {
        assert_eq!(parse_proc_wireless(PI_SAMPLE, "wlan0"), Some((90, -47, -256)));
    }

    #[test]
    fn test_parse_proc_wireless_absent_interface() {
        assert_eq!(parse_proc_wireless(PI_SAMPLE, "wlan1"), None);
        assert_eq!(parse_proc_wireless(EMPTY_SAMPLE, "wlan0"), None);
    }

    #[test]
    fn test_parse_proc_wireless_quality_clamps_to_scale() {
        let s = "h\nh\n wlan0: 0000  100.  -30.  -256        0 0 0 0 0 0";
        assert_eq!(parse_proc_wireless(s, "wlan0"), Some((100, -30, -256)));
    }

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape(r#"a&b<c>"d""#), "a&amp;b&lt;c&gt;&quot;d&quot;");
    }

    fn wired() -> NetInfo {
        NetInfo {
            iface: "eth0".into(),
            wireless: false,
            ip: "192.168.210.147".into(),
            netmask: "255.255.255.0".into(),
            gateway: "192.168.210.1".into(),
            dns1: "192.168.210.1".into(),
            dns2: String::new(),
        }
    }

    /// The three states of the Info screen's network page. The no-network one
    /// is the case that used to fall through to a fixture and show a
    /// fabricated Ethernet at 192.168.1.50 (chumby-pi claude/issues.md).
    #[test]
    fn test_network_xml_wired_reports_lan() {
        let xml = network_xml(&wired(), String::new());
        assert!(xml.contains(r#"type="lan""#), "{xml}");
        assert!(xml.contains(r#"ip="192.168.210.147""#), "{xml}");
        assert!(!xml.contains("<error"), "{xml}");
    }

    #[test]
    fn test_network_xml_wireless_carries_the_ssid() {
        let mut n = wired();
        n.iface = "wlan0".into();
        n.wireless = true;
        let xml = network_xml(&n, "LXC & co".into());
        assert!(xml.contains(r#"type="wlan""#), "{xml}");
        assert!(xml.contains(r#"ssid="LXC &amp; co""#), "{xml}");
    }

    #[test]
    fn test_no_network_answer_is_the_panels_error_protocol() {
        // <error/> inside <interface> is what clears hasNetwork (frame_2
        // gotNetworkStatus); the Info screen then prints "network: not
        // connected". No address, no type, no MAC may appear.
        assert!(NO_NETWORK.contains("<interface><error/></interface>"));
        assert!(!NO_NETWORK.contains("ip="));
        assert!(!NO_NETWORK.contains("type="));
        assert_eq!(NO_WIFI, r#"<wifi connected="0"/>"#);
    }

    /// A MAC exists whether or not the machine has a route, so this must
    /// answer on any box with an interface — never the old 00:11:22:33:44:55.
    #[test]
    fn test_mac_is_read_from_the_system_or_empty() {
        let m = mac();
        assert_ne!(m, "00:11:22:33:44:55");
        if !m.is_empty() {
            assert_eq!(m.split(':').count(), 6, "{m}");
        }
    }
}
