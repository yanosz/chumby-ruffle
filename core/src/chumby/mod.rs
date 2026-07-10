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
//! - `avm`: the `ASnative(5, N)` function table (per-index docs: README)
//! - `navigator`: `NavigatorBackend` decorator intercepting `exec://` and
//!   chumby HTTP endpoints
//! - `audio`: mpv-backed player behind the audio native family
//! - `backup_alarm`: the dead-man beep chumbalarmd provided on real hardware
//! - `input`: simulated-input control channel (stdin + FIFO line commands)
//! - `ui_policy`: declarative dim/disable of panel controls the host
//!   platform does not support (rules compiled in from `ui-policy.toml`)

pub mod audio;
pub mod avm;
pub mod backup_alarm;
pub mod fixture;
pub mod host;
pub mod input;
pub mod navigator;
pub mod real_ident;
pub mod real_net;
pub mod ui_policy;

pub use fixture::FixtureHost;
pub use host::{set_bent, set_host, take_pointer, ChumbyHost, PointerAction};
pub use real_net::RealNetHost;
