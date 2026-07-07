//! Declarative UI policy: neutralize panel controls the host platform does
//! not support, without touching the SWF.
//!
//! Loaded from `<fixtures>/ui-policy.toml` (rules are data, shipped with the
//! fixtures; see the chumby-pi repo's `fixtures/ui-policy.toml` and
//! `claude-docs/reference/18-clock-screen-and-ui-policy.md` for the catalog
//! and the CHECKPOINT UI1 decisions). Applied from `avm::method` on every
//! chumby native call — the panel polls `_bent` (5,25) once per frame, so
//! this re-applies at frame cadence and survives SWF-side re-inits; the walk
//! is a handful of child lookups per rule and costs nothing measurable.
//!
//! File format (TOML):
//! ```toml
//! [[rule]]
//! id        = "clock-ntp-toggle"
//! action    = "disable"            # hide | disable | readonly
//! selectors = [
//!   "controlPanel/depth:15/depth:1/name:timePanel/name:ntpButton",
//! ]
//! ```
//! A selector is a `/`-separated path from the SWF root. Each segment is
//! `name:<instanceName>` (bare text means the same) or `depth:<N>` (raw
//! PlaceObject tag depth) — depth segments exist because several links in
//! the panel's display chain are placed without instance names, and AVM1's
//! auto `instanceN` names depend on navigation history. Alternatives are
//! tried in order; the first fully resolved one wins.
//!
//! Actions (type-generic, applied through the AVM1 property interface, so
//! they behave exactly as if the SWF did it itself):
//! - `hide`     — `_visible = false`.
//! - `disable`  — `enabled = false` on the target AND its direct children
//!   (AVM1 `enabled` does not cascade, and the panel's controls put their
//!   hit handler on an inner clip like `box`/`b`), plus `_alpha = 45` for
//!   a grayed look. State stays visible: "on, but not changeable".
//! - `readonly` — input TextField → `type = "dynamic"`, `selectable = false`.
//!
//! Diagnostics: a rule logs once (info) each time it (re)acquires its
//! target. If a selector resolves the parent path but the leaf is missing —
//! the screen is up but the control is not where the catalog says — that is
//! a warning: the SWF variant likely changed and the policy needs updating.

use crate::avm1::{Activation, Value};
use crate::display_object::{DisplayObject, TDisplayObject, TDisplayObjectContainer};
use crate::string::{AvmString, WString};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use swf::Fixed8;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Hide,
    Disable,
    Readonly,
    /// Flat-recolour the target (AVM1 `Color.setRGB` semantics: zero the RGB
    /// multipliers, offset to the given `0xRRGGBB`). Used to repaint the
    /// dashboard wifi meter blue as a wired "link up" indicator.
    Tint(u32),
}

#[derive(Debug)]
enum Segment {
    /// Instance name; converted to a WString per lookup (WString is not
    /// Send/Sync, and the policy lives in a static).
    Name(String),
    Depth(i32),
}

#[derive(Debug)]
struct Rule {
    id: String,
    action: Action,
    selectors: Vec<Vec<Segment>>,
}

static POLICY: OnceLock<Vec<Rule>> = OnceLock::new();

/// Log damping: rule index -> resolution state on the previous pass
/// (0 = none, 1 = parent-only, 2 = resolved). Screens are recreated on
/// every navigation, so rules resolve and unresolve many times per
/// session; log only the transitions.
static LAST_STATE: OnceLock<Mutex<HashMap<usize, u8>>> = OnceLock::new();

fn last_state() -> &'static Mutex<HashMap<usize, u8>> {
    LAST_STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Parse `<fixtures>/ui-policy.toml`. Called once at startup, next to
/// `set_host` (desktop `main.rs`). Missing file = empty policy, not an error.
pub fn load(fixtures_root: &Path) {
    let path = fixtures_root.join("ui-policy.toml");
    let rules = match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text, &path),
        Err(_) => {
            tracing::info!(target: "chumby_host",
                "no ui-policy.toml in {} — UI policy empty", fixtures_root.display());
            Vec::new()
        }
    };
    if !rules.is_empty() {
        tracing::info!(target: "chumby_host",
            "UI policy loaded: {} rule(s) from {}", rules.len(), path.display());
    }
    let _ = POLICY.set(rules);
}

fn parse(text: &str, path: &Path) -> Vec<Rule> {
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(target: "chumby_host",
                "ui-policy: cannot parse {}: {e}", path.display());
            return Vec::new();
        }
    };
    let Some(toml::Value::Array(entries)) = table.get("rule") else {
        tracing::warn!(target: "chumby_host",
            "ui-policy: no [[rule]] entries in {}", path.display());
        return Vec::new();
    };

    let mut rules = Vec::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("<unnamed>")
            .to_owned();
        let action = match entry.get("action").and_then(|v| v.as_str()) {
            Some("hide") => Action::Hide,
            Some("disable") => Action::Disable,
            Some("readonly") => Action::Readonly,
            Some("tint") => match entry.get("color").and_then(|v| v.as_str()).and_then(parse_hex_color) {
                Some(rgb) => Action::Tint(rgb),
                None => {
                    tracing::warn!(target: "chumby_host",
                        "ui-policy rule '{id}': tint needs a color = \"#RRGGBB\" — rule skipped");
                    continue;
                }
            },
            other => {
                tracing::warn!(target: "chumby_host",
                    "ui-policy rule '{id}': unknown action {other:?} — rule skipped");
                continue;
            }
        };
        let selectors: Vec<Vec<Segment>> = entry
            .get("selectors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(parse_selector)
                    .collect()
            })
            .unwrap_or_default();
        if selectors.is_empty() {
            tracing::warn!(target: "chumby_host",
                "ui-policy rule '{id}': no selectors — rule skipped");
            continue;
        }
        rules.push(Rule { id, action, selectors });
    }
    rules
}

fn parse_selector(text: &str) -> Vec<Segment> {
    text.split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|seg| {
            if let Some(depth) = seg.strip_prefix("depth:") {
                match depth.trim().parse::<i32>() {
                    Ok(d) => Segment::Depth(d),
                    Err(_) => Segment::Name(seg.to_owned()),
                }
            } else {
                let name = seg.strip_prefix("name:").unwrap_or(seg);
                Segment::Name(name.to_owned())
            }
        })
        .collect()
}

enum Resolution<'gc> {
    Full(DisplayObject<'gc>),
    /// Everything but the last segment resolved: the screen is up but the
    /// control is missing — worth a warning, the catalog may be stale.
    ParentOnly,
    None,
}

fn resolve<'gc>(root: DisplayObject<'gc>, selector: &[Segment]) -> Resolution<'gc> {
    let mut current = root;
    for (i, segment) in selector.iter().enumerate() {
        let Some(container) = current.as_container() else {
            return Resolution::None;
        };
        let child = match segment {
            Segment::Name(name) => {
                container.child_by_name(WString::from_utf8(name).as_wstr(), false)
            }
            Segment::Depth(depth) => container.child_by_depth(*depth),
        };
        match child {
            Some(c) => current = c,
            None if i + 1 == selector.len() => return Resolution::ParentOnly,
            None => return Resolution::None,
        }
    }
    Resolution::Full(current)
}

/// Apply the policy to the current display tree. Called from `avm::method`;
/// idempotent, cheap, re-applies whenever a screen instance (re)appears.
pub fn apply<'gc>(activation: &mut Activation<'_, 'gc>) {
    let Some(rules) = POLICY.get() else { return };
    if rules.is_empty() {
        return;
    }
    let Some(root) = activation.context.stage.root_clip() else {
        return;
    };

    for (index, rule) in rules.iter().enumerate() {
        let mut target = None;
        let mut parent_only = false;
        for (nth, selector) in rule.selectors.iter().enumerate() {
            match resolve(root, selector) {
                Resolution::Full(d) => {
                    target = Some(d);
                    break;
                }
                // Only the primary selector's parent-only state is evidence
                // of a stale catalog: depth-based fallbacks can walk into a
                // DIFFERENT frame of the same containers (the settings grid
                // reuses depths across frames) and miss the leaf there.
                Resolution::ParentOnly if nth == 0 => parent_only = true,
                Resolution::ParentOnly | Resolution::None => {}
            }
        }

        let now: u8 = match (&target, parent_only) {
            (Some(_), _) => 2,
            (None, true) => 1,
            (None, false) => 0,
        };
        let mut seen = last_state().lock().unwrap();
        let before = seen.get(&index).copied().unwrap_or(0);
        seen.insert(index, now);
        drop(seen);

        if now != before {
            match now {
                2 => tracing::info!(target: "chumby_host",
                    "ui-policy '{}': target acquired — applying {:?}", rule.id, rule.action),
                1 => tracing::warn!(target: "chumby_host",
                    "ui-policy '{}': screen present but control missing — selectors stale?",
                    rule.id),
                _ => {}
            }
        }
        if let Some(display_object) = target {
            apply_action(activation, rule.action, display_object);
        }
    }
}

fn apply_action<'gc>(
    activation: &mut Activation<'_, 'gc>,
    action: Action,
    target: DisplayObject<'gc>,
) {
    let Some(obj) = target.object1() else {
        return;
    };
    let set = |activation: &mut Activation<'_, 'gc>,
               obj: crate::avm1::Object<'gc>,
               name: &str,
               value: Value<'gc>| {
        let name = AvmString::new_utf8(activation.gc(), name);
        let _ = obj.set(name, value, activation);
    };

    match action {
        Action::Hide => {
            set(activation, obj, "_visible", Value::Bool(false));
        }
        Action::Disable => {
            set(activation, obj, "enabled", Value::Bool(false));
            set(activation, obj, "_alpha", Value::Number(45.0));
            // AVM1 `enabled` does not cascade; the panel's hit handlers sit
            // on inner clips (`box`, `b`), so kill the direct children too.
            if let Some(container) = target.as_container() {
                for child in container.iter_render_list() {
                    if let Some(child_obj) = child.object1() {
                        set(activation, child_obj, "enabled", Value::Bool(false));
                    }
                }
            }
        }
        Action::Readonly => {
            set(activation, obj, "selectable", Value::Bool(false));
            let dynamic = AvmString::new_utf8(activation.gc(), "dynamic");
            set(activation, obj, "type", Value::String(dynamic));
        }
        Action::Tint(rgb) => {
            // Mirror AVM1 Color.setRGB (globals/color.rs): flat-fill the clip.
            let [b, g, r, _] = (rgb as i32).to_le_bytes();
            target.set_transformed_by_script(true);
            if let Some(parent) = target.parent() {
                parent.invalidate_cached_bitmap();
            }
            let mut ct = target.base().color_transform();
            ct.r_multiply = Fixed8::ZERO;
            ct.g_multiply = Fixed8::ZERO;
            ct.b_multiply = Fixed8::ZERO;
            ct.r_add = r.into();
            ct.g_add = g.into();
            ct.b_add = b.into();
            target.set_color_transform(ct);
        }
    }
}

/// Parse a `#RRGGBB` (or `RRGGBB`) hex colour to `0xRRGGBB`.
fn parse_hex_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rules_and_selectors() {
        let text = r#"
[[rule]]
id        = "a"
action    = "disable"
selectors = ["controlPanel/depth:15/name:timePanel/ntpButton"]

[[rule]]
id        = "bad-action"
action    = "explode"
selectors = ["x"]

[[rule]]
id        = "no-selectors"
action    = "hide"
selectors = []
"#;
        let rules = parse(text, Path::new("test"));
        // The malformed rules are skipped with a warning, not fatal.
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "a");
        assert_eq!(rules[0].action, Action::Disable);
        let sel = &rules[0].selectors[0];
        assert_eq!(sel.len(), 4);
        assert!(matches!(&sel[0], Segment::Name(n) if n == "controlPanel"));
        assert!(matches!(sel[1], Segment::Depth(15)));
        assert!(matches!(&sel[2], Segment::Name(n) if n == "timePanel"));
        assert!(matches!(&sel[3], Segment::Name(n) if n == "ntpButton"));
    }
}
