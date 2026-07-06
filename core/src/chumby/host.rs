//! The `ChumbyHost` trait: what the chumby environment answers.
//!
//! One method per environment-contract category (chumby-pi project,
//! `docs/reference/03-environment-contract.md`). Implementations answer from
//! fixtures (`FixtureHost`, Milestone 2) or, later, the real system.

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

/// Filesystem category: the virtual rootfs behind `_getFile`/`_putFile`/
/// `_fileExists`/`_fileSize`/`_unlink`/`_getDirectoryEntry` and `file://`
/// directory listings.
pub trait ChumbyFs: Send + Sync {
    fn get_file(&self, path: &str) -> Option<Vec<u8>>;
    fn put_file(&self, path: &str, data: &[u8]) -> Result<(), HostError>;
    fn file_exists(&self, path: &str) -> bool;
    fn file_size(&self, path: &str) -> Option<u64>;
    fn unlink(&self, path: &str) -> Result<(), HostError>;
    /// Directory entry by index; `None` = invalid path or index out of range.
    /// Returns (name, is_directory).
    fn dir_entry(&self, path: &str, index: u32) -> Option<(String, bool)>;
}

/// The full host. All errors are normal results — the control panel handles
/// failure on every observed path (gap analysis, 04 runs 1-8).
pub trait ChumbyHost: Send + Sync {
    /// ASnative(5, N) calls that need host state (volume, platform, env,
    /// slave vars, ...). Pure functions never reach this. `name` is the
    /// canonical wrapper name from the environment contract (03 §1).
    fn native(&self, index: u16, name: &str, args: &[HostValue]) -> HostValue;

    /// Shell execution: synchronous `_backtick` (5,52) and the `exec://`
    /// URL scheme. Returns the command's stdout.
    fn exec(&self, command: &str) -> Result<Vec<u8>, HostError>;

    /// URL fetch interception. `None` = not ours, pass through to the real
    /// navigator backend.
    fn fetch(&self, url: &str) -> Option<Result<Vec<u8>, HostError>>;

    /// The virtual rootfs.
    fn fs(&self) -> &dyn ChumbyFs;
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
