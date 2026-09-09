//! `tzdump <zone>`: the Dash's DST source. `time/TimeZoneTransitions.load`
//! backticks it and parses `<zone><time utc= gmtoff= isdst= abbrev=/>…</zone>`,
//! then `transitionFor(utc)` picks the last entry at or before now (first
//! entry when none is), so the list must be ascending. The classic panel
//! never calls it. Read from the system tzdata (TZif, RFC 8536), the same
//! files the box's own clock uses; Debian ships them with every transition
//! through 2037 spelled out, so the POSIX footer is not expanded.

use std::path::Path;

const ZONEINFO: &str = "/usr/share/zoneinfo";

/// One transition: seconds since the epoch, offset from UTC, DST flag, name.
#[derive(Debug, PartialEq, Eq)]
pub struct Transition {
    pub utc: i64,
    pub gmtoff: i32,
    pub isdst: bool,
    pub abbrev: String,
}

/// The XML the panel parses, or `None` for a zone the system does not know.
pub fn tzdump(zone: &str) -> Option<String> {
    let transitions = transitions(zone)?;
    let mut out = format!("<zone name=\"{}\">", xml_escape(zone));
    for t in &transitions {
        out.push_str(&format!(
            "<time utc=\"{}\" gmtoff=\"{}\" isdst=\"{}\" abbrev=\"{}\"/>",
            t.utc,
            t.gmtoff,
            u8::from(t.isdst),
            xml_escape(&t.abbrev)
        ));
    }
    out.push_str("</zone>\n");
    Some(out)
}

/// Transitions from 1970 on, ascending; a zone without any (UTC) yields its
/// single type at utc 0 so the panel always has one entry.
pub fn transitions(zone: &str) -> Option<Vec<Transition>> {
    if zone.is_empty()
        || zone.starts_with('/')
        || zone.split('/').any(|c| c.is_empty() || c == "." || c == "..")
    {
        return None;
    }
    let data = std::fs::read(Path::new(ZONEINFO).join(zone)).ok()?;
    parse_tzif(&data)
}

/// TZif header counts, in file order.
struct Header {
    isutcnt: usize,
    isstdcnt: usize,
    leapcnt: usize,
    timecnt: usize,
    typecnt: usize,
    charcnt: usize,
}

fn header(d: &[u8]) -> Option<(u8, Header)> {
    if d.len() < 44 || &d[..4] != b"TZif" {
        return None;
    }
    let n = |o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
    Some((
        d[4],
        Header {
            isutcnt: n(20),
            isstdcnt: n(24),
            leapcnt: n(28),
            timecnt: n(32),
            typecnt: n(36),
            charcnt: n(40),
        },
    ))
}

fn parse_tzif(d: &[u8]) -> Option<Vec<Transition>> {
    let (version, h1) = header(d)?;
    // Version 1 block: 32-bit times. Skip it for the 64-bit block when there is one.
    let v1_len = 44 + h1.timecnt * 5 + h1.typecnt * 6 + h1.charcnt + h1.leapcnt * 8 + h1.isstdcnt + h1.isutcnt;
    let (body, h, time_size) = if version >= b'2' {
        let (_, h2) = header(d.get(v1_len..)?)?;
        (&d[v1_len + 44..], h2, 8)
    } else {
        (&d[44..], h1, 4)
    };
    let times = body.get(..h.timecnt * time_size)?;
    let idx = body.get(h.timecnt * time_size..h.timecnt * (time_size + 1))?;
    let types_at = h.timecnt * (time_size + 1);
    let types = body.get(types_at..types_at + h.typecnt * 6)?;
    let chars = body.get(types_at + h.typecnt * 6..types_at + h.typecnt * 6 + h.charcnt)?;
    let ttinfo = |i: usize| -> Option<(i32, bool, String)> {
        let e = types.get(i * 6..i * 6 + 6)?;
        let off = i32::from_be_bytes(e[..4].try_into().ok()?);
        let abbr = chars.get(e[5] as usize..)?;
        let end = abbr.iter().position(|&b| b == 0).unwrap_or(abbr.len());
        Some((off, e[4] != 0, String::from_utf8_lossy(&abbr[..end]).into_owned()))
    };
    let mut out = Vec::with_capacity(h.timecnt);
    for i in 0..h.timecnt {
        let utc = if time_size == 8 {
            i64::from_be_bytes(times[i * 8..i * 8 + 8].try_into().ok()?)
        } else {
            i32::from_be_bytes(times[i * 4..i * 4 + 4].try_into().ok()?) as i64
        };
        if utc < 0 {
            continue;
        }
        let (gmtoff, isdst, abbrev) = ttinfo(*idx.get(i)? as usize)?;
        out.push(Transition { utc, gmtoff, isdst, abbrev });
    }
    if out.is_empty() {
        let (gmtoff, isdst, abbrev) = ttinfo(0)?;
        out.push(Transition { utc: 0, gmtoff, isdst, abbrev });
    }
    Some(out)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// New York alternates EST/EDT, ascending, with the 2026 spring change
    /// on the second Sunday of March (2026-03-08 07:00 UTC).
    #[test]
    fn new_york_transitions_from_system_tzdata() {
        let Some(t) = transitions("America/New_York") else {
            eprintln!("no tzdata on this machine, skipping");
            return;
        };
        assert!(t.windows(2).all(|w| w[0].utc < w[1].utc), "ascending");
        let spring_2026 = t.iter().find(|x| x.utc == 1_772_953_200).expect("2026-03-08 07:00 UTC");
        assert_eq!((spring_2026.gmtoff, spring_2026.isdst, spring_2026.abbrev.as_str()), (-14400, true, "EDT"));
        let xml = tzdump("America/New_York").unwrap();
        assert!(xml.starts_with("<zone name=\"America/New_York\"><time utc=\""));
        assert!(xml.contains("isdst=\"1\" abbrev=\"EDT\"/>"));
    }

    #[test]
    fn utc_has_one_entry() {
        if let Some(t) = transitions("UTC") {
            assert_eq!(t, vec![Transition { utc: 0, gmtoff: 0, isdst: false, abbrev: "UTC".into() }]);
        }
    }

    #[test]
    fn zone_names_cannot_leave_zoneinfo() {
        assert!(transitions("../../etc/passwd").is_none());
        assert!(transitions("/etc/passwd").is_none());
        assert!(transitions("").is_none());
        assert!(transitions("No/Such/Zone").is_none());
    }
}
