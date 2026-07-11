//! The `ASnative(5, N)` chumby function table.
//!
//! Dispatched from upstream `core/src/avm1/globals/asnative.rs`, which
//! routes all of category 5 here. Every registered index — purpose, args,
//! return value, fixture behavior — is documented in this repo's README
//! ("ASnative(5,N) reference"); `wrapper_name` below is the index → name
//! map. Every call is logged under target `chumby_host` — this is the
//! project's runtime visibility layer (stock Ruffle gives no signal for
//! natives).

use super::host::{self, HostValue};
use crate::avm1::{Activation, Error, Object, Value};
use ruffle_common::avm_string::AvmString;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// One-time AS2 surgery: `WidgetPlayer.prototype.onPress` (a click-stats
/// handler) puts the widget container into AS2 button mode, swallowing every
/// click meant for the widget's own buttons. Harmless on real hardware
/// (widgets run in a slave player with raw touch routed past the master),
/// fatal in our localCache in-movie mode. Deleting it restores real-device
/// behavior: clicks reach the widget. Retried on every native call until
/// the panel has defined the handler (frame 2).
static ONPRESS_REMOVED: AtomicBool = AtomicBool::new(false);

/// Per-index deduplication state: (last_args_repr, last_result_repr, repeat_count).
/// Consecutive identical calls for the same index are suppressed; the count
/// is flushed (as "× N") the next time that index is called with different
/// args/result. This keeps high-frequency polls (_bent, _getAudioPlayerState,
/// etc.) from flooding the log during a walkthrough.
static DEDUP: OnceLock<Mutex<HashMap<u16, (String, String, u32)>>> = OnceLock::new();

fn dedup() -> &'static Mutex<HashMap<u16, (String, String, u32)>> {
    DEDUP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remove_widget_player_on_press<'gc>(activation: &mut Activation<'_, 'gc>) -> Option<()> {
    let widget_player = AvmString::new_utf8(activation.gc(), "WidgetPlayer");
    let Ok(Value::Object(wp)) = activation.global_object().get(widget_player, activation)
    else {
        return None;
    };
    let prototype = AvmString::new_utf8(activation.gc(), "prototype");
    let Ok(Value::Object(proto)) = wp.get(prototype, activation) else {
        return None;
    };
    let on_press = AvmString::new_utf8(activation.gc(), "onPress");
    match proto.get(on_press, activation) {
        Ok(Value::Undefined) | Err(_) => return None, // not defined yet
        Ok(_) => {}
    }
    let deleted = proto.delete(activation, on_press);
    tracing::info!(target: "chumby_host",
        "removed WidgetPlayer.prototype.onPress (deleted={deleted}) — widget clicks unblocked");
    deleted.then_some(())
}

/// Entry point, `TableNativeFunction` signature.
pub fn method<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Object<'gc>,
    args: &[Value<'gc>],
    index: u16,
) -> Result<Value<'gc>, Error<'gc>> {
    let name = wrapper_name(index);
    let host_args: Vec<HostValue> = args.iter().map(to_host_value).collect();

    if !ONPRESS_REMOVED.load(Ordering::Relaxed)
        && remove_widget_player_on_press(activation).is_some()
    {
        ONPRESS_REMOVED.store(true, Ordering::Relaxed);
    }

    // UI policy rides the native-call cadence (the panel polls _bent every
    // frame), re-applying idempotently so screen re-entry can't undo it.
    super::ui_policy::apply(activation);
    // One-shot (with retry until frame 2 defines the array): hide the
    // music sources the appliance cannot serve.
    super::music_sources::apply(activation);

    let result = dispatch(activation, index, name, args, &host_args)?;

    log_deduped(index, name, &host_args, &to_host_value(&result));
    Ok(result)
}

fn log_deduped(index: u16, name: &str, args: &[HostValue], result: &HostValue) {
    let args_s = format_args_short(args);
    let res_s = format!("{result:?}");
    let mut map = dedup().lock().unwrap();
    match map.get_mut(&index) {
        Some((last_args, last_res, count)) if *last_args == args_s && *last_res == res_s => {
            *count += 1; // same call again — suppress
        }
        Some((last_args, last_res, count)) => {
            // value changed: flush the repeat count, then log the new value
            let repeat = *count;
            if repeat > 1 {
                tracing::info!(target: "chumby_host",
                    "ASnative(5,{index}) {name} × {repeat}");
            }
            *last_args = args_s.clone();
            *last_res = res_s.clone();
            *count = 1;
            tracing::info!(target: "chumby_host",
                "ASnative(5,{index}) {name}({args_s}) -> {res_s}");
        }
        None => {
            // first time seeing this index
            map.insert(index, (args_s.clone(), res_s.clone(), 1));
            tracing::info!(target: "chumby_host",
                "ASnative(5,{index}) {name}({args_s}) -> {res_s}");
        }
    }
}

fn dispatch<'gc>(
    activation: &mut Activation<'_, 'gc>,
    index: u16,
    name: &'static str,
    args: &[Value<'gc>],
    host_args: &[HostValue],
) -> Result<Value<'gc>, Error<'gc>> {
    let Some(host) = host::host() else {
        // Feature compiled in but no --chumby-fixtures given: behave like
        // stock Ruffle (Undefined), but say so once per index would be
        // nicer; keep it simple and log every time.
        tracing::warn!(target: "chumby_host",
            "ASnative(5,{index}) {name} called with no ChumbyHost configured");
        return Ok(Value::Undefined);
    };

    let str_arg = |i: usize| -> String {
        match host_args.get(i) {
            Some(HostValue::String(s)) => s.clone(),
            Some(HostValue::Number(n)) => n.to_string(),
            _ => String::new(),
        }
    };

    let result = match index {
        // --- Filesystem natives: the virtual rootfs ---
        // (5,50) _getFile(path) -> contents, or undefined if missing
        50 => match host.fs().get_file(&str_arg(0)) {
            Some(data) => HostValue::String(String::from_utf8_lossy(&data).into_owned()),
            None => HostValue::Undefined,
        },
        // (5,51) _putFile(path, data)
        51 => {
            let _ = host.fs().put_file(&str_arg(0), str_arg(1).as_bytes());
            HostValue::Undefined
        }
        // (5,53) _fileExists(path) -> 1/0
        53 => HostValue::Number(host.fs().file_exists(&str_arg(0)) as u8 as f64),
        // (5,54) _fileSize(path) -> bytes, 0 if missing
        54 => match host.fs().file_size(&str_arg(0)) {
            Some(n) => HostValue::Number(n as f64),
            None => HostValue::Number(0.0),
        },
        // (5,55) _unlink(path)
        55 => {
            let _ = host.fs().unlink(&str_arg(0));
            HostValue::Undefined
        }
        // (5,320) _getDirectoryEntry(obj, path, index): fills obj with
        // _name/_path/_isDir/_isDirLink/_isFile; returns 1 = entry,
        // 0 = end of listing, -1 = bad path. The panel iterates ascending
        // indices and does its own filtering (dotfiles, dir symlinks,
        // usb-* dirs, music extensions).
        320 => {
            let path = str_arg(1);
            let entry_index = match host_args.get(2) {
                Some(HostValue::Number(n)) if *n >= 0.0 => *n as u32,
                _ => 0,
            };
            match host.fs().dir_entry(&path, entry_index) {
                host::DirEntryResult::Entry(e) => {
                    if let Some(Value::Object(obj)) = args.first() {
                        // _path in normalized panel space (the callers pass
                        // "//mnt/usb/"-style paths; the value feeds the
                        // breadcrumb, _fileExists and _playAudio).
                        let full = panel_path_join(&path, &e.name);
                        let gc = activation.gc();
                        let fields: [(&str, Value<'gc>); 5] = [
                            ("_name", AvmString::new_utf8(gc, &e.name).into()),
                            ("_path", AvmString::new_utf8(gc, &full).into()),
                            ("_isDir", e.is_dir.into()),
                            ("_isDirLink", e.is_dir_link.into()),
                            ("_isFile", e.is_file.into()),
                        ];
                        for (key, value) in fields {
                            let key = AvmString::new_utf8(activation.gc(), key);
                            obj.set(key, value, activation)?;
                        }
                    }
                    HostValue::Number(1.0)
                }
                host::DirEntryResult::End => HostValue::Number(0.0),
                host::DirEntryResult::InvalidPath => HostValue::Number(-1.0),
            }
        }

        // (5,52) _backtick(cmd) -> stdout (synchronous shell)
        52 => match host.exec(&str_arg(0)) {
            Ok(out) => HostValue::String(String::from_utf8_lossy(&out).into_owned()),
            Err(_) => HostValue::String(String::new()),
        },

        // --- Pure functions (never reach the host) ---
        // (5,160) _base64Encode(s) / (5,161) _base64Decode(s)
        160 => HostValue::String(base64_encode(str_arg(0).as_bytes())),
        161 => HostValue::String(
            String::from_utf8_lossy(&base64_decode(&str_arg(0))).into_owned(),
        ),
        // (5,162) _md5Sum(s) -> hex digest
        162 => HostValue::String(super::real_ident::md5_hex(str_arg(0).as_bytes())),

        // --- Everything else: host state or logging default ---
        _ => host.native(index, name, host_args),
    };

    Ok(from_host_value(activation, result))
}

/// `_path` for `_getDirectoryEntry`: join directory and entry name in
/// normalized panel space, collapsing the panel's multi-slash path forms.
fn panel_path_join(dir: &str, name: &str) -> String {
    let mut full = String::new();
    for c in dir.split('/').filter(|c| !c.is_empty() && *c != ".") {
        full.push('/');
        full.push_str(c);
    }
    full.push('/');
    full.push_str(name);
    full
}

fn to_host_value(value: &Value) -> HostValue {
    match value {
        Value::Undefined | Value::Null => HostValue::Undefined,
        Value::Bool(b) => HostValue::Bool(*b),
        Value::Number(n) => HostValue::Number(*n),
        Value::String(s) => HostValue::String(s.to_utf8_lossy().into_owned()),
        Value::Object(_) | Value::MovieClip(_) => HostValue::String("<object>".into()),
    }
}

fn from_host_value<'gc>(activation: &mut Activation<'_, 'gc>, value: HostValue) -> Value<'gc> {
    match value {
        HostValue::Undefined => Value::Undefined,
        HostValue::Bool(b) => Value::Bool(b),
        HostValue::Number(n) => Value::Number(n),
        HostValue::String(s) => AvmString::new_utf8(activation.gc(), s).into(),
    }
}

fn format_args_short(args: &[HostValue]) -> String {
    let parts: Vec<String> = args
        .iter()
        .map(|a| match a {
            HostValue::Undefined => "undefined".into(),
            HostValue::Bool(b) => b.to_string(),
            HostValue::Number(n) => n.to_string(),
            HostValue::String(s) if s.len() <= 80 => format!("{s:?}"),
            HostValue::String(s) => {
                let mut end = 77;
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{:?}…[{}B]", &s[..end], s.len())
            }
        })
        .collect();
    parts.join(", ")
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        let chars = [
            ALPHABET[(n >> 18 & 63) as usize],
            ALPHABET[(n >> 12 & 63) as usize],
            ALPHABET[(n >> 6 & 63) as usize],
            ALPHABET[(n & 63) as usize],
        ];
        let keep = chunk.len() + 1;
        for (i, c) in chars.iter().enumerate() {
            out.push(if i < keep { *c as char } else { '=' });
        }
    }
    out
}

fn base64_decode(text: &str) -> Vec<u8> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let chars: Vec<u32> = text.bytes().filter_map(val).collect();
    let mut out = Vec::with_capacity(chars.len() / 4 * 3);
    for chunk in chars.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= c << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    out
}

/// Canonical index → wrapper-name map, extracted from the bindings the
/// control panel itself creates at startup (frame 2). Used for logging and
/// for `ChumbyHost::native` dispatch-by-name. Per-index documentation:
/// README, "ASnative(5,N) reference". `_slaveSetting72`/`_slaveSetting74`
/// are project-invented placeholders (the panel binds those indices to no
/// name); anything not listed logs as `_unknown`.
fn wrapper_name(index: u16) -> &'static str {
    match index {
        10 => "_rawX",
        11 => "_rawY",
        12 => "_setCalibration",
        13 => "_writeCalibration",
        14 => "_bendLevel",
        15 => "_lightLevel",
        16 => "_dcVolts",
        17 => "_getSpeakerMute",
        18 => "_setSpeakerMute",
        19 => "_getLCDMute",
        20 => "_setLCDMute",
        21 => "_getLCDBrightness",
        22 => "_setLCDBrightness",
        23 => "_getBrightnessThresholdForRange",
        24 => "_setBrightnessThresholdForRange",
        25 => "_bent",
        26 => "_getBendThreshold",
        27 => "_setBendThreshold",
        28 => "_bendAverage",
        38 => "_headphonesIn",
        39 => "_batteryVolts",
        40 => "_powerDown",
        41 => "_powerSource",
        42 => "_fscommand2",
        43 => "_getTouchClick",
        44 => "_setTouchClick",
        50 => "_getFile",
        51 => "_putFile",
        52 => "_backtick",
        53 => "_fileExists",
        54 => "_fileSize",
        55 => "_unlink",
        60 => "_accelerometer",
        61 => "_accelerometerSigned",
        70 => "_csccd",
        71 => "_gsccd",
        72 => "_slaveSetting72",
        74 => "_slaveSetting74",
        80 => "_setSlaveVar",
        81 => "_getSlaveVar",
        82 => "_routeUIEvents",
        83 => "_setDisplay",
        84 => "_startSlave",
        85 => "_stopSlave",
        86 => "_pauseResumeSlave",
        87 => "_getDefaultSlaveInstance",
        88 => "_getDisplay",
        89 => "_getSlaveLoadStatus",
        90 => "_getKeyboardEventMask",
        91 => "_hasKeyboard",
        92 => "_setKeyboardEventMask",
        93 => "_keyboardGetScanCode",
        94 => "_getMouseButtonState",
        95 => "_getNextMouseEvent",
        96 => "_getGamepadState",
        97 => "_getNextGamepadEvent",
        98 => "_getKeyEventOfferMask",
        99 => "_setKeyEventOfferMask",
        100 => "_expireCache",
        101 => "_expireCacheFiltered",
        110 => "_setOverlayVisibility",
        111 => "_getOverlayVisibility",
        112 => "_setOverlayBlendingEnabled",
        113 => "_getOverlayBlendingEnabled",
        114 => "_setOverlayChromaBlendingEnabled",
        115 => "_getOverlayChromaBlendingEnabled",
        116 => "_setOverlayChromaBlendColor",
        117 => "_getOverlayChromaBlendColor",
        118 => "_enableMasterUpdates",
        119 => "_enableSlaveUpdates",
        120 => "_exitOpportunity",
        121 => "_secondsBeforeRestart",
        122 => "_restartNow",
        130 => "_getAudioPlayerPID",
        131 => "_getAudioPlayerState",
        132 => "_pauseAudioPlayer",
        133 => "_resumeAudioPlayer",
        134 => "_stopAudioPlayer",
        135 => "_getAudioPlayerTrackAttributes",
        140 => "_isAudioPlayerAvailable",
        141 => "_terminateAudioPlayer",
        142 => "_startAudioPlayer",
        143 => "_restartAudioPlayer",
        144 => "_playAudio",
        145 => "_playAudioNext",
        146 => "_playAudioLoopCount",
        147 => "_skipAudioNext",
        148 => "_skipAudioPrev",
        149 => "_playAudioAddPlaylist",
        150 => "_playAudioResetPlaylist",
        151 => "_playAudioNow",
        152 => "_sendMediaPlayerCommand",
        160 => "_base64Encode",
        161 => "_base64Decode",
        162 => "_md5Sum",
        163 => "_blowfishEncrypt",
        164 => "_blowfishDecrypt",
        170 => "_getstalt",
        172 => "_getNetworkConnectTimeout",
        173 => "_setNetworkConnectTimeout",
        174 => "_getNetworkTimeout",
        175 => "_setNetworkTimeout",
        176 => "_setSystemTime",
        177 => "_getTimeZone",
        178 => "_setTimeZone",
        180 => "_getSystemVolume",
        181 => "_setSystemVolume",
        182 => "_getSystemBalance",
        183 => "_setSystemBalance",
        184 => "_getSystemMute",
        185 => "_setSystemMute",
        190 => "_pipeDaemon",
        191 => "_pipeOpen",
        192 => "_pipeSetInput",
        193 => "_pipeRead",
        194 => "_pipeWrite",
        195 => "_pipeClose",
        200 => "_getScreenWidth",
        201 => "_getScreenHeight",
        202 => "_getPlatform",
        203 => "_getUsingTsdev",
        204 => "_getTSCalibrationPath",
        205 => "_getEnvironment",
        207 => "_getpid",
        208 => "_getTotalPlayerMemory",
        209 => "_getFreePlayerMemory",
        210 => "_setSlaveAS3Var",
        211 => "_setURLEncodedVars",
        220 => "_enableMasterUpdatesPriv",
        300 => "_getBitmapSmoothing",
        301 => "_setBitmapSmoothing",
        320 => "_getDirectoryEntry",
        330 => "_slaveMouseDown",
        331 => "_slaveMouseMove",
        332 => "_slaveMouseUp",
        333 => "_getLastGesture",
        340 => "_pauseMediaPlayer",
        341 => "_resumeMediaPlayer",
        342 => "_stopMediaPlayer",
        360 => "_grantTempSlavePrivileges",
        361 => "_revokeTempSlavePrivileges",
        362 => "_grantSlavePrivileges",
        363 => "_revokeSlavePrivileges",
        364 => "_getEffectivePrivileges",
        370 => "_resetTransform",
        371 => "_addRotation",
        372 => "_addScale",
        373 => "_addTranslate",
        380 => "_setBackgroundAlpha",
        381 => "_getSWFDimensions",
        382 => "_mapMovieToViewportSpace",
        383 => "_mapViewportToMovieSpace",
        384 => "_fillFrameBufferPixels",
        385 => "_fillFrameBufferBytes",
        386 => "_setDisplayRect",
        387 => "_setDisplayRectEventTranslate",
        420 => "_SetOnLocationCallback",
        _ => "_unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::panel_path_join;

    /// _path must come out clean even from the panel's messy path forms —
    /// it feeds the breadcrumb, _fileExists and _playAudio.
    #[test]
    fn test_panel_path_join_normalizes() {
        assert_eq!(panel_path_join("//mnt/usb/", "a.mp3"), "/mnt/usb/a.mp3");
        assert_eq!(panel_path_join("/mnt/usb/Album", "t.mp3"), "/mnt/usb/Album/t.mp3");
        assert_eq!(panel_path_join("////mnt", "usb"), "/mnt/usb");
        assert_eq!(panel_path_join("/mnt/./usb", "x"), "/mnt/usb/x");
    }
}
