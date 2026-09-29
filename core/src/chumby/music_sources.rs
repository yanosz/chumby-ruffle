//! Hide music sources the appliance cannot serve (scope re-decision,
//! 2026-07-11): entries are spliced out of `MusicPlayer.musicSources` at
//! VM level once frame 2 defines it — same surgery precedent as the
//! `WidgetPlayer.onPress` deletion (avm.rs). That one array feeds both the
//! Music source list (`availableSources`, which force-shows ipod past its
//! probe on non-insignia platforms) and the alarm audio list, so removing
//! an entry covers every screen. Sources whose own probe already fails
//! cleanly on us (fmradio, mp3tunes, internode) need no entry here.

use super::config::PlayerConfig;
use super::host;
use crate::avm1::{Activation, Value};
use ruffle_common::avm_string::AvmString;
use std::sync::atomic::{AtomicBool, Ordering};

/// Dead regardless of configuration: no iPod daemon on this appliance;
/// NOAA (Wunderground's relay is sunset upstream) and CBS Podcasts
/// confirmed non-working on a real chumby (Jan, 2026-07-11).
const ALWAYS_HIDDEN: &[&str] = &["ipod", "noaa", "cbspodcasts"];

/// Alive via the revived chumby.com music proxies (verified 2026-07-11);
/// shown only when player.toml grants chumby.com access, which also lets
/// their two hosts pass through the navigator (fixture.rs).
const CHUMBY_COM_BACKED: &[&str] = &["shoutcast", "chumbcast", "sleepcast"];

pub fn hidden_selectors(config: &PlayerConfig) -> Vec<&'static str> {
    let mut hidden = ALWAYS_HIDDEN.to_vec();
    if !config.access_chumby_com {
        hidden.extend(CHUMBY_COM_BACKED);
    }
    // Panel-side complete (playIP → the proven mpv stream path), but no
    // verified server exists and Lyrion is out of scope (Jan, 2026-07-11).
    if !config.enable_lyrion {
        hidden.push("slimserver");
    }
    hidden
}

/// One-shot with retry: frame 2 defines the array once, before any music
/// screen can list it; `reorderSources` and externalmusic.xml only permute
/// or insert, never restore a removed entry.
static APPLIED: AtomicBool = AtomicBool::new(false);

pub fn apply(activation: &mut Activation<'_, '_>) {
    if !APPLIED.load(Ordering::Relaxed) && filter_sources(activation).is_some() {
        APPLIED.store(true, Ordering::Relaxed);
    }
}

fn filter_sources(activation: &mut Activation<'_, '_>) -> Option<()> {
    let hidden = hidden_selectors(host::host()?.config());

    let name = AvmString::new_utf8(activation.gc(), "MusicPlayer");
    let Ok(Value::Object(player)) = activation.global_object().get(name, activation) else {
        return None;
    };
    let name = AvmString::new_utf8(activation.gc(), "musicSources");
    let Ok(Value::Object(sources)) = player.get(name, activation) else {
        return None;
    };
    let len = sources.length(activation).ok()?;
    if len == 0 {
        return None;
    }

    let mut kept = Vec::with_capacity(len as usize);
    let mut removed = Vec::new();
    for i in 0..len {
        let entry = sources.get_element(activation, i);
        let selector = match entry {
            Value::Object(o) => {
                let key = AvmString::new_utf8(activation.gc(), "selector");
                match o.get(key, activation) {
                    Ok(Value::String(s)) => s.to_utf8_lossy().into_owned(),
                    _ => String::new(),
                }
            }
            _ => String::new(),
        };
        if hidden.contains(&selector.as_str()) {
            removed.push(selector);
        } else {
            kept.push(entry);
        }
    }
    if removed.is_empty() {
        return Some(());
    }
    for (i, entry) in kept.iter().enumerate() {
        sources.set_element(activation, i as i32, *entry).ok()?;
    }
    sources.set_length(activation, kept.len() as i32).ok()?;
    tracing::info!(target: "chumby_host",
        "music sources hidden: {removed:?} ({} remain)", kept.len());
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chumby.com-backed trio is hidden exactly when access is off,
    /// Squeezebox exactly when enable_lyrion is off; the dead set always.
    #[test]
    fn test_hidden_selectors_follow_config() {
        let defaults = PlayerConfig::default();
        assert_eq!(
            hidden_selectors(&defaults),
            ["ipod", "noaa", "cbspodcasts", "shoutcast", "chumbcast", "sleepcast", "slimserver"]
        );
        let online = PlayerConfig {
            access_chumby_com: true,
            enable_lyrion: true,
            ..PlayerConfig::default()
        };
        assert_eq!(hidden_selectors(&online), ["ipod", "noaa", "cbspodcasts"]);
    }
}
