//! An empty widget channel enters clock mode instead of a black screen.
//!
//! With zero widget instances the panel's rotation dead-ends:
//! `fetchWidgetInstanceXML` (F2:4581) indexes an empty `g_widgetInstances`
//! and nothing ever paints. The stock fallback `beAClock` (F2:5322) —
//! whose localCache branch attaches `bi_clock`, the clock embedded in the
//! panel itself — only fires on the unauthorized/offline/timeout paths,
//! which the fixtures never take. So the prototype method is wrapped
//! (surgery precedent: intro.rs): an empty rotation enters clock mode,
//! any other call takes the original path unchanged. Widgets appearing
//! later (profile reload) leave clock mode via `nextWidget` as on stock.

use crate::avm1::{Activation, Error, ExecutionReason, FunctionObject, Object, Value};
use ruffle_common::avm_string::AvmString;
use std::sync::atomic::{AtomicBool, Ordering};

/// One-shot with retry until frame 2 defines the prototype.
static APPLIED: AtomicBool = AtomicBool::new(false);

/// Where the wrapper parks the panel's own implementation.
const ORIG_KEY: &str = "__chumby_fetchWidgetInstanceXML";

pub fn apply(activation: &mut Activation<'_, '_>) {
    if !APPLIED.load(Ordering::Relaxed) && wrap_fetch(activation).is_some() {
        APPLIED.store(true, Ordering::Relaxed);
    }
}

fn s<'gc>(activation: &mut Activation<'_, 'gc>, text: &str) -> AvmString<'gc> {
    AvmString::new_utf8(activation.gc(), text)
}

fn wrap_fetch<'gc>(activation: &mut Activation<'_, 'gc>) -> Option<()> {
    let name = s(activation, "WidgetPlayer");
    let Ok(Value::Object(wp)) = activation.global_object().get(name, activation) else {
        return None;
    };
    let name = s(activation, "prototype");
    let Ok(Value::Object(proto)) = wp.get(name, activation) else {
        return None;
    };
    let fetch_name = s(activation, "fetchWidgetInstanceXML");
    let original = match proto.get(fetch_name, activation) {
        Ok(Value::Object(f)) => f,
        _ => return None, // not defined yet
    };
    let orig_key = s(activation, ORIG_KEY);
    proto.set(orig_key, Value::Object(original), activation).ok()?;
    let fn_proto = activation.prototypes().function;
    let wrapper =
        FunctionObject::native(fetch_or_clock).build(activation.strings(), fn_proto, None);
    proto.set(fetch_name, wrapper, activation).ok()?;
    tracing::info!(target: "chumby_host",
        "wrapped WidgetPlayer.prototype.fetchWidgetInstanceXML — an empty channel becomes a clock");
    Some(())
}

/// `Object._chumby.g_widgetInstances.length`, or 0 when the array is absent
/// (the "bad profile" path, F2:4440 — the fetch would dead-end there too).
fn rotation_len<'gc>(activation: &mut Activation<'_, 'gc>) -> Result<f64, Error<'gc>> {
    let name = s(activation, "Object");
    let Ok(Value::Object(object)) = activation.global_object().get(name, activation) else {
        return Ok(0.0);
    };
    let name = s(activation, "_chumby");
    let Ok(Value::Object(chumby)) = object.get(name, activation) else {
        return Ok(0.0);
    };
    let name = s(activation, "g_widgetInstances");
    let Ok(Value::Object(instances)) = chumby.get(name, activation) else {
        return Ok(0.0);
    };
    let name = s(activation, "length");
    instances.get(name, activation)?.coerce_to_f64(activation)
}

fn fetch_or_clock<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    if rotation_len(activation)? > 0.0 {
        let name = s(activation, ORIG_KEY);
        return this.call_method(name, args, activation, ExecutionReason::FunctionCall);
    }
    tracing::info!(target: "chumby_host", "empty widget channel — entering clock mode (bi_clock)");
    let name = s(activation, "beAClock");
    this.call_method(name, &[], activation, ExecutionReason::FunctionCall)?;
    Ok(Value::Undefined)
}
