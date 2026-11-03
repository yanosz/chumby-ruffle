//! Simulated-input control channel.
//!
//! Neither the dev machine nor the Pi has chumby hardware inputs (the bend
//! sensor first of all), so the host accepts line commands from ruffle's own
//! stdin and, optionally, a FIFO (`--chumby-control PATH`) that any shell
//! can write to: `echo bend > PATH`. One command per line:
//!
//! ```text
//! bend | tap  tap: pressed for one poll, then released (summons B2 / snoozes)
//! bend down   press and hold
//! bend up     release
//! click X Y   left-click at window coordinates (screenshot pixels)
//! drag X1 Y1 X2 Y2   press at 1, glide to 2, release (sliders)
//! restart-when-idle  quit once no audio alarm rings/snoozes and the screen
//!             has been untouched for 60 s (restart.rs; the supervisor's)
//! restart-cancel     withdraw that request
//! event KIND VALUE [COMMENT]  a soft event: the panel's
//!             ExtendedEvents.KIND listeners get onEvent(VALUE, COMMENT);
//!             COMMENT is the rest of the line, spaces included, "" if
//!             absent (extended_events.rs), e.g. `event WidgetPlayer
//!             setWidget Yoga Basics`, `event MusicPlayer setVolume 30`
//! ```
//!
//! Unknown commands are logged and ignored, so the protocol can grow with
//! future simulated inputs (headphones, power source, ...) without breaking
//! older scripts.

use super::host;
use std::io::BufRead;
use std::path::PathBuf;

/// Spawn the reader threads. Called once at startup from desktop `main`.
pub fn spawn(fifo: Option<PathBuf>) {
    let _ = std::thread::Builder::new()
        .name("chumby-stdin".into())
        .spawn(|| {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                handle(line.trim());
            }
            // EOF: launched detached, or the terminal closed. Not an error.
            tracing::info!(target: "chumby_host", "stdin control channel closed");
        });

    #[cfg(unix)]
    if let Some(path) = fifo {
        let _ = std::thread::Builder::new()
            .name("chumby-fifo".into())
            .spawn(move || fifo_loop(&path));
    }
    #[cfg(not(unix))]
    if fifo.is_some() {
        tracing::warn!(target: "chumby_host", "--chumby-control needs a unix FIFO; ignored");
    }
}

#[cfg(unix)]
fn fifo_loop(path: &std::path::Path) {
    use std::os::unix::fs::FileTypeExt;
    loop {
        // A regular file would replay its commands forever.
        if let Ok(m) = std::fs::metadata(path) {
            if !m.file_type().is_fifo() {
                tracing::warn!(target: "chumby_host",
                    "control path {path:?} is not a FIFO (mkfifo it); channel disabled");
                return;
            }
        }
        // Read-write: the open does not wait for a writer, and the reader
        // never sees EOF (it counts as a writer itself). A read-only reader
        // reopening after each writer loses whatever is written between its
        // EOF and the reopen.
        match std::fs::OpenOptions::new().read(true).write(true).open(path) {
            Ok(f) => {
                for line in std::io::BufReader::new(f).lines() {
                    let Ok(line) = line else { break };
                    handle(line.trim());
                }
            }
            Err(e) => {
                tracing::warn!(target: "chumby_host",
                    "control FIFO {path:?}: {e}; retrying in 5s");
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    }
}

fn handle(command: &str) {
    match command {
        "" => {}
        "bend" | "tap" => host::tap_bend(),
        "bend down" => host::set_bent(true),
        "bend up" => host::set_bent(false),
        "restart-when-idle" => super::restart::request(),
        "restart-cancel" => super::restart::cancel(),
        other => {
            if !pointer_command(other) && !event_command(other) {
                tracing::warn!(target: "chumby_host", "unknown control command {other:?}");
                return;
            }
        }
    }
    tracing::info!(target: "chumby_host", "control command: {command}");
}

/// Parse `click X Y` / `drag X1 Y1 X2 Y2` and queue the pointer actions.
/// Returns false if the command is neither (or has malformed numbers).
fn pointer_command(command: &str) -> bool {
    use host::PointerAction::{Down, Move, Up};
    let mut it = command.split_whitespace();
    let verb = it.next().unwrap_or("");
    let args: Option<Vec<f64>> = it.map(|w| w.parse().ok()).collect();
    let Some(args) = args else { return false };
    match (verb, args.as_slice()) {
        ("click", &[x, y]) => {
            host::push_pointer(Move(x, y));
            host::push_pointer(Down(x, y));
            host::push_pointer(Up(x, y));
        }
        ("drag", &[x1, y1, x2, y2]) => {
            host::push_pointer(Move(x1, y1));
            host::push_pointer(Down(x1, y1));
            // Glide in steps so drag-tracking widgets (sliders) see the
            // knob follow the pointer; one action is applied per frame.
            const STEPS: u32 = 8;
            for i in 1..=STEPS {
                let t = f64::from(i) / f64::from(STEPS);
                host::push_pointer(Move(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t));
            }
            host::push_pointer(Up(x2, y2));
        }
        _ => return false,
    }
    true
}

/// Parse `event KIND VALUE [COMMENT]` and queue the soft event.
fn event_command(command: &str) -> bool {
    match parse_event(command) {
        Some(event) => {
            host::push_event(event);
            true
        }
        None => false,
    }
}

fn parse_event(command: &str) -> Option<host::SoftEvent> {
    fn word(text: &str) -> (&str, &str) {
        match text.split_once(char::is_whitespace) {
            Some((w, rest)) => (w, rest.trim_start()),
            None => (text, ""),
        }
    }
    let (verb, rest) = word(command);
    let (kind, rest) = word(rest);
    let (value, comment) = word(rest);
    if verb != "event" || kind.is_empty() || value.is_empty() {
        return None;
    }
    Some(host::SoftEvent {
        kind: kind.to_owned(),
        value: value.to_owned(),
        comment: comment.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, value: &str, comment: &str) -> Option<host::SoftEvent> {
        Some(host::SoftEvent {
            kind: kind.into(),
            value: value.into(),
            comment: comment.into(),
        })
    }

    #[test]
    fn test_parse_event() {
        assert_eq!(
            parse_event("event WidgetPlayer nextWidget"),
            ev("WidgetPlayer", "nextWidget", "")
        );
        assert_eq!(
            parse_event("event WidgetPlayer setWidget Yoga  Basics"),
            ev("WidgetPlayer", "setWidget", "Yoga  Basics")
        );
        assert_eq!(
            parse_event("event MusicPlayer setVolume 30"),
            ev("MusicPlayer", "setVolume", "30")
        );
        assert_eq!(parse_event("event NightMode"), None);
        assert_eq!(parse_event("event"), None);
        assert_eq!(parse_event("eventful WidgetPlayer nextWidget"), None);
    }
}
