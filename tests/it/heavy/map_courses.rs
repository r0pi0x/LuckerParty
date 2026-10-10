//! Course maps (surf_, bhop_, kz_) and gungame arenas (gg_) from the
//! user's content cache played headless (docs/plans/active/community-maps.md,
//! "Course flows"): the whole game with one scripted player per team, every
//! mechanism the map's entities set up checked against what a player meets:
//! spawns, fail teleports landing on their destination (velocity kept, view
//! snapped), stage checkpoints (a trigger naming the player, the stage's
//! fail teleport filtered by that name), bhop blocks (doors that drop when
//! stood on; "multihop" triggers naming the player a moment after landing
//! under a filtered teleport), boosters (trigger_push, AddOutput
//! basevelocity), gravity and speed zones, what triggers set off, buttons,
//! and the next round. Each step is a row: `MASHUP_FLOW_TABLE=1 cargo test
//! --features dev --test it map_courses:: -- --nocapture`; each test skips
//! when its map isn't in the content cache.

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use mashup::{
    core::{BaseVelocity, EntityGravity, God, MapControls, MovementState, Velocity},
    games::cs_source::{self, movement::to_source},
    logic::{
        EntId, Logic, Who,
        classes::Class,
        triggers::{self, Kind},
        world::LogicWorld,
    },
};

use super::map_flows::{Flow, near, next_round};

const UNIT_SCALE: f32 = cs_source::bsp::METERS_PER_UNIT;

/// A step's tally: how many of its items passed, the first failures.
#[derive(Default)]
struct Tally {
    total: usize,
    failed: Vec<String>,
    /// Items that fail for a known reason (another session's, a map bug).
    known: Vec<String>,
}

impl Tally {
    fn add(&mut self, ok: bool, what: impl FnOnce() -> String) {
        self.total += 1;
        if !ok {
            self.failed.push(what());
        }
    }
    fn known(&mut self, what: String) {
        self.total += 1;
        self.known.push(what);
    }
    fn row(self, f: &mut Flow, step: &str) {
        if self.total == 0 {
            return;
        }
        let pass = self.total - self.failed.len() - self.known.len();
        let mut detail = format!("{pass}/{}", self.total);
        if !self.known.is_empty() {
            detail += &format!(
                "; known ({}): {}",
                self.known.len(),
                self.known[..self.known.len().min(3)].join("; ")
            );
        }
        if !self.failed.is_empty() {
            detail += &format!(": {}", self.failed[..self.failed.len().min(4)].join("; "));
        }
        f.check(step, self.failed.is_empty(), detail);
    }
}

fn world(f: &Flow) -> &LogicWorld {
    &f.sim.app.world().resource::<Logic>().world
}

fn world_mut(f: &mut Flow) -> Mut<'_, LogicWorld> {
    f.sim
        .app
        .world_mut()
        .resource_mut::<Logic>()
        .map_unchanged(|l| &mut l.world)
}

fn trigger(w: &LogicWorld, id: EntId) -> Option<&triggers::Trigger> {
    match w.get(id).map(|e| &e.class) {
        Some(Class::Trigger(t)) => Some(t),
        _ => None,
    }
}

/// A trigger's volume: its bounds, and a point inside its biggest brush.
fn volume(w: &LogicWorld, id: EntId) -> Option<(Vec3, Vec3, Vec3)> {
    let t = trigger(w, id)?;
    let lo = t.brushes.iter().fold(Vec3::MAX, |a, b| a.min(b.min));
    let hi = t.brushes.iter().fold(Vec3::MIN, |a, b| a.max(b.max));
    let big = t.brushes.iter().max_by(|a, b| {
        (a.max - a.min)
            .element_product()
            .total_cmp(&(b.max - b.min).element_product())
    })?;
    Some((lo, hi, (big.min + big.max) / 2.0))
}

/// Whether a standing player's box with feet at `feet` (Source units)
/// is inside the world's brushes or terrain.
fn blocked(f: &mut Flow, feet: Vec3) -> bool {
    // Props and other bodies the movement collides with.
    let centre = cs_source::movement::to_engine(feet + Vec3::Z * 31.0);
    let bodies = f
        .sim
        .app
        .world_mut()
        .run_system_once(
            move |q: avian3d::prelude::SpatialQuery, chars: Query<Entity, With<mashup::core::Intent>>| {
                // A unit in from each side: brush colliders just touching
                // the box count as hits.
                let skin = 2.0 * cs_source::bsp::METERS_PER_UNIT;
                let shape = avian3d::prelude::Collider::cuboid(
                    32.0 * cs_source::bsp::METERS_PER_UNIT - skin,
                    62.0 * cs_source::bsp::METERS_PER_UNIT - skin,
                    32.0 * cs_source::bsp::METERS_PER_UNIT - skin,
                );
                let filter = avian3d::prelude::SpatialQueryFilter::from_excluded_entities(chars.iter())
                    .with_mask(mashup::core::SOLID_LAYERS);
                !q.shape_intersections(&shape, centre, Quat::IDENTITY, &filter)
                    .is_empty()
            },
        )
        .unwrap_or(false);
    if bodies {
        return true;
    }
    let world = f.sim.app.world();
    let brushes = &world.resource::<mashup::core::MapBrushes>().0;
    let terrain = world.get_resource::<mashup::core::MapTerrain>();
    let c = cs_source::movement::to_engine(feet + Vec3::Z * 31.0);
    let h = Vec3::new(16.0, 31.0, 16.0) * cs_source::bsp::METERS_PER_UNIT;
    // Brush entities (doors, bhop blocks, func_brush) where they are now.
    let movers: Vec<mashup::core::MapBrush> = world
        .try_query::<&mashup::core::MovingSolid>()
        .map(|mut q| {
            q.iter(world)
                .filter(|m| m.solid)
                .flat_map(|m| m.brushes.clone())
                .collect()
        })
        .unwrap_or_default();
    brushes
        .iter()
        .chain(terrain.iter().flat_map(|t| t.brushes.iter()))
        .chain(movers.iter())
        .any(|b| {
            b.max.cmpgt(c - h).all()
                && b.min.cmplt(c + h).all()
                && b.planes.iter().all(|(n, d)| n.dot(c) - (d + n.abs().dot(h)) < -1e-6)
        })
}

/// Feet for a player standing in trigger `id`: its box overlapping the
/// trigger and free of the world (a trigger hugging a floor or wrapping
/// a block has its centre in the solid), lowest first.
fn feet_in(f: &mut Flow, id: EntId) -> Option<Vec3> {
    let w = world(f);
    let t = trigger(w, id)?;
    let mut brushes: Vec<mashup::core::MapBrush> = t.brushes.clone();
    brushes.sort_by(|a, b| {
        (b.max - b.min)
            .element_product()
            .total_cmp(&(a.max - a.min).element_product())
    });
    for b in brushes.iter().take(4) {
        let c = (b.min + b.max) / 2.0;
        // Up through the trigger, then below it (the box reaches 72 up).
        // (A thin trigger on a floor: feet just inside its slab.)
        let mut zs: Vec<f32> = vec![b.min.z + 1.0, (b.min.z + b.max.z) / 2.0, b.max.z - 0.25];
        let mut z = b.min.z + 9.0;
        while z < b.max.z - 1.0 && zs.len() < 64 {
            zs.push(z);
            z += 8.0;
        }
        zs.extend((1..18).map(|k| b.min.z + 1.0 - k as f32 * 4.0));
        for z in zs {
            let feet = Vec3::new(c.x, c.y, z);
            // With a margin: the logic tests the box from the transform, a
            // float off this one (a slanted face near its tip misses then).
            let inside = mashup::logic::world::box_touches(
                b,
                feet + Vec3::new(-14.0, -14.0, 0.1),
                feet + Vec3::new(14.0, 14.0, 61.9),
            );
            if inside && !blocked(f, feet) {
                return Some(feet);
            }
        }
    }
    None
}

/// Whether the standing box at `feet` overlaps brush `b` (entity space).
fn brushes_overlap(b: &mashup::core::MapBrush, feet: Vec3) -> bool {
    mashup::logic::world::box_touches(
        b,
        feet + Vec3::new(-16.0, -16.0, 0.0),
        feet + Vec3::new(16.0, 16.0, 62.0),
    )
}

/// Bounds of an entity's brushes where it is now (doors).
fn brush_bounds(f: &Flow, id: EntId) -> Option<(Vec3, Vec3)> {
    let w = world(f);
    let e = w.get(id)?;
    let (origin, angles) =
        mashup::logic::movers::pusher(&e.class).map_or((e.origin, e.angles), |p| (p.origin, p.angles));
    let rot = mashup::map::entities::entity_rotation(angles);
    let mut lo = Vec3::MAX;
    let mut hi = Vec3::MIN;
    for h in &e.hulls {
        let b = mashup::logic::world::place_hull(h, rot, origin);
        lo = lo.min(b.min);
        hi = hi.max(b.max);
    }
    (lo.x <= hi.x).then_some((lo, hi))
}

fn classname(w: &LogicWorld, id: EntId) -> String {
    w.get(id).map(|e| e.classname.to_ascii_lowercase()).unwrap_or_default()
}

/// Every output connection of an entity: (output, target, input, param, delay).
fn connections(w: &LogicWorld, id: EntId) -> Vec<(String, String, String, String, f32)> {
    w.get(id)
        .map(|e| {
            e.outputs
                .iter()
                .flat_map(|(o, cs)| {
                    cs.iter().map(move |c| {
                        (
                            o.to_ascii_lowercase(),
                            c.target.clone(),
                            c.input.to_ascii_lowercase(),
                            c.param.clone().unwrap_or_default(),
                            c.delay,
                        )
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// AddOutput `key value` sent to `!activator` by `id`: (output, value, delay).
fn activator_sets(w: &LogicWorld, id: EntId, key: &str) -> Vec<(String, String, f32)> {
    connections(w, id)
        .into_iter()
        .filter(|(_, t, i, _, _)| t.eq_ignore_ascii_case("!activator") && i == "addoutput")
        .filter_map(|(o, _, _, p, d)| {
            let (k, v) = p.trim().split_once(' ')?;
            k.eq_ignore_ascii_case(key).then(|| (o, v.trim().to_string(), d))
        })
        .collect()
}

/// The logic's player list is the characters of the tick it ran (taken
/// back afterwards); filters and `!activator` between ticks need `p` in
/// it.
fn present(f: &mut Flow, p: Entity) {
    let feet = f.feet(p);
    let team = f.sim.app.world().get::<mashup::core::Team>(p).map_or(0, |t| t.0 + 1);
    let mut w = world_mut(f);
    if w.player(p).is_none() {
        let mut pl = mashup::logic::world::Player::new(p, feet);
        // CS:S's standing hull: 62 high.
        pl.maxs.z = 62.0;
        pl.team = team;
        w.players.push(pl);
    }
}

/// The player's name and class as the logic sees them.
fn set_identity(f: &mut Flow, p: Entity, name: &str, class: &str) {
    present(f, p);
    let mut w = world_mut(f);
    w.player_names.retain(|(e, _)| *e != p);
    if !name.is_empty() {
        w.player_names.push((p, name.to_string()));
    }
    w.player_classes.retain(|(e, _)| *e != p);
    if !class.is_empty() && !class.eq_ignore_ascii_case("player") {
        w.player_classes.push((p, class.to_string()));
    }
}

fn name_of(f: &Flow, p: Entity) -> String {
    world(f).name_of(Who::Player(p))
}

/// A name and class the player can take that lets it through trigger
/// `id` (names and classes the map gives players, then none); the empty
/// pair first when it passes as it is.
fn identity_for(f: &mut Flow, p: Entity, id: EntId, names: &[String], classes: &[String]) -> Option<(String, String)> {
    let mut tries = vec![(String::new(), String::new())];
    tries.extend(names.iter().map(|n| (n.clone(), String::new())));
    tries.extend(classes.iter().map(|c| (String::new(), c.clone())));
    for n in names {
        for c in classes {
            tries.push((n.clone(), c.clone()));
        }
    }
    let found = tries.into_iter().find(|(n, c)| {
        set_identity(f, p, n, c);
        triggers::passes(world(f), id, Who::Player(p))
    });
    set_identity(f, p, "", "");
    found
}

fn velocity(f: &Flow, p: Entity) -> Vec3 {
    to_source(f.sim.app.world().get::<Velocity>(p).unwrap().0)
}

fn set_velocity(f: &mut Flow, p: Entity, v: Vec3) {
    f.sim.app.world_mut().get_mut::<Velocity>(p).unwrap().0 = cs_source::movement::to_engine(v);
}

fn on_ground(f: &Flow, p: Entity) -> bool {
    f.sim.app.world().get::<MovementState>(p).is_some_and(|s| s.on_ground)
}

fn gravity(f: &Flow, p: Entity) -> f32 {
    f.sim.app.world().get::<EntityGravity>(p).map_or(1.0, |g| g.0)
}

/// The player's view (pitch, yaw) in degrees, Source angles.
fn view(f: &mut Flow, p: Entity) -> (f32, f32) {
    let i = f.sim.intent(p);
    (-i.pitch.to_degrees(), (i.yaw.to_degrees() + 90.0).rem_euclid(360.0))
}

fn yaw_close(a: f32, b: f32) -> bool {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d) < 2.0
}

/// Ticks of `f`'s map.
fn ticks(f: &mut Flow, n: u32) {
    for _ in 0..n {
        f.secs(1.0 / 66.0);
    }
}

/// Undo what one check left on the player: name, class, gravity, base
/// velocity, speed; standing still at `home`.
fn reset(f: &mut Flow, p: Entity, home: Vec3) {
    set_identity(f, p, "", "");
    {
        let mut w = world_mut(f);
        if let Some(pl) = w.player_mut(p) {
            pl.gravity = 1.0;
            pl.base_velocity = Vec3::ZERO;
            pl.base_touched = false;
        }
    }
    let world = f.sim.app.world_mut();
    world
        .entity_mut(p)
        .insert((EntityGravity(1.0), BaseVelocity::default()));
    if let Some(mut c) = world.get_mut::<MapControls>(p) {
        c.time_scale = 1.0;
        c.disabled.clear();
    }
    if let Some(mut h) = world.get_mut::<mashup::core::Health>(p) {
        h.current = h.max;
    }
    // Out of every trigger (high above the map: the spawn may be in a
    // start zone, which then wouldn't see the next entry as a start
    // touch), until the logic's queued name changes have run, then clear
    // them.
    let limbo = home + Vec3::Z * 30_000.0;
    f.put(p, limbo);
    ticks(f, 2);
    f.secs(0.3);
    set_identity(f, p, "", "");
    f.put(p, limbo);
    ticks(f, 1);
}

/// Where the trigger `id` sends a player standing in it: the destination
/// point (feet) and view angles, and whether a landmark keeps the offset.
fn destination(f: &mut Flow, id: EntId, p: Entity) -> Option<(Vec3, Vec3, Option<Vec3>)> {
    present(f, p);
    let w = world(f);
    let Kind::Teleport { target, landmark } = &trigger(w, id)?.kind else {
        return None;
    };
    let dest = w
        .resolve(target, Some(Who::Player(p)), Some(Who::Ent(id)))
        .into_iter()
        .next()?;
    let Who::Ent(d) = dest else { return None };
    let e = w.get(d)?;
    let mark = (!landmark.is_empty())
        .then(|| w.find(landmark))
        .flatten()
        .and_then(|l| w.get(l).map(|e| e.origin));
    Some((e.origin, e.angles, mark))
}

/// Everything a course map sets up, checked (see the module comment).
/// The test player's velocity after this tick's movement, before the
/// logic's triggers run (what a teleport must keep).
#[derive(Resource, Default)]
struct MovedVelocity(Vec3);

fn record_moved(mut out: ResMut<MovedVelocity>, q: Query<&Velocity, With<God>>) {
    if let Some(v) = q.iter().next() {
        out.0 = to_source(v.0);
    }
}

fn play(map: &'static str) {
    let Some(mut f) = Flow::new(map, 1, 0) else { return };
    f.sim.app.init_resource::<MovedVelocity>().add_systems(
        FixedUpdate,
        record_moved
            .after(mashup::core::SimSet::Movement)
            .before(mashup::logic::LogicSet::Post),
    );
    let all = f.all();
    let p = all[0];
    // Spawns: alive, standing within 2 s, not sent away by a trigger.
    f.secs(1.0);
    let homes: Vec<Vec3> = all.iter().map(|q| f.feet(*q)).collect();
    f.secs(1.0);
    let mut spawn = Tally::default();
    for (q, home) in all.iter().zip(&homes) {
        let ok = f.alive(*q) && on_ground(&f, *q) && near(f.feet(*q), *home, 48.0);
        let at = f.feet(*q);
        spawn.add(ok, || format!("{q} at {at:.0}, ground {}", on_ground(&f, *q)));
    }
    spawn.row(&mut f, "spawns: alive, standing");
    // Spawn equipment: the map's game_player_equip (not "Use Only"), and
    // its placed weapons lying where it puts them.
    let ents = f
        .sim
        .app
        .world()
        .resource::<mashup::map::entities::MapEntities>()
        .entities
        .clone();
    let kit: Vec<String> = ents
        .iter()
        .filter(|e| {
            e.classname() == "game_player_equip"
                && e.get("spawnflags").and_then(|s| s.parse::<u32>().ok()).unwrap_or(0) & 1 == 0
        })
        .flat_map(|e| e.keyvalues.iter().map(|(k, _)| k.to_ascii_lowercase()))
        .filter(|k| k.starts_with("weapon_"))
        .collect();
    if !kit.is_empty() {
        let has = f.weapons(all[0]);
        let ok = kit.iter().all(|k| has.contains(k));
        f.check(
            "spawn equipment (game_player_equip)",
            ok,
            format!("has {has:?}, map gives {kit:?}"),
        );
    }
    let placed = ents.iter().filter(|e| e.classname().starts_with("weapon_")).count();
    if placed > 0 {
        let world = f.sim.app.world_mut();
        let lying = world
            .query::<(&mashup::weapon::drop::Loose, &mashup::weapon::equip::MapWeapon)>()
            .iter(world)
            .count();
        f.check(
            "placed weapons lie where the map puts them",
            lying == placed,
            format!("{lying}/{placed}"),
        );
    }
    let home = f.feet(p);
    for q in &all {
        f.sim.app.world_mut().entity_mut(*q).insert(God);
    }

    // What the map names players (checkpoints) and calls them (classes).
    let ids = world(&f).ids();
    let mut names: Vec<String> = Vec::new();
    let mut classes: Vec<String> = Vec::new();
    for id in &ids {
        for (_, v, _) in activator_sets(world(&f), *id, "targetname") {
            if !names.contains(&v) {
                names.push(v);
            }
        }
        for (_, v, _) in activator_sets(world(&f), *id, "classname") {
            if !classes.contains(&v) {
                classes.push(v);
            }
        }
    }

    teleports(&mut f, p, home, &names, &classes);
    checkpoints(&mut f, p, home, &names, &classes);
    bhop_blocks(&mut f, p, home);
    pushes(&mut f, p, home, &names, &classes);
    keyvalue_zones(&mut f, p, home);
    trigger_outputs(&mut f, p, home, &names, &classes);
    buttons(&mut f, p, home);

    for q in &all {
        f.sim.app.world_mut().entity_mut(*q).remove::<God>();
    }
    next_round(&mut f, &all);
    let failed = f.finish();
    assert!(failed.is_empty(), "{failed:#?}");
}

/// Every trigger_teleport players can use: standing in it (with the
/// name or class its filter wants) puts the player on its destination
/// the same tick, keeping the velocity in world space and snapping the
/// view to the destination's angles (no landmark); a second later the
/// player is still there (the destination isn't in another teleport) and
/// standing.
fn teleports(f: &mut Flow, p: Entity, home: Vec3, names: &[String], classes: &[String]) {
    let mut land = Tally::default();
    let mut settle = Tally::default();
    let ids: Vec<EntId> = world(f)
        .ids()
        .into_iter()
        .filter(|id| matches!(trigger(world(f), *id), Some(t) if matches!(t.kind, Kind::Teleport { .. }) && t.enabled))
        .collect();
    let mut settled: Vec<Vec3> = Vec::new();
    for id in ids {
        if world(f).get(id).is_none() {
            continue;
        }
        let tname = format!("{}#{}", world(f).get(id).unwrap().targetname, id.index);
        let Some((_, _, inside)) = volume(world(f), id) else {
            continue;
        };
        let Some(identity) = identity_for(f, p, id, names, classes) else {
            land.known(format!("{tname}: no player passes its filter"));
            continue;
        };
        reset(f, p, home);
        set_identity(f, p, &identity.0, &identity.1);
        let Some((dest, angles, mark)) = destination(f, id, p) else {
            land.known(format!("{tname}: destination missing (map)"));
            continue;
        };
        let Some(at) = feet_in(f, id) else {
            land.known(format!("{tname}: no room for a player in it"));
            continue;
        };
        let _ = inside;
        let expected = match mark {
            Some(l) => dest + (at - l),
            None => dest,
        };
        let v0 = Vec3::new(120.0, -80.0, 0.0);
        f.put(p, at);
        set_velocity(f, p, v0);
        if std::env::var("MASHUP_COURSE_DEBUG").is_ok_and(|v| v == id.index.to_string()) {
            fn probe(tag: &'static str) -> impl FnMut(Query<(&Velocity, &Transform), With<God>>) {
                move |q| {
                    for (v, t) in &q {
                        eprintln!("  {tag}: v {:.1} at {:.1}", to_source(v.0), to_source(t.translation));
                    }
                }
            }
            for b in &trigger(world(f), id).unwrap().brushes {
                eprintln!(
                    "debug #{}: brush {:.2} - {:.2} ({} planes)",
                    id.index,
                    b.min,
                    b.max,
                    b.planes.len()
                );
            }
            use mashup::{core::SimSet, logic::LogicSet};
            f.sim.app.add_systems(
                FixedUpdate,
                probe("before movement").before(SimSet::Movement).after(LogicSet::Pre),
            );
            f.sim.app.add_systems(
                FixedUpdate,
                probe("after movement").after(SimSet::Movement).before(LogicSet::Post),
            );
            f.sim
                .app
                .add_systems(FixedUpdate, probe("after post").after(LogicSet::Post));
            f.sim.app.add_systems(FixedPostUpdate, probe("fixed post update"));
            for _ in 0..3 {
                let s = f.sim.app.world().get::<MovementState>(p).cloned();
                eprintln!(
                    "debug #{}: feet {:.1} v {:.1} ground {:?} blocked {}",
                    id.index,
                    f.feet(p),
                    velocity(f, p),
                    s.map(|s| s.on_ground),
                    {
                        let at = f.feet(p);
                        blocked(f, at)
                    }
                );
                f.sim.ticks(1);
                f.drain();
                let tr = trigger(world(f), id).unwrap();
                eprintln!(
                    "  links {:?} touching {:?} enabled {} flags {}",
                    tr.links,
                    tr.touching,
                    tr.enabled,
                    world(f).get(id).unwrap().spawnflags
                );
                let w = f.sim.app.world();
                let t = w.get::<Transform>(p).unwrap().translation;
                let s = w.get::<MovementState>(p).unwrap();
                let u = UNIT_SCALE;
                let o = mashup::map::entities::engine_to_entity(t, u);
                let (a, b) = (
                    mashup::map::entities::engine_to_entity(s.hull_min, u),
                    mashup::map::entities::engine_to_entity(s.hull_max, u),
                );
                let (lo, hi) = (o + a.min(b), o + a.max(b));
                eprintln!(
                    "  logic box {lo:.2} - {hi:.2}: touches {}",
                    tr.brushes
                        .iter()
                        .any(|br| mashup::logic::world::box_touches(br, lo, hi))
                );
                for (n, d) in &tr.brushes[0].planes {
                    let c = (lo + hi) / 2.0;
                    let h = (hi - lo) / 2.0;
                    let v = n.dot(c) - n.abs().dot(h) - d;
                    if v >= 0.0 {
                        eprintln!("  separating plane {n:.3} {d:.2}: {v:.3}");
                    }
                }
            }
            f.put(p, at);
            set_velocity(f, p, v0);
        }
        ticks(f, 1);
        let got = f.feet(p);
        let v = velocity(f, p);
        let (pitch, yaw) = view(f, p);
        let placed = near(got, expected, 40.0);
        let moved = f.sim.app.world().resource::<MovedVelocity>().0;
        let kept = (v - moved).length() < 0.5;
        let snapped = mark.is_some() || (yaw_close(yaw, angles.y) && (pitch - angles.x).abs() < 2.0);
        // A destination inside another teleport: sent on in the same tick
        // (triggers.md, edge cases).
        if !placed && let Some(next) = teleport_at(f, p, expected, id) {
            land.known(format!("{tname}: lands in teleport #{} (chained)", next.index));
            continue;
        }
        // Another teleport over the same spot: which one wins is the
        // touch order, an open question (triggers.md 3); ours is the
        // entities' order, each one touched.
        if !placed && let Some(other) = teleport_at(f, p, at, id) {
            land.known(format!(
                "{tname}: overlaps teleport #{} (touch order: open question)",
                other.index
            ));
            continue;
        }
        land.add(placed && kept && snapped, || {
            format!(
                "{tname} -> {expected:.0}: at {got:.0}{}{}",
                if kept {
                    String::new()
                } else {
                    format!(", v {v:.0} (was {moved:.0})")
                },
                if snapped {
                    String::new()
                } else {
                    format!(", view {pitch:.0} {yaw:.0} (want {:.0} {:.0})", angles.x, angles.y)
                }
            )
        });
        if !placed || settled.iter().any(|s| s.distance(expected) < 8.0) {
            continue;
        }
        settled.push(expected);
        set_velocity(f, p, Vec3::ZERO);
        let mut jumped = None;
        let mut last = f.feet(p);
        let debug = std::env::var("MASHUP_COURSE_DEBUG").is_ok_and(|v| v == id.index.to_string());
        for k in 0..90 {
            ticks(f, 1);
            let now = f.feet(p);
            if debug && k < 5 {
                let b = blocked(f, now);
                let s = f.sim.app.world().get::<MovementState>(p).cloned().unwrap();
                eprintln!(
                    "debug settle #{}: feet {now:.2} v {:.1} ground {} blocked {b} hull {:?}",
                    id.index,
                    velocity(f, p),
                    s.on_ground,
                    (s.hull_min, s.hull_max)
                );
                eprintln!("  controls {:?}", f.sim.app.world().get::<MapControls>(p));
                if k == 0 {
                    let world = f.sim.app.world();
                    let terrain = world.get_resource::<mashup::core::MapTerrain>();
                    for (kind, b) in world
                        .resource::<mashup::core::MapBrushes>()
                        .0
                        .iter()
                        .map(|b| ("brush", b))
                        .chain(terrain.iter().flat_map(|t| t.brushes.iter()).map(|b| ("terrain", b)))
                    {
                        let (lo, hi) = (
                            to_source(b.min).min(to_source(b.max)),
                            to_source(b.min).max(to_source(b.max)),
                        );
                        if lo.x - 16.0 < now.x
                            && hi.x + 16.0 > now.x
                            && lo.y - 16.0 < now.y
                            && hi.y + 16.0 > now.y
                            && (hi.z - now.z).abs() < 40.0
                        {
                            eprintln!("  {kind} {lo:.2} - {hi:.2} planes {}", b.planes.len());
                        }
                    }
                }
                eprintln!(
                    "  movement {:?}",
                    f.sim.app.world().get::<cs_source::movement::SourceMovement>(p)
                );
            }
            if now.distance(last) > 64.0 && jumped.is_none() {
                jumped = Some(now);
            }
            last = now;
        }
        // Standing, or on its way somewhere (a surf start drops onto its
        // ramp, slides off it with no keys held and falls into a fail
        // teleport: fine); not hanging in place off the ground (stuck in
        // a solid, or held above a floor).
        let standing = on_ground(f, p);
        let hung = !standing && jumped.is_none() && last.distance(expected) < 1.0;
        settle.add(!hung, || {
            format!("{tname}: hangs at {expected:.0}, not standing, 1.4 s on")
        });
    }
    land.row(f, "teleports land on their destination");
    settle.row(f, "teleport destinations: no hanging in place");
    reset(f, p, home);
}

/// An enabled teleport other than `not` that a player standing at
/// `feet` touches and passes.
fn teleport_at(f: &mut Flow, p: Entity, feet: Vec3, not: EntId) -> Option<EntId> {
    present(f, p);
    let w = world(f);
    w.ids().into_iter().find(|t| {
        *t != not
            && matches!(trigger(w, *t), Some(tr) if matches!(tr.kind, Kind::Teleport { .. }) && tr.enabled
                && tr.brushes.iter().any(|b| brushes_overlap(b, feet)))
            && triggers::passes(w, *t, Who::Player(p))
    })
}

/// Stage checkpoints: a trigger naming the player (AddOutput targetname
/// on !activator), then a fail teleport whose filter lets only that name
/// through sends the player to that stage's destination.
fn checkpoints(f: &mut Flow, p: Entity, home: Vec3, names: &[String], classes: &[String]) {
    let mut chain = Tally::default();
    let ids = world(f).ids();
    for n in names {
        // The first trigger naming players `n` once (multihop triggers name
        // twice: bhop blocks, below).
        let Some((cp, out, delay)) = ids.iter().find_map(|id| {
            let w = world(f);
            trigger(w, *id)?;
            let sets = activator_sets(w, *id, "targetname");
            (sets.len() == 1 && sets[0].1 == *n).then(|| (*id, sets[0].0.clone(), sets[0].2))
        }) else {
            continue;
        };
        // A teleport that lets `n` through but not a player without a name.
        let Some(tp) = ids.iter().copied().find(|id| {
            matches!(trigger(world(f), *id), Some(t) if matches!(t.kind, Kind::Teleport { .. }) && t.enabled && t.filter.is_some())
                && {
                    set_identity(f, p, n, "");
                    let with = triggers::passes(world(f), *id, Who::Player(p));
                    set_identity(f, p, "", "");
                    let without = triggers::passes(world(f), *id, Who::Player(p));
                    with && !without
                }
        }) else {
            continue;
        };
        // The checkpoint may itself want an earlier stage's name (a stage's
        // end teleport naming the player for the next).
        let Some(identity) = identity_for(f, p, cp, names, classes) else {
            continue;
        };
        reset(f, p, home);
        set_identity(f, p, &identity.0, &identity.1);
        let Some((lo, hi, inside)) = volume(world(f), cp) else {
            continue;
        };
        let Some(at) = feet_in(f, cp) else {
            chain.known(format!("{n}: no room for a player in its checkpoint"));
            continue;
        };
        f.put(p, at);
        ticks(f, 1);
        if out == "onendtouch" {
            // Leave it: just above.
            f.put(p, Vec3::new(inside.x, inside.y, hi.z + 2.0));
            ticks(f, 1);
        }
        let _ = lo;
        f.secs(delay as f64 + 0.05);
        let named = name_of(f, p);
        if volume(world(f), tp).is_none() {
            continue;
        }
        let Some((dest, _, mark)) = destination(f, tp, p) else {
            chain.known(format!("{n}: its teleport's destination is missing (map)"));
            continue;
        };
        let Some(at) = feet_in(f, tp) else {
            chain.known(format!("{n}: no room for a player in its teleport"));
            continue;
        };
        let expected = mark.map_or(dest, |l| dest + (at - l));
        f.put(p, at);
        ticks(f, 1);
        let got = f.feet(p);
        if named.eq_ignore_ascii_case(n)
            && !near(got, expected, 40.0)
            && let Some(other) = teleport_at(f, p, at, tp).or_else(|| teleport_at(f, p, expected, tp))
        {
            chain.known(format!(
                "{n}: its teleport overlaps teleport #{} (touch order)",
                other.index
            ));
            continue;
        }
        chain.add(named.eq_ignore_ascii_case(n) && near(got, expected, 40.0), || {
            format!("{n} ({out} +{delay}s): named '{named}', at {got:.0}, stage start {expected:.0}")
        });
    }
    chain.row(f, "checkpoint names send fails to their stage");
    reset(f, p, home);
}

/// Bhop blocks. Doors that open when touched (spawnflag 1024), moving
/// down: standing on one drops it, and it comes back. "Multihop" blocks:
/// a trigger names whoever stands on the block a moment later and back
/// (two AddOutput targetname with delays), over a teleport filtered by
/// that name: standing there teleports, a quick hop off doesn't.
fn bhop_blocks(f: &mut Flow, p: Entity, home: Vec3) {
    let mut doors = Tally::default();
    let mut back = Tally::default();
    let ids = world(f).ids();
    let touch_doors: Vec<EntId> = ids
        .iter()
        .copied()
        .filter(|id| {
            let w = world(f);
            matches!(w.get(*id).map(|e| &e.class), Some(Class::Door(_))) && w.get(*id).unwrap().has_flag(1024)
        })
        .collect();
    for (k, id) in touch_doors.iter().enumerate() {
        // Every door on maps with a few, a sample of 12 on bhop courses.
        if touch_doors.len() > 12 && k % (touch_doors.len() / 12) != 0 {
            continue;
        }
        let Some((lo, hi)) = brush_bounds(f, *id) else { continue };
        let name = format!("{}#{}", world(f).get(*id).unwrap().targetname, id.index);
        reset(f, p, home);
        // On top (a bhop block), else against a side (a gate in a doorway,
        // its top in the wall above).
        let c = (lo + hi) / 2.0;
        let candidates = [
            Vec3::new(c.x, c.y, hi.z + 1.0),
            Vec3::new(lo.x - 16.5, c.y, lo.z + 1.0),
            Vec3::new(hi.x + 16.5, c.y, lo.z + 1.0),
            Vec3::new(c.x, lo.y - 16.5, lo.z + 1.0),
            Vec3::new(c.x, hi.y + 16.5, lo.z + 1.0),
        ];
        let top = candidates
            .into_iter()
            .find(|at| !blocked(f, *at))
            .unwrap_or(candidates[0]);
        f.put(p, top);
        // Where its brushes start; how far they get (a bhop block drops,
        // an arena gate rises).
        let start = brush_bounds(f, *id).unwrap().0;
        let moved = |f: &Flow| brush_bounds(f, *id).map_or(0.0, |b| b.0.distance(start));
        let mut most = 0.0f32;
        for _ in 0..40 {
            ticks(f, 1);
            most = most.max(moved(f));
        }
        doors.add(most > 2.0, || format!("{name} at {top:.0}: didn't move"));
        // Away, and wait for it to come back.
        f.put(p, home);
        let mut returned = false;
        for _ in 0..(66 * 8) {
            ticks(f, 1);
            if moved(f) < 0.5 {
                returned = true;
                break;
            }
        }
        if most > 2.0 {
            back.add(returned, || format!("{name}: not back 8 s later"));
        }
    }
    doors.row(f, "touch doors (bhop blocks) move when stood on");
    back.row(f, "touch doors come back");

    let mut hop = Tally::default();
    let mut stay = Tally::default();
    let multi: Vec<(EntId, String, f32)> = ids
        .iter()
        .filter_map(|id| {
            let w = world(f);
            trigger(w, *id)?;
            let mut sets = activator_sets(w, *id, "targetname");
            if sets.len() < 2 {
                return None;
            }
            sets.sort_by(|a, b| a.2.total_cmp(&b.2));
            Some((*id, sets[0].1.clone(), sets[0].2))
        })
        .collect();
    for (k, (id, n, delay)) in multi.iter().enumerate() {
        if multi.len() > 12 && k % (multi.len() / 12) != 0 {
            continue;
        }
        let Some((lo, hi, inside)) = volume(world(f), *id) else {
            continue;
        };
        // The filtered teleport around it.
        let near_tps: Vec<EntId> = world(f)
            .ids()
            .into_iter()
            .filter(|t| {
                let w = world(f);
                let Some(tr) = trigger(w, *t) else { return false };
                if !matches!(tr.kind, Kind::Teleport { .. }) || tr.filter.is_none() {
                    return false;
                }
                let Some((tlo, thi, _)) = volume(w, *t) else {
                    return false;
                };
                tlo.x < hi.x
                    && thi.x > lo.x
                    && tlo.y < hi.y
                    && thi.y > lo.y
                    && tlo.z < hi.z + 72.0
                    && thi.z > lo.z - 72.0
            })
            .collect();
        // The one the block's name lets through.
        set_identity(f, p, n, "");
        let tp = near_tps
            .into_iter()
            .find(|t| triggers::passes(world(f), *t, Who::Player(p)));
        set_identity(f, p, "", "");
        let Some(tp) = tp else { continue };
        reset(f, p, home);
        let Some((dest, _, _)) = destination(f, tp, p) else {
            continue;
        };
        let name = format!("{}#{}", world(f).get(*id).unwrap().targetname, id.index);
        // Land on the block (from a little above, as a hop lands: a
        // player put down within 2 units of a floor rests above it).
        let Some(feet) = feet_in(f, *id) else {
            stay.known(format!("{name}: no room for a player on it"));
            continue;
        };
        let _ = (lo, inside);
        let drop = [16.0, 8.0, 4.0, 2.0]
            .into_iter()
            .map(|up| feet + Vec3::Z * up)
            .find(|d| !blocked(f, *d))
            .unwrap_or(feet);
        let land = |f: &mut Flow| {
            f.put(p, drop);
            for _ in 0..40 {
                ticks(f, 1);
                if on_ground(f, p) || near(f.feet(p), dest, 64.0) {
                    break;
                }
            }
            // A landing can rest up to 2 units above the floor (movement.md,
            // ground detection) until a ground move snaps it down: a step,
            // as a hopping player keeps moving.
            f.sim.intent(p).move_axis = Vec2::Y;
            ticks(f, 2);
            f.sim.intent(p).move_axis = Vec2::ZERO;
        };
        let debug = std::env::var("MASHUP_COURSE_DEBUG").is_ok_and(|v| v == id.index.to_string());
        if debug {
            for t in [*id, tp] {
                let tr = trigger(world(f), t).unwrap();
                for b in &tr.brushes {
                    eprintln!("debug #{}: brush {:.2} - {:.2}", t.index, b.min, b.max);
                }
            }
        }
        // Sent back: to the teleport's start, or anywhere far in one tick
        // (another teleport of the block).
        land(f);
        let mut sent = near(f.feet(p), dest, 64.0) || f.feet(p).distance(drop) > 128.0;
        let mut last = f.feet(p);
        for _ in 0..(33 + (delay * 66.0) as u32) {
            if sent {
                break;
            }
            ticks(f, 1);
            sent = f.feet(p).distance(last) > 128.0;
            last = f.feet(p);
            if debug {
                eprintln!(
                    "debug #{}: feet {:.1} v {:.1} ground {} name '{}' blocked {}",
                    id.index,
                    f.feet(p),
                    velocity(f, p),
                    on_ground(f, p),
                    name_of(f, p),
                    {
                        let at = f.feet(p);
                        blocked(f, at)
                    }
                );
            }
            sent |= near(f.feet(p), dest, 64.0);
        }
        let at = f.feet(p);
        // Its teleport ends at or under the block's top: a player standing
        // there doesn't overlap it (our rule: positive overlap; whether a
        // face contact touches is triggers.md's open question 1).
        let under = volume(world(f), tp).is_some_and(|(_, thi, _)| thi.z <= at.z + 0.05);
        if !sent && under {
            stay.known(format!("{name}: its teleport is under the block's top (face contact)"));
        } else {
            stay.add(sent, || {
                format!(
                    "{name} ({n}): landed from {drop:.0}, at {at:.0} {:.1} s later, start {dest:.0}",
                    delay + 0.5
                )
            });
        }
        // Two ticks on it, then away: no teleport after.
        reset(f, p, home);
        land(f);
        ticks(f, 2);
        // Off it into the clear (out of every trigger, high above).
        let off = home + Vec3::Z * 30_000.0;
        let _ = (inside, hi);
        f.put(p, off);
        set_velocity(f, p, Vec3::ZERO);
        let mut tele = false;
        for _ in 0..20 {
            ticks(f, 1);
            if near(f.feet(p), dest, 64.0) {
                tele = true;
            }
        }
        hop.add(!tele, || {
            format!("{name} ({n}): a 2-tick touch still sent the player to {dest:.0}")
        });
    }
    stay.row(f, "multihop blocks: standing teleports");
    hop.row(f, "multihop blocks: a quick hop doesn't");
    reset(f, p, home);
}

/// Boosters: trigger_push (base velocity while inside, kept as momentum;
/// "Once Only" adds to the velocity and goes) and triggers that AddOutput
/// basevelocity on the player (added to its velocity on its next move, x
/// (1 + dt/2): triggers.md, trigger_push step 1).
fn pushes(f: &mut Flow, p: Entity, home: Vec3, names: &[String], classes: &[String]) {
    let mut push = Tally::default();
    let ids = world(f).ids();
    let push_ids: Vec<EntId> = ids
        .iter()
        .copied()
        .filter(|id| matches!(trigger(world(f), *id), Some(t) if matches!(t.kind, Kind::Push { .. }) && t.enabled))
        .collect();
    for id in push_ids {
        let Some(identity) = identity_for(f, p, id, names, classes) else {
            continue;
        };
        let Some(Kind::Push { dir, speed }) = trigger(world(f), id).map(|t| t.kind.clone()) else {
            continue;
        };
        let once = world(f).get(id).unwrap().has_flag(128);
        let name = format!("{}#{}", world(f).get(id).unwrap().targetname, id.index);
        reset(f, p, home);
        set_identity(f, p, &identity.0, &identity.1);
        let Some(at) = feet_in(f, id) else {
            push.known(format!("{name}: no room for a player in it"));
            continue;
        };
        f.put(p, at);
        set_velocity(f, p, Vec3::ZERO);
        let want = dir * speed;
        ticks(f, 1);
        let ok = if once {
            let v = velocity(f, p);
            // Added this tick, then one tick of gravity at most.
            (v.truncate() - want.truncate()).length() < 2.0 && (v.z - want.z).abs() < 20.0 && world(f).get(id).is_none()
        } else {
            // Base velocity set (plus overlapping pushes).
            let b = f
                .sim
                .app
                .world()
                .get::<BaseVelocity>(p)
                .map(|b| to_source(b.velocity))
                .unwrap_or_default();
            b.dot(want.normalize_or_zero()) >= speed * 0.98
        };
        let b = f
            .sim
            .app
            .world()
            .get::<BaseVelocity>(p)
            .map(|b| to_source(b.velocity))
            .unwrap_or_default();
        let v = velocity(f, p);
        push.add(ok, || {
            format!(
                "{name} ({}{speed} along {dir:.2}): base {b:.0}, v {v:.0}",
                if once { "once, " } else { "" }
            )
        });
    }
    push.row(f, "trigger_push boosters");

    let mut boost = Tally::default();
    for id in ids {
        let w = world(f);
        if trigger(w, id).is_none_or(|t| !t.enabled) {
            continue;
        }
        let sets = activator_sets(w, id, "basevelocity");
        if sets.is_empty() {
            continue;
        }
        let Some(identity) = identity_for(f, p, id, names, classes) else {
            continue;
        };
        let name = format!("{}#{}", world(f).get(id).unwrap().targetname, id.index);
        let (out, value, delay) = sets[0].clone();
        let want = mashup::map::entities::parse_vector(&value);
        reset(f, p, home);
        set_identity(f, p, &identity.0, &identity.1);
        let Some((_, hi, inside)) = volume(world(f), id) else {
            continue;
        };
        let Some(at) = feet_in(f, id) else {
            boost.known(format!("{name}: no room for a player in it"));
            continue;
        };
        f.put(p, at);
        set_velocity(f, p, Vec3::ZERO);
        ticks(f, 1);
        if std::env::var("MASHUP_COURSE_DEBUG").is_ok_and(|v| v == id.index.to_string()) {
            let tr = trigger(world(f), id).unwrap();
            eprintln!(
                "debug boost #{}: at {at:.1} name '{}' links {:?} touching {:?} base {:?}",
                id.index,
                name_of(f, p),
                tr.links,
                tr.touching,
                f.sim.app.world().get::<BaseVelocity>(p)
            );
            let world = f.sim.app.world();
            let c = cs_source::movement::to_engine(at + Vec3::Z * 31.0);
            let h = Vec3::new(16.0, 31.0, 16.0) * UNIT_SCALE;
            let terrain = world.get_resource::<mashup::core::MapTerrain>();
            for (kind, b) in world
                .resource::<mashup::core::MapBrushes>()
                .0
                .iter()
                .map(|b| ("brush", b))
                .chain(terrain.iter().flat_map(|t| t.brushes.iter()).map(|b| ("terrain", b)))
            {
                if b.max.cmpgt(c - h * 1.5).all() && b.min.cmplt(c + h * 1.5).all() {
                    let worst = b
                        .planes
                        .iter()
                        .map(|(n, d)| (n.dot(c) - (d + n.abs().dot(h))) / UNIT_SCALE)
                        .fold(f32::MIN, f32::max);
                    eprintln!(
                        "  {kind}: {} planes, deepest separation {worst:.4} units",
                        b.planes.len()
                    );
                }
            }
        }
        if out == "onendtouch" {
            f.put(p, Vec3::new(inside.x, inside.y, hi.z + 2.0));
            set_velocity(f, p, Vec3::ZERO);
        }
        // Up to the delay, then the move that takes it in.
        // The tick that takes it in: the biggest velocity change along it.
        // Gravity acts that tick as on any other (sv_gravity x dt, x the
        // player's gravity: 12 units/s).
        let mut best = Vec3::ZERO;
        let mut last = velocity(f, p);
        for _ in 0..(3 + (delay * 66.0).ceil() as u32) {
            ticks(f, 1);
            let v = velocity(f, p);
            let dv = v - last;
            if std::env::var("MASHUP_COURSE_DEBUG").is_ok_and(|v| v == id.index.to_string()) {
                eprintln!(
                    "debug boost #{}: v {v:.1} feet {:.1} base {:?} ground {}",
                    id.index,
                    f.feet(p),
                    f.sim.app.world().get::<BaseVelocity>(p),
                    on_ground(f, p)
                );
            }
            if dv.dot(want) > best.dot(want) {
                best = dv;
            }
            last = v;
        }
        let expect = want * (1.0 + 0.015 / 2.0) - Vec3::Z * 800.0 * 0.015 * gravity(f, p);
        // On the ground (a booster pad without lift), that tick's friction
        // takes its share (sv_friction 4: 6 % of the speed) and the
        // vertical velocity stays 0.
        let rubbed = expect.truncate() * (1.0 - 4.0 * 0.015);
        let ok = ((best.truncate() - expect.truncate()).length() < 1.0 && (best.z - expect.z).abs() < 1.0)
            || ((best.truncate() - rubbed).length() < 2.0 && want.z <= 0.0 && best.z.abs() < 1.0);
        // Sent away by a teleport sharing the booster's volume (whose
        // destination may set its own base velocity: anti-prespeed).
        if !ok && f.feet(p).distance(at) > 64.0 + want.length() * 0.1 {
            boost.known(format!("{name}: a teleport over it moves the player first"));
            continue;
        }
        boost.add(ok, || {
            format!("{name} ({out}, basevelocity {value}): velocity change {best:.1}, want {expect:.1}")
        });
    }
    boost.row(f, "basevelocity boosters");
    reset(f, p, home);
}

/// Gravity (trigger_gravity, AddOutput gravity) and speed zones
/// (player_speedmod's ModifySpeed): after the touch (or leaving), the
/// player has the zone's value.
fn keyvalue_zones(f: &mut Flow, p: Entity, home: Vec3) {
    let mut grav = Tally::default();
    let mut speed = Tally::default();
    let ids = world(f).ids();
    for id in ids {
        present(f, p);
        let w = world(f);
        let Some(t) = trigger(w, id) else { continue };
        if !t.enabled || !triggers::passes(w, id, Who::Player(p)) {
            continue;
        }
        let mut wants: Vec<(String, f32, f32, bool)> = Vec::new();
        if let Kind::Gravity(g) = t.kind {
            wants.push(("ontouch".into(), g, 0.0, true));
        }
        for (o, v, d) in activator_sets(w, id, "gravity") {
            wants.push((o, mashup::logic::value::atof(&v), d, true));
        }
        for (o, target, input, param, d) in connections(w, id) {
            if input == "modifyspeed" && w.find(&target).is_some_and(|s| classname(w, s) == "player_speedmod") {
                wants.push((o, mashup::logic::value::atof(&param), d, false));
            }
        }
        let name = format!("{}#{}", w.get(id).unwrap().targetname, id.index);
        // One value per output kind: the end touch's undoes the start's.
        for (out, want, delay, is_gravity) in wants {
            reset(f, p, home);
            let Some((_, hi, inside)) = volume(world(f), id) else {
                continue;
            };
            let Some(at) = feet_in(f, id) else {
                if is_gravity {
                    grav.known(format!("{name}: no room for a player in it"))
                } else {
                    speed.known(format!("{name}: no room for a player in it"))
                }
                continue;
            };
            f.put(p, at);
            // Whether the value was there on some tick up to the delay's end
            // (a gravity 40 zone drops the player out of it at once, and its
            // end touch puts gravity back).
            let read = |f: &Flow| {
                if is_gravity {
                    gravity(f, p)
                } else {
                    f.sim.app.world().get::<MapControls>(p).map_or(1.0, |c| c.time_scale)
                }
            };
            let mut seen = Vec::new();
            ticks(f, 1);
            seen.push(read(f));
            if out == "onendtouch" {
                f.put(p, Vec3::new(inside.x, inside.y, hi.z + 2.0));
                ticks(f, 1);
                seen.push(read(f));
            }
            for _ in 0..(4 + (delay * 66.0).ceil() as u32) {
                ticks(f, 1);
                seen.push(read(f));
            }
            // Gravity 0 is normal gravity (triggers.md).
            let ok = seen
                .iter()
                .any(|g| (g - want).abs() < 1e-3 || (is_gravity && want == 0.0 && *g == 1.0));
            // Another gravity zone over the same spot (a low-gravity pit
            // walled by gravity 1 ones): the last touched wins, the touch
            // order an open question (triggers.md 3).
            let other = is_gravity && !ok && {
                let w = world(f);
                w.ids().into_iter().any(|t| {
                    t != id
                        && matches!(trigger(w, t), Some(tr) if matches!(tr.kind, Kind::Gravity(_))
                            && tr.brushes.iter().any(|b| brushes_overlap(b, at)))
                })
            };
            if other {
                grav.known(format!("{name}: overlaps another gravity zone (touch order)"));
            } else if is_gravity {
                grav.add(ok, || format!("{name} ({out} gravity {want}): {seen:?}"));
            } else {
                speed.add(ok, || format!("{name} ({out} ModifySpeed {want}): {seen:?}"));
            }
        }
    }
    grav.row(f, "gravity zones");
    speed.row(f, "speed zones (player_speedmod)");
    reset(f, p, home);
}

/// What triggers set off when the player walks in (end zones' texts and
/// sounds, doors opening per stage, relays): each named target of an
/// OnStartTouch/OnTrigger connection gets its input.
fn trigger_outputs(f: &mut Flow, p: Entity, home: Vec3, names: &[String], classes: &[String]) {
    let mut fired = Tally::default();
    let ids = world(f).ids();
    let mut checked = 0;
    for id in ids {
        present(f, p);
        let w = world(f);
        let Some(t) = trigger(w, id) else { continue };
        if !t.enabled || matches!(t.kind, Kind::Teleport { .. }) {
            continue;
        }
        let wants: Vec<(String, String, f32)> = connections(w, id)
            .into_iter()
            .filter(|(o, target, input, _, delay)| {
                matches!(o.as_str(), "onstarttouch" | "ontrigger" | "onstarttouchall")
                    && !target.starts_with('!')
                    && *delay < 3.0
                    && input != "addoutput"
                    && w.find(target).is_some()
            })
            .map(|(_, t, i, _, d)| (t, i, d))
            .collect();
        if wants.is_empty() {
            continue;
        }
        checked += 1;
        if checked > 60 {
            break;
        }
        let name = format!("{}#{}", w.get(id).unwrap().targetname, id.index);
        let Some(identity) = identity_for(f, p, id, names, classes) else {
            continue;
        };
        reset(f, p, home);
        set_identity(f, p, &identity.0, &identity.1);
        let before = f.delivered.len();
        let Some(at) = feet_in(f, id) else {
            fired.known(format!("{name}: no room for a player in it"));
            continue;
        };
        f.put(p, at);
        ticks(f, 1);
        let wait = wants.iter().map(|w| w.2).fold(0.0f32, f32::max);
        f.secs(wait as f64 + 0.1);
        for (target, input, _) in wants {
            let got = f.delivered[before..]
                .iter()
                .any(|(_, n, i, _)| mashup::logic::world::name_matches(&target, n) && i.eq_ignore_ascii_case(&input));
            fired.add(got, || format!("{name}: {target}.{input} not delivered"));
        }
    }
    fired.row(f, "triggers set off their targets");
    reset(f, p, home);
}

/// Every func_button: +use (or a touch) presses it.
fn buttons(f: &mut Flow, p: Entity, home: Vec3) {
    let mut press = Tally::default();
    let ids: Vec<EntId> = world(f)
        .ids()
        .into_iter()
        .filter(|id| matches!(world(f).get(*id).map(|e| &e.class), Some(Class::Button(_))))
        .collect();
    for id in ids.into_iter().take(24) {
        let e = world(f).get(id).unwrap();
        if e.has_flag(256) && !e.has_flag(1024) {
            continue;
        }
        let name = format!("{}#{}", e.targetname, id.index);
        let locked = matches!(&e.class, Class::Button(b) if b.locked);
        reset(f, p, home);
        if locked {
            press.known(format!("{name}: starts locked"));
            continue;
        }
        let ok = f.press(p, id);
        press.add(ok, || format!("{name}: +use did nothing"));
    }
    press.row(f, "buttons press");
    reset(f, p, home);
}

macro_rules! courses {
    ($($test:ident => $map:literal,)*) => {
        $(
            #[test]
            fn $test() {
                play($map);
            }
        )*
    };
}

courses! {
    bhop_addict_v2_3xl => "bhop_addict_v2_3xl",
    bhop_backport_css => "bhop_backport_css",
    bhop_flatzone => "bhop_flatzone",
    bhop_myztek => "bhop_myztek",
    kz_11342 => "kz_11342",
    kz_ancient_ruins => "kz_ancient_ruins",
    kz_bhop_izanami => "kz_bhop_izanami",
    kz_bhop_sakura => "kz_bhop_sakura",
    kz_bhop_skodna => "kz_bhop_skodna",
    kz_hikari_od_nh_v2 => "kz_hikari_od_nh_v2",
    kz_rockb1ock => "kz_rockb1ock",
    surf_apollo => "surf_apollo",
    surf_boreas => "surf_boreas",
    surf_botanica => "surf_botanica",
    surf_demise => "surf_demise",
    surf_halloween_tf2 => "surf_halloween_tf2",
    surf_happyhands => "surf_happyhands",
    surf_hellenic => "surf_hellenic",
    surf_holiday => "surf_holiday",
    surf_inferno => "surf_inferno",
    surf_jive => "surf_jive",
    surf_kismet => "surf_kismet",
    surf_nebula => "surf_nebula",
    surf_nsz_fix => "surf_nsz_fix",
    surf_sacrifice => "surf_sacrifice",
    surf_sedona => "surf_sedona",
    surf_slob => "surf_slob",
    surf_stickybutt_alpha => "surf_stickybutt_alpha",
    surf_surreal => "surf_surreal",
    surf_threnody => "surf_threnody",
}

// Gungame arenas: no course, the same checks (spawns, equipment, placed
// weapons, teleports, triggers, buttons, rounds). The weapon ladder
// itself is a server plugin's (GunGame), not the map's.
courses! {
    gg_4mida_arena => "gg_4mida_arena",
    gg_beacon => "gg_beacon",
    gg_bk_warehouse_v1 => "gg_bk_warehouse_v1",
    gg_blue_arena_32a => "gg_blue_arena_32a",
    gg_cb_arctic => "gg_cb_arctic",
    gg_churches_x_final_fixed => "gg_churches_x_final_fixed",
    gg_construct_city => "gg_construct_city",
    gg_deagle7k => "gg_deagle7k",
    gg_desert_paintball => "gg_desert_paintball",
    gg_dev_moment_v1 => "gg_dev_moment_v1",
    gg_dinoiceworld => "gg_dinoiceworld",
    gg_fusion_trx => "gg_fusion_trx",
    gg_future => "gg_future",
    gg_fy_funtimes => "gg_fy_funtimes",
    gg_fy_tactic_fight => "gg_fy_tactic_fight",
    gg_hex => "gg_hex",
    gg_iceworld_l33t => "gg_iceworld_l33t",
    gg_ilu => "gg_ilu",
    gg_legendary_fun_v1a => "gg_legendary_fun_v1a",
    gg_lego_2floor_v2 => "gg_lego_2floor_v2",
    gg_lego_mafia_arena => "gg_lego_mafia_arena",
    gg_lego_spacetower2 => "gg_lego_spacetower2",
    gg_mario_vs_wario => "gg_mario_vs_wario",
    gg_mario_world_2 => "gg_mario_world_2",
    gg_minesweeper => "gg_minesweeper",
    gg_mr_pillar_v1 => "gg_mr_pillar_v1",
    gg_nukkon_hdr => "gg_nukkon_hdr",
    gg_nutty => "gg_nutty",
    gg_pokemonz => "gg_pokemonz",
    gg_sex_fix => "gg_sex_fix",
    gg_simpsons_arabtoon => "gg_simpsons_arabtoon",
    gg_simpsons_bam => "gg_simpsons_bam",
    gg_simpsons_d => "gg_simpsons_d",
    gg_simpsons_dusty_2 => "gg_simpsons_dusty_2",
    gg_simpsons_funfreaks_v2 => "gg_simpsons_funfreaks_v2",
    gg_spacoid_pg => "gg_spacoid_pg",
    gg_tbr_water_basin => "gg_tbr_water_basin",
    gg_tip_octal => "gg_tip_octal",
    gg_toondorf => "gg_toondorf",
    gg_towerwars_v2 => "gg_towerwars_v2",
    gg_usp_deagle => "gg_usp_deagle",
    gg_wolfenstein_3d => "gg_wolfenstein_3d",
}
