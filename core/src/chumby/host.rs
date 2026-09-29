//! The `ChumbyHost` trait: what the chumby environment answers.
//!
//! One method per category of environment traffic (ASnative state, shell,
//! HTTP, filesystem — see the README). Implementations answer from
//! fixtures (`FixtureHost`) or, later, the real system.

use super::config::PlayerConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Value passed between the AVM table and a host. Deliberately tiny —
/// chumby natives only traffic in strings, numbers and booleans.
#[derive(Debug, Clone, PartialEq)]
pub enum HostValue {
    Undefined,
    Bool(bool),
    Number(f64),
    String(String),
}

#[derive(Debug)]
pub enum HostError {
    NotFound(String),
    Io(std::io::Error),
}

/// One directory entry as `_getDirectoryEntry` (5,320) reports it. The
/// panel reads `_name`, `_path`, `_isDir`, `_isDirLink`, `_isFile` off the
/// filled object and does its own filtering: dotfiles, directory symlinks
/// (`is_dir_link` — its loop protection), `usb-*` dirs, music extensions.
#[derive(Debug, Clone, PartialEq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_dir_link: bool,
    pub is_file: bool,
}

/// The panel's `DIRECTORY_ENTRY_*` status codes (frame 2 `ChumbyNative`).
#[derive(Debug, Clone, PartialEq)]
pub enum DirEntryResult {
    /// DIRECTORY_ENTRY_SUCCESS (1)
    Entry(DirEntry),
    /// DIRECTORY_ENTRY_INVALID_INDEX (0) — clean end of listing
    End,
    /// DIRECTORY_ENTRY_INVALID_PATH (-1)
    InvalidPath,
}

/// Filesystem category: the virtual rootfs behind the filesystem natives
/// `_getFile` (5,50), `_putFile` (5,51), `_fileExists` (5,53),
/// `_fileSize` (5,54) and `_unlink` (5,55).
pub trait ChumbyFs: Send + Sync {
    fn get_file(&self, path: &str) -> Option<Vec<u8>>;
    fn put_file(&self, path: &str, data: &[u8]) -> Result<(), HostError>;
    fn file_exists(&self, path: &str) -> bool;
    fn file_size(&self, path: &str) -> Option<u64>;
    fn unlink(&self, path: &str) -> Result<(), HostError>;
    /// Directory entry by index, for `_getDirectoryEntry` (5,320). The
    /// panel iterates ascending indices, resumable across frames
    /// (`FileFinderPOSIX` re-posts 200 entries per frame), so the listing
    /// order must be stable call-to-call.
    fn dir_entry(&self, path: &str, index: u32) -> DirEntryResult;
}

/// The full host. All errors are normal results — the control panel handles
/// failure on every observed path.
pub trait ChumbyHost: Send + Sync {
    /// ASnative(5, N) calls that need host state (volume, platform, env,
    /// slave vars, ...). Pure functions never reach this. `name` is the
    /// canonical wrapper name from `avm::wrapper_name`.
    fn native(&self, index: u16, name: &str, args: &[HostValue]) -> HostValue;

    /// Shell execution: synchronous `_backtick` (5,52) and the `exec://`
    /// URL scheme. Returns the command's stdout.
    fn exec(&self, command: &str) -> Result<Vec<u8>, HostError>;

    /// URL fetch interception. `None` = not ours, pass through to the real
    /// navigator backend.
    fn fetch(&self, url: &str) -> Option<Result<Vec<u8>, HostError>>;

    /// The virtual rootfs.
    fn fs(&self) -> &dyn ChumbyFs;

    /// Owner-level knobs from `<fixtures>/player.toml` (config.rs).
    fn config(&self) -> &PlayerConfig;

    /// True when a brightness backend exists — a kernel backlight or a
    /// configured `brightness_ctl` (brightness.rs). Lifts the
    /// `settings-brightness` ui-policy rule.
    fn brightness_available(&self) -> bool;
}

/// Process-global host registry.
///
/// Rationale: reaching the host from AVM1 native functions would otherwise
/// require threading a field through `UpdateContext`/`PlayerBuilder` and
/// thus more upstream hooks. The desktop frontend runs one player per
/// process; a `OnceLock` keeps the upstream patch surface at two lines.
/// Revisit if Ruffle's multi-player embedding ever matters for us.
static HOST: OnceLock<Arc<dyn ChumbyHost>> = OnceLock::new();

pub fn set_host(host: Arc<dyn ChumbyHost>) {
    if HOST.set(host).is_err() {
        tracing::warn!(target: "chumby_host", "set_host called twice; keeping first host");
    }
}

pub fn host() -> Option<&'static Arc<dyn ChumbyHost>> {
    HOST.get()
}

/// Bend-sensor state (the squeezable top button on the Chumby Classic).
///
/// The control panel polls `ASnative(5,25) _bent` every frame
/// (`BendSensor.onEnterFrame`, frame_2) and fires onBend/onUnbend on edge
/// changes; release in widget mode summons the main button bar (screen B2).
/// The frontend sets this from whatever physical input plays the bend role
/// (desktop: a key; Pi later: GPIO). Global for the same reason as `HOST`.
static BENT: AtomicBool = AtomicBool::new(false);
static BEND_TAP: AtomicBool = AtomicBool::new(false);

pub fn set_bent(bent: bool) {
    BENT.store(bent, Ordering::Relaxed);
}

/// Queue a bend tap: the next poll reads pressed, the one after released.
/// The panel polls every frame, so this yields one onBend/onUnbend pair —
/// enough for everything the panel does with the bend (B2 summon, snooze);
/// hold duration only feeds the dormant BendTapper and the tilt gesture.
pub fn tap_bend() {
    BEND_TAP.store(true, Ordering::Relaxed);
}

pub fn bent() -> bool {
    BEND_TAP.swap(false, Ordering::Relaxed) || BENT.load(Ordering::Relaxed)
}

/// Simulated pointer input (`click X Y` / `drag X1 Y1 X2 Y2` on the control
/// channel).
///
/// The Pi has no pointer device until the real touchscreen is wired, and
/// virtual-pointer injection from outside the compositor doesn't reach a
/// headless cage — so pointer input travels the same path as the bend:
/// pushed here by the control channel, drained by the frontend event loop
/// **one action per iteration** (sliders and other press-tracking widgets
/// need the down/move/up sequence spread over player ticks). Coordinates
/// are window pixels. Global for the same reason as `HOST`.
pub enum PointerAction {
    Move(f64, f64),
    Down(f64, f64),
    Up(f64, f64),
}

static POINTER: Mutex<Vec<PointerAction>> = Mutex::new(Vec::new());

pub fn push_pointer(action: PointerAction) {
    if let Ok(mut q) = POINTER.lock() {
        q.push(action);
    }
}

pub fn take_pointer() -> Option<PointerAction> {
    match POINTER.lock() {
        Ok(mut q) if !q.is_empty() => Some(q.remove(0)),
        _ => None,
    }
}
