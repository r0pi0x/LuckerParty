//! Entities that only change what is drawn: sprites (env_sprite,
//! specs/cs_source/sprites_dust.md 1), dust volumes (func_dustmotes,
//! func_dustcloud, same spec 7) and switchable lights (`light`,
//! `light_spot`: a light style the lightmaps hold apart). The map draws
//! them; the logic keeps whether each is on (`LogicWorld::part_states`,
//! `LogicWorld::light_styles`).

use super::classes::Class;
use super::movers::DoorState;
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

/// env_tonemap_controller inputs (the public entity docs' list; the
/// controller drives the local player's HDR camera): SetAutoExposureMin,
/// SetAutoExposureMax, SetBloomScale, SetTonemapRate and
/// UseDefaultAutoExposure. Other inputs of the class are accepted and do
/// nothing yet.
pub(super) fn tonemap_input(w: &mut LogicWorld, input: &str, value: &Value) -> bool {
    let t = &mut w.tonemap;
    let v = value.to_float();
    match input {
        "setautoexposuremin" => {
            t.exposure_min = v;
            t.default_exposure = false;
        }
        "setautoexposuremax" => {
            t.exposure_max = v;
            t.default_exposure = false;
        }
        "setbloomscale" => t.bloom_scale = v,
        "settonemaprate" => t.rate = v,
        "usedefaultautoexposure" => {
            t.exposure_min = None;
            t.exposure_max = None;
            t.default_exposure = true;
        }
        "setbloomscalerange" | "settonemapscale" | "blendtonemapscale" | "usedefaultbloomscale" => {}
        _ => return false,
    }
    true
}

impl LogicWorld {
    /// What env_tonemap_controller inputs have set.
    pub fn tonemap(&self) -> &crate::map::TonemapInputs {
        &self.tonemap
    }

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

    /// The keys of the areaportals that are closed now, sorted: linked
    /// ones by their door (closed while every door of that name is fully
    /// closed; a name matching no door leaves the portal's own state),
    /// the rest by their own state.
    pub fn closed_area_portals(&self) -> Vec<u16> {
        let ids = self.ids();
        let mut closed: Vec<u16> = ids
            .iter()
            .filter_map(|&id| {
                let Class::AreaPortal(p) = &self.get(id)?.class else { return None };
                let doors: Vec<bool> = if p.door.is_empty() {
                    Vec::new()
                } else {
                    ids.iter()
                        .filter_map(|&d| {
                            let e = self.get(d)?;
                            if e.targetname.is_empty() || !super::world::name_matches(&p.door, &e.targetname) {
                                return None;
                            }
                            match &e.class {
                                Class::Door(door) => Some(door.state == DoorState::Closed),
                                Class::PropDoor(door) => Some(door.state == DoorState::Closed),
                                _ => None,
                            }
                        })
                        .collect()
                };
                let open = if doors.is_empty() { p.open } else { doors.iter().any(|closed| !closed) };
                (!open).then_some(p.key)
            })
            .collect();
        closed.sort_unstable();
        closed.dedup();
        closed
    }
}

/// func_areaportal, func_areaportalwindow (public entity documentation):
/// a portal between two of the map's areas, by its compiled
/// `portalnumber`. Linked to a door (`target`), it is closed while that
/// door is fully closed and open otherwise; unlinked, `StartOpen` and the
/// Open/Close/Toggle inputs set it. A window's distance fade is the map's
/// (`map::vis::AreaPortal::fade`), so here it stays open.
#[derive(Clone, Debug)]
pub struct AreaPortal {
    pub key: u16,
    pub open: bool,
    /// The linked door's name (empty: none).
    pub door: String,
}

pub(super) fn spawn_area_portal(w: &LogicWorld, id: EntId) -> Option<AreaPortal> {
    let e = w.get(id).unwrap();
    let key = e.kv("portalnumber")?.trim().parse().ok()?;
    let window = e.classname.eq_ignore_ascii_case("func_areaportalwindow");
    Some(AreaPortal {
        key,
        // StartOpen: open unless it says 0 (an absent key too).
        open: window || e.kv("StartOpen").is_none_or(|v| super::value::atoi(v) != 0),
        door: if window { String::new() } else { e.kv("target").unwrap_or("").to_string() },
    })
}

/// func_occluder (public entity documentation): the occluder its compiled
/// `occludernumber` names hides what lies behind it while active;
/// `StartActive` (absent: 1) and the Activate/Deactivate/Toggle inputs set
/// that (`map::vis::Occluder`).
#[derive(Clone, Debug)]
pub struct Occluder {
    pub key: u16,
    pub active: bool,
}

pub(super) fn spawn_occluder(w: &LogicWorld, id: EntId) -> Option<Occluder> {
    let e = w.get(id).unwrap();
    Some(Occluder {
        key: e.kv("occludernumber")?.trim().parse().ok()?,
        active: e.kv("StartActive").is_none_or(|v| super::value::atoi(v) != 0),
    })
}

/// Occluder inputs; false when the entity isn't one or the input isn't
/// Activate, Deactivate or Toggle.
pub(super) fn occluder_input(w: &mut LogicWorld, id: EntId, input: &str) -> bool {
    let Some(Class::Occluder(o)) = w.get_mut(id).map(|e| &mut e.class) else {
        return false;
    };
    o.active = match input {
        "activate" => true,
        "deactivate" => false,
        "toggle" => !o.active,
        _ => return false,
    };
    true
}

impl LogicWorld {
    /// The keys of the occluders that are off now, sorted.
    pub fn inactive_occluders(&self) -> Vec<u16> {
        let mut off: Vec<u16> = self
            .ids()
            .into_iter()
            .filter_map(|id| match &self.get(id)?.class {
                Class::Occluder(o) if !o.active => Some(o.key),
                _ => None,
            })
            .collect();
        off.sort_unstable();
        off.dedup();
        off
    }
}

/// Areaportal inputs; false when the entity isn't one or the input isn't
/// Open, Close or Toggle.
pub(super) fn area_portal_input(w: &mut LogicWorld, id: EntId, input: &str) -> bool {
    let Some(Class::AreaPortal(p)) = w.get_mut(id).map(|e| &mut e.class) else {
        return false;
    };
    p.open = match input {
        "open" => true,
        "close" => false,
        "toggle" => !p.open,
        _ => return false,
    };
    true
}
