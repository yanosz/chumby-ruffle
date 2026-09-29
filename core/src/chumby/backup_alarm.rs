//! Backup alarm: the dead-man beep that `chumbalarmd` provided on real
//! hardware.
//!
//! The contract is one file. The panel arms by writing the fire time (epoch
//! seconds = primary alarm + backupDelay) to `/psp/ifalarm` and disarms by
//! deleting it when the ring screen is answered. If the time passes while
//! the file still exists, nobody responded to the primary alarm and a loud
//! tone must sound.
//!
//! A watcher thread polls the file every few seconds instead of computing
//! one long sleep: that makes the file the single source of truth (arming,
//! re-arming and boot-time missed alarms all fall out of the same check) and
//! is immune to wall-clock jumps — the Pi has no RTC, so the clock *will*
//! step after boot.
//!
//! Its level is `/psp/backup_alarm_volume` (default 100) times the
//! appliance's `volume_cap` — the ceiling applies to every stage that
//! reaches the amplifier.
//!
//! The tone deliberately shares no fate with the primary alarm's playback:
//! its own mpv child (never `AudioPlayer`'s slot or IPC socket), fed from a
//! local file, falling back to an mpv-generated sine tone — no network
//! anywhere on the path. WLAN loss mid-stream is the failure this exists
//! for (mpv plays its ~5 s cache, sits silent for its 60 s network timeout,
//! then exits; the panel never notices).

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const POLL: Duration = Duration::from_secs(2);
/// A fire time older than this is a stale leftover (device off for a day),
/// not a wake-up anyone still wants; it is cleared without sounding.
const STALE_WINDOW: i64 = 3600;
/// Defaults behind the optional /psp knobs chumbalarmd also read.
const DEFAULT_DURATION_SECS: u64 = 60;
const DEFAULT_VOLUME: u32 = 100;
const TONE_FILE: &str = "usr/chumby/alarmtones/Klaxon.mp3";
const TONE_FALLBACK: &str = "av://lavfi:sine=frequency=880";

pub struct BackupAlarm {
    beep: Arc<Mutex<Option<Child>>>,
}

impl BackupAlarm {
    /// Spawn the watcher thread. The thread runs for the life of the
    /// process; the host it belongs to is a process-global singleton.
    /// `volume_cap` is the appliance's ceiling (config.rs): the tone is a
    /// dead-man beep, but it comes out of the same amplifier as everything
    /// else, so it obeys the same ceiling (Jan, 2026-09-10).
    pub fn start(rootfs: PathBuf, volume_cap: f64) -> Self {
        let beep = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&beep);
        std::thread::Builder::new()
            .name("chumby-backup-alarm".into())
            .spawn(move || watch(rootfs, slot, volume_cap))
            .expect("spawn backup-alarm watcher");
        Self { beep }
    }

    /// Stop a sounding tone. Called when the panel dismisses the alarm
    /// (`rm /psp/ifalarm; reload_backup_alarm`).
    pub fn dismiss(&self) {
        if let Some(mut child) = self.beep.lock().unwrap().take() {
            tracing::info!(target: "chumby_backup_alarm", "dismissed — stopping tone");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for BackupAlarm {
    fn drop(&mut self) {
        self.dismiss();
    }
}

enum Action {
    Wait,
    Fire { late_by: i64 },
    ClearStale { late_by: i64 },
}

fn decide(now: i64, armed: i64) -> Action {
    let late_by = now - armed;
    if late_by < 0 {
        Action::Wait
    } else if late_by <= STALE_WINDOW {
        Action::Fire { late_by }
    } else {
        Action::ClearStale { late_by }
    }
}

fn watch(rootfs: PathBuf, beep: Arc<Mutex<Option<Child>>>, volume_cap: f64) {
    let ifalarm = rootfs.join("psp/ifalarm");
    tracing::info!(target: "chumby_backup_alarm",
        "watching {} (stale window {STALE_WINDOW}s)", ifalarm.display());
    loop {
        if let Some(armed) = read_epoch(&ifalarm) {
            match decide(unix_now(), armed) {
                Action::Wait => {}
                Action::Fire { late_by } => {
                    let _ = std::fs::remove_file(&ifalarm);
                    tracing::warn!(target: "chumby_backup_alarm",
                        "primary alarm unanswered ({late_by}s past fire time) — sounding");
                    sound(&rootfs, &beep, volume_cap);
                }
                Action::ClearStale { late_by } => {
                    let _ = std::fs::remove_file(&ifalarm);
                    tracing::info!(target: "chumby_backup_alarm",
                        "clearing stale ifalarm ({late_by}s old)");
                }
            }
        }
        std::thread::sleep(POLL);
    }
}

/// Spawn the tone and babysit it for the configured duration, or until a
/// dismissal empties the slot. Blocks the watcher thread — a new arm written
/// mid-tone is picked up by the next poll.
fn sound(rootfs: &PathBuf, beep: &Arc<Mutex<Option<Child>>>, volume_cap: f64) {
    let knob = read_knob(rootfs, "psp/backup_alarm_volume", DEFAULT_VOLUME as i64);
    let volume = tone_volume(knob, volume_cap);
    let duration = read_knob(rootfs, "psp/backup_alarm_duration", DEFAULT_DURATION_SECS as i64)
        .max(1) as u64;

    let tone_path = rootfs.join(TONE_FILE);
    let source = if tone_path.exists() {
        tone_path.to_string_lossy().into_owned()
    } else {
        tracing::warn!(target: "chumby_backup_alarm",
            "{} missing — falling back to generated tone", tone_path.display());
        TONE_FALLBACK.to_owned()
    };

    let mut cmd = Command::new("mpv");
    cmd.args([
        "--no-video",
        "--no-terminal",
        "--really-quiet",
        "--loop=inf",
        &format!("--volume={volume}"),
    ]);
    if let Ok(dev) = std::env::var("CHUMBY_AUDIO_DEVICE") {
        if !dev.is_empty() {
            cmd.arg(format!("--audio-device={dev}"));
        }
    }
    cmd.arg(&source);

    match cmd.spawn() {
        Ok(child) => {
            tracing::info!(target: "chumby_backup_alarm",
                "tone pid={} source={source:?} vol={volume} (knob {knob} × cap \
                 {volume_cap}%) duration={duration}s", child.id());
            *beep.lock().unwrap() = Some(child);
        }
        Err(e) => {
            tracing::error!(target: "chumby_backup_alarm", "cannot spawn mpv: {e}");
            return;
        }
    }

    let deadline = SystemTime::now() + Duration::from_secs(duration);
    while SystemTime::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        let mut slot = beep.lock().unwrap();
        match slot.as_mut() {
            None => return, // dismissed
            Some(child) => {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    *slot = None;
                    tracing::warn!(target: "chumby_backup_alarm", "tone exited early");
                    return;
                }
            }
        }
    }
    if let Some(mut child) = beep.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
        tracing::info!(target: "chumby_backup_alarm", "tone finished ({duration}s)");
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn read_epoch(path: &std::path::Path) -> Option<i64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The tone's mpv volume: the /psp knob under the appliance ceiling.
fn tone_volume(knob: i64, cap: f64) -> i64 {
    (knob.clamp(0, 100) as f64 * cap.clamp(0.0, 100.0) / 100.0).round() as i64
}

fn read_knob(rootfs: &std::path::Path, rel: &str, default: i64) -> i64 {
    std::fs::read_to_string(rootfs.join(rel))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_before_fire_time() {
        assert!(matches!(decide(1000, 1001), Action::Wait));
    }

    #[test]
    fn fires_at_and_after_fire_time() {
        assert!(matches!(decide(1000, 1000), Action::Fire { late_by: 0 }));
        assert!(matches!(decide(1000 + STALE_WINDOW, 1000), Action::Fire { .. }));
    }

    /// The dead-man tone is not exempt from the appliance ceiling
    /// (Jan, 2026-09-10): knob × cap, both clamped, nothing rounding
    /// a wanted tone to silence except a knob or cap of 0.
    #[test]
    fn tone_volume_obeys_the_cap() {
        assert_eq!(tone_volume(100, 50.0), 50);
        assert_eq!(tone_volume(100, 100.0), 100);
        assert_eq!(tone_volume(60, 50.0), 30);
        assert_eq!(tone_volume(1, 50.0), 1); // 0.5 rounds up
        assert_eq!(tone_volume(0, 50.0), 0);
        assert_eq!(tone_volume(150, 50.0), 50); // knob clamps first
        assert_eq!(tone_volume(100, 150.0), 100); // so does the cap
    }

    #[test]
    fn clears_stale_without_firing() {
        assert!(matches!(
            decide(1000 + STALE_WINDOW + 1, 1000),
            Action::ClearStale { .. }
        ));
    }
}
