//! `FixtureHost`: answers every host category from a fixtures directory.
//!
//! Layout (chumby-pi project, `claude-docs/design/chumby-host.md` §5):
//! ```text
//! fixtures/
//!   rootfs/    virtual filesystem for the fs category (paths map 1:1)
//!   exec/      manifest.txt (TAB-separated: prefix<TAB>file) + response files
//!   http/      <host>/<path> response files (fetch category)
//! ```
//! Fixture keys are the panel's own request strings — a logged unmatched
//! call names the fixture file to create.

use super::audio::{AudioPlayer, AudioState};
use super::host::{self, ChumbyFs, ChumbyHost, HostError, HostValue};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct FixtureHost {
    root: PathBuf,
    fs: RootFs,
    /// exec manifest: (command prefix, response file), longest prefix wins.
    exec_manifest: Vec<(String, PathBuf)>,
    /// Master/slave variable exchange, `_setSlaveVar` (5,80) /
    /// `_getSlaveVar` (5,81). `_chumby_widget_done` defaults to "true" so
    /// intro/widget handoffs never hang.
    slave_vars: Mutex<HashMap<String, String>>,
    /// Stateful native get/set pairs (volume, mute, balance, touchclick...).
    native_state: Mutex<HashMap<&'static str, HostValue>>,
    /// mpv-backed audio player for alarm tones and URL streams.
    audio: Mutex<AudioPlayer>,
}

impl FixtureHost {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        // Absolute, so {FIXTURES} expansion (expand_tokens) yields usable
        // file:// URLs regardless of the launch directory.
        let root = root.canonicalize().unwrap_or(root);
        let rootfs_path = root.join("rootfs");
        let exec_manifest = load_exec_manifest(&root.join("exec"));
        tracing::info!(target: "chumby_host",
            "FixtureHost at {} ({} exec fixtures)", root.display(), exec_manifest.len());

        // Seed volume from the persisted /psp/volume fixture so that
        // _getSystemVolume returns the last saved level after a restart.
        let mut initial_state: HashMap<&'static str, HostValue> = HashMap::new();
        let volume_path = rootfs_path.join("psp").join("volume");
        if let Ok(text) = std::fs::read_to_string(&volume_path) {
            if let Ok(v) = text.trim().parse::<f64>() {
                initial_state.insert("SystemVolume", HostValue::Number(v));
            }
        }

        Self {
            fs: RootFs { root: rootfs_path.clone() },
            exec_manifest,
            slave_vars: Mutex::new(HashMap::new()),
            native_state: Mutex::new(initial_state),
            audio: Mutex::new(AudioPlayer::new(rootfs_path)),
            root,
        }
    }

    /// Expand `{FIXTURES}` in a fixture body to the absolute fixtures
    /// directory. Fixture files must not hardcode install paths (the
    /// profile XML references widget SWFs by `file://` URL); the token
    /// keeps the same fixture tree working on the dev box and the Pi.
    fn expand_tokens(&self, body: Vec<u8>) -> Vec<u8> {
        const TOKEN: &[u8] = b"{FIXTURES}";
        if !body.windows(TOKEN.len()).any(|w| w == TOKEN) {
            return body;
        }
        let root = self.root.as_os_str().as_encoded_bytes();
        let mut out = Vec::with_capacity(body.len() + root.len());
        let mut rest = &body[..];
        while let Some(pos) = rest.windows(TOKEN.len()).position(|w| w == TOKEN) {
            out.extend_from_slice(&rest[..pos]);
            out.extend_from_slice(root);
            rest = &rest[pos + TOKEN.len()..];
        }
        out.extend_from_slice(rest);
        out
    }
}

fn load_exec_manifest(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut entries = Vec::new();
    if let Ok(text) = std::fs::read_to_string(dir.join("manifest.txt")) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((prefix, file)) = line.split_once('\t') {
                entries.push((prefix.to_owned(), dir.join(file.trim())));
            } else {
                tracing::warn!(target: "chumby_host",
                    "exec manifest line without TAB separator ignored: {line:?}");
            }
        }
    }
    // Longest prefix first so the most specific entry wins.
    entries.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
    entries
}

impl ChumbyHost for FixtureHost {
    fn native(&self, _index: u16, name: &str, args: &[HostValue]) -> HostValue {
        match name {
            // (5,202): the hardware config name; we emulate a Chumby Classic.
            "_getPlatform" => HostValue::String("ironforge".into()),
            // (5,205): the two env vars the panel reads at startup.
            "_getEnvironment" => match args.first() {
                Some(HostValue::String(var)) if var == "LANGUAGE" => {
                    HostValue::String("en_US".into())
                }
                Some(HostValue::String(var)) if var == "CONFIGNAME" => {
                    HostValue::String("ironforge".into())
                }
                _ => HostValue::String(String::new()),
            },
            "_dcVolts" => HostValue::Number(12.0),      // (5,16) nominal supply
            "_powerSource" => HostValue::Number(1.0),   // (5,41) 1 = external power
            "_headphonesIn" => HostValue::Number(0.0),  // (5,38) not plugged
            // (5,25): the bend sensor, polled every frame; answered from the
            // simulated bend state (control channel / Home key / long-press).
            "_bent" => HostValue::Number(host::bent() as u8 as f64),
            // (5,177): reads the rootfs file the panel also uses directly.
            "_getTimeZone" => HostValue::String(
                self.fs
                    .get_file("/psp/timezone")
                    .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
                    .unwrap_or_else(|| "UTC".into()),
            ),
            // (5,178): writes the same rootfs file _getTimeZone reads (no
            // trailing newline, like the seeded fixture), so set -> get
            // round-trips as it does on real hardware.
            "_setTimeZone" => {
                if let Some(HostValue::String(tz)) = args.first() {
                    let _ = self.fs.put_file("/psp/timezone", tz.trim().as_bytes());
                }
                HostValue::Undefined
            }
            "_setSlaveVar" => {
                if let [HostValue::String(k), v] = args {
                    let v = match v {
                        HostValue::String(s) => s.clone(),
                        HostValue::Number(n) => n.to_string(),
                        HostValue::Bool(b) => b.to_string(),
                        HostValue::Undefined => String::new(),
                    };
                    self.slave_vars.lock().unwrap().insert(k.clone(), v);
                }
                HostValue::Undefined
            }
            "_getSlaveVar" => match args.first() {
                Some(HostValue::String(k)) => {
                    let vars = self.slave_vars.lock().unwrap();
                    match vars.get(k.as_str()) {
                        Some(v) => HostValue::String(v.clone()),
                        // The localCache decision: pretend the slave widget
                        // finished so the panel never hangs on intro/widgets.
                        None if k == "_chumby_widget_done" => {
                            HostValue::String("true".into())
                        }
                        None => HostValue::Undefined,
                    }
                }
                _ => HostValue::Undefined,
            },
            // --- Audio player (5,130–152), mapped onto mpv (audio.rs) ---
            // (5,140): 1 = the audio backend (mpv) is installed.
            "_isAudioPlayerAvailable" => {
                let available = self.audio.lock().unwrap().bin.is_some();
                HostValue::Number(if available { 1.0 } else { 0.0 })
            }
            // (5,131)
            "_getAudioPlayerState" => {
                let state = self.audio.lock().unwrap().poll_state();
                // SWF constants (frame_2): IDLE:-1 PAUSED:0 WAITING:1 PLAYING:2.
                // TrackedBTPlayer watchdogs on ==2 and stops the player after
                // 5 s otherwise — a wrong mapping kills every stream.
                HostValue::Number(match state {
                    AudioState::Stopped => -1.0,
                    AudioState::Playing => 2.0,
                    AudioState::Paused => 0.0,
                })
            }
            // (5,146): loop count applies to the next _playAudio.
            "_playAudioLoopCount" => {
                if let Some(HostValue::Number(n)) = args.first() {
                    self.audio.lock().unwrap().pending_loops = (*n).max(0.0) as u32;
                }
                HostValue::Undefined
            }
            // (5,144): spawn mpv; absolute chumby paths resolve in the rootfs.
            "_playAudio" => {
                if let Some(HostValue::String(url)) = args.first() {
                    let vol = self.current_volume();
                    self.audio.lock().unwrap().play(url, vol);
                }
                HostValue::Undefined
            }
            // (5,134) / (5,141)
            "_stopAudioPlayer" | "_terminateAudioPlayer" => {
                self.audio.lock().unwrap().stop();
                HostValue::Undefined
            }
            // (5,132)
            "_pauseAudioPlayer" => {
                self.audio.lock().unwrap().pause();
                HostValue::Undefined
            }
            // (5,133)
            "_resumeAudioPlayer" => {
                self.audio.lock().unwrap().resume();
                HostValue::Undefined
            }
            // (5,142) / (5,143): btplay daemon lifecycle — not applicable to
            // our mpv model (mpv is spawned per _playAudio).
            "_startAudioPlayer" | "_restartAudioPlayer" => HostValue::Undefined,

            // --- Volume: intercept _setSystemVolume (5,181) and
            //     _setSystemMute (5,185) for live mpv control and
            //     persistence across restarts. ---
            "_setSystemVolume" => {
                if let Some(HostValue::Number(n)) = args.first() {
                    let vol = (*n).clamp(0.0, 100.0);
                    self.native_state
                        .lock()
                        .unwrap()
                        .insert("SystemVolume", HostValue::Number(vol));
                    let _ = self.fs.put_file("/psp/volume", vol.to_string().as_bytes());
                    if !self.is_muted() {
                        self.audio.lock().unwrap().set_volume(vol);
                    }
                }
                HostValue::Undefined
            }
            "_setSystemMute" => {
                if let Some(HostValue::Number(mute)) = args.first() {
                    self.native_state
                        .lock()
                        .unwrap()
                        .insert("SystemMute", HostValue::Number(*mute));
                    if *mute != 0.0 {
                        self.audio.lock().unwrap().set_volume(0.0);
                    } else {
                        let vol = self.current_volume();
                        self.audio.lock().unwrap().set_volume(vol);
                    }
                }
                HostValue::Undefined
            }

            // Every remaining _setX/_getX pair (volume/balance/mute defaults,
            // LCD brightness, touch click, overlay state, timeouts, ...)
            // shares a name-keyed store: the setter stores its first argument
            // under X, the getter returns it (or a plausible default below).
            name if name.starts_with("_set") => {
                if let Some(v) = args.first() {
                    self.native_state
                        .lock()
                        .unwrap()
                        .insert(leak_state_key(name), v.clone());
                }
                HostValue::Undefined
            }
            name if name.starts_with("_get") => {
                let state = self.native_state.lock().unwrap();
                state
                    .get(state_key(name))
                    .cloned()
                    .unwrap_or_else(|| default_for_getter(name))
            }
            _ => HostValue::Undefined,
        }
    }

    fn exec(&self, command: &str) -> Result<Vec<u8>, HostError> {
        for (prefix, file) in &self.exec_manifest {
            if command.starts_with(prefix.as_str()) {
                return std::fs::read(file)
                    .map(|body| self.expand_tokens(body))
                    .map_err(HostError::Io);
            }
        }
        tracing::warn!(target: "chumby_host",
            "exec fixture MISSING for command: {command:?} (add to {}/exec/manifest.txt)",
            self.root.display());
        Ok(Vec::new())
    }

    fn fetch(&self, url: &str) -> Option<Result<Vec<u8>, HostError>> {
        let stripped = url.strip_prefix("http://").or(url.strip_prefix("https://"))?;
        let (host, path) = stripped.split_once('/').unwrap_or((stripped, ""));
        if !is_chumby_host(host) {
            return None;
        }
        // Strip query string and trailing slash: fixture files are keyed by
        // path only (the panel requests e.g. "/xml/chumbies/?id=...").
        let path = path.split('?').next().unwrap_or("").trim_end_matches('/');
        let file = self.root.join("http").join(host).join(path);
        match std::fs::read(&file) {
            Ok(body) => Some(Ok(self.expand_tokens(body))),
            Err(_) => {
                tracing::warn!(target: "chumby_host",
                    "http fixture MISSING for {url:?} (expected file {})", file.display());
                Some(Err(HostError::NotFound(url.to_owned())))
            }
        }
    }

    fn fs(&self) -> &dyn ChumbyFs {
        &self.fs
    }
}

impl FixtureHost {
    /// Current system volume (0–100). Reads from native_state, which is
    /// seeded from /psp/volume at startup and updated on every
    /// _setSystemVolume call.
    fn current_volume(&self) -> f64 {
        match self.native_state.lock().unwrap().get("SystemVolume").cloned() {
            Some(HostValue::Number(v)) => v,
            _ => 60.0,
        }
    }

    /// True if the system is currently muted (_setSystemMute called with 1).
    fn is_muted(&self) -> bool {
        matches!(
            self.native_state.lock().unwrap().get("SystemMute").cloned(),
            Some(HostValue::Number(v)) if v != 0.0
        )
    }
}

fn is_chumby_host(host: &str) -> bool {
    let host = host.split(':').next().unwrap_or(host);
    host == "chumby.com"
        || host.ends_with(".chumby.com")
        || host == "127.0.0.1"
        || host == "localhost"
}

/// "_setSystemVolume" -> "SystemVolume"
fn state_key(name: &str) -> &str {
    &name[4..]
}

fn leak_state_key(name: &str) -> &'static str {
    // Keys come from the fixed wrapper-name table in avm.rs, which only
    // contains 'static names — leak is bounded by that table's size.
    Box::leak(state_key(name).to_owned().into_boxed_str())
}

/// Defaults for store-backed getters whose setter has not run yet.
/// Every other getter answers Undefined, which the panel tolerates.
fn default_for_getter(name: &str) -> HostValue {
    match name {
        "_getSystemVolume" => HostValue::Number(60.0),  // (5,180) audible
        "_getSystemBalance" => HostValue::Number(0.0),  // (5,182) centered
        // (5,184) / (5,17) / (5,19): nothing muted
        "_getSystemMute" | "_getSpeakerMute" | "_getLCDMute" => HostValue::Number(0.0),
        "_getTouchClick" => HostValue::Number(0.0),     // (5,43) click sound off
        "_getLCDBrightness" => HostValue::Number(65536.0), // (5,21) full
        _ => HostValue::Undefined,
    }
}

/// The virtual rootfs: all panel paths resolve inside `fixtures/rootfs/`.
/// Writes are confined (no `..`, absolute panel paths become relative).
struct RootFs {
    root: PathBuf,
}

impl RootFs {
    fn resolve(&self, path: &str) -> Option<PathBuf> {
        // The panel uses messy multi-slash paths ("//mnt/usb", "////usr/...").
        let clean: Vec<&str> = path
            .split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .collect();
        if clean.iter().any(|c| *c == "..") {
            tracing::warn!(target: "chumby_host", "rejected rootfs path {path:?}");
            return None;
        }
        let mut p = self.root.clone();
        p.extend(&clean);
        Some(p)
    }
}

impl ChumbyFs for RootFs {
    fn get_file(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.resolve(path)?).ok()
    }

    fn put_file(&self, path: &str, data: &[u8]) -> Result<(), HostError> {
        let p = self
            .resolve(path)
            .ok_or_else(|| HostError::NotFound(path.to_owned()))?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(HostError::Io)?;
        }
        std::fs::write(p, data).map_err(HostError::Io)
    }

    fn file_exists(&self, path: &str) -> bool {
        self.resolve(path).is_some_and(|p| p.exists())
    }

    fn file_size(&self, path: &str) -> Option<u64> {
        std::fs::metadata(self.resolve(path)?).ok().map(|m| m.len())
    }

    fn unlink(&self, path: &str) -> Result<(), HostError> {
        let p = self
            .resolve(path)
            .ok_or_else(|| HostError::NotFound(path.to_owned()))?;
        std::fs::remove_file(p).map_err(HostError::Io)
    }

    fn dir_entry(&self, path: &str, index: u32) -> Option<(String, bool)> {
        let dir = self.resolve(path)?;
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        let entry = entries.get(index as usize)?;
        let is_dir = entry.file_type().ok()?.is_dir();
        Some((entry.file_name().to_string_lossy().into_owned(), is_dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// _setTimeZone (5,178) -> _getTimeZone (5,177) must round-trip through
    /// /psp/timezone in the virtual rootfs, as it does on real hardware
    /// (verified against a real Chumby Classic, 2026-07-07).
    #[test]
    fn test_timezone_set_get_round_trip() {
        let root = std::env::temp_dir()
            .join(format!("chumby-fixture-tz-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("rootfs/psp")).unwrap();
        // Seeded like fixtures/rootfs/psp/timezone: no trailing newline.
        std::fs::write(root.join("rootfs/psp/timezone"), "Europe/Oslo").unwrap();

        let host = FixtureHost::new(&root);
        assert_eq!(
            host.native(177, "_getTimeZone", &[]),
            HostValue::String("Europe/Oslo".into())
        );

        host.native(
            178,
            "_setTimeZone",
            &[HostValue::String("America/New_York\n".into())],
        );
        assert_eq!(
            host.native(177, "_getTimeZone", &[]),
            HostValue::String("America/New_York".into())
        );
        assert_eq!(
            std::fs::read_to_string(root.join("rootfs/psp/timezone")).unwrap(),
            "America/New_York"
        );

        std::fs::remove_dir_all(&root).ok();
    }
}
