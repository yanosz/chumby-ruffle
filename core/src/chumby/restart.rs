//! Restart on request, when the panel is idle.
//!
//! The appliance's supervisor (chumby-pi `claude/watchdog-plan.md`, D4)
//! asks for a panel restart over the control FIFO (`restart-when-idle`,
//! withdrawn by `restart-cancel`). The decision to yield is taken here, in
//! the process that holds the state, so an alarm cannot start ringing
//! between the check and the kill. The player quits once
//! - no audio alarm is ringing or snoozing — a snooze lives only in panel
//!   memory, and a restarted panel does not ring an alarm again; and
//! - the screen has not been pressed for [`TAP_QUIET`].
//!
//! The audio layer cannot tell an alarm from music (`AudioPlayer::play`
//! takes a URL and a volume), so the panel's own flags are read:
//! `AlarmSet.alarmSet._alarms[i]` → `_type`, `_alarmRinging`,
//! `_alarmSnoozing` (F2:11779, 11230-11236). Silent `type="none"` alarms
//! never hold a restart.
//!
//! [`apply`] must run where no panel function is half-way through — the
//! caller (`avm.rs`) uses the per-frame `_bent` poll.
//!
//! Quitting takes the panel's own `fscommand("quit")` route, so the process
//! exits 0; the supervisor knows it asked.

use crate::avm1::{Activation, Object, Value};
use ruffle_common::avm_string::AvmString;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long after the last press the screen counts as in use.
pub const TAP_QUIET: Duration = Duration::from_secs(60);

static REQUESTED: AtomicBool = AtomicBool::new(false);
static LAST_TAP: Mutex<Option<Instant>> = Mutex::new(None);
/// Last logged reason for holding a request, so the log shows changes
/// only (the check runs every frame).
static HELD_BY: AtomicU8 = AtomicU8::new(Hold::None as u8);

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Hold {
    None,
    Alarm,
    Tap,
}

pub fn request() {
    REQUESTED.store(true, Ordering::Relaxed);
}

pub fn cancel() {
    REQUESTED.store(false, Ordering::Relaxed);
    HELD_BY.store(Hold::None as u8, Ordering::Relaxed);
}

pub fn requested() -> bool {
    REQUESTED.load(Ordering::Relaxed)
}

/// Called while the left button is down (touch maps to it).
pub fn stamp_tap() {
    if let Ok(mut t) = LAST_TAP.lock() {
        *t = Some(Instant::now());
    }
}

fn tapped_recently(now: Instant) -> bool {
    match LAST_TAP.lock() {
        Ok(t) => t.is_some_and(|at| now.duration_since(at) < TAP_QUIET),
        Err(_) => false,
    }
}

pub fn apply(activation: &mut Activation<'_, '_>) {
    if !requested() {
        return;
    }
    let hold = if audio_alarm_active(activation) {
        Hold::Alarm
    } else if tapped_recently(Instant::now()) {
        Hold::Tap
    } else {
        Hold::None
    };
    if HELD_BY.swap(hold as u8, Ordering::Relaxed) != hold as u8 {
        match hold {
            Hold::Alarm => tracing::info!(target: "chumby_host",
                "restart held: an audio alarm is ringing or snoozing"),
            Hold::Tap => tracing::info!(target: "chumby_host",
                "restart held: screen pressed less than {} s ago", TAP_QUIET.as_secs()),
            Hold::None => {}
        }
    }
    if hold != Hold::None {
        return;
    }
    cancel();
    tracing::info!(target: "chumby_host", "panel idle — quitting for the requested restart");
    activation
        .context
        .external_interface
        .invoke_fs_command("quit", "");
}

fn s<'gc>(activation: &mut Activation<'_, 'gc>, text: &str) -> AvmString<'gc> {
    AvmString::new_utf8(activation.gc(), text)
}

fn get_object<'gc>(
    activation: &mut Activation<'_, 'gc>,
    from: Object<'gc>,
    name: &str,
) -> Option<Object<'gc>> {
    let key = s(activation, name);
    match from.get(key, activation) {
        Ok(Value::Object(o)) => Some(o),
        _ => None,
    }
}

fn get_bool<'gc>(activation: &mut Activation<'_, 'gc>, from: Object<'gc>, name: &str) -> bool {
    let key = s(activation, name);
    from.get(key, activation)
        .map(|v| v.as_bool(activation.swf_version()))
        .unwrap_or(false)
}

/// Before frame 2 defines `AlarmSet` there are no alarms to protect.
fn audio_alarm_active(activation: &mut Activation<'_, '_>) -> bool {
    let global = activation.global_object();
    let Some(alarms) = get_object(activation, global, "AlarmSet")
        .and_then(|set| get_object(activation, set, "alarmSet"))
        .and_then(|set| get_object(activation, set, "_alarms"))
    else {
        return false;
    };
    let len = alarms.length(activation).unwrap_or(0);
    (0..len).any(|i| {
        let Value::Object(alarm) = alarms.get_element(activation, i) else {
            return false;
        };
        let key = s(activation, "_type");
        let silent = matches!(alarm.get(key, activation),
            Ok(Value::String(t)) if t.to_utf8_lossy() == "none");
        !silent
            && (get_bool(activation, alarm, "_alarmRinging")
                || get_bool(activation, alarm, "_alarmSnoozing"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tap_quiet_window() {
        let now = Instant::now();
        *LAST_TAP.lock().unwrap() = None;
        assert!(!tapped_recently(now));
        *LAST_TAP.lock().unwrap() = Some(now);
        assert!(tapped_recently(now + Duration::from_secs(59)));
        assert!(!tapped_recently(now + TAP_QUIET));
    }

    #[test]
    fn test_request_and_cancel() {
        request();
        assert!(requested());
        cancel();
        assert!(!requested());
    }
}
