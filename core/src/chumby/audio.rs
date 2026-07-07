//! mpv-backed audio player for chumby alarm tones and URL streams.
//!
//! Called from `FixtureHost::native()` in response to the audio native family
//! (5,131–146). mpv is controlled via its Unix-socket IPC interface, which
//! allows live volume changes while audio is playing.
//!
//! If mpv is not installed the player runs in **silent-stub mode**: the state
//! machine responds correctly (the panel UI works), but no sound is produced
//! and no process is spawned. Every error degrades to silent-stub rather than
//! panicking.

use std::io::Write as IoWrite;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioState {
    Stopped,
    Playing,
    Paused,
}

pub struct AudioPlayer {
    /// Path to the mpv binary; `None` = silent-stub mode.
    pub bin: Option<PathBuf>,
    /// Virtual-rootfs root for resolving absolute chumby paths.
    rootfs: PathBuf,
    /// Running mpv process.
    child: Option<Child>,
    /// IPC socket for live control.
    socket: Option<UnixStream>,
    /// Current playback state (updated lazily via `poll_state`).
    pub state: AudioState,
    /// Loop count pre-set by `_playAudioLoopCount` (5,146) before `_playAudio`.
    pub pending_loops: u32,
}

const SOCKET_PATH: &str = "/tmp/chumby-mpv.sock";

/// How long `play()` waits for mpv's IPC socket. mpv needs ~150 ms to create
/// it on a fast desktop — expect more on a Pi or a loaded machine. Only a
/// broken-but-present mpv ever costs the full wait.
const SOCKET_WAIT: Duration = Duration::from_millis(1000);

impl AudioPlayer {
    pub fn new(rootfs: PathBuf) -> Self {
        let bin = find_mpv();
        if let Some(ref p) = bin {
            tracing::info!(target: "chumby_audio", "audio backend: {}", p.display());
        } else {
            tracing::warn!(target: "chumby_audio",
                "mpv not found — audio will be a silent stub (install mpv to enable)");
        }
        Self {
            bin,
            rootfs,
            child: None,
            socket: None,
            state: AudioState::Stopped,
            pending_loops: 1,
        }
    }

    /// Start playing `url` at `volume` (0–100).
    ///
    /// Stops any current playback first. Consumes `pending_loops` (reset to 1
    /// afterwards). Blocks up to `SOCKET_WAIT` waiting for the IPC socket; if
    /// it does not appear, volume control falls back to the `--volume` flag
    /// set at spawn time — audio still plays correctly.
    pub fn play(&mut self, url: &str, volume: f64) {
        self.stop_internal();

        let Some(ref bin) = self.bin else {
            tracing::info!(target: "chumby_audio", "play (silent stub): {url:?}");
            self.state = AudioState::Playing;
            self.pending_loops = 1;
            return;
        };

        let path = self.resolve_url(url);
        let vol = volume.clamp(0.0, 100.0) as u32;
        let loops = self.pending_loops;
        self.pending_loops = 1;

        let mut cmd = Command::new(bin);
        cmd.args([
            "--no-video",
            "--no-terminal",
            "--really-quiet",
            &format!("--volume={vol}"),
            &format!("--input-ipc-server={SOCKET_PATH}"),
        ]);
        if loops > 1 {
            cmd.arg(format!("--loop={loops}"));
        }
        // Route to a specific output (e.g. the Pi's USB card) without
        // touching the system-wide ALSA default: the launcher sets
        // CHUMBY_AUDIO_DEVICE to an mpv device name like
        // "alsa/plughw:CARD=UACDemoV10" (list with `mpv --audio-device=help`).
        if let Ok(dev) = std::env::var("CHUMBY_AUDIO_DEVICE") {
            if !dev.is_empty() {
                cmd.arg(format!("--audio-device={dev}"));
            }
        }
        cmd.arg(&path);

        match cmd.spawn() {
            Ok(child) => {
                tracing::info!(target: "chumby_audio",
                    "mpv pid={} url={url:?} vol={vol} loops={loops}", child.id());
                self.child = Some(child);
                self.socket = wait_for_socket(Path::new(SOCKET_PATH), SOCKET_WAIT);
                if self.socket.is_none() {
                    tracing::warn!(target: "chumby_audio",
                        "IPC socket not ready — volume control limited to spawn-time setting");
                }
                self.state = AudioState::Playing;
            }
            Err(e) => {
                tracing::warn!(target: "chumby_audio", "mpv spawn failed: {e} — silent stub");
                self.state = AudioState::Stopped;
            }
        }
    }

    /// Send a volume change to the running player (0–100). No-op if stopped.
    pub fn set_volume(&mut self, volume: f64) {
        let vol = volume.clamp(0.0, 100.0) as u32;
        self.send_ipc(&format!(r#"{{"command":["set_property","volume",{vol}]}}"#));
    }

    pub fn pause(&mut self) {
        if self.state == AudioState::Playing {
            self.send_ipc(r#"{"command":["set_property","pause",true]}"#);
            self.state = AudioState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state == AudioState::Paused {
            self.send_ipc(r#"{"command":["set_property","pause",false]}"#);
            self.state = AudioState::Playing;
        }
    }

    pub fn stop(&mut self) {
        self.stop_internal();
    }

    /// Non-blocking poll: if the mpv process has exited, transition to Stopped.
    /// Call this from `_getAudioPlayerState` to detect natural end or crashes.
    pub fn poll_state(&mut self) -> AudioState {
        if self.child.is_some() {
            let exited = if let Some(ref mut c) = self.child {
                match c.try_wait() {
                    Ok(Some(status)) => {
                        tracing::info!(target: "chumby_audio", "mpv exited: {status}");
                        true
                    }
                    Ok(None) => false,
                    Err(e) => {
                        tracing::warn!(target: "chumby_audio", "try_wait error: {e}");
                        false
                    }
                }
            } else {
                false
            };
            if exited {
                self.child = None;
                self.socket = None;
                let _ = std::fs::remove_file(SOCKET_PATH);
                self.state = AudioState::Stopped;
            }
        }
        self.state
    }

    // ---- private helpers ----

    fn stop_internal(&mut self) {
        self.send_ipc(r#"{"command":["quit"]}"#);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill(); // SIGKILL fallback
            let _ = child.wait(); // reap — instant after SIGKILL, else zombie
        }
        self.socket = None;
        let _ = std::fs::remove_file(SOCKET_PATH);
        self.state = AudioState::Stopped;
    }

    /// Map an absolute chumby path through the virtual rootfs.
    /// HTTP/HTTPS URLs are returned unchanged.
    fn resolve_url(&self, url: &str) -> String {
        if !url.starts_with('/') {
            return url.to_owned();
        }
        let clean: Vec<&str> = url
            .split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .collect();
        if clean.iter().any(|c| *c == "..") {
            tracing::warn!(target: "chumby_audio", "rejected path traversal in {url:?}");
            return url.to_owned();
        }
        let mut p = self.rootfs.clone();
        p.extend(&clean);
        p.to_string_lossy().into_owned()
    }

    /// Write a JSON command to the IPC socket — fire and forget.
    /// On error the socket is closed; the next `play()` will open a fresh one.
    fn send_ipc(&mut self, json: &str) {
        if self.socket.is_none() && self.child.is_some() {
            // mpv can take >SOCKET_WAIT to create the socket on a loaded Pi
            // (seen at 13:41 2026-07-06: alarm fade-in muted forever). The
            // panel keeps sending volume updates, so retry the connect here.
            self.socket = UnixStream::connect(SOCKET_PATH).ok();
            if self.socket.is_some() {
                tracing::info!(target: "chumby_audio", "IPC socket connected late");
            }
        }
        let Some(ref mut sock) = self.socket else { return };
        let mut line = json.to_owned();
        line.push('\n');
        if let Err(e) = sock.write_all(line.as_bytes()) {
            tracing::warn!(target: "chumby_audio", "IPC write failed ({e}) — clearing socket");
            self.socket = None;
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop_internal();
    }
}

fn find_mpv() -> Option<PathBuf> {
    if let Ok(out) = Command::new("which").arg("mpv").output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

/// Spin-wait for the IPC socket to appear, then connect.
/// Returns `None` if the socket is not ready within `timeout`.
fn wait_for_socket(path: &Path, timeout: Duration) -> Option<UnixStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(path) {
            Ok(s) => return Some(s),
            Err(_) => {
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration test for the mpv audio backend.
    ///
    /// Requires mpv to be installed and the chumby alarm-tone fixtures to exist.
    /// Run with:
    ///   CHUMBY_FIXTURES=/path/to/chumby-pi/fixtures \
    ///   cargo test -p ruffle_core chumby::audio::tests \
    ///   -- --nocapture
    ///
    /// Skips gracefully if mpv is absent or fixtures are missing.
    #[test]
    fn test_mpv_play_volume_pause_stop() {
        // Locate the fixture rootfs from the env var or a sibling-repo heuristic.
        let rootfs = if let Ok(f) = std::env::var("CHUMBY_FIXTURES") {
            PathBuf::from(f).join("rootfs")
        } else {
            // CARGO_MANIFEST_DIR is <chumby-pi>/resources/ruffle/core.
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent().unwrap()   // resources/ruffle/
                .parent().unwrap()   // resources/
                .parent().unwrap()   // chumby-pi/
                .join("fixtures/rootfs")
        };

        let tone = rootfs.join("usr/chumby/alarmtones/Beep.mp3");
        let mut player = AudioPlayer::new(rootfs.clone());

        if player.bin.is_none() {
            eprintln!("SKIP: mpv not found");
            return;
        }
        if !tone.exists() {
            eprintln!("SKIP: fixture tone not found at {}", tone.display());
            return;
        }

        // --- play ---
        player.pending_loops = 5;
        player.play("/usr/chumby/alarmtones/Beep.mp3", 30.0);
        assert_eq!(player.state, AudioState::Playing, "state should be Playing after play()");
        assert!(player.child.is_some(), "child process should exist");
        assert!(player.socket.is_some(), "IPC socket should be open");

        // Give mpv a moment to settle.
        std::thread::sleep(Duration::from_millis(300));

        // --- live volume change ---
        player.set_volume(80.0);
        // No panic / socket-closed = IPC is working.
        assert!(player.socket.is_some(), "socket should still be open after set_volume");

        // --- pause / resume ---
        player.pause();
        assert_eq!(player.state, AudioState::Paused);
        std::thread::sleep(Duration::from_millis(100));
        player.resume();
        assert_eq!(player.state, AudioState::Playing);

        // --- stop ---
        player.stop();
        assert_eq!(player.state, AudioState::Stopped);
        assert!(player.child.is_none(), "child should be gone after stop()");
        assert!(player.socket.is_none(), "socket should be gone after stop()");

        // --- poll after stop confirms stopped (no crash) ---
        let state = player.poll_state();
        assert_eq!(state, AudioState::Stopped);

        eprintln!("mpv integration test passed");
    }
}
