//! Chumby control panel host environment.
//!
//! Everything chumby-specific lives in this module, compiled only with the
//! `chumby` cargo feature. Upstream Ruffle files receive only minimal
//! registration hooks; each hook is documented in the chumby-pi project's
//! `docs/patch-notes.md`. Design: `docs/design/chumby-host.md` there.
//!
//! Submodules:
//! - `host`: the `ChumbyHost` trait (native / exec / fetch / fs categories)
//! - `fixture`: `FixtureHost`, answering from a fixtures directory
//! - `avm`: the `ASnative(5, N)` function table
//! - `navigator` (planned): `NavigatorBackend` decorator intercepting
//!   `exec://`, chumby HTTP endpoints, and `file://` directory listings
//! - `input`: simulated-input control channel (stdin + FIFO line commands)

pub mod audio;
pub mod avm;
pub mod fixture;
pub mod host;
pub mod input;
pub mod navigator;

pub use fixture::FixtureHost;
pub use host::{set_bent, set_host, take_pointer, ChumbyHost, PointerAction};
