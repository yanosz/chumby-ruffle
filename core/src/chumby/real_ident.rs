//! Real device identity — GUID and hardware serial derived from the host
//! machine, replacing the crypto processor the original hardware had.
//!
//! On real hardware `guidgen.sh` (= `cpi.sh -p`) and `chumby_version -n`
//! both read the crypto chip. Here the stable seed is the SoC serial from
//! the device tree (survives reflashes and SD swaps), falling back to
//! `/etc/machine-id` on a dev box; if neither exists (CI) the caller falls
//! back to the fixture. The GUID is a salted md5 of the serial in the
//! 8-4-4-4-12 shape the fixture also uses — the panel treats it as an
//! opaque string, and it never leaves the process (every chumby.com
//! endpoint is answered in-process; NFR6).

use md5::{Digest, Md5};

const SERIAL_SOURCES: &[&str] = &["/proc/device-tree/serial-number", "/etc/machine-id"];
const MODEL_SOURCE: &str = "/proc/device-tree/model";
/// Keeps the GUID from being a plain dictionary-hash of a guessable serial.
const GUID_SALT: &str = "chumby-pi";

/// The machine's stable serial, or `None` when no source exists.
pub fn serial() -> Option<String> {
    SERIAL_SOURCES.iter().find_map(|path| read_cstr(path))
}

/// `chumby_version -n` (the Info screen's `HW#:` line): model tag plus the
/// serial uppercased with leading zeros stripped (the original Perl script's
/// `s/^0+//`). The panel has no model field of its own, so the tag rides on
/// the serial line — display-only, plus intercepted chumby.com URLs.
pub fn hw_serial() -> Option<String> {
    let s = serial()?.to_uppercase();
    let s = s.trim_start_matches('0');
    (!s.is_empty()).then(|| format!("{}-{s}", model_acronym()))
}

/// Short model tag: "RPI3B" for "Raspberry Pi 3 Model B Rev 1.2", "PC" when
/// the device-tree model is absent or not a Raspberry Pi.
fn model_acronym() -> String {
    read_cstr(MODEL_SOURCE)
        .and_then(|m| acronym_from(&m))
        .unwrap_or_else(|| "PC".to_owned())
}

fn acronym_from(model: &str) -> Option<String> {
    let rest = model.strip_prefix("Raspberry Pi")?;
    let mut tag = String::from("RPI");
    for token in rest.split_whitespace() {
        if token.eq_ignore_ascii_case("Model") {
            continue;
        }
        if token.eq_ignore_ascii_case("Rev") {
            break;
        }
        tag.push_str(&token.to_uppercase());
    }
    Some(tag)
}

/// Read a file that may be a NUL-terminated device-tree string.
fn read_cstr(path: &str) -> Option<String> {
    let raw = std::fs::read(path).ok()?;
    let s: String = raw
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// `guidgen.sh`: salted md5 of the serial as an uppercase 8-4-4-4-12 GUID.
pub fn guid() -> Option<String> {
    serial().map(|s| format_guid(&md5_hex(format!("{GUID_SALT}:{s}").as_bytes())))
}

/// `md5sum <path>` output line for the given file content.
pub fn md5sum_line(content: &[u8], path: &str) -> String {
    format!("{}  {path}\n", md5_hex(content))
}

fn md5_hex(data: &[u8]) -> String {
    let digest = Md5::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn format_guid(hex32: &str) -> String {
    let h = hex32.to_uppercase();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_coreutils() {
        // printf 'hello' | md5sum
        assert_eq!(md5_hex(b"hello"), "5d41402abc4b2a76b9719d911017c592");
    }

    #[test]
    fn md5sum_line_matches_coreutils_format() {
        assert_eq!(
            md5sum_line(b"hello", "/tmp/.guidhash"),
            "5d41402abc4b2a76b9719d911017c592  /tmp/.guidhash\n"
        );
    }

    #[test]
    fn model_acronyms() {
        assert_eq!(
            acronym_from("Raspberry Pi 3 Model B Rev 1.2").as_deref(),
            Some("RPI3B")
        );
        assert_eq!(
            acronym_from("Raspberry Pi 4 Model B Rev 1.5").as_deref(),
            Some("RPI4B")
        );
        assert_eq!(
            acronym_from("Raspberry Pi Zero 2 W Rev 1.0").as_deref(),
            Some("RPIZERO2W")
        );
        assert_eq!(acronym_from("Some x86 Board"), None);
    }

    #[test]
    fn guid_shape() {
        let g = format_guid(&md5_hex(b"chumby-pi:10000000abcdef12"));
        assert_eq!(g.len(), 36);
        assert!(
            g.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase() || c == '-')
        );
        assert_eq!(g.match_indices('-').count(), 4);
        // Deterministic: same seed, same GUID.
        assert_eq!(g, format_guid(&md5_hex(b"chumby-pi:10000000abcdef12")));
    }
}
