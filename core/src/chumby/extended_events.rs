//! The panel's soft events: the `ExtendedEvents` class of the chumby's
//! Flash player.
//!
//! On the chumby a service appends `<event type= value= comment=/>` to
//! `/tmp/flashplayer.event` and SIGHUPs the player
//! (`/usr/chumby/scripts/signal_soft_event.sh`); the panel subscribes with
//! `ExtendedEvents.<KIND>.addListener(obj)` and gets `obj.onEvent(value,
//! comment)`. Here the events arrive as `event KIND VALUE [COMMENT]` on the
//! control channel (input.rs) and wait in a host queue (`host::push_event`).
//!
//! Each kind is an `AsBroadcaster` object (`addListener`, `removeListener`,
//! `_listeners`), so the dispatch is the kind's `broadcastMessage("onEvent",
//! value, comment)`.
//!
//! [`install`] runs on the first native call: frame 2 calls `_bent` at
//! F2:3395, before its first `addListener` (`ScreenManager.setupEvents()`,
//! F2:9183). [`dispatch`] runs at the per-frame `_bent` poll (avm.rs), the
//! first statement of `BendSensor.onEnterFrame` (F2:3423), where no panel
//! function is half-way through — the place restart.rs chose for the same
//! reason.

use super::host;
use crate::avm1::{Activation, ExecutionReason, Object, Value};
use ruffle_common::avm_string::AvmString;
use std::sync::atomic::{AtomicBool, Ordering};

/// The kinds the panel subscribes to: F2:5404, 5412, 9160, 12192, 13057,
/// 14436, 16216, 21919; F6:124.
const KINDS: [&str; 9] = [
    "WidgetPlayer",
    "chumbrowser",
    "ScreenManager",
    "AlarmPlayer",
    "MusicPlayer",
    "FMRadio",
    "UserPlayer",
    "IntercomPlayer",
    "NightMode",
];

static INSTALLED: AtomicBool = AtomicBool::new(false);

fn s<'gc>(activation: &mut Activation<'_, 'gc>, text: &str) -> AvmString<'gc> {
    AvmString::new_utf8(activation.gc(), text)
}

pub fn install(activation: &mut Activation<'_, '_>) {
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    let object_proto = activation.prototypes().object;
    let array_proto = activation.prototypes().array;
    let broadcaster = activation
        .context
        .avm1
        .broadcaster_functions(activation.swf_version());
    let events = Object::new(activation.strings(), Some(object_proto));
    for kind in KINDS {
        let channel = Object::new(activation.strings(), Some(object_proto));
        broadcaster.initialize(activation.strings(), channel, array_proto);
        let key = s(activation, kind);
        let _ = events.set(key, channel, activation);
    }
    let key = s(activation, "ExtendedEvents");
    let _ = activation.global_object().set(key, events, activation);
    tracing::info!(target: "chumby_host", "ExtendedEvents installed: {}", KINDS.join(", "));
}

/// Hand every queued event to its kind's listeners, oldest first.
pub fn dispatch(activation: &mut Activation<'_, '_>) {
    let key = s(activation, "ExtendedEvents");
    // A movie with another case-sensitivity has its own global object;
    // leave the queue for the panel's next poll.
    let Ok(Value::Object(events)) = activation.global_object().get(key, activation) else {
        return;
    };
    for event in host::take_events() {
        let key = s(activation, &event.kind);
        let Ok(Value::Object(channel)) = events.get(key, activation) else {
            tracing::warn!(target: "chumby_host",
                "soft event {} {}: unknown kind, dropped", event.kind, event.value);
            continue;
        };
        let key = s(activation, "_listeners");
        let listeners = match channel.get(key, activation) {
            Ok(Value::Object(list)) => list.length(activation).unwrap_or(0),
            _ => 0,
        };
        tracing::info!(target: "chumby_host",
            "soft event {} {} {:?} -> {listeners} listener(s)",
            event.kind, event.value, event.comment);
        let args = [
            s(activation, "onEvent").into(),
            s(activation, &event.value).into(),
            s(activation, &event.comment).into(),
        ];
        let name = s(activation, "broadcastMessage");
        let reason = ExecutionReason::FunctionCall;
        if let Err(e) = channel.call_method(name, &args, activation, reason) {
            tracing::warn!(target: "chumby_host",
                "soft event {} {}: listener failed: {e:?}", event.kind, event.value);
        }
    }
}
