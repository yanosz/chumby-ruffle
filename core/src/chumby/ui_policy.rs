//! Declarative UI policy: neutralize panel controls the host platform does
//! not support, without touching the SWF.
//!
//! The rules live in `ui-policy.toml` next to this file and are compiled in:
//! this fork exists to run one SWF, so which of that SWF's controls are dead
//! is a property of the fork, not of whoever packages it. Applied from
//! `avm::method` on every chumby native call — the panel polls `_bent` (5,25)
//! once per frame, so this re-applies at frame cadence and survives SWF-side
//! re-inits; the walk is a handful of child lookups per rule and costs
//! nothing measurable. Mechanism: `claude-docs/design.md` §5.
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
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Hide,
    Disable,
    Readonly,
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
    /// When true, the rule applies only while chumby.com access is *off*.
    /// These are dead-ends without the remote service (channel management,
    /// delete) that come alive under `access_chumby_com`.
    only_without_chumby_access: bool,
    /// When true, the rule applies only while no brightness backend exists
    /// (no kernel backlight, no brightness_ctl — brightness.rs).
    only_without_brightness: bool,
    /// When true, the rule applies only while the intro movie is absent
    /// from the rootfs (`/usr/widgets/intro.swf` — owner-copied, intro.rs).
    only_without_intro: bool,
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

/// The compiled-in rule set, parsed on first use. A malformed rule is skipped
/// with a warning rather than being fatal; `test_embedded_policy_parses`
/// guards the shipped file so that never happens unnoticed.
fn policy() -> &'static Vec<Rule> {
    POLICY.get_or_init(|| {
        let rules = parse(include_str!("ui-policy.toml"));
        tracing::info!(target: "chumby_host", "UI policy: {} rule(s)", rules.len());
        rules
    })
}

fn parse(text: &str) -> Vec<Rule> {
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(target: "chumby_host", "ui-policy: cannot parse: {e}");
            return Vec::new();
        }
    };
    let Some(toml::Value::Array(entries)) = table.get("rule") else {
        tracing::warn!(target: "chumby_host", "ui-policy: no [[rule]] entries");
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
        let only_without_chumby_access = entry
            .get("only_without_chumby_access")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let only_without_brightness = entry
            .get("only_without_brightness")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let only_without_intro = entry
            .get("only_without_intro")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        rules.push(Rule {
            id,
            action,
            selectors,
            only_without_chumby_access,
            only_without_brightness,
            only_without_intro,
        });
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
    let rules = policy();
    if rules.is_empty() {
        return;
    }
    let Some(root) = activation.context.stage.root_clip() else {
        return;
    };

    // Rules tagged `only_without_chumby_access` are dead-ends the remote
    // service resolves; skip them once remote channels are actually
    // reachable. That is the same gate the passthrough uses (fixture.rs):
    // the owner flag AND a stable identity (hardware serial or configured
    // device_guid). A box with neither can't reach chumby.com even with the
    // flag on, so the buttons stay the dead-ends they are and remain disabled.
    let chumby_access = super::host::host()
        .map(|h| {
            h.config().access_chumby_com && super::real_ident::has_wire_identity(h.config())
        })
        .unwrap_or(false);
    // Same shape for brightness: the rule stands until a backend exists.
    let brightness = super::host::host()
        .map(|h| h.brightness_available())
        .unwrap_or(false);
    // And for the intro: the rule stands until the owner-copied movie
    // exists where the replaced playIntro stages it from (intro.rs). The
    // rootfs view follows symlinks, so the launcher's dangling link to a
    // not-yet-copied owner file counts as absent.
    let intro = super::host::host()
        .map(|h| h.fs().file_exists("/usr/widgets/intro.swf"))
        .unwrap_or(false);

    for (index, rule) in rules.iter().enumerate() {
        if rule.only_without_chumby_access && chumby_access {
            continue;
        }
        if rule.only_without_brightness && brightness {
            continue;
        }
        if rule.only_without_intro && intro {
            continue;
        }
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
    }
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
        let rules = parse(text);
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

    /// The shipped policy is compiled in, so a typo in it would otherwise
    /// only surface as a control that silently stays live on the device.
    #[test]
    fn test_embedded_policy_parses() {
        let text = include_str!("ui-policy.toml");
        let rules = parse(text);
        assert_eq!(
            rules.len(),
            text.matches("\n[[rule]]").count(),
            "a rule in the shipped ui-policy.toml was skipped as malformed"
        );
        assert!(rules.iter().any(|r| r.id == "clock-ntp-toggle"));
        assert!(rules.iter().all(|r| !r.selectors.is_empty()));
        // The channel/delete dead-ends must stay gated on chumby.com access
        // so they lift under the flag; everything else applies always.
        let gated: Vec<&str> = rules
            .iter()
            .filter(|r| r.only_without_chumby_access)
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(gated, ["main-channel", "main-delete"]);
        // Brightness lifts as soon as a backend exists (backlight or ctl).
        let gated: Vec<&str> = rules
            .iter()
            .filter(|r| r.only_without_brightness)
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(gated, ["settings-brightness"]);
        // INTRO lifts as soon as the owner-copied intro.swf exists.
        let gated: Vec<&str> = rules
            .iter()
            .filter(|r| r.only_without_intro)
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(gated, ["info-intro"]);
    }
}
