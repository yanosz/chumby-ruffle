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
        // Opening a FIFO read-only blocks until a writer connects and
        // returns EOF when it closes — reopen for the next writer.
        match std::fs::File::open(path) {
            Ok(f) => {
                match f.metadata() {
                    // A regular file would replay its commands forever.
                    Ok(m) if !m.file_type().is_fifo() => {
                        tracing::warn!(target: "chumby_host",
                            "control path {path:?} is not a FIFO (mkfifo it); channel disabled");
                        return;
                    }
                    _ => {}
                }
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
        other => {
            if !pointer_command(other) {
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
