//! `FixtureHost`: answers every host category from a fixtures directory.
//!
//! Layout (the tree lives in the chumby-pi project; see `claude-docs/design.md` §2):
//! ```text
//! fixtures/
//!   rootfs/    virtual filesystem for the fs category (paths map 1:1)
//!   exec/      manifest.txt (TAB-separated: prefix<TAB>file) + response files
//!   http/      <host>/<path> response files (fetch category)
//! ```
//! Fixture keys are the panel's own request strings — a logged unmatched
//! call names the fixture file to create.

use super::audio::{AudioPlayer, AudioState};
use super::backup_alarm::BackupAlarm;
use super::host::{self, ChumbyFs, ChumbyHost, DirEntry, DirEntryResult, HostError, HostValue};
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
    /// Watches /psp/ifalarm and sounds the dead-man tone (chumbalarmd's job).
    backup_alarm: BackupAlarm,
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
            audio: Mutex::new(AudioPlayer::new(rootfs_path.clone())),
            backup_alarm: BackupAlarm::start(rootfs_path),
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
        // Backup-alarm protocol (AlarmSet, F2:11952): these two commands have
        // real semantics, not fixtures. Dismissal must actually delete
        // /psp/ifalarm or the dead-man tone would fire after every answered
        // alarm. The bare reload is a no-op: the watcher polls the file.
        if command.starts_with("rm /psp/ifalarm") {
            let _ = std::fs::remove_file(self.fs.root.join("psp/ifalarm"));
            self.backup_alarm.dismiss();
            return Ok(Vec::new());
        }
        if command == "reload_backup_alarm" {
            return Ok(Vec::new());
        }
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

    fn dir_entry(&self, path: &str, index: u32) -> DirEntryResult {
        let Some(dir) = self.resolve(path) else {
            return DirEntryResult::InvalidPath;
        };
        let Ok(rd) = std::fs::read_dir(dir) else {
            return DirEntryResult::InvalidPath;
        };
        // Sorted by name: the panel iterates by ascending index across
        // frames, so the order must be stable or entries skip/duplicate.
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        let Some(entry) = entries.get(index as usize) else {
            return DirEntryResult::End;
        };
        let p = entry.path();
        // metadata() follows symlinks (a symlinked dir is a dir);
        // symlink_metadata() marks it a link — _isDirLink is the panel's
        // recursion loop protection. A dangling link reports neither dir
        // nor file and the panel skips it.
        let (is_dir, is_file) = std::fs::metadata(&p)
            .map(|m| (m.is_dir(), m.is_file()))
            .unwrap_or((false, false));
        let is_link = std::fs::symlink_metadata(&p)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        DirEntryResult::Entry(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir,
            is_dir_link: is_dir && is_link,
            is_file,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `_getDirectoryEntry` (5,320) backend. The panel iterates ascending
    /// indices resumably across frames (FileFinderPOSIX), so ordering must
    /// be stable; it distinguishes dir symlinks (`_isDirLink`) for loop
    /// protection; and it treats -1/0 as invalid path / end of listing.
    #[test]
    fn test_dir_entry_listing_flags_and_status_codes() {
        let root = std::env::temp_dir()
            .join(format!("chumby-fixture-dirent-test-{}", std::process::id()));
        let usb = root.join("mnt/usb");
        std::fs::create_dir_all(usb.join("Album")).unwrap();
        std::fs::write(usb.join("Album/track.mp3"), b"x").unwrap();
        std::fs::write(usb.join("a.mp3"), b"x").unwrap();
        std::fs::write(usb.join("b.ogg"), b"x").unwrap();
        std::os::unix::fs::symlink(usb.join("Album"), usb.join("loop")).unwrap();

        let fs = RootFs { root: root.clone() };

        // Sorted by name (byte order): Album, a.mp3, b.ogg, loop —
        // and the messy panel path form resolves.
        let names: Vec<String> = (0..)
            .map_while(|i| match fs.dir_entry("//mnt/usb/", i) {
                DirEntryResult::Entry(e) => Some(e.name),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["Album", "a.mp3", "b.ogg", "loop"]);

        let DirEntryResult::Entry(album) = fs.dir_entry("/mnt/usb", 0) else {
            panic!("expected entry")
        };
        assert!(album.is_dir && !album.is_dir_link && !album.is_file);
        let DirEntryResult::Entry(file) = fs.dir_entry("/mnt/usb", 1) else {
            panic!("expected entry")
        };
        assert!(file.is_file && !file.is_dir);
        let DirEntryResult::Entry(link) = fs.dir_entry("/mnt/usb", 3) else {
            panic!("expected entry")
        };
        assert!(link.is_dir && link.is_dir_link, "symlinked dir must set both flags");

        // Subdirectory listing works; end / bad path report the panel codes.
        assert!(matches!(fs.dir_entry("/mnt/usb/Album", 0), DirEntryResult::Entry(_)));
        assert_eq!(fs.dir_entry("/mnt/usb", 4), DirEntryResult::End);
        assert_eq!(fs.dir_entry("/mnt/nosuch", 0), DirEntryResult::InvalidPath);
        assert_eq!(fs.dir_entry("/mnt/usb/a.mp3", 0), DirEntryResult::InvalidPath);

        std::fs::remove_dir_all(&root).ok();
    }

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

    /// The panel's alarm dismissal (`rm /psp/ifalarm; reload_backup_alarm`,
    /// AlarmSet F2:11986) must really delete the file — a canned fixture here
    /// would leave the dead-man tone armed after every answered alarm.
    #[test]
    fn test_backup_alarm_dismissal_deletes_ifalarm() {
        let root = std::env::temp_dir()
            .join(format!("chumby-fixture-ifalarm-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("rootfs/psp")).unwrap();
        let ifalarm = root.join("rootfs/psp/ifalarm");
        // Far future so the watcher thread cannot fire during the test.
        std::fs::write(&ifalarm, "4102444800").unwrap();

        let host = FixtureHost::new(&root);
        host.exec("reload_backup_alarm").unwrap();
        assert!(ifalarm.exists(), "bare reload must not touch the file");
        host.exec("rm /psp/ifalarm; reload_backup_alarm").unwrap();
        assert!(!ifalarm.exists(), "dismissal must delete /psp/ifalarm");

        std::fs::remove_dir_all(&root).ok();
    }
}
