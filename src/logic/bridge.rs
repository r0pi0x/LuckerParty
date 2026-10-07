//! The logic world in the ECS: built from the loaded map's entities, run
//! each fixed tick in the spec's order around the characters' movement
//! (entity_io.md "Per-tick order"): thinks, movers and +use before
//! movement; trigger touches, untouch, the event queue and removals after.

use avian3d::prelude::{
    AngularVelocity, Collider, ColliderAabb, ColliderDisabled, LinearVelocity, Position, RigidBody, RigidBodyDisabled,
    Rotation,
};
use bevy::{ecs::message::MessageCursor, prelude::*};

use super::hud::HudMessages;
use super::world::{Collision, Effect, EntId, LogicWorld, Player, SOLID_SKIN, SWEEP_EPS};
use crate::console::Console;
use crate::core::{
    BaseVelocity, Damage, DamageKind, Damageable, EntityGravity, Health, Hitgroup, Intent, LocalPlayer, MapBrush,
    MapBrushes, MapTerrain, MovementState, MovingSolid, RoundRestarts, SimSet, Team, Velocity,
};
use crate::map::breakables::{GibPiece, GlassShatter, SpawnGibs};
use crate::map::entities::{engine_to_entity, entity_rotation, entity_to_engine, rotation_to_engine};
use crate::map::vis::{LogicHidden, VisClusters};
use crate::map::{
    BrushPanes, MapBrushEntity, MapEntities, PlaySound, PropEntity, PropHome, SoundControl, SoundKey, SoundLevel,
    StartSound,
};

/// The running logic world of the loaded map.
#[derive(Resource)]
pub struct Logic {
    pub world: LogicWorld,
    /// Meters per entity unit.
    pub scale: f32,
    /// Mover entity -> its ECS node.
    pub nodes: Vec<(EntId, Entity)>,
    /// Prop entity (`classes::Class::Prop`) -> its prop node
    /// (`map::PropEntity`).
    pub props: Vec<(EntId, Entity)>,
    /// Server settings a map changed, with their values before (put back
    /// when another map loads).
    pub restore: Vec<(String, String)>,
    /// The `MapEntities` this world was built from.
    source: std::sync::Arc<Vec<crate::map::MapEntity>>,
    /// `core::RoundRestarts` this world has seen.
    restarts: u32,
}

/// Where the logic runs in the fixed tick.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogicSet {
    /// Thinks, movers, +use (before movement).
    Pre,
    /// Touches, untouch, the event queue (after movement).
    Post,
    /// Damage dealt to breakables this tick (after weapons).
    Damage,
}

pub struct LogicPlugin;

impl Plugin for LogicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudMessages>()
            .add_message::<PlaySound>()
            .add_message::<SoundControl>()
            .add_message::<SpawnGibs>()
            .add_message::<GlassShatter>()
            .configure_sets(
                FixedUpdate,
                (
                    LogicSet::Pre.after(SimSet::Rules).before(SimSet::Movement),
                    LogicSet::Post.after(SimSet::Movement).before(SimSet::Weapons),
                    LogicSet::Damage.after(SimSet::Weapons),
                ),
            )
            .add_systems(FixedUpdate, (load, pre).chain().in_set(LogicSet::Pre))
            .add_systems(FixedUpdate, post.in_set(LogicSet::Post))
            .add_systems(FixedUpdate, damage.in_set(LogicSet::Damage));
    }
}

/// Build (or drop) the logic world when the map's entities change, and
/// re-create it at a round restart (`core::RoundRestarts`).
fn load(world: &mut World) {
    let current = world.get_resource::<MapEntities>().cloned();
    let built = world.get_resource::<Logic>().map(|l| l.source.clone());
    let restarts = world.get_resource::<RoundRestarts>().map_or(0, |r| r.0);
    match (current, built) {
        (None, Some(_)) => {
            restore_settings(world);
            world.write_message(SoundControl::StopAll);
            world.remove_resource::<Logic>();
        }
        (Some(m), b) if b.as_ref().is_none_or(|b| !std::sync::Arc::ptr_eq(b, &m.entities)) => {
            restore_settings(world);
            world.write_message(SoundControl::StopAll);
            let dt = world.resource::<Time<Fixed>>().timestep().as_secs_f32();
            let mut logic = LogicWorld::new(dt);
            let ids = logic.load_map(&m.entities);
            let nodes = attach_nodes(world, &logic, &ids);
            let props = attach_props(world, &logic, &ids, false);
            world.insert_resource(Logic {
                world: logic,
                scale: m.scale,
                nodes,
                props,
                restore: Vec::new(),
                source: m.entities.clone(),
                restarts,
            });
        }
        _ => {
            let Some(mut logic) = world.remove_resource::<Logic>() else { return };
            if logic.restarts != restarts {
                logic.restarts = restarts;
                // The entities' sounds stop; re-created ones start again.
                world.write_message(SoundControl::StopAll);
                let source = logic.source.clone();
                let ids = logic.world.round_restart(&source);
                logic.nodes = attach_nodes(world, &logic.world, &ids);
                logic.props = attach_props(world, &logic.world, &ids, true);
                // HUD messages from the last round go too.
                if let Some(mut hud) = world.get_resource_mut::<HudMessages>() {
                    *hud = HudMessages::default();
                }
            }
            world.insert_resource(logic);
        }
    }
}

/// Pair every mover node with its logic entity (`ids` in map order).
/// Nodes whose entity is gone are hidden; the others are shown and solid
/// (the next sync places them), and breakables take damage from weapons.
fn attach_nodes(world: &mut World, logic: &LogicWorld, ids: &[EntId]) -> Vec<(EntId, Entity)> {
    let all: Vec<(Entity, usize)> = world
        .query::<(Entity, &MapBrushEntity)>()
        .iter(world)
        .map(|(e, n)| (e, n.0))
        .collect();
    let mut nodes = Vec::new();
    for (node, index) in all {
        let Some(id) = ids.get(index).copied().filter(|id| logic.get(*id).is_some()) else {
            hide_node(world, node);
            continue;
        };
        let breakable = matches!(
            logic.get(id).map(|e| &e.class),
            Some(super::classes::Class::Breakable(_))
        );
        let mut e = world.entity_mut(node);
        e.remove::<(ColliderDisabled, MovingSolid)>();
        if e.get::<Visibility>().is_some_and(|v| *v == Visibility::Hidden) {
            e.insert(Visibility::Inherited);
        }
        if breakable {
            e.insert(Damageable);
        } else {
            e.remove::<Damageable>();
        }
        nodes.push((id, node));
    }
    nodes
}

/// A node whose logic entity is gone (killed, broken): not drawn, hit or
/// stood on, until a round restart brings the entity back.
fn hide_node(world: &mut World, node: Entity) {
    if let Ok(mut e) = world.get_entity_mut(node) {
        // Only when missing: re-inserting ColliderDisabled replaces it,
        // and avian puts a replaced one's collider back in its query tree.
        if !e.contains::<ColliderDisabled>() {
            e.insert(ColliderDisabled);
        }
        e.insert((Visibility::Hidden, MovingSolid::default())).remove::<Damageable>();
    }
}

/// Pair every prop node (`map::PropEntity`) with its logic prop (`ids` in
/// map order). They are shown and solid again; with `restart`, a moved
/// physics prop goes back where the map placed it, at rest.
fn attach_props(world: &mut World, logic: &LogicWorld, ids: &[EntId], restart: bool) -> Vec<(EntId, Entity)> {
    let all: Vec<(Entity, usize, Option<Transform>)> = world
        .query::<(Entity, &PropEntity, Option<&PropHome>)>()
        .iter(world)
        .map(|(e, p, h)| (e, p.0, h.map(|h| h.0)))
        .collect();
    let states = logic.prop_states();
    let mut out = Vec::new();
    for (node, index, home) in all {
        let Some(id) = ids.get(index).copied() else { continue };
        let Some(&(_, _, visible, solid, damageable)) = states.iter().find(|s| s.0 == id) else {
            continue;
        };
        if restart
            && let Some(home) = home
            && let Ok(mut e) = world.get_entity_mut(node)
            && e.contains::<RigidBody>()
        {
            if let Some(mut t) = e.get_mut::<Transform>() {
                *t = home;
            }
            if let Some(mut p) = e.get_mut::<Position>() {
                p.0 = home.translation;
            }
            if let Some(mut r) = e.get_mut::<Rotation>() {
                *r = Rotation::from(home.rotation);
            }
            if let Some(mut v) = e.get_mut::<LinearVelocity>() {
                v.0 = Vec3::ZERO;
            }
            if let Some(mut v) = e.get_mut::<AngularVelocity>() {
                v.0 = Vec3::ZERO;
            }
        }
        set_prop_shown(world, node, visible, solid, true);
        if let Ok(mut e) = world.get_entity_mut(node) {
            if damageable {
                e.insert(Damageable);
            } else {
                e.remove::<Damageable>();
            }
        }
        out.push((id, node));
    }
    out
}

/// Show or hide a prop node and turn its collision (its own collider and
/// its children's) on or off; `exists` false also stops its body.
fn set_prop_shown(world: &mut World, node: Entity, visible: bool, solid: bool, exists: bool) {
    let Ok(e) = world.get_entity(node) else { return };
    let hidden = e.contains::<LogicHidden>();
    if visible == hidden {
        let shown_by_vis = e.get::<VisClusters>().is_none_or(|v| v.potentially_visible);
        let mut e = world.entity_mut(node);
        if visible {
            e.remove::<LogicHidden>();
            if shown_by_vis {
                e.insert(Visibility::Inherited);
            }
        } else {
            e.insert((LogicHidden, Visibility::Hidden));
        }
    }
    let colliders: Vec<Entity> = std::iter::once(node)
        .chain(
            world
                .get::<Children>(node)
                .map(|c| c.to_vec())
                .unwrap_or_default(),
        )
        .filter(|c| world.get::<Collider>(*c).is_some())
        .collect();
    for c in colliders {
        let mut e = world.entity_mut(c);
        // Only on change: a re-inserted ColliderDisabled re-adds the
        // collider to avian's query tree (see `hide_node`).
        if solid == e.contains::<ColliderDisabled>() {
            if solid {
                e.remove::<ColliderDisabled>();
            } else {
                e.insert(ColliderDisabled);
            }
        }
    }
    let mut e = world.entity_mut(node);
    if e.contains::<RigidBody>() && exists == e.contains::<RigidBodyDisabled>() {
        if exists {
            e.remove::<RigidBodyDisabled>();
        } else {
            e.insert(RigidBodyDisabled);
        }
    }
    if !exists {
        e.remove::<Damageable>();
    }
}

/// Prop nodes follow their logic prop: hidden and not solid once it is
/// gone (broken, killed) until a round restart, or as its inputs say.
fn sync_props(world: &mut World, logic: &mut Logic) {
    let states = logic.world.prop_states();
    for (id, node) in logic.props.clone() {
        match states.iter().find(|s| s.0 == id) {
            Some(&(_, _, visible, solid, _)) => set_prop_shown(world, node, visible, solid, true),
            None => set_prop_shown(world, node, false, false, false),
        }
    }
}

fn restore_settings(world: &mut World) {
    let Some(restore) = world.get_resource_mut::<Logic>().map(|mut l| std::mem::take(&mut l.restore)) else {
        return;
    };
    if let Some(mut c) = world.get_resource_mut::<Console>() {
        for (name, value) in restore {
            c.submit(format!("{name} {}", crate::console::quote(&value)));
        }
    }
}

/// Static collision through the map's brushes and terrain.
struct WorldCollision<'a> {
    brushes: Option<&'a MapBrushes>,
    terrain: Option<&'a MapTerrain>,
    scale: f32,
}

impl WorldCollision<'_> {
    /// Engine centre and half size of an entity-space box.
    fn engine_box(&self, mins: Vec3, maxs: Vec3, at: Vec3) -> (Vec3, Vec3) {
        let h = (maxs - mins) / 2.0;
        (
            entity_to_engine(at + (mins + maxs) / 2.0, self.scale),
            Vec3::new(h.x, h.z, h.y) * self.scale,
        )
    }

    fn near(&self, lo: Vec3, hi: Vec3) -> Vec<&MapBrush> {
        let mut out: Vec<&MapBrush> = self.brushes.map(|b| b.0.iter().collect()).unwrap_or_default();
        if let Some(t) = self.terrain {
            let mut idx = Vec::new();
            t.near(lo, hi, &mut idx);
            out.extend(idx.into_iter().map(|i| &t.brushes[i as usize]));
        }
        out
    }
}

impl Collision for WorldCollision<'_> {
    fn sweep(&self, mins: Vec3, maxs: Vec3, from: Vec3, to: Vec3) -> f32 {
        let (a, half) = self.engine_box(mins, maxs, from);
        let (b, _) = self.engine_box(mins, maxs, to);
        let eps = SWEEP_EPS * self.scale;
        self.near(a.min(b) - half, a.max(b) + half)
            .into_iter()
            .filter_map(|br| br.sweep_box(half, a, b, eps))
            .map(|(f, _)| f)
            .fold(1.0, f32::min)
    }

    fn solid(&self, mins: Vec3, maxs: Vec3, at: Vec3) -> bool {
        let (c, half) = self.engine_box(mins, maxs, at);
        let skin = SOLID_SKIN * self.scale;
        self.near(c - half, c + half)
            .into_iter()
            .any(|b| b.overlaps_box(c, half, skin))
    }
}

/// An entity-space brush in engine space.
fn brush_to_engine(b: &MapBrush, scale: f32) -> MapBrush {
    let planes = b
        .planes
        .iter()
        .map(|(n, d)| (Vec3::new(n.x, n.z, -n.y), d * scale))
        .collect();
    let (a, c) = (entity_to_engine(b.min, scale), entity_to_engine(b.max, scale));
    MapBrush {
        planes,
        min: a.min(c),
        max: a.max(c),
        ladder: b.ladder,
        surface: b.surface.clone(),
    }
}

type CharacterQuery<'a> = (
    Entity,
    &'a Transform,
    &'a Velocity,
    &'a MovementState,
    &'a Intent,
    Option<&'a Health>,
    Option<&'a Team>,
    Option<&'a BaseVelocity>,
    Option<&'a EntityGravity>,
    Option<&'a LocalPlayer>,
    Option<&'a ColliderAabb>,
);

/// The characters as logic players: the local player first (`!player`
/// is slot 1), then by entity.
fn snapshot(world: &mut World, logic: &Logic) -> Vec<Player> {
    let scale = logic.scale;
    let mut list: Vec<(bool, Player)> = world
        .query::<CharacterQuery>()
        .iter(world)
        .map(|(e, t, v, state, intent, health, team, base, gravity, local, aabb)| {
            let origin = engine_to_entity(t.translation, scale);
            let (lo, hi) = if state.hull_min != state.hull_max {
                (state.hull_min, state.hull_max)
            } else if let Some(a) = aabb {
                (a.min - t.translation, a.max - t.translation)
            } else {
                (Vec3::splat(-0.4), Vec3::splat(0.4))
            };
            let (a, b) = (engine_to_entity(lo, scale), engine_to_entity(hi, scale));
            let mut p = Player::new(e, origin);
            p.mins = a.min(b);
            p.maxs = a.max(b);
            p.eye = engine_to_entity(state.eye_offset, scale);
            p.velocity = engine_to_entity(v.0, scale);
            if let Some(b) = base {
                p.base_velocity = engine_to_entity(b.velocity, scale);
                p.base_touched = b.touched;
            }
            p.on_ground = state.on_ground;
            p.ground = state
                .ground
                .and_then(|g| logic.nodes.iter().find(|(_, n)| *n == g).map(|(id, _)| *id));
            p.view = Vec3::new(-intent.pitch.to_degrees(), intent.yaw.to_degrees() + 90.0, 0.0);
            p.alive = health.is_none_or(|h| h.current > 0.0);
            // Our teams 1 and 2 are Source's 2 (T) and 3 (CT).
            p.team = team.map_or(0, |t| if t.0 == 0 { 0 } else { t.0 + 1 });
            p.gravity = gravity.map_or(1.0, |g| g.0);
            p.use_key = intent.use_key;
            (local.is_some(), p)
        })
        .collect();
    list.sort_by_key(|(local, p)| (!*local, p.entity));
    list.into_iter().map(|(_, p)| p).collect()
}

/// Write back what the logic changed on players.
fn write_back(world: &mut World, scale: f32, before: &[Player], after: &[Player]) {
    for p in after {
        let Some(old) = before.iter().find(|b| b.entity == p.entity) else { continue };
        let Ok(mut e) = world.get_entity_mut(p.entity) else { continue };
        if p.moved || p.teleported {
            if let Some(mut t) = e.get_mut::<Transform>() {
                t.translation = entity_to_engine(p.origin, scale);
            }
        }
        if p.velocity != old.velocity
            && let Some(mut v) = e.get_mut::<Velocity>()
        {
            v.0 = entity_to_engine(p.velocity, scale);
        }
        if p.view != old.view
            && let Some(mut i) = e.get_mut::<Intent>()
        {
            i.yaw = (p.view.y - 90.0).to_radians().rem_euclid(std::f32::consts::TAU);
            i.pitch = (-p.view.x).to_radians().clamp(-89f32.to_radians(), 89f32.to_radians());
        }
        if p.base_velocity != old.base_velocity
            || p.base_touched != old.base_touched
            || p.unground
            || p.teleported
        {
            let unground = p.unground || p.teleported;
            let base = BaseVelocity {
                velocity: entity_to_engine(p.base_velocity, scale),
                touched: p.base_touched,
                unground,
            };
            e.insert(base);
        }
        if p.gravity != old.gravity {
            e.insert(EntityGravity(p.gravity));
        }
    }
}

/// Mover nodes follow their logic entity; nodes of removed entities
/// (killed, broken) are hidden (a round restart shows them again).
fn sync_movers(world: &mut World, logic: &mut Logic) {
    logic.nodes.retain(|(id, node)| {
        let alive = logic.world.get(*id).is_some();
        if !alive {
            hide_node(world, *node);
        }
        alive
    });
    let logic = &*logic;
    for (id, origin, angles, velocity, visible, solid) in logic.world.mover_poses() {
        let Some((_, node)) = logic.nodes.iter().find(|(m, _)| *m == id) else { continue };
        let shootable = logic.world.shootable(id);
        let panes = logic.world.window(id).filter(|w| w.window_broken).map(|w| {
            let e = logic.world.get(id).unwrap();
            let s = logic.scale;
            let n = w.normal;
            let depth = e
                .hulls
                .iter()
                .flat_map(|h| &h.points)
                .map(|p| (*p - w.corner).dot(n))
                .fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)));
            let depth = if depth.0 > depth.1 { (0.0, 0.0) } else { (depth.0 * s, depth.1 * s) };
            BrushPanes {
                corner: entity_to_engine(w.corner, s),
                u: entity_to_engine(w.u, s),
                v: entity_to_engine(w.v, s),
                depth,
                cols: w.cols,
                rows: w.rows,
                broken: w.broken.clone(),
            }
        });
        let brushes: Vec<MapBrush> = logic
            .world
            .mover_solid(id)
            .map(|b| b.iter().map(|b| brush_to_engine(b, logic.scale)).collect())
            .unwrap_or_default();
        let Ok(mut e) = world.get_entity_mut(*node) else { continue };
        let transform = Transform::from_translation(entity_to_engine(origin, logic.scale))
            .with_rotation(rotation_to_engine(entity_rotation(angles)));
        if let Some(mut t) = e.get_mut::<Transform>() {
            if *t != transform {
                *t = transform;
            }
        }
        if let Some(mut v) = e.get_mut::<Visibility>() {
            let want = if visible { Visibility::Inherited } else { Visibility::Hidden };
            if *v != want {
                *v = want;
            }
        }
        e.insert(MovingSolid {
            brushes,
            velocity: entity_to_engine(velocity, logic.scale),
            solid,
        });
        // Shots and physics pass what is not there (a broken breakable,
        // a disabled func_brush); a broken window keeps its panes.
        if let Some(p) = panes {
            if e.get::<BrushPanes>() != Some(&p) {
                e.insert(p);
            }
        } else if shootable == e.contains::<ColliderDisabled>() {
            if shootable {
                e.remove::<ColliderDisabled>();
            } else {
                e.insert(ColliderDisabled);
            }
        }
    }
}

fn apply_effects(world: &mut World, effects: Vec<Effect>, scale: f32) {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let local = world
        .query_filtered::<Entity, With<LocalPlayer>>()
        .iter(world)
        .next();
    for effect in effects {
        match effect {
            Effect::Damage { target, amount, crush } => {
                let point = world.get::<Transform>(target).map_or(Vec3::ZERO, |t| t.translation);
                world.write_message(Damage {
                    target,
                    attacker: None,
                    amount: amount / 100.0,
                    point,
                    dir: Vec3::NEG_Y,
                    hitgroup: Hitgroup::Generic,
                    kind: if crush { DamageKind::Crush } else { DamageKind::Generic },
                });
            }
            Effect::Gibs { set, glass, pieces } => {
                let pieces = pieces
                    .into_iter()
                    .map(|g| GibPiece {
                        position: entity_to_engine(g.position, scale),
                        velocity: entity_to_engine(g.velocity, scale),
                        spin: entity_to_engine(g.spin * std::f32::consts::PI / 180.0, 1.0),
                        life: g.life,
                    })
                    .collect();
                world.write_message(SpawnGibs { set, glass, pieces });
            }
            Effect::PaneShatter {
                at,
                normal,
                size,
                velocity,
                tile,
            } => {
                world.write_message(GlassShatter {
                    at: entity_to_engine(at, scale),
                    normal: entity_to_engine(normal, 1.0),
                    size: size * scale,
                    velocity: entity_to_engine(velocity, scale),
                    tile,
                });
            }
            Effect::Heal { target, amount } => {
                if let Some(mut h) = world.get_mut::<Health>(target)
                    && h.current > 0.0
                {
                    h.current = (h.current + amount / 100.0).min(h.max);
                }
            }
            Effect::SetHealth { target, health } => {
                if let Some(mut h) = world.get_mut::<Health>(target) {
                    h.current = (health / 100.0).clamp(0.0, h.max.max(health / 100.0));
                }
            }
            Effect::Sound { entry, at } => {
                world.write_message(PlaySound::at(entry, entity_to_engine(at, scale)));
            }
            Effect::AmbientStart {
                id,
                entry,
                at,
                source,
                volume,
                pitch,
                level,
            } => {
                let follow = source.and_then(|s| entity_node(world, s));
                world.write_message(SoundControl::Start(StartSound {
                    key: sound_key(id),
                    entry,
                    at: Some(entity_to_engine(at, scale)),
                    follow,
                    volume,
                    pitch,
                    level: level.map(SoundLevel::Db),
                }));
            }
            Effect::AmbientChange { id, volume, pitch } => {
                world.write_message(SoundControl::Change {
                    key: sound_key(id),
                    volume,
                    pitch,
                });
            }
            Effect::AmbientStop { id } => {
                world.write_message(SoundControl::Stop(sound_key(id)));
            }
            Effect::GameText { to, message } => {
                if to.is_none() || to == local {
                    world.resource_mut::<HudMessages>().show(message, now);
                }
            }
            Effect::ServerCommand(line) => server_command(world, &line),
            Effect::ClientCommand { player, command } => {
                if Some(player) == local
                    && let Some(mut c) = world.get_resource_mut::<Console>()
                {
                    c.submit(command);
                }
            }
        }
    }
}

/// The long-lived sound of a logic entity.
fn sound_key(id: EntId) -> SoundKey {
    SoundKey(1 << 63 | (id.generation as u64) << 32 | id.index as u64)
}

/// The ECS node standing for a logic entity, if it has one: a mover's
/// node or a prop the entity placed.
fn entity_node(world: &mut World, id: EntId) -> Option<Entity> {
    let logic = world.get_resource::<Logic>()?;
    if let Some((_, node)) = logic.nodes.iter().find(|(m, _)| *m == id) {
        return Some(*node);
    }
    let index = logic.world.get(id)?.map_index?;
    world
        .query::<(Entity, &PropEntity)>()
        .iter(world)
        .find(|(_, p)| p.0 == index)
        .map(|(e, _)| e)
}

/// An allowed server command: `say` prints; settings remember their old
/// value (put back at the next map) and run through the console.
fn server_command(world: &mut World, line: &str) {
    let Some(mut console) = world.get_resource_mut::<Console>() else { return };
    if let Some(text) = line.strip_prefix("say ") {
        console.info(format!("Console: {text}"));
        return;
    }
    let name = line.split_whitespace().next().unwrap_or("").to_string();
    let cvar = console.cvar(&name).cloned();
    console.submit(line.to_string());
    if let Some(cvar) = cvar {
        let old = (cvar.get)(world).unwrap_or_default();
        if let Some(mut l) = world.get_resource_mut::<Logic>()
            && !l.restore.iter().any(|(n, _)| *n == name)
        {
            l.restore.push((name, old));
        }
    }
}

/// Run one logic phase with the players and the static world.
fn run_phase(world: &mut World, phase: impl FnOnce(&mut LogicWorld, &dyn Collision)) {
    let Some(mut logic) = world.remove_resource::<Logic>() else { return };
    // The tick can change after the map loads (a game sets its own).
    logic.world.dt = world.resource::<Time<Fixed>>().timestep().as_secs_f32();
    let players = snapshot(world, &logic);
    logic.world.players = players.clone();
    {
        let col = WorldCollision {
            brushes: world.get_resource::<MapBrushes>(),
            terrain: world.get_resource::<MapTerrain>(),
            scale: logic.scale,
        };
        phase(&mut logic.world, &col);
    }
    let after = std::mem::take(&mut logic.world.players);
    write_back(world, logic.scale, &players, &after);
    sync_movers(world, &mut logic);
    sync_props(world, &mut logic);
    let effects = std::mem::take(&mut logic.world.effects);
    for line in logic.world.log.drain(..) {
        if line.contains("refused") {
            warn!("{line}");
        } else {
            debug!("{line}");
        }
    }
    let scale = logic.scale;
    world.insert_resource(logic);
    apply_effects(world, effects, scale);
}

fn pre(world: &mut World) {
    run_phase(world, |w, col| {
        w.run_thinks();
        w.step_movers(col);
        w.player_uses(col);
    });
}

fn post(world: &mut World) {
    run_phase(world, |w, col| {
        w.touch_triggers(col);
        w.touch_breakables();
        w.untouch();
        w.service_queue();
        w.end_frame();
    });
}

/// Damage dealt to mover nodes (breakables) and prop nodes this tick, into
/// the logic.
fn damage(world: &mut World, mut cursor: Local<MessageCursor<Damage>>) {
    let hits: Vec<Damage> = match world.get_resource::<Messages<Damage>>() {
        Some(m) => cursor.read(m).cloned().collect(),
        None => return,
    };
    let Some(logic) = world.get_resource::<Logic>() else {
        return;
    };
    let scale = logic.scale;
    let targeted: Vec<(EntId, Damage)> = hits
        .into_iter()
        .filter_map(|d| {
            let (id, _) = logic
                .nodes
                .iter()
                .chain(&logic.props)
                .find(|(_, n)| *n == d.target)?;
            Some((*id, d))
        })
        .collect();
    if targeted.is_empty() {
        return;
    }
    run_phase(world, |w, _| {
        for (id, d) in targeted {
            let attacker = d
                .attacker
                .filter(|a| w.player(*a).is_some())
                .map(super::world::Who::Player);
            let dir = Vec3::new(d.dir.x, -d.dir.z, d.dir.y);
            w.damage(
                id,
                d.amount * 100.0,
                d.kind,
                attacker,
                engine_to_entity(d.point, scale),
                dir,
            );
        }
    });
}
