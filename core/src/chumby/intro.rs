//! Make the INTRO button play `intro.swf` under localCache (roadmap item 4).
//!
//! The panel's `playIntro` (F2:5283) never attempts the intro in localCache
//! mode — its localCache branch substitutes the built-in clock, and only the
//! slave branch (`_startSlave`, which we do not run) names intro.swf. So
//! there is nothing to intercept at (5,84); instead the prototype method is
//! replaced wholesale — the same surgery precedent as the `onPress` deletion
//! (avm.rs) — with a native that mirrors the slave branch's semantics onto
//! the panel's own widgetProxy path (the localCache mechanics of
//! `gotCachedWidget`, F2:4942).
//!
//! `introAdvanceTimerHandler` is replaced too, not just installed as the
//! onEnterFrame poll: leaving the panel's own version (which polls
//! `_getSlaveVar`, no localCache branch) breaks the moment the info screen
//! closes — `setState` (F2:3642) re-installs the handler from the prototype,
//! and a stale `_chumby_widget_done=true` in the slave-var store then ends
//! the intro instantly. The replacement reads the proxy variable the
//! localCache paths use (`widgetProxy.proxy._chumby_widget_done`). The
//! panel's own `advanceTimerHandler` was considered and rejected: it only
//! advances when auto-advance is on and never resets `g_playingIntro`; the
//! intro handler advances unconditionally.

use crate::avm1::{Activation, Error, ExecutionReason, FunctionObject, Object, Value};
use ruffle_common::avm_string::AvmString;
use std::sync::atomic::{AtomicBool, Ordering};

const INTRO_URL: &str = "file:////usr/widgets/intro.swf";
const INTRO_INSTANCE_ID: &str = "00000000-0000-0000-0000-000000000000";

/// One-shot with retry until frame 2 defines `WidgetPlayer.prototype.playIntro`.
static APPLIED: AtomicBool = AtomicBool::new(false);

pub fn apply(activation: &mut Activation<'_, '_>) {
    if !APPLIED.load(Ordering::Relaxed) && replace_play_intro(activation).is_some() {
        APPLIED.store(true, Ordering::Relaxed);
    }
}

fn s<'gc>(activation: &mut Activation<'_, 'gc>, text: &str) -> AvmString<'gc> {
    AvmString::new_utf8(activation.gc(), text)
}

fn replace_play_intro<'gc>(activation: &mut Activation<'_, 'gc>) -> Option<()> {
    let name = s(activation, "WidgetPlayer");
    let Ok(Value::Object(wp)) = activation.global_object().get(name, activation) else {
        return None;
    };
    let name = s(activation, "prototype");
    let Ok(Value::Object(proto)) = wp.get(name, activation) else {
        return None;
    };
    let play_intro_name = s(activation, "playIntro");
    match proto.get(play_intro_name, activation) {
        Ok(Value::Object(_)) => {}
        _ => return None, // not defined yet
    }
    let fn_proto = activation.prototypes().function;
    let replacement = FunctionObject::native(play_intro).build(activation.strings(), fn_proto, None);
    proto
        .set(play_intro_name, replacement, activation)
        .ok()?;
    let handler_name = s(activation, "introAdvanceTimerHandler");
    let poll = FunctionObject::native(intro_poll).build(activation.strings(), fn_proto, None);
    proto.set(handler_name, poll, activation).ok()?;
    tracing::info!(target: "chumby_host",
        "replaced WidgetPlayer.prototype.playIntro + introAdvanceTimerHandler — \
         intro rides the widgetProxy path");
    Some(())
}

/// `Object._chumby`, the panel's globals bag.
fn chumby_object<'gc>(activation: &mut Activation<'_, 'gc>) -> Option<Object<'gc>> {
    let name = s(activation, "Object");
    let Ok(Value::Object(object)) = activation.global_object().get(name, activation) else {
        return None;
    };
    let name = s(activation, "_chumby");
    match object.get(name, activation) {
        Ok(Value::Object(chumby)) => Some(chumby),
        _ => None,
    }
}

/// Replacement `playIntro`: the slave branch of F2:5283, restaged on the
/// widgetProxy path. `this` is the WidgetPlayer movie clip.
fn play_intro<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    tracing::info!(target: "chumby_host", "playIntro: staging {INTRO_URL}");

    let name = s(activation, "clearAdvanceTimer");
    this.call_method(name, &[], activation, ExecutionReason::FunctionCall)?;
    let on_enter_frame = s(activation, "onEnterFrame");
    this.set(on_enter_frame, Value::Undefined, activation)?;

    let object_proto = activation.prototypes().object;
    let init = Object::new(activation.strings(), Some(object_proto));
    let key = s(activation, "_chumby_widget_instance_id");
    let val = s(activation, INTRO_INSTANCE_ID);
    init.set(key, val, activation)?;
    // The localCache ingredient: widgetProxy's own frame script loadMovies
    // this URL into its `proxy` child (DefineSprite_209).
    let key = s(activation, "_chumby_movie_url");
    let val = s(activation, INTRO_URL);
    init.set(key, val, activation)?;

    if let Some(chumby) = chumby_object(activation) {
        let key = s(activation, "g_widgetName");
        let val = s(activation, "Introduction");
        chumby.set(key, val, activation)?;
    }

    let widget_proxy = s(activation, "widgetProxy");
    if let Ok(Value::Object(old)) = this.get(widget_proxy, activation) {
        let name = s(activation, "removeMovieClip");
        old.call_method(name, &[], activation, ExecutionReason::FunctionCall)?;
    }
    let name = s(activation, "attachMovie");
    this.call_method(
        name,
        &[
            widget_proxy.into(),
            widget_proxy.into(),
            Value::Number(1.0),
            init.into(),
        ],
        activation,
        ExecutionReason::FunctionCall,
    )?;

    for (key, value) in [
        ("g_playingWidgetInstanceID", Value::Number(0.0)),
        ("g_timer_expires", Value::Number(-1.0)),
        ("g_remaining_time", Value::Number(10_000_000.0)),
    ] {
        let key = s(activation, key);
        this.set(key, value, activation)?;
    }
    if let Some(chumby) = chumby_object(activation) {
        for (key, value) in [
            ("g_instanceID", Value::Number(0.0)),
            ("g_playingIntro", Value::Bool(true)),
            ("g_widgetThumbnailURL", Value::Undefined),
            ("g_widgetIndex", Value::Number(-1.0)),
        ] {
            let key = s(activation, key);
            chumby.set(key, value, activation)?;
        }
    }

    // As the original: install the (replaced) prototype handler, so the
    // panel's own re-installs (setState, F2:3642) reference the same one.
    let handler_name = s(activation, "introAdvanceTimerHandler");
    let poll = this.get(handler_name, activation)?;
    this.set(on_enter_frame, poll, activation)?;
    Ok(Value::Undefined)
}

/// Replacement done-poll: `introAdvanceTimerHandler` (F2:5311) reading the
/// localCache flag `widgetProxy.proxy._chumby_widget_done`.
fn intro_poll<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let name = s(activation, "widgetProxy");
    let Ok(Value::Object(proxy_clip)) = this.get(name, activation) else {
        return Ok(Value::Undefined);
    };
    let name = s(activation, "proxy");
    let Ok(Value::Object(proxy)) = proxy_clip.get(name, activation) else {
        return Ok(Value::Undefined);
    };
    let name = s(activation, "_chumby_widget_done");
    let done = proxy
        .get(name, activation)
        .map(|v| v.as_bool(activation.swf_version()))
        .unwrap_or(false);
    if !done {
        return Ok(Value::Undefined);
    }

    tracing::info!(target: "chumby_host", "intro completed — advancing to the next widget");
    if let Some(chumby) = chumby_object(activation) {
        let key = s(activation, "g_playingIntro");
        chumby.set(key, Value::Bool(false), activation)?;
    }
    let key = s(activation, "onEnterFrame");
    this.set(key, Value::Undefined, activation)?;
    let name = s(activation, "nextWidget");
    this.call_method(name, &[], activation, ExecutionReason::FunctionCall)?;
    Ok(Value::Undefined)
}

/// While the intro plays *inside* the panel, its `fscommand("quit")`
/// (intro.swf frame 12) must not exit the player — on real hardware it only
/// killed the slave instance. Standalone runs (boot-time intro: no panel, no
/// `g_playingIntro`) keep upstream behavior, so quitting still ends the
/// process and boot can continue. The panel's own quit paths (update flow)
/// are likewise untouched.
pub fn swallow_fscommand_quit(command: &str, activation: &mut Activation<'_, '_>) -> bool {
    if command != "quit" {
        return false;
    }
    let Some(chumby) = chumby_object(activation) else {
        return false;
    };
    let key = s(activation, "g_playingIntro");
    let playing = chumby
        .get(key, activation)
        .map(|v| v.as_bool(activation.swf_version()))
        .unwrap_or(false);
    if playing {
        tracing::info!(target: "chumby_host",
            "fscommand(quit) from the in-panel intro swallowed");
    }
    playing
}
