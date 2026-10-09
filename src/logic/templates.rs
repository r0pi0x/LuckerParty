//! point_template and env_entity_maker (specs/source/
//! viewcontrol_and_templates.md 2-3): a template keeps the map text of
//! the entities it names (taking them out of the map unless told not to)
//! and spawns copies of them on ForceSpawn, placed relative to itself (or
//! to the maker), names fixed up per copy so each copy's members talk to
//! each other. Copies of brush entities and props need their node copied
//! too: `Effect::Spawned` asks the host.

use bevy::prelude::*;

use super::anchors::angles_of;
use super::classes::Class;
use super::value::Value;
use super::world::{Effect, EntId, LogicWorld, Who, name_matches};
use crate::map::MapHull;
use crate::map::entities::{entity_rotation, parse_vector};

/// point_template spawnflags.
pub const SF_TEMPLATE_KEEP: u32 = 1;
pub const SF_TEMPLATE_NO_FIXUP: u32 = 2;
/// The name fix-up marker every copy's suffix replaces.
pub const FIXUP_MARKER: &str = "&0000";
/// The instance counter wraps here.
pub const FIXUP_WRAP: u32 = 10_000;
/// Template slots (Template01..Template16).
pub const TEMPLATE_SLOTS: usize = 16;
/// env_entity_maker's autospawn re-check, seconds.
pub const MAKER_PERIOD: f32 = 0.5;

/// One stored member: its map text (fix-up markers inserted), its
/// volumes, the map entity it came from and its place relative to the
/// template when the map loaded.
#[derive(Clone, Debug)]
pub struct Member {
    pub keyvalues: Vec<(String, String)>,
    pub hulls: Vec<MapHull>,
    pub source: usize,
    pub offset: Vec3,
    pub rotation: Quat,
    /// Fix-up applies to its text.
    pub fixup: bool,
}

/// A point_template's members.
#[derive(Clone, Debug, Default)]
pub struct Template {
    pub members: Vec<Member>,
}

/// An env_entity_maker: the instance it made last ("current instance"
/// and "blocker", with where the blocker was), and the first instance's
/// box relative to its origin.
#[derive(Clone, Debug, Default)]
pub struct Maker {
    pub current: Option<EntId>,
    pub blocker: Option<(EntId, Vec3)>,
    pub spawn_box: Option<(Vec3, Vec3)>,
}

/// env_entity_maker spawnflags.
pub const SF_MAKER_AUTOSPAWN: u32 = 1;
pub const SF_MAKER_WAIT_DESTROY: u32 = 2;
pub const SF_MAKER_EVEN_LOOKING: u32 = 4;
pub const SF_MAKER_ROOM: u32 = 8;
pub const SF_MAKER_NOT_LOOKING: u32 = 16;

/// A value's first field (before a comma or ESC) and the rest.
fn first_field(v: &str) -> (&str, &str) {
    match v.find([',', '\u{1b}']) {
        Some(i) => (&v[..i], &v[i..]),
        None => (v, ""),
    }
}

/// Prepare a group's name fix-up (2.1 step 4): values (other than
/// targetname) whose first field names a member get the marker after the
/// name, and the members so referenced get it on their own name.
pub fn prepare_fixup(members: &mut [Member]) {
    let names: Vec<String> = members
        .iter()
        .map(|m| kv(&m.keyvalues, "targetname").unwrap_or("").to_ascii_lowercase())
        .collect();
    let mut rename = vec![false; members.len()];
    for m in members.iter_mut() {
        for (k, v) in m.keyvalues.iter_mut() {
            if k.eq_ignore_ascii_case("targetname") {
                continue;
            }
            let (first, rest) = first_field(v);
            let lower = first.to_ascii_lowercase();
            if first.is_empty() {
                continue;
            }
            let Some(j) = names.iter().position(|n| *n == lower) else { continue };
            rename[j] = true;
            *v = format!("{first}{FIXUP_MARKER}{rest}");
            m.fixup = true;
        }
    }
    for (m, r) in members.iter_mut().zip(rename) {
        if !r {
            continue;
        }
        m.fixup = true;
        if let Some((_, v)) = m.keyvalues.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case("targetname")) {
            v.push_str(FIXUP_MARKER);
        }
    }
}

/// Replace every "&" followed by four digits with `suffix`.
pub fn apply_suffix(text: &str, suffix: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' && i + 5 <= b.len() && b[i + 1..i + 5].iter().all(u8::is_ascii_digit) {
            out.push_str(suffix);
            i += 5;
            continue;
        }
        let c = text[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn kv<'a>(keyvalues: &'a [(String, String)], key: &str) -> Option<&'a str> {
    keyvalues
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

impl LogicWorld {
    /// After a map's entities spawned (2.1): each point_template collects
    /// its members' map text and places, and takes them out of the map
    /// unless flag 1. Runs before activation.
    pub(super) fn collect_templates(&mut self, entities: &[crate::map::MapEntity], ids: &[EntId]) {
        let templates: Vec<EntId> = ids
            .iter()
            .copied()
            .filter(|id| matches!(self.get(*id).map(|e| &e.class), Some(Class::Template(_))))
            .collect();
        for t in templates {
            let Some(te) = self.get(t) else { continue };
            let (to, ta, flags) = (te.origin, te.angles, te.spawnflags);
            let slots: Vec<String> = (1..=TEMPLATE_SLOTS)
                .filter_map(|i| te.kv(&format!("Template{i:02}")).map(str::trim).map(String::from))
                .filter(|n| !n.is_empty())
                .collect();
            let trot = entity_rotation(ta);
            let mut members = Vec::new();
            let mut taken = Vec::new();
            for name in &slots {
                let mut found = false;
                for (i, e) in entities.iter().enumerate() {
                    let Some(n) = e.get("targetname").filter(|n| !n.is_empty()) else {
                        continue;
                    };
                    if !name_matches(name, n) || taken.contains(&i) {
                        continue;
                    }
                    found = true;
                    taken.push(i);
                    members.push(Member {
                        keyvalues: e.keyvalues.clone(),
                        hulls: e.hulls.clone(),
                        source: i,
                        offset: trot.inverse() * (e.origin() - to),
                        rotation: trot.inverse() * entity_rotation(e.angles()),
                        fixup: false,
                    });
                }
                if !found {
                    self.log.push(format!("point_template: no entity '{name}'"));
                }
            }
            if flags & SF_TEMPLATE_NO_FIXUP == 0 {
                prepare_fixup(&mut members);
            }
            if flags & SF_TEMPLATE_KEEP == 0 {
                for &i in &taken {
                    if let Some(id) = ids.get(i).copied() {
                        self.remove_now(id);
                    }
                }
            }
            if let Some(Class::Template(tp)) = self.get_mut(t).map(|e| &mut e.class) {
                tp.members = members;
            }
            // The counter also advances once per template at precache.
            self.template_serial = (self.template_serial + 1) % FIXUP_WRAP;
        }
    }

    /// Take an entity out at once (never existed: no removal effects).
    pub(super) fn remove_now(&mut self, id: EntId) {
        if self.get(id).is_some() {
            self.forget(id);
        }
    }

    /// Spawn a copy of a template's members (2.2) placed relative to
    /// `at` (origin, angles; the template's own pose when None). Returns
    /// the new entities (none: nothing to copy).
    pub fn spawn_template(&mut self, template: EntId, at: Option<(Vec3, Vec3)>) -> Vec<EntId> {
        let Some(te) = self.get(template) else { return Vec::new() };
        let Class::Template(t) = &te.class else { return Vec::new() };
        if t.members.is_empty() {
            return Vec::new();
        }
        let members = t.members.clone();
        let no_fixup = te.has_flag(SF_TEMPLATE_NO_FIXUP);
        let (origin, angles) = at.unwrap_or((te.origin, te.angles));
        self.template_serial = (self.template_serial + 1) % FIXUP_WRAP;
        let suffix = format!("&{:04}", self.template_serial);
        let rot = entity_rotation(angles);
        let mut new = Vec::new();
        for m in &members {
            let o = origin + rot * m.offset;
            let r = rot * m.rotation;
            let a = angles_of(r);
            let mut keyvalues: Vec<(String, String)> = m
                .keyvalues
                .iter()
                .map(|(k, v)| {
                    let v = if m.fixup && !no_fixup { apply_suffix(v, &suffix) } else { v.clone() };
                    (k.clone(), v)
                })
                .collect();
            set_kv(&mut keyvalues, "origin", &format!("{} {} {}", o.x, o.y, o.z));
            set_kv(&mut keyvalues, "angles", &format!("{} {} {}", a.x, a.y, a.z));
            let id = self.spawn(&keyvalues, m.hulls.clone());
            new.push((id, m.source, o, a));
        }
        for &(id, ..) in &new {
            super::classes::class_activate(self, id);
        }
        for &(id, source, origin, angles) in &new {
            self.effects.push(Effect::Spawned {
                id,
                source,
                origin,
                angles,
            });
        }
        new.into_iter().map(|(id, ..)| id).collect()
    }

    /// The first point_template named `name`.
    fn template_named(&self, name: &str) -> Option<EntId> {
        self.ids().into_iter().find(|id| {
            self.get(*id).is_some_and(|e| {
                matches!(e.class, Class::Template(_)) && !e.targetname.is_empty() && name_matches(name, &e.targetname)
            })
        })
    }
}

fn set_kv(keyvalues: &mut Vec<(String, String)>, key: &str, value: &str) {
    match keyvalues.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
        Some(kv) => kv.1 = value.to_string(),
        None => keyvalues.push((key.to_string(), value.to_string())),
    }
}

/// point_template and env_entity_maker inputs; false when not handled.
pub(super) fn input(w: &mut LogicWorld, id: EntId, input: &str, value: &Value, activator: Option<Who>, caller: Option<Who>) -> bool {
    let Some(class) = w.get(id).map(|e| e.class.clone()) else { return true };
    match (class, input) {
        (Class::Template(_), "forcespawn") => {
            if !w.spawn_template(id, None).is_empty() {
                w.fire_output(id, "OnEntitySpawned", Some(Who::Ent(id)), Value::Void);
            }
        }
        (Class::Maker(_), "forcespawn") => maker_force(w, id),
        (Class::Maker(_), "forcespawnatentityorigin") => {
            let name = value.as_text(|x| w.name_of(x));
            let at = w
                .resolve(name.trim(), activator, caller)
                .into_iter()
                .find_map(|t| match t {
                    Who::Ent(e) => w.get(e).map(|e| (e.origin, e.angles)),
                    Who::Player(p) => w.player(p).map(|p| (p.origin, Vec3::new(0.0, p.view.y, 0.0))),
                });
            if let Some(at) = at {
                maker_spawn(w, id, Some(at));
            }
        }
        _ => return false,
    }
    true
}

fn maker(w: &mut LogicWorld, id: EntId) -> Option<&mut Maker> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Maker(m)) => Some(m),
        _ => None,
    }
}

/// env_entity_maker activation: without a template name it removes
/// itself; with autospawn, one spawn now and checks every 0.5 s.
pub(super) fn maker_activate(w: &mut LogicWorld, id: EntId) {
    let Some(e) = w.get(id) else { return };
    if e.kv("EntityTemplate").is_none_or(|t| t.trim().is_empty()) {
        w.kill(id);
        return;
    }
    if e.has_flag(SF_MAKER_AUTOSPAWN) {
        maker_spawn(w, id, None);
        w.think_in(id, MAKER_PERIOD);
    }
}

/// The autospawn check (3, "Autospawn").
pub(super) fn maker_think(w: &mut LogicWorld, id: EntId) {
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    if flags & SF_MAKER_AUTOSPAWN == 0 {
        return;
    }
    w.think_in(id, MAKER_PERIOD);
    let current = maker(w, id).and_then(|m| m.current);
    if flags & SF_MAKER_WAIT_DESTROY != 0 && current.is_some_and(|c| w.get(c).is_some()) {
        return;
    }
    if !has_room(w, id) {
        return;
    }
    if flags & SF_MAKER_EVEN_LOOKING == 0 && someone_looking(w, id) {
        return;
    }
    maker_spawn(w, id, None);
}

fn maker_force(w: &mut LogicWorld, id: EntId) {
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let name = w.get(id).and_then(|e| e.kv("EntityTemplate")).unwrap_or("").to_string();
    if w.template_named(name.trim()).is_none() {
        return;
    }
    let me = Some(Who::Ent(id));
    if (flags & SF_MAKER_ROOM != 0 && !has_room(w, id)) || (flags & SF_MAKER_NOT_LOOKING != 0 && someone_looking(w, id)) {
        w.fire_output(id, "OnEntityFailedSpawn", me, Value::Void);
        return;
    }
    maker_spawn(w, id, None);
}

/// Spawn the maker's template at `at` (the maker's own pose when None).
fn maker_spawn(w: &mut LogicWorld, id: EntId, at: Option<(Vec3, Vec3)>) {
    let Some(e) = w.get(id) else { return };
    let name = e.kv("EntityTemplate").unwrap_or("").trim().to_string();
    let pose = at.unwrap_or((e.origin, e.angles));
    let (speed, dir, variance, inherit) = (
        e.kv_f("PostSpawnSpeed"),
        e.kv("PostSpawnDirection").map_or(Vec3::ZERO, parse_vector),
        e.kv_f("PostSpawnDirectionVariance"),
        e.kv_i("PostSpawnInheritAngles") != 0,
    );
    let maker_angles = e.angles;
    let Some(t) = w.template_named(&name) else { return };
    let new = w.spawn_template(t, Some(pose));
    let Some(first) = new.first().copied() else { return };
    let first_origin = w.get(first).map_or(pose.0, |e| e.origin);
    let first_box = w.get(first).and_then(|e| entity_box(&e.hulls));
    if let Some(m) = maker(w, id) {
        m.current = Some(first);
        m.blocker = Some((first, first_origin));
        if m.spawn_box.is_none() {
            m.spawn_box = first_box;
        }
    }
    w.fire_output(id, "OnEntitySpawned", Some(Who::Ent(id)), Value::Void);
    if speed != 0.0 {
        let base = if inherit { dir + maker_angles } else { dir };
        let rot = entity_rotation(base);
        let (f, r, u) = (rot * Vec3::X, rot * Vec3::NEG_Y, rot * Vec3::Z);
        for e in new {
            let mut u3 = || w.random() * 2.0 - 1.0;
            let (a, b, c) = (u3(), u3(), u3());
            let d = (f + r * a * variance + f * b * variance + u * c * variance).normalize_or_zero();
            w.effects.push(Effect::BodyVelocity { id: e, velocity: d * speed });
        }
    }
}

/// The local box of a brush entity's volumes.
fn entity_box(hulls: &[MapHull]) -> Option<(Vec3, Vec3)> {
    let mut lo = Vec3::MAX;
    let mut hi = Vec3::MIN;
    for p in hulls.iter().flat_map(|h| &h.points) {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    (lo.x <= hi.x).then_some((lo, hi))
}

/// The room test (3): no room while the blocker sits exactly where it
/// was made; else the spawn box at the maker must touch nothing solid
/// (the static world, movers, players).
fn has_room(w: &mut LogicWorld, id: EntId) -> bool {
    let Some(m) = maker(w, id).cloned() else { return true };
    if let Some((b, at)) = m.blocker
        && w.get(b).is_some_and(|e| e.origin == at)
    {
        return false;
    }
    let Some((lo, hi)) = m.spawn_box else { return true };
    let origin = w.get(id).map_or(Vec3::ZERO, |e| e.origin);
    let (a, b) = (origin + lo, origin + hi);
    if w.players.iter().any(|p| {
        let (pa, pb) = (p.origin + p.mins, p.origin + p.maxs);
        pa.cmplt(b).all() && pb.cmpgt(a).all()
    }) {
        return false;
    }
    if w.collision.as_ref().is_some_and(|c| c.solid(lo, hi, origin)) {
        return false;
    }
    true
}

/// Whether any player has the maker in front of its eye plane (3).
fn someone_looking(w: &LogicWorld, id: EntId) -> bool {
    let Some(at) = w.get(id).map(|e| e.origin) else { return false };
    w.players.iter().any(|p| {
        let eye = p.origin + p.eye;
        p.forward().dot((at - eye).normalize_or_zero()) > 0.0
    })
}
