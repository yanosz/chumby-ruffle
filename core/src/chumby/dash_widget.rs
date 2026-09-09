//! Dash widgets on the panel's own proxy path, not the slave player.
//!
//! `WidgetSequencer` (Dash export `widgetbrowser/WidgetSequencer.as`) carries
//! two complete branches keyed on its `_isChumby` field (:67, copied from
//! `Chumby.isChumby`): the device branch hands the widget to the slave
//! player (`_startSlave`, :357) and places it with `_setDisplayRect` (:606);
//! the other loads it with a `MovieClipLoader` into `__widgetProxy`
//! (:365-370), injects the `_chumby_*` parameters on load (:553-558), polls
//! `_chumby_widget_done` on the clip (:531) and places it by position and
//! scale in the theme's rectangle (:596-600). We run no slave (requirements
//! FR2 M6), so the field reads false: a virtual property on the prototype
//! whose setter swallows the constructor's assignment. `Chumby.isChumby`
//! itself stays true — backticks, `file:///` prefixes and the startup
//! route all depend on it. The classic has no such class; the lookup
//! simply never succeeds there.

use crate::avm1::{Activation, Error, ExecutionReason, FunctionObject, Object, Value};
use ruffle_common::avm_string::AvmString;
use std::sync::atomic::{AtomicBool, Ordering};

/// One-shot with retry until the class is defined.
static APPLIED: AtomicBool = AtomicBool::new(false);

const CLASS_PATH: [&str; 6] =
    ["com", "chumby", "controlpanel", "widgetbrowser", "WidgetSequencer", "prototype"];

pub fn apply(activation: &mut Activation<'_, '_>) {
    if !APPLIED.load(Ordering::Relaxed) && pin_is_chumby(activation).is_some() {
        APPLIED.store(true, Ordering::Relaxed);
    }
}

fn s<'gc>(activation: &mut Activation<'_, 'gc>, text: &str) -> AvmString<'gc> {
    AvmString::new_utf8(activation.gc(), text)
}

fn pin_is_chumby<'gc>(activation: &mut Activation<'_, 'gc>) -> Option<()> {
    let mut proto: Object<'gc> = activation.global_object();
    for segment in CLASS_PATH {
        let name = s(activation, segment);
        proto = match proto.get(name, activation) {
            Ok(Value::Object(o)) => o,
            _ => return None, // not defined yet, or not this panel
        };
    }
    let fn_proto = activation.prototypes().function;
    let getter = FunctionObject::native(read_false).build(activation.strings(), fn_proto, None);
    let setter = FunctionObject::native(ignore).build(activation.strings(), fn_proto, None);
    let add_property = s(activation, "addProperty");
    let field = s(activation, "_isChumby");
    proto
        .call_method(
            add_property,
            &[field.into(), Value::Object(getter), Value::Object(setter)],
            activation,
            ExecutionReason::FunctionCall,
        )
        .ok()?;
    tracing::info!(target: "chumby_host",
        "WidgetSequencer.prototype._isChumby pinned to false — widgets ride the proxy path");
    Some(())
}

fn read_false<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(Value::Bool(false))
}

fn ignore<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(Value::Undefined)
}
