//! A silent alarm must not cancel a sounding one.
//!
//! `Alarm.ringAlarm` (F2:11178) opens with `_alarmSet.stopAlarmsExcept(this)`,
//! and `stopAlarmsExcept` (F2:12039) calls `stopAlarm(true)` on every *other*
//! ringing alarm. A `type="none"` alarm — the silent kind whose only job is a
//! side effect such as leaving night mode (F2:11187-11191) — runs that cancel
//! too, so a nightmode alarm at 08:00 kills a stream alarm still ringing from
//! 07:59 (measured on the device, chumby-pi issue 13, 2026-09-01). The guard
//! exists to keep two *sounding* alarms from overlapping; an alarm that makes
//! no sound has nothing to protect.
//!
//! So the prototype method is wrapped (surgery precedent: empty_channel.rs):
//! a silent canceller cancels nobody, every other call takes the original
//! path unchanged. This is a deliberate deviation from stock chumby
//! behaviour — requirements.md records it.
//!
//! Wrapped here rather than at the `ringAlarm` call site because the ringing
//! alarm passes itself as `anAlarm`, so canceller and survivor are the same
//! object on the only reachable path; the other three callers (F2:12033,
//! 12238, 12244) pass `undefined` and are unreachable on our stack.

use crate::avm1::{Activation, Error, ExecutionReason, FunctionObject, Object, Value};
use ruffle_common::avm_string::AvmString;
use std::sync::atomic::{AtomicBool, Ordering};

/// One-shot with retry until frame 2 defines the prototype.
static APPLIED: AtomicBool = AtomicBool::new(false);

/// Where the wrapper parks the panel's own implementation.
const ORIG_KEY: &str = "__chumby_stopAlarmsExcept";

/// `Alarm.TYPE_NONE` (F2:10170).
const TYPE_NONE: &str = "none";

pub fn apply(activation: &mut Activation<'_, '_>) {
    if !APPLIED.load(Ordering::Relaxed) && wrap_stop_alarms_except(activation).is_some() {
        APPLIED.store(true, Ordering::Relaxed);
    }
}

fn s<'gc>(activation: &mut Activation<'_, 'gc>, text: &str) -> AvmString<'gc> {
    AvmString::new_utf8(activation.gc(), text)
}

fn wrap_stop_alarms_except<'gc>(activation: &mut Activation<'_, 'gc>) -> Option<()> {
    let name = s(activation, "AlarmSet");
    let Ok(Value::Object(alarm_set)) = activation.global_object().get(name, activation) else {
        return None;
    };
    let name = s(activation, "prototype");
    let Ok(Value::Object(proto)) = alarm_set.get(name, activation) else {
        return None;
    };
    let method_name = s(activation, "stopAlarmsExcept");
    let original = match proto.get(method_name, activation) {
        Ok(Value::Object(f)) => f,
        _ => return None, // not defined yet
    };
    let orig_key = s(activation, ORIG_KEY);
    proto.set(orig_key, Value::Object(original), activation).ok()?;
    let fn_proto = activation.prototypes().function;
    let wrapper =
        FunctionObject::native(stop_unless_silent).build(activation.strings(), fn_proto, None);
    proto.set(method_name, wrapper, activation).ok()?;
    tracing::info!(target: "chumby_host",
        "wrapped AlarmSet.prototype.stopAlarmsExcept — a silent alarm no longer cancels a sounding one");
    Some(())
}

/// String property of an alarm, empty when absent or not a string.
fn alarm_field<'gc>(
    activation: &mut Activation<'_, 'gc>,
    alarm: Object<'gc>,
    field: &str,
) -> String {
    let name = s(activation, field);
    match alarm.get(name, activation) {
        Ok(Value::String(v)) => v.to_utf8_lossy().into_owned(),
        _ => String::new(),
    }
}

fn stop_unless_silent<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    if let Some(&Value::Object(alarm)) = args.first() {
        if alarm_field(activation, alarm, "_type") == TYPE_NONE {
            let name = alarm_field(activation, alarm, "_name");
            tracing::info!(target: "chumby_host",
                "silent alarm {name:?} rang — not cancelling ringing alarms");
            return Ok(Value::Undefined);
        }
    }
    let name = s(activation, ORIG_KEY);
    this.call_method(name, args, activation, ExecutionReason::FunctionCall)
}
