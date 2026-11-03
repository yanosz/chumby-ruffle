//! Chumby control panel host environment.
//!
//! Everything chumby-specific lives in this module, always compiled in
//! this fork. Upstream Ruffle files receive only minimal registration
//! hooks, each marked with the word `chumby`; `claude/patch-surface.md`
//! tracks them against upstream drift.
//!
//! The host boundary:
//! - `host`: the `ChumbyHost` trait (native / exec / fetch / fs categories)
//! - `fixture`: `FixtureHost`, answering from a fixtures directory
//! - `real_net`: wraps it, answering the network touchpoints from live
//!   kernel state; `real_ident`: device GUID and serial from the machine
//! - `config`: owner-level knobs from `<fixtures>/player.toml`, read once
//! - `avm`: the `ASnative(5, N)` function table (per-index docs: README)
//! - `navigator`: `NavigatorBackend` decorator intercepting `exec://` and
//!   chumby HTTP endpoints
//!
//! Device behaviour behind it:
//! - `audio`: mpv-backed player behind the audio native family
//! - `backup_alarm`: the dead-man beep chumbalarmd provided on real hardware
//! - `brightness`: real backlight behind the panel's brightness writes
//! - `input`: simulated-input control channel (stdin + FIFO line commands)
//! - `restart`: restart-when-idle requests from the appliance supervisor
//!
//! Surgery on the panel's own AVM1 objects, applied from `avm::method`:
//! - `ui_policy`: dim/disable controls the host cannot honour
//!   (rules compiled in from `ui-policy.toml`)
//! - `music_sources`: hide music sources the appliance cannot serve
//! - `intro`: the INTRO button under localCache
//! - `empty_channel`: an empty channel shows the built-in clock
//! - `alarm_guard`: a silent alarm must not cancel a sounding one
//! - `extended_events`: the soft events (`ExtendedEvents`) the chumby's
//!   player raised, fed from the control channel

pub mod alarm_guard;
pub mod audio;
pub mod avm;
pub mod backup_alarm;
pub mod brightness;
pub mod config;
pub mod empty_channel;
pub mod extended_events;
pub mod fixture;
pub mod host;
pub mod input;
pub mod intro;
pub mod music_sources;
pub mod navigator;
pub mod real_ident;
pub mod real_net;
pub mod restart;
pub mod ui_policy;

pub use fixture::FixtureHost;
pub use host::{set_bent, set_host, take_pointer, ChumbyHost, PointerAction};
pub use real_net::RealNetHost;
