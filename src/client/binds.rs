//! Key binds, as Source keeps them: key name -> console line
//! (`console::Console::binds`), saved in config.cfg as `bind` lines.
//! Every game key is a bind: the defaults (`DEFAULT_BINDS`, CS:S's own
//! plus ours) go in before config.cfg runs, `binddefaults` puts them back
//! (the options' "Use Defaults"), and the options' keyboard tab rebinds
//! them.
//!
//! Binds whose whole line is one of `POLLED` (`+forward`, `slot1`,
//! `buymenu` ...) are read by the system that owns the action, from the
//! keys held this frame (`pressed`, `just_pressed`), with that system's
//! own rules (only while playing, not while typing ...); `console::run_binds`
//! leaves them alone. Every other bind runs its line through the console.

use std::collections::BTreeMap;

use bevy::{
    input::mouse::{AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
};

use crate::console::{Console, ConsoleAppExt};

/// Source key names for keyboard keys.
pub const KEY_NAMES: &[(KeyCode, &str)] = &[
    (KeyCode::KeyA, "a"),
    (KeyCode::KeyB, "b"),
    (KeyCode::KeyC, "c"),
    (KeyCode::KeyD, "d"),
    (KeyCode::KeyE, "e"),
    (KeyCode::KeyF, "f"),
    (KeyCode::KeyG, "g"),
    (KeyCode::KeyH, "h"),
    (KeyCode::KeyI, "i"),
    (KeyCode::KeyJ, "j"),
    (KeyCode::KeyK, "k"),
    (KeyCode::KeyL, "l"),
    (KeyCode::KeyM, "m"),
    (KeyCode::KeyN, "n"),
    (KeyCode::KeyO, "o"),
    (KeyCode::KeyP, "p"),
    (KeyCode::KeyQ, "q"),
    (KeyCode::KeyR, "r"),
    (KeyCode::KeyS, "s"),
    (KeyCode::KeyT, "t"),
    (KeyCode::KeyU, "u"),
    (KeyCode::KeyV, "v"),
    (KeyCode::KeyW, "w"),
    (KeyCode::KeyX, "x"),
    (KeyCode::KeyY, "y"),
    (KeyCode::KeyZ, "z"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Digit1, "1"),
    (KeyCode::Digit2, "2"),
    (KeyCode::Digit3, "3"),
    (KeyCode::Digit4, "4"),
    (KeyCode::Digit5, "5"),
    (KeyCode::Digit6, "6"),
    (KeyCode::Digit7, "7"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::Space, "space"),
    (KeyCode::Enter, "enter"),
    (KeyCode::Tab, "tab"),
    (KeyCode::Backspace, "backspace"),
    (KeyCode::CapsLock, "capslock"),
    (KeyCode::ShiftLeft, "shift"),
    (KeyCode::ShiftRight, "rshift"),
    (KeyCode::ControlLeft, "ctrl"),
    (KeyCode::ControlRight, "rctrl"),
    (KeyCode::AltLeft, "alt"),
    (KeyCode::AltRight, "ralt"),
    (KeyCode::ArrowUp, "uparrow"),
    (KeyCode::ArrowDown, "downarrow"),
    (KeyCode::ArrowLeft, "leftarrow"),
    (KeyCode::ArrowRight, "rightarrow"),
    (KeyCode::Insert, "ins"),
    (KeyCode::Delete, "del"),
    (KeyCode::Home, "home"),
    (KeyCode::End, "end"),
    (KeyCode::PageUp, "pgup"),
    (KeyCode::PageDown, "pgdn"),
    (KeyCode::Pause, "pause"),
    (KeyCode::F1, "f1"),
    (KeyCode::F2, "f2"),
    (KeyCode::F3, "f3"),
    (KeyCode::F4, "f4"),
    (KeyCode::F5, "f5"),
    (KeyCode::F6, "f6"),
    (KeyCode::F7, "f7"),
    (KeyCode::F8, "f8"),
    (KeyCode::F9, "f9"),
    (KeyCode::F10, "f10"),
    (KeyCode::F11, "f11"),
    (KeyCode::F12, "f12"),
    (KeyCode::Numpad0, "kp_ins"),
    (KeyCode::Numpad1, "kp_end"),
    (KeyCode::Numpad2, "kp_downarrow"),
    (KeyCode::Numpad3, "kp_pgdn"),
    (KeyCode::Numpad4, "kp_leftarrow"),
    (KeyCode::Numpad5, "kp_5"),
    (KeyCode::Numpad6, "kp_rightarrow"),
    (KeyCode::Numpad7, "kp_home"),
    (KeyCode::Numpad8, "kp_uparrow"),
    (KeyCode::Numpad9, "kp_pgup"),
    (KeyCode::NumpadDecimal, "kp_del"),
    (KeyCode::NumpadDivide, "kp_slash"),
    (KeyCode::NumpadMultiply, "kp_multiply"),
    (KeyCode::NumpadSubtract, "kp_minus"),
    (KeyCode::NumpadAdd, "kp_plus"),
    (KeyCode::NumpadEnter, "kp_enter"),
    (KeyCode::Minus, "-"),
    (KeyCode::Equal, "="),
    (KeyCode::BracketLeft, "["),
    (KeyCode::BracketRight, "]"),
    (KeyCode::Backslash, "\\"),
    (KeyCode::Semicolon, "semicolon"),
    (KeyCode::Quote, "'"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::Slash, "/"),
];

/// Source names for mouse buttons.
pub const MOUSE_NAMES: &[(MouseButton, &str)] = &[
    (MouseButton::Left, "mouse1"),
    (MouseButton::Right, "mouse2"),
    (MouseButton::Middle, "mouse3"),
    (MouseButton::Back, "mouse4"),
    (MouseButton::Forward, "mouse5"),
];

/// The wheel's two "keys".
pub const WHEEL_UP: &str = "mwheelup";
pub const WHEEL_DOWN: &str = "mwheeldown";

/// The binds a fresh config starts with: CS:S's defaults for what mashup
/// does, and mashup's own (the wheel jumps, Alt looks around, F9 files a
/// bug report). Esc (the game menu) and ` (the console) are not binds.
pub const DEFAULT_BINDS: &[(&str, &str)] = &[
    ("w", "+forward"),
    ("s", "+back"),
    ("a", "+moveleft"),
    ("d", "+moveright"),
    ("space", "+jump"),
    ("ctrl", "+duck"),
    ("shift", "+speed"),
    ("e", "+use"),
    ("r", "+reload"),
    ("mouse1", "+attack"),
    ("mouse2", "+attack2"),
    ("1", "slot1"),
    ("2", "slot2"),
    ("3", "slot3"),
    ("4", "slot4"),
    ("5", "slot5"),
    ("q", "lastinv"),
    ("g", "drop"),
    ("b", "buymenu"),
    ("m", "chooseteam"),
    ("tab", "+showscores"),
    ("z", "radio1"),
    ("x", "radio2"),
    ("c", "radio3"),
    ("y", "messagemode"),
    ("u", "messagemode2"),
    ("alt", "+freelook"),
    ("v", "noclip"),
    ("mwheelup", "+jump"),
    ("mwheeldown", "+jump"),
    ("f9", "bug"),
];

/// Commands the systems that own them read from the held keys instead of
/// the console running them (see the module docs).
pub const POLLED: &[&str] = &[
    "+forward",
    "+back",
    "+moveleft",
    "+moveright",
    "+jump",
    "+duck",
    "+speed",
    "+use",
    "+reload",
    "+attack",
    "+attack2",
    "+freelook",
    "+showscores",
    "slot1",
    "slot2",
    "slot3",
    "slot4",
    "slot5",
    "lastinv",
    "drop",
    "buymenu",
    "chooseteam",
    "radio1",
    "radio2",
    "radio3",
    "messagemode",
    "messagemode2",
    "bug",
    "bugreport",
];

/// A bind's line normalised for comparing (trimmed, lower-case).
fn norm(line: &str) -> String {
    line.trim().to_lowercase()
}

/// Whether a bind's line is read by its system rather than run.
pub fn is_polled(line: &str) -> bool {
    let l = norm(line);
    POLLED.contains(&l.as_str())
}

/// The keys bound to exactly `command`, in key order.
pub fn keys_for<'a>(binds: &'a BTreeMap<String, String>, command: &str) -> Vec<&'a str> {
    let c = norm(command);
    binds
        .iter()
        .filter(|(_, v)| norm(v) == c)
        .map(|(k, _)| k.as_str())
        .collect()
}

/// A key name as the options list shows it (`SPACE`, `MOUSE1`).
pub fn display(key: &str) -> String {
    key.to_uppercase()
}

/// Whether a key (by Source name) is held.
pub fn key_held(name: &str, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> bool {
    KEY_NAMES
        .iter()
        .find(|(_, n)| *n == name)
        .is_some_and(|(k, _)| keys.pressed(*k))
        || MOUSE_NAMES
            .iter()
            .find(|(_, n)| *n == name)
            .is_some_and(|(b, _)| mouse.pressed(*b))
}

fn key_just_pressed(name: &str, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> bool {
    KEY_NAMES
        .iter()
        .find(|(_, n)| *n == name)
        .is_some_and(|(k, _)| keys.just_pressed(*k))
        || MOUSE_NAMES
            .iter()
            .find(|(_, n)| *n == name)
            .is_some_and(|(b, _)| mouse.just_pressed(*b))
}

/// Whether a key bound to `command` is held.
pub fn pressed(
    binds: &BTreeMap<String, String>,
    keys: &ButtonInput<KeyCode>,
    mouse: &ButtonInput<MouseButton>,
    command: &str,
) -> bool {
    keys_for(binds, command).into_iter().any(|k| key_held(k, keys, mouse))
}

/// Whether a key bound to `command` went down this frame.
pub fn just_pressed(
    binds: &BTreeMap<String, String>,
    keys: &ButtonInput<KeyCode>,
    mouse: &ButtonInput<MouseButton>,
    command: &str,
) -> bool {
    keys_for(binds, command)
        .into_iter()
        .any(|k| key_just_pressed(k, keys, mouse))
}

/// Wheel notches this frame on wheel directions bound to `command`.
pub fn wheel_notches(binds: &BTreeMap<String, String>, scroll: &AccumulatedMouseScroll, command: &str) -> u32 {
    /// Pixel-unit scrolling (touchpads, smooth wheels): pixels per notch.
    const PIXELS_PER_NOTCH: f32 = 40.0;
    let y = scroll.delta.y;
    if y == 0.0 {
        return 0;
    }
    let key = if y > 0.0 { WHEEL_UP } else { WHEEL_DOWN };
    if !keys_for(binds, command).contains(&key) {
        return 0;
    }
    (match scroll.unit {
        MouseScrollUnit::Line => y.abs().round(),
        MouseScrollUnit::Pixel => (y.abs() / PIXELS_PER_NOTCH).ceil(),
    }) as u32
}

/// The first key, button or wheel notch that went down this frame, by
/// Source name (for rebinding). Esc and ` are not offered.
pub fn first_pressed(
    keys: &ButtonInput<KeyCode>,
    mouse: &ButtonInput<MouseButton>,
    scroll: Option<&AccumulatedMouseScroll>,
) -> Option<&'static str> {
    KEY_NAMES
        .iter()
        .find(|(k, _)| keys.just_pressed(*k))
        .map(|(_, n)| *n)
        .or_else(|| MOUSE_NAMES.iter().find(|(b, _)| mouse.just_pressed(*b)).map(|(_, n)| *n))
        .or_else(|| {
            let y = scroll?.delta.y;
            (y != 0.0).then_some(if y > 0.0 { WHEEL_UP } else { WHEEL_DOWN })
        })
}

/// Put the default binds in: every one (`all`, after clearing the rest),
/// or only those whose key is free and whose command no key runs (for a
/// config written before its keys were binds).
pub fn bind_defaults(binds: &mut BTreeMap<String, String>, all: bool) {
    if all {
        binds.clear();
    }
    for (key, command) in DEFAULT_BINDS {
        if all || (!binds.contains_key(*key) && keys_for(binds, command).is_empty()) {
            binds.insert(key.to_string(), command.to_string());
        }
    }
}

/// The line that sets `command`'s key to `key` alone: its other keys
/// unbound, `key` bound to it (whatever it ran before).
pub fn rebind_line(binds: &BTreeMap<String, String>, command: &str, key: &str) -> String {
    let mut parts: Vec<String> = keys_for(binds, command)
        .into_iter()
        .filter(|k| *k != key)
        .map(|k| format!("unbind {}", crate::console::quote(k)))
        .collect();
    parts.push(format!(
        "bind {} {}",
        crate::console::quote(key),
        crate::console::quote(command)
    ));
    parts.join("; ")
}

/// The line that clears every key of `command` (None: it has none).
pub fn clear_line(binds: &BTreeMap<String, String>, command: &str) -> Option<String> {
    let keys = keys_for(binds, command);
    (!keys.is_empty()).then(|| {
        keys.iter()
            .map(|k| format!("unbind {}", crate::console::quote(k)))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// A config.cfg written since every key became a bind says so; one without
/// the mark gets the default binds its `unbindall` dropped back.
pub const CONFIG_MARK: &str = "// binds: all";

pub(super) fn commands(app: &mut App) {
    app.console_command(
        "binddefaults",
        "binddefaults [missing]: put the default binds back (missing: only on free keys for unbound commands).",
        |w, a| {
            let all = a.first().map(String::as_str) != Some("missing");
            let mut c = w.resource_mut::<Console>();
            let before = c.binds.clone();
            bind_defaults(&mut c.binds, all);
            if c.binds != before {
                c.dirty = true;
            }
            Ok(None)
        },
    )
    .console_command("bug", "bug [note]: file a bug report (CS:S's name for `bugreport`).", |w, a| {
        let note: Vec<String> = a.iter().map(|x| crate::console::quote(x)).collect();
        w.resource_mut::<Console>()
            .submit(format!("bugreport {}", note.join(" ")).trim_end().to_string());
        Ok(None)
    });
}

/// For tests of systems that read binds: the mouse's buttons and the
/// default binds in an app with the console.
#[cfg(test)]
pub(super) fn test_binds(app: &mut App) {
    app.init_resource::<ButtonInput<MouseButton>>();
    bind_defaults(&mut app.world_mut().resource_mut::<Console>().binds, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binds(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn defaults_have_known_keys_and_one_line_per_key() {
        let names: Vec<&str> = KEY_NAMES
            .iter()
            .map(|(_, n)| *n)
            .chain(MOUSE_NAMES.iter().map(|(_, n)| *n))
            .chain([WHEEL_UP, WHEEL_DOWN])
            .collect();
        for (k, c) in DEFAULT_BINDS {
            assert!(names.contains(k), "{k} is not a key name");
            // Console commands run by `console::run_binds`; the rest are
            // read by their systems.
            assert!(is_polled(c) || ["noclip"].contains(c), "{c}: not polled and not a known console default");
        }
        let mut keys: Vec<&str> = DEFAULT_BINDS.iter().map(|(k, _)| *k).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), DEFAULT_BINDS.len());
    }

    #[test]
    fn rebinding_moves_the_action_to_the_new_key() {
        let b = binds(&[("w", "+forward"), ("uparrow", "+forward"), ("f", "+use")]);
        assert_eq!(keys_for(&b, "+FORWARD "), ["uparrow", "w"]);
        assert_eq!(
            rebind_line(&b, "+forward", "f"),
            "unbind uparrow; unbind w; bind f +forward"
        );
        assert_eq!(rebind_line(&b, "+forward", "w"), "unbind uparrow; bind w +forward");
        assert_eq!(clear_line(&b, "+use").as_deref(), Some("unbind f"));
        assert_eq!(clear_line(&b, "+jump"), None);
        // Keys that need quoting stay one word.
        assert_eq!(rebind_line(&b, "say hi", ";"), "bind \";\" \"say hi\"");
    }

    #[test]
    fn missing_defaults_fill_only_free_keys_for_unbound_commands() {
        // An old config: only the player's own binds survived unbindall.
        let mut b = binds(&[("f", "+jump"), ("w", "say hello")]);
        bind_defaults(&mut b, false);
        assert_eq!(b.get("space"), None, "+jump already has a key");
        assert_eq!(b["w"], "say hello", "a taken key is kept");
        assert!(keys_for(&b, "+forward").is_empty(), "w was taken, so +forward stays unbound");
        assert_eq!(b["a"], "+moveleft");
        bind_defaults(&mut b, true);
        assert_eq!(b.len(), DEFAULT_BINDS.len());
        assert_eq!(b["w"], "+forward");
    }

    #[test]
    fn binds_save_to_config_and_load_back() {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin);
        commands(&mut app);
        app.world_mut()
            .resource_mut::<Console>()
            .submit("binddefaults; unbind space; bind k +jump; bind [ \"say hi\"");
        app.update();
        let saved = app.world().resource::<Console>().binds.clone();
        assert_eq!(saved.get("k").map(String::as_str), Some("+jump"));
        let text = crate::console::config_text(app.world_mut());
        assert!(text.contains(CONFIG_MARK), "{text}");
        assert!(text.contains("bind k +jump\n"), "{text}");

        // A fresh start: the defaults, then the config.
        let mut fresh = App::new();
        fresh.add_plugins(crate::console::ConsolePlugin);
        commands(&mut fresh);
        let mut c = fresh.world_mut().resource_mut::<Console>();
        c.submit("binddefaults");
        for line in text.lines() {
            c.submit(line);
        }
        fresh.update();
        assert_eq!(fresh.world().resource::<Console>().binds, saved);
    }

    #[test]
    fn held_and_pressed_keys_by_command() {
        let b = binds(&[("f", "+duck"), ("mouse4", "+duck"), ("b", "buymenu")]);
        let mut keys = ButtonInput::<KeyCode>::default();
        let mut mouse = ButtonInput::<MouseButton>::default();
        assert!(!pressed(&b, &keys, &mouse, "+duck"));
        mouse.press(MouseButton::Back);
        assert!(pressed(&b, &keys, &mouse, "+duck"));
        keys.press(KeyCode::KeyB);
        assert!(just_pressed(&b, &keys, &mouse, "buymenu"));
        keys.clear();
        mouse.clear();
        assert!(!just_pressed(&b, &keys, &mouse, "buymenu"));
        assert_eq!(first_pressed(&keys, &mouse, None), None);
        keys.press(KeyCode::Numpad5);
        assert_eq!(first_pressed(&keys, &mouse, None), Some("kp_5"));
    }
}
