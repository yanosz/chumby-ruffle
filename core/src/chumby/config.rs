//! Player configuration: `<fixtures>/player.toml`, read once at start.
//!
//! Owner-level knobs. The file lives at the fixtures root, deliberately
//! *outside* `rootfs/` — everything under the virtual rootfs is reachable
//! by the panel's own `_putFile`, and the panel must not be able to
//! reconfigure the player. Missing file means defaults; a parse error is
//! logged and answered with defaults (NFR3: degrade, don't crash).
//!
//! ```toml
//! volume_cap = 70        # percent: the panel's 100% maps to this
//! access_chumby_com = 0  # music-proxy passthrough (FR15); default 0 (NFR6)
//! enable_lyrion = 0      # show the Squeezebox Server source
//! ```
//! The committed template is `fixtures/player.toml.example`.

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerConfig {
    /// Scales what reaches the audio backend: effective volume =
    /// panel volume × cap / 100. Panel space stays 0–100 everywhere the
    /// panel reads it back. The backup-alarm Klaxon (FR13) deliberately
    /// ignores the cap — it has its own `/psp/backup_alarm_volume` knob.
    pub volume_cap: f64,
    /// Opt-in chumby.com traffic. Today it gates exactly the music
    /// proxies: the SHOUTcast/blue-octy sources appear in the panel and
    /// their hosts pass through the navigator (music_sources.rs,
    /// fixture.rs). Off (the default), NFR6 holds: nothing reaches
    /// chumby.com. The remote-channels milestone will widen this.
    pub access_chumby_com: bool,
    /// Shows the Squeezebox Server source. The panel side is complete
    /// (playIP → mpv, the proven stream path); whether a modern Lyrion
    /// server still answers the legacy `/stream.mp3` player protocol is
    /// unverified and out of scope (Jan, 2026-07-11) — hence off.
    pub enable_lyrion: bool,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            volume_cap: 100.0,
            access_chumby_com: false,
            enable_lyrion: false,
        }
    }
}

pub fn load(path: &Path) -> PlayerConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            tracing::info!(target: "chumby_host",
                "no player config at {} — defaults", path.display());
            return PlayerConfig::default();
        }
    };
    let config = parse(&text);
    tracing::info!(target: "chumby_host",
        "player config {}: volume_cap={} access_chumby_com={}",
        path.display(), config.volume_cap, config.access_chumby_com);
    if config.access_chumby_com {
        tracing::warn!(target: "chumby_host",
            "access_chumby_com=1: music proxies pass through to chumby.com \
             (music_sources.rs); remote channels/registration stay unimplemented");
    }
    config
}

fn parse(text: &str) -> PlayerConfig {
    let mut config = PlayerConfig::default();
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(target: "chumby_host",
                "player config: cannot parse: {e} — defaults");
            return config;
        }
    };
    for (key, value) in &table {
        match (key.as_str(), value) {
            ("volume_cap", v) => match as_number(v) {
                Some(n) if (0.0..=100.0).contains(&n) => config.volume_cap = n,
                _ => tracing::warn!(target: "chumby_host",
                    "player config: volume_cap must be 0–100, got {value} — keeping {}",
                    config.volume_cap),
            },
            ("access_chumby_com", v) => match as_flag(v) {
                Some(b) => config.access_chumby_com = b,
                None => tracing::warn!(target: "chumby_host",
                    "player config: access_chumby_com must be 0 or 1, got {value}"),
            },
            ("enable_lyrion", v) => match as_flag(v) {
                Some(b) => config.enable_lyrion = b,
                None => tracing::warn!(target: "chumby_host",
                    "player config: enable_lyrion must be 0 or 1, got {value}"),
            },
            _ => tracing::warn!(target: "chumby_host",
                "player config: unknown key {key:?} ignored"),
        }
    }
    config
}

fn as_number(value: &toml::Value) -> Option<f64> {
    match value {
        toml::Value::Integer(i) => Some(*i as f64),
        toml::Value::Float(f) => Some(*f),
        _ => None,
    }
}

fn as_flag(value: &toml::Value) -> Option<bool> {
    match value {
        toml::Value::Integer(0) => Some(false),
        toml::Value::Integer(1) => Some(true),
        toml::Value::Boolean(b) => Some(*b),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_file_yields_defaults() {
        let config = load(Path::new("/nonexistent/player.toml"));
        assert_eq!(config, PlayerConfig::default());
        assert_eq!(config.volume_cap, 100.0);
        assert!(!config.access_chumby_com);
    }

    #[test]
    fn test_parse_values() {
        let config = parse("volume_cap = 70\naccess_chumby_com = 1\nenable_lyrion = 1\n");
        assert_eq!(config.volume_cap, 70.0);
        assert!(config.access_chumby_com);
        assert!(config.enable_lyrion);
        // Floats and TOML booleans are accepted too.
        let config = parse("volume_cap = 55.5\naccess_chumby_com = false\n");
        assert_eq!(config.volume_cap, 55.5);
        assert!(!config.access_chumby_com);
    }

    #[test]
    fn test_bad_values_keep_defaults() {
        // Garbage file, out-of-range cap, wrong types, unknown keys:
        // none may be fatal, all fall back to defaults (NFR3).
        assert_eq!(parse("not toml at ["), PlayerConfig::default());
        assert_eq!(parse("volume_cap = 150"), PlayerConfig::default());
        assert_eq!(parse("volume_cap = -1"), PlayerConfig::default());
        assert_eq!(parse("volume_cap = \"loud\""), PlayerConfig::default());
        assert_eq!(parse("access_chumby_com = 2"), PlayerConfig::default());
        assert_eq!(parse("some_future_key = 1"), PlayerConfig::default());
    }
}
