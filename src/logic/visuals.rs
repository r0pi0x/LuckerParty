//! Entities that only change what is drawn: sprites (env_sprite,
//! specs/cs_source/sprites_dust.md 1), dust volumes (func_dustmotes,
//! func_dustcloud, same spec 7) and switchable lights (`light`,
//! `light_spot`: a light style the lightmaps hold apart). The map draws
//! them; the logic keeps whether each is on (`LogicWorld::part_states`,
//! `LogicWorld::light_styles`).

use super::classes::Class;
use super::value::Value;
use super::world::{EntId, LogicWorld};

/// env_sprite spawnflag: starts shown (only matters for a named sprite).
pub const SF_SPRITE_START_ON: u32 = 1;
/// light spawnflag: starts dark.
pub const SF_LIGHT_START_OFF: u32 = 1;
/// The first switchable light style (lower ones are lit always or
/// animated presets).
pub const FIRST_SWITCHABLE_STYLE: i32 = 32;

/// What kind of drawn-only entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Sprite,
    Dust,
}

/// A sprite or dust volume: shown (sprites) or spawning (dust) while on.
#[derive(Clone, Debug)]
pub struct Part {
    pub kind: PartKind,
    pub on: bool,
}

/// A light with a switchable style.
#[derive(Clone, Debug)]
pub struct Light {
    pub style: Option<u8>,
}

pub(super) fn spawn_part(w: &LogicWorld, id: EntId, kind: PartKind) -> Part {
    let e = w.get(id).unwrap();
    let on = match kind {
        // A named sprite without "Start on" starts hidden; unnamed ones are
        // always shown.
        PartKind::Sprite => e.targetname.is_empty() || e.has_flag(SF_SPRITE_START_ON),
        // StartDisabled: its first character '1' starts it off.
        PartKind::Dust => !e.kv("StartDisabled").is_some_and(|v| v.trim_start().starts_with('1')),
    };
    Part { kind, on }
}

/// A light's style (32..=63 switchable) and its starting state in the
/// world's style table.
pub(super) fn spawn_light(w: &mut LogicWorld, id: EntId) -> Light {
    let e = w.get(id).unwrap();
    let style = e
        .kv("style")
        .map(super::value::atoi)
        .filter(|s| (FIRST_SWITCHABLE_STYLE..=255).contains(s))
        .map(|s| s as u8);
    let on = !e.has_flag(SF_LIGHT_START_OFF);
    if let Some(s) = style {
        w.set_light_style(s, on);
    }
    Light { style }
}

/// Sprite, dust and light inputs; false when not one of them.
pub(super) fn input(w: &mut LogicWorld, id: EntId, input: &str, _value: &Value) -> bool {
    match w.get(id).map(|e| e.class.clone()) {
        Some(Class::Part(p)) => {
            let on = match (p.kind, input) {
                (PartKind::Sprite, "showsprite") | (PartKind::Dust, "turnon") => true,
                (PartKind::Sprite, "hidesprite") | (PartKind::Dust, "turnoff") => false,
                (PartKind::Sprite, "togglesprite") => !p.on,
                _ => return false,
            };
            if let Some(Class::Part(p)) = w.get_mut(id).map(|e| &mut e.class) {
                p.on = on;
            }
            true
        }
        Some(Class::Light(l)) => {
            let Some(style) = l.style else {
                // Not switchable: the inputs exist but do nothing.
                return matches!(input, "turnon" | "turnoff" | "toggle");
            };
            let on = match input {
                "turnon" => true,
                "turnoff" => false,
                "toggle" => !w.light_style(style),
                _ => return false,
            };
            w.set_light_style(style, on);
            true
        }
        _ => false,
    }
}

impl LogicWorld {
    /// Sprites and dust volumes from the map, in entity order: (map
    /// index, on). Removed ones are missing.
    pub fn part_states(&self) -> Vec<(usize, PartKind, bool)> {
        self.ids()
            .into_iter()
            .filter_map(|id| {
                let e = self.get(id)?;
                let Class::Part(p) = &e.class else { return None };
                Some((e.map_index?, p.kind, p.on))
            })
            .collect()
    }

    /// A switchable light style's state (styles no light set are on).
    pub fn light_style(&self, style: u8) -> bool {
        self.light_styles
            .iter()
            .find(|(s, _)| *s == style)
            .is_none_or(|(_, on)| *on)
    }

    /// Set a light style on or off (every light sharing it follows).
    pub fn set_light_style(&mut self, style: u8, on: bool) {
        match self.light_styles.iter_mut().find(|(s, _)| *s == style) {
            Some(s) => s.1 = on,
            None => self.light_styles.push((style, on)),
        }
    }

    /// The switchable light styles lights set, in the order first set.
    pub fn light_styles(&self) -> &[(u8, bool)] {
        &self.light_styles
    }
}
