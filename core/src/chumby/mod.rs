//! Chumby control panel host environment.
//!
//! Everything chumby-specific lives in this module, always compiled in
//! this fork. Upstream Ruffle files receive only minimal registration
//! hooks; the README lists them, and `claude-docs/design.md` §8 tracks them
//! against upstream drift.
//!
//! Submodules:
//! - `host`: the `ChumbyHost` trait (native / exec / fetch / fs categories)
//! - `fixture`: `FixtureHost`, answering from a fixtures directory
//! - `config`: owner-level knobs from `<fixtures>/player.toml`, read once
//!   at start (volume cap, the future chumby.com switch)
//! - `avm`: the `ASnative(5, N)` function table (per-index docs: README)
//! - `navigator`: `NavigatorBackend` decorator intercepting `exec://` and
//!   chumby HTTP endpoints
//! - `audio`: mpv-backed player behind the audio native family
//! - `backup_alarm`: the dead-man beep chumbalarmd provided on real hardware
//! - `brightness`: real backlight behind the panel's brightness writes
//!   (kernel backlight sysfs, or the owner's `brightness_ctl` executable)
//! - `input`: simulated-input control channel (stdin + FIFO line commands)
//! - `music_sources`: VM-level removal of music sources the appliance
//!   cannot serve from `MusicPlayer.musicSources`
//! - `ui_policy`: declarative dim/disable of panel controls the host
//!   platform does not support (rules compiled in from `ui-policy.toml`)

pub mod alarm_guard;
pub mod audio;
pub mod avm;
pub mod backup_alarm;
pub mod brightness;
pub mod config;
pub mod dash_theme;
pub mod dash_widget;
pub mod empty_channel;
pub mod fixture;
pub mod host;
pub mod input;
pub mod intro;
pub mod music_sources;
pub mod navigator;
pub mod real_ident;
pub mod real_net;
pub mod tzdump;
pub mod ui_policy;

pub use fixture::FixtureHost;
pub use host::{set_bent, set_host, take_pointer, ChumbyHost, PointerAction};
pub use real_net::RealNetHost;
