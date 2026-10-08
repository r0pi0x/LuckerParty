//! The logic world in the ECS: built from the loaded map's entities, run
//! each fixed tick in the spec's order around the characters' movement
//! (entity_io.md "Per-tick order"): thinks, movers and +use before
//! movement; trigger touches, untouch, the event queue and removals after.

use avian3d::prelude::{
    AngularVelocity, Collider, ColliderAabb, ColliderDisabled, CollisionStart, ComputedMass, LinearVelocity,
    PhysicsSystems, Position, RigidBody, RigidBodyDisabled, Rotation,
};
use bevy::{ecs::message::MessageCursor, prelude::*};

use super::classes::ServerLine;
use super::hud::HudMessages;
use super::prop_damage::{Hit, Motion, PropExplosion};
use super::world::{Collision, Effect, EntId, LogicWorld, Player, SOLID_SKIN, SWEEP_EPS, Who};
use crate::console::{Console, ConsoleAppExt, Level};
use crate::core::{
    BaseVelocity, Damage, DamageKind, Damageable, EntityGravity, Explosion, Health, Hitgroup, Intent, LocalPlayer,
    MapBrush, MapBrushes, MapTerrain, MovementState, MovingSolid, RoundRestarts, SimSet, Team, Velocity,
};
use crate::map::breakables::{FallingPane, GibPiece, GlassImpact, GlassShatter, SpawnGibs};
use crate::map::entities::{engine_to_entity, entity_rotation, entity_to_engine, rotation_to_engine};
use crate::map::vis::{LogicHidden, VisClusters};
use crate::map::{
    BreakProp, BrushPanes, EntityPart, LightStyles, MapBrushEntity, MapEntities, PlaySound, PropEntity, PropHome,
    PropIndex, PropLook, PropSequence, SoundControl, SoundKey, SoundLevel, SoundscapeSwitches, SoundscapeTouches,
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
            .add_message::<GlassImpact>()
            .add_message::<FallingPane>()
            .configure_sets(
                FixedUpdate,
                (
                    LogicSet::Pre.after(SimSet::Rules).before(SimSet::Movement),
                    LogicSet::Post.after(SimSet::Movement).before(SimSet::Weapons),
                    LogicSet::Damage.after(SimSet::Weapons),
                )
                    // Map logic is the server's.
                    .run_if(crate::core::authoritative),
            )
            .init_resource::<LogicRecord>()
            .add_systems(FixedUpdate, (load, apply_record, pre).chain().in_set(LogicSet::Pre))
            .add_systems(FixedUpdate, post.in_set(LogicSet::Post))
            .add_systems(FixedUpdate, (damage, fire_outputs).in_set(LogicSet::Damage))
            .add_message::<crate::map::entities::FireEntityOutput>()
            // Physics impacts on props and players (after the physics
            // step; the damage lands next tick).
            .add_message::<CollisionStart>()
            .add_message::<BreakProp>()
            .add_message::<Explosion>()
            .add_message::<crate::map::prop_physics::PropAwakened>()
            .init_resource::<PreStep>()
            .init_resource::<Impacts>()
            .add_systems(
                FixedPostUpdate,
                record_velocities.before(PhysicsSystems::StepSimulation),
            )
            .add_systems(FixedPostUpdate, impacts.after(PhysicsSystems::Writeback));
        app.console_command(
            "ent_fire",
            "ent_fire <target> <input> [value]: send a map entity an input (names, wildcards, classnames), \
             the local player as activator.",
            |w, a| {
                let (Some(target), Some(input)) = (a.first(), a.get(1)) else {
                    return Err("ent_fire <target> <input> [value]".into());
                };
                let value = match a.get(2..).filter(|v| !v.is_empty()) {
                    Some(v) => super::Value::Str(v.join(" ")),
                    None => super::Value::Void,
                };
                let local = w
                    .query_filtered::<Entity, With<LocalPlayer>>()
                    .iter(w)
                    .next()
                    .map(super::world::Who::Player);
                let mut logic = w.get_resource_mut::<Logic>().ok_or("no map logic loaded")?;
                logic.world.queue_input(target, input, value.clone(), 0.0, local);
                info!("ent_fire {target} {input} {value:?}");
                Ok(None)
            },
        );
        crate::console::resource_cvar::<LogicRecord, u8>(
            app,
            "mashup_logic_record",
            "1: record the outputs map entities fire and the inputs they get (the latest few hundred; \
             the debug UI's Logic tab lists them).",
            |r| &mut r.0,
        );
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
            world.remove_resource::<LightStyles>();
            world.remove_resource::<crate::map::TonemapInputs>();
            world.remove_resource::<crate::map::vis::AreaPortalStates>();
            world.remove_resource::<crate::map::vis::OccluderStates>();
            world.remove_resource::<SoundscapeTouches>();
            world.remove_resource::<SoundscapeSwitches>();
            world.remove_resource::<crate::map::fire::MapFires>();
        }
        (Some(m), b) if b.as_ref().is_none_or(|b| !std::sync::Arc::ptr_eq(b, &m.entities)) => {
            restore_settings(world);
            world.write_message(SoundControl::StopAll);
            let dt = world.resource::<Time<Fixed>>().timestep().as_secs_f32();
            let mut logic = LogicWorld::new(dt);
            logic.collision = static_collision(world, m.scale);
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
            let Some(mut logic) = world.remove_resource::<Logic>() else {
                return;
            };
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
        set_node_shown(&mut e, true);
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
        set_node_shown(&mut e, false);
        e.insert(MovingSolid::default()).remove::<Damageable>();
    }
}

/// Show or hide a mover node for the logic (`LogicHidden`): shown, it is
/// drawn when visibility culling (`VisClusters`, by its bounds) allows.
fn set_node_shown(e: &mut EntityWorldMut, visible: bool) {
    let hidden = e.contains::<LogicHidden>();
    if visible {
        let culled = e.get::<VisClusters>().is_some_and(|v| !v.potentially_visible);
        if hidden {
            e.remove::<LogicHidden>();
        }
        let want = if culled { Visibility::Hidden } else { Visibility::Inherited };
        if e.get::<Visibility>() != Some(&want) {
            e.insert(want);
        }
    } else if !hidden || e.get::<Visibility>() != Some(&Visibility::Hidden) {
        e.insert((LogicHidden, Visibility::Hidden));
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
        let Some(state) = states.iter().find(|s| s.id == id) else {
            continue;
        };
        let (visible, solid, damageable) = (state.visible, state.solid, state.damageable);
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
        if restart {
            crate::map::prop_physics::reset(world, node, state.asleep);
        }
        set_prop_shown(world, node, visible, solid, true);
        set_prop_look(world, node, state, logic.round);
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
    // The body comes back before its colliders do: avian's query tree
    // copies "body disabled" into a collider's proxy when the collider
    // joins it and doesn't clear it when RigidBodyDisabled goes away. A
    // stale flag makes each new contact pair of that collider start out
    // generating no constraints and then switch on in the step it starts
    // touching, which links the contact into an island twice and trips
    // avian's island assertion a few rounds later (tests/it/map_logic.rs
    // `restored_prop_contacts_join_islands_once`).
    let enable_body = {
        let mut e = world.entity_mut(node);
        let enable = exists && e.contains::<RigidBody>() && e.contains::<RigidBodyDisabled>();
        if enable {
            e.remove::<RigidBodyDisabled>();
        }
        enable
    };
    let colliders: Vec<Entity> = std::iter::once(node)
        .chain(world.get::<Children>(node).map(|c| c.to_vec()).unwrap_or_default())
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
        } else if solid && enable_body {
            // Solid all along under a disabled body: rejoin the tree so
            // its proxy forgets the disabled body.
            e.insert(ColliderDisabled).remove::<ColliderDisabled>();
        }
    }
    let mut e = world.entity_mut(node);
    // A prop's own brushes (movement) follow its collision.
    if let Some(mut m) = e.get_mut::<MovingSolid>()
        && m.solid != solid
    {
        m.solid = solid;
    }
    if !exists && e.contains::<RigidBody>() && !e.contains::<RigidBodyDisabled>() {
        e.insert(RigidBodyDisabled);
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
        match states.iter().find(|s| s.id == id) {
            Some(state) => {
                set_prop_shown(world, node, state.visible, state.solid, true);
                set_prop_look(world, node, state, logic.world.round);
            }
            None => set_prop_shown(world, node, false, false, false),
        }
    }
}

/// A prop node shows the logic prop's skin, body group and sequence
/// (`map::PropLook`, `map::PropSequence`; written only when they change).
fn set_prop_look(world: &mut World, node: Entity, state: &super::props::PropState, round: u32) {
    let Ok(mut e) = world.get_entity_mut(node) else { return };
    let look = PropLook {
        skin: state.skin,
        body_group: state.body_group,
    };
    if e.get::<PropLook>() != Some(&look) {
        e.insert(look);
    }
    let sequence = PropSequence {
        play: state.sequence.clone(),
        default: state.default_sequence.clone(),
        round,
    };
    if e.get::<PropSequence>() != Some(&sequence) {
        e.insert(sequence);
    }
}

/// Sprites and dust volumes (`map::EntityPart`) follow their entity: on,
/// off, or gone (killed) until a round restart; light styles follow the
/// lights (`map::LightStyles`), areaportals their doors and inputs
/// (`map::vis::AreaPortalStates`), occluders their inputs
/// (`map::vis::OccluderStates`).
fn sync_visuals(world: &mut World, logic: &Logic) {
    let mut states = logic.world.part_states();
    states.sort_by_key(|s| s.0);
    let mut parts = world.query::<&mut EntityPart>();
    for mut part in parts.iter_mut(world) {
        let want = match states.binary_search_by_key(&part.entity, |s| s.0) {
            Ok(i) => (states[i].2, true),
            Err(_) => (false, false),
        };
        if (part.on, part.exists) != want {
            part.on = want.0;
            part.exists = want.1;
        }
    }
    let styles = LightStyles {
        styles: logic.world.light_styles().to_vec(),
    };
    if world.get_resource::<LightStyles>() != Some(&styles) {
        world.insert_resource(styles);
    }
    let tonemap = logic.world.tonemap();
    if world.get_resource::<crate::map::TonemapInputs>() != Some(tonemap) {
        world.insert_resource(tonemap.clone());
    }
    let portals = crate::map::vis::AreaPortalStates {
        closed: logic.world.closed_area_portals(),
    };
    if world.get_resource::<crate::map::vis::AreaPortalStates>() != Some(&portals) {
        world.insert_resource(portals);
    }
    let occluders = crate::map::vis::OccluderStates {
        inactive: logic.world.inactive_occluders(),
    };
    if world.get_resource::<crate::map::vis::OccluderStates>() != Some(&occluders) {
        world.insert_resource(occluders);
    }
}

/// The trigger_soundscape volumes the local player touches, for the
/// soundscape selection (`map::SoundscapeTouches`).
fn sync_soundscapes(world: &mut World, logic: &Logic) {
    let local = world.query_filtered::<Entity, With<LocalPlayer>>().iter(world).next();
    let touched: Vec<usize> = match local {
        Some(p) => logic
            .world
            .ids()
            .into_iter()
            .filter_map(|id| {
                let e = logic.world.get(id)?;
                if !e.classname.eq_ignore_ascii_case("trigger_soundscape") {
                    return None;
                }
                let super::classes::Class::Trigger(t) = &e.class else {
                    return None;
                };
                (t.enabled && t.touching.contains(&super::world::Who::Player(p))).then_some(e.map_index?)
            })
            .collect(),
        None => Vec::new(),
    };
    let want = SoundscapeTouches(Some(touched));
    if world.get_resource::<SoundscapeTouches>() != Some(&want) {
        world.insert_resource(want);
    }
    // env_soundscapes the Enable/Disable inputs left on.
    let enabled: Vec<usize> = logic
        .world
        .part_states()
        .into_iter()
        .filter(|(_, kind, on)| *kind == super::visuals::PartKind::Soundscape && *on)
        .map(|(i, ..)| i)
        .collect();
    let switches = SoundscapeSwitches(Some(enabled));
    if world.get_resource::<SoundscapeSwitches>() != Some(&switches) {
        world.insert_resource(switches);
    }
}

fn restore_settings(world: &mut World) {
    let Some(restore) = world
        .get_resource_mut::<Logic>()
        .map(|mut l| std::mem::take(&mut l.restore))
    else {
        return;
    };
    if let Some(mut c) = world.get_resource_mut::<Console>() {
        for (name, value) in restore {
            info!("point_servercommand: map unloaded, {name} back to {value}");
            c.info(format!("point_servercommand: map unloaded, {name} back to {value}"));
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

/// The static world owned (the logic keeps it for traces outside a
/// phase: fires dropping inside an input, their line-of-sight checks).
struct StaticCollision {
    brushes: MapBrushes,
    terrain: Option<MapTerrain>,
    scale: f32,
}

impl StaticCollision {
    fn view(&self) -> WorldCollision<'_> {
        WorldCollision {
            brushes: Some(&self.brushes),
            terrain: self.terrain.as_ref(),
            scale: self.scale,
        }
    }
}

impl Collision for StaticCollision {
    fn sweep(&self, mins: Vec3, maxs: Vec3, from: Vec3, to: Vec3) -> f32 {
        self.view().sweep(mins, maxs, from, to)
    }
    fn solid(&self, mins: Vec3, maxs: Vec3, at: Vec3) -> bool {
        self.view().solid(mins, maxs, at)
    }
}

/// The map's static world for the logic, once the map has one.
fn static_collision(world: &World, scale: f32) -> Option<std::sync::Arc<dyn Collision + Send + Sync>> {
    let mut brushes = world.get_resource::<MapBrushes>()?.clone();
    // Fire's traces pass player clips and grates.
    if let Some(skip) = world.get_resource::<crate::map::MapTraceSkip>() {
        let skip: std::collections::HashSet<usize> = skip.0.iter().copied().collect();
        let mut i = 0;
        brushes.0.retain(|_| {
            i += 1;
            !skip.contains(&(i - 1))
        });
    }
    Some(std::sync::Arc::new(StaticCollision {
        brushes,
        terrain: world.get_resource::<MapTerrain>().cloned(),
        scale,
    }))
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
            p.ground = state.ground.and_then(|g| {
                logic
                    .nodes
                    .iter()
                    .chain(&logic.props)
                    .find(|(_, n)| *n == g)
                    .map(|(id, _)| *id)
            });
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
        let Some(old) = before.iter().find(|b| b.entity == p.entity) else {
            continue;
        };
        let Ok(mut e) = world.get_entity_mut(p.entity) else {
            continue;
        };
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
        if p.base_velocity != old.base_velocity || p.base_touched != old.base_touched || p.unground || p.teleported {
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
    let nodes: std::collections::HashMap<EntId, Entity> = logic.nodes.iter().copied().collect();
    for (id, origin, angles, velocity, visible, solid) in logic.world.mover_poses() {
        let Some(node) = nodes.get(&id) else {
            continue;
        };
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
            let depth = if depth.0 > depth.1 {
                (0.0, 0.0)
            } else {
                (depth.0 * s, depth.1 * s)
            };
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
        let Ok(mut e) = world.get_entity_mut(*node) else {
            continue;
        };
        let transform = Transform::from_translation(entity_to_engine(origin, logic.scale))
            .with_rotation(rotation_to_engine(entity_rotation(angles)));
        let mut moved = true;
        if let Some(mut t) = e.get_mut::<Transform>() {
            moved = *t != transform;
            if moved {
                *t = transform;
            }
        }
        set_node_shown(&mut e, visible);
        // Its brushes follow its pose: rebuilt (and the component written,
        // which movement reads) only when it moved or changed.
        let solids = logic.world.mover_solid(id);
        let velocity = entity_to_engine(velocity, logic.scale);
        let same = !moved
            && e.get::<MovingSolid>().is_some_and(|m| {
                m.solid == solid && m.velocity == velocity && m.brushes.len() == solids.map_or(0, |b| b.len())
            });
        if !same {
            let brushes: Vec<MapBrush> = solids
                .map(|b| b.iter().map(|b| brush_to_engine(b, logic.scale)).collect())
                .unwrap_or_default();
            e.insert(MovingSolid {
                brushes,
                velocity,
                solid,
            });
        }
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
    let local = world.query_filtered::<Entity, With<LocalPlayer>>().iter(world).next();
    for effect in effects {
        match effect {
            Effect::Damage { target, amount, crush } => {
                let point = world.get::<Transform>(target).map_or(Vec3::ZERO, |t| t.translation);
                world.write_message(Damage {
                    force: bevy::math::Vec3::ZERO,
                    target,
                    attacker: None,
                    amount: amount / 100.0,
                    point,
                    dir: Vec3::NEG_Y,
                    hitgroup: Hitgroup::Generic,
                    kind: if crush { DamageKind::Crush } else { DamageKind::Generic },
                    weapon: None,
                });
            }
            Effect::Burn { target, amount } => {
                let point = world.get::<Transform>(target).map_or(Vec3::ZERO, |t| t.translation);
                world.write_message(Damage {
                    force: Vec3::ZERO,
                    target,
                    attacker: None,
                    amount: amount / 100.0,
                    point,
                    dir: Vec3::NEG_Y,
                    hitgroup: Hitgroup::Generic,
                    kind: DamageKind::Burn,
                    weapon: None,
                });
            }
            Effect::FlameStart { id, target } => {
                let follow = match target {
                    Who::Player(p) => Some(p),
                    Who::Ent(e) => prop_node(world, e),
                };
                let at = follow.and_then(|f| world.get::<Transform>(f)).map(|t| t.translation);
                world.write_message(SoundControl::Start(StartSound {
                    key: sound_key(id),
                    entry: super::fire::BURNING_SOUND.into(),
                    at,
                    follow,
                    volume: None,
                    pitch: None,
                    level: None,
                    ..Default::default()
                }));
            }
            Effect::Gibs {
                set,
                glass,
                pieces,
                bounce,
            } => {
                let pieces = pieces
                    .into_iter()
                    .map(|g| {
                        GibPiece::thrown(
                            entity_to_engine(g.position, scale),
                            entity_to_engine(g.velocity, scale),
                            entity_to_engine(g.spin * std::f32::consts::PI / 180.0, 1.0),
                            g.life,
                        )
                    })
                    .collect();
                world.write_message(SpawnGibs {
                    set,
                    glass,
                    prop: false,
                    pieces,
                    shatters: None,
                    bounce: bounce.map(str::to_string),
                });
            }
            Effect::PropBreak { id, sound, explode } => prop_broke(world, id, sound, explode, scale),
            Effect::PropMotion { id, motion } => {
                let Some(node) = prop_node(world, id) else { continue };
                match motion {
                    Motion::Enable => {
                        crate::map::prop_physics::enable_motion(world, node);
                    }
                    Motion::Disable => crate::map::prop_physics::disable_motion(world, node),
                    Motion::Wake => crate::map::prop_physics::wake(world, node),
                    Motion::Sleep => crate::map::prop_physics::sleep(world, node),
                }
            }
            Effect::PaneShatter {
                at,
                normal,
                size,
                velocity,
                tile,
                shard,
            } => {
                world.write_message(GlassShatter {
                    at: entity_to_engine(at, scale),
                    normal: entity_to_engine(normal, 1.0),
                    size: size * scale,
                    velocity: entity_to_engine(velocity, scale),
                    tile,
                    shard: shard * scale,
                });
            }
            Effect::PaneFall {
                at,
                axes,
                size,
                body,
                spin,
            } => {
                world.write_message(FallingPane {
                    at: entity_to_engine(at, scale),
                    axes: axes.map(|a| entity_to_engine(a, 1.0)),
                    size: size * scale,
                    body,
                    spin: entity_to_engine(spin * std::f32::consts::PI / 180.0, 1.0),
                    shard: super::breakables::SMALL_SHARD * scale,
                });
            }
            Effect::Explosion {
                at,
                damage,
                radius,
                attacker,
                inflictor,
            } => {
                let attacker = match attacker {
                    Some(Who::Player(e)) => Some(e),
                    Some(Who::Ent(e)) => entity_node(world, e),
                    None => None,
                };
                let inflictor = entity_node(world, inflictor);
                world.write_message(Explosion {
                    origin: entity_to_engine(at, scale),
                    damage: damage / 100.0,
                    radius: radius * scale,
                    attacker,
                    inflictor,
                    sound: None,
                    weapon: None,
                });
            }
            Effect::GlassImpact { at, normal } => {
                world.write_message(GlassImpact {
                    at: entity_to_engine(at, scale),
                    normal: entity_to_engine(normal, 1.0),
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
            Effect::Sound {
                entry,
                at,
                volume,
                pitch,
            } => {
                let mut sound = PlaySound::at(entry, entity_to_engine(at, scale));
                sound.volume = volume;
                sound.pitch = pitch;
                world.write_message(sound);
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
                    ..Default::default()
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
            Effect::DamageFilter { target, filter } => {
                let Ok(mut e) = world.get_entity_mut(target) else { continue };
                match filter {
                    Some((bits, negated)) => {
                        // A damage passes when its Source type bits equal
                        // the filter's (entity_io.md, filter_damage_type).
                        let blocked = DAMAGE_KINDS
                            .iter()
                            .filter(|(_, b)| (*b == bits) == negated)
                            .map(|(k, _)| *k)
                            .collect();
                        e.insert(crate::core::DamageFilter { blocked });
                    }
                    None => {
                        e.remove::<crate::core::DamageFilter>();
                    }
                }
            }
            Effect::Equip { target, items, strip } => {
                world.write_message(crate::core::Equip { target, items, strip });
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

/// Our damage kinds as Source damage-type bits (DMG_GENERIC 0, CRUSH 1,
/// BULLET 2, SLASH 4, BURN 8, FALL 32, BLAST 64; the public SDK's
/// names). Melee as slash and bullets without their extra flag bits are
/// our reading; fall, the one maps filter, is exact.
const DAMAGE_KINDS: [(DamageKind, u32); 7] = [
    (DamageKind::Generic, 0),
    (DamageKind::Crush, 1),
    (DamageKind::Bullet, 2),
    (DamageKind::Melee, 4),
    (DamageKind::Burn, 8),
    (DamageKind::Fall, 32),
    (DamageKind::Blast, 64),
];

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

/// Game commands a map may run as if they were settings: cvars in Source,
/// console commands here. They change nothing to put back.
const SETTING_COMMANDS: &[&str] = &["mp_restartgame"];

/// A server command that passed `classes::check_server_command`: `say`
/// prints; a setting our console has as a cvar remembers its value from
/// before the map first changed it (put back when the map unloads,
/// `restore_settings`) and runs through the console; anything else is
/// logged and ignored. Each outcome prints a console line.
fn server_command(world: &mut World, line: &ServerLine) {
    let Some(mut console) = world.get_resource_mut::<Console>() else {
        return;
    };
    let (name, value) = match line {
        ServerLine::Say(text) => {
            console.info(format!("Console: {text}"));
            return;
        }
        ServerLine::Set { name, value } => (name.clone(), value.clone()),
    };
    let run = format!("{name} {}", crate::console::quote(&value));
    let Some(cvar) = console.cvar(&name).cloned() else {
        if SETTING_COMMANDS.contains(&name.as_str()) && console.command(&name).is_some() {
            console.info(format!("point_servercommand: {name} {value}"));
            console.submit(run);
        } else {
            let what = if console.command(&name).is_some() { "a command, not a setting" } else { "no such setting" };
            console.print(Level::Warn, format!("point_servercommand: ignored '{name} {value}' ({what})"));
            warn!("point_servercommand: ignored '{name} {value}' ({what})");
        }
        return;
    };
    console.submit(run);
    // The line runs from the queue later, so this is still the value
    // before the map's.
    let old = (cvar.get)(world).unwrap_or_default();
    let mut first = false;
    if let Some(mut l) = world.get_resource_mut::<Logic>()
        && !l.restore.iter().any(|(n, _)| *n == name)
    {
        l.restore.push((name.clone(), old.clone()));
        first = true;
    }
    let was = if first { format!(" (was {old})") } else { String::new() };
    info!("point_servercommand: {name} {value}{was}");
    world
        .resource_mut::<Console>()
        .info(format!("point_servercommand: {name} {value}{was}"));
}

/// Run one logic phase with the players and the static world.
fn run_phase(world: &mut World, phase: impl FnOnce(&mut LogicWorld, &dyn Collision)) {
    let Some(mut logic) = world.remove_resource::<Logic>() else {
        return;
    };
    // The tick can change after the map loads (a game sets its own).
    logic.world.dt = world.resource::<Time<Fixed>>().timestep().as_secs_f32();
    if logic.world.collision.is_none() {
        logic.world.collision = static_collision(world, logic.scale);
    }
    prop_bounds(world, &mut logic);
    let players = snapshot(world, &logic);
    logic.world.players = players.clone();
    {
        let col = WorldCollision {
            brushes: world.get_resource::<MapBrushes>(),
            terrain: world.get_resource::<MapTerrain>(),
            scale: logic.scale,
        };
        let _span = info_span!("logic: phase").entered();
        phase(&mut logic.world, &col);
    }
    // Spans for traces (docs/performance.md): this exclusive system's parts.
    let span = info_span!("logic: sync to the ECS").entered();
    sync_fires(world, &logic);
    let after = std::mem::take(&mut logic.world.players);
    write_back(world, logic.scale, &players, &after);
    info_span!("logic: sync_movers").in_scope(|| sync_movers(world, &mut logic));
    sync_props(world, &mut logic);
    sync_visuals(world, &logic);
    sync_soundscapes(world, &logic);
    drop(span);
    let effects = std::mem::take(&mut logic.world.effects);
    for line in logic.world.log.drain(..) {
        if line.contains("refused") {
            warn!("{line}");
            // What a map's server and client commands were refused shows
            // in the console too.
            if line.starts_with("point_")
                && let Some(mut c) = world.get_resource_mut::<Console>()
            {
                c.print(Level::Warn, line);
            }
        } else {
            debug!("{line}");
        }
    }
    let scale = logic.scale;
    world.insert_resource(logic);
    apply_effects(world, effects, scale);
}

/// Where each logic prop's body is now (its collider's world box), for
/// fire (flame size and place, damage boxes).
fn prop_bounds(world: &mut World, logic: &mut Logic) {
    let scale = logic.scale;
    for (id, node) in logic.props.clone() {
        let aabb = std::iter::once(node)
            .chain(world.get::<Children>(node).map(|c| c.to_vec()).unwrap_or_default())
            .find_map(|e| world.get::<ColliderAabb>(e).copied());
        if let Some(a) = aabb {
            let (lo, hi) = (engine_to_entity(a.min, scale), engine_to_entity(a.max, scale));
            logic.world.set_prop_bounds(id, (lo.min(hi), lo.max(hi)));
        }
    }
}

/// Lit fires and burning entities for the game to draw
/// (`map::fire::MapFires`).
fn sync_fires(world: &mut World, logic: &Logic) {
    use crate::map::fire::{FireLook, FlameLook, MapFires};
    let scale = logic.scale;
    let key = |id: EntId, serial: u32| (serial as u64) << 48 | (id.generation as u64 & 0xffff) << 32 | id.index as u64;
    let fires: Vec<FireLook> = logic
        .world
        .fire_looks()
        .into_iter()
        .filter(|f| !f.plasma)
        .map(|f| FireLook {
            key: key(f.id, f.serial),
            at: entity_to_engine(f.at, scale),
            size: f.size,
            smokeless: f.smokeless,
        })
        .collect();
    let flames: Vec<FlameLook> = logic
        .world
        .flame_looks()
        .into_iter()
        .filter_map(|f| {
            let (follow, lo, hi) = match f.target {
                Who::Player(p) => {
                    let pl = logic.world.player(p)?;
                    (Some(p), pl.origin + pl.mins, pl.origin + pl.maxs)
                }
                Who::Ent(e) => {
                    let node = logic.props.iter().find(|(id, _)| *id == e).map(|(_, n)| *n);
                    let (lo, hi) = logic.world.prop(e)?.bounds.unwrap_or_else(|| {
                        let o = logic.world.get(e).map_or(Vec3::ZERO, |x| x.origin);
                        (o - Vec3::splat(8.0), o + Vec3::splat(8.0))
                    });
                    (node, lo, hi)
                }
            };
            let (a, b) = (entity_to_engine(lo, scale), entity_to_engine(hi, scale));
            Some(FlameLook {
                key: key(f.id, 0),
                follow,
                min: a.min(b),
                max: a.max(b),
            })
        })
        .collect();
    let want = MapFires { fires, flames };
    if world.get_resource::<MapFires>() != Some(&want) {
        world.insert_resource(want);
    }
}

/// `mashup_logic_record 1`: the logic world records the outputs it fires
/// and the inputs it delivers (the debug UI's Logic tab lists them),
/// keeping the latest `RECORD_KEEP`.
#[derive(Resource, Default)]
struct LogicRecord(u8);

const RECORD_KEEP: usize = 256;

fn apply_record(rec: Res<LogicRecord>, logic: Option<ResMut<Logic>>, mut last: Local<u8>) {
    let Some(mut logic) = logic else { return };
    // Turned off: stop (tests that record by hand leave the cvar at 0).
    if rec.0 != *last {
        *last = rec.0;
        logic.world.record = rec.0 != 0;
    }
    if rec.0 == 0 {
        return;
    }
    if !logic.world.record {
        logic.world.record = true;
    }
    for len in [logic.world.fired.len(), logic.world.deliveries.len()] {
        if len > RECORD_KEEP * 2 {
            let w = &mut logic.world;
            let fired = w.fired.len().saturating_sub(RECORD_KEEP);
            w.fired.drain(..fired);
            let delivered = w.deliveries.len().saturating_sub(RECORD_KEEP);
            w.deliveries.drain(..delivered);
            break;
        }
    }
}

fn pre(world: &mut World) {
    run_phase(world, |w, col| {
        w.run_thinks();
        w.step_movers(col);
        w.player_uses(col);
    });
}

fn post(world: &mut World, mut awake: Local<MessageCursor<crate::map::prop_physics::PropAwakened>>) {
    // Props that started asleep and woke (map::prop_physics).
    let woke: Vec<Entity> = match world.get_resource::<Messages<crate::map::prop_physics::PropAwakened>>() {
        Some(m) => awake.read(m).map(|a| a.0).collect(),
        None => Vec::new(),
    };
    let woke: Vec<EntId> = match world.get_resource::<Logic>() {
        Some(l) => woke
            .into_iter()
            .filter_map(|n| l.props.iter().find(|(_, p)| *p == n).map(|(id, _)| *id))
            .collect(),
        None => Vec::new(),
    };
    run_phase(world, |w, col| {
        for id in woke {
            w.prop_awakened(id);
        }
        w.touch_triggers(col);
        w.touch_breakables();
        w.pressure_props();
        w.untouch();
        w.service_queue();
        w.end_frame();
    });
}

/// Damage dealt to mover nodes (breakables) and prop nodes this tick, into
/// the logic (with the push that came with it, and its attacker: a
/// player, or a prop that hit it).
fn damage(world: &mut World, mut cursor: Local<MessageCursor<Damage>>) {
    let hits: Vec<Damage> = match world.get_resource::<Messages<Damage>>() {
        Some(m) => cursor.read(m).cloned().collect(),
        None => return,
    };
    let Some(logic) = world.get_resource::<Logic>() else {
        return;
    };
    let scale = logic.scale;
    let node_id = |e: Entity| {
        logic
            .nodes
            .iter()
            .chain(&logic.props)
            .find(|(_, n)| *n == e)
            .map(|(id, _)| *id)
    };
    let targeted: Vec<(EntId, Damage, Option<Who>)> = hits
        .into_iter()
        .filter_map(|d| {
            let id = node_id(d.target)?;
            let by = d.attacker.and_then(node_id).map(Who::Ent);
            Some((id, d, by))
        })
        .collect();
    if targeted.is_empty() {
        return;
    }
    run_phase(world, |w, _| {
        for (id, d, by) in targeted {
            let attacker = d.attacker.filter(|a| w.player(*a).is_some()).map(Who::Player).or(by);
            let dir = Vec3::new(d.dir.x, -d.dir.z, d.dir.y);
            let point = engine_to_entity(d.point, scale);
            let hit = Hit {
                amount: d.amount * 100.0,
                kind: d.kind,
                attacker,
                point,
                dir,
                force: d.force.length() / scale,
                direct: false,
            };
            if !w.prop_hit(id, hit) {
                w.damage(id, hit.amount, d.kind, attacker, point, dir);
            }
        }
    });
}

/// The node of a logic prop.
fn prop_node(world: &World, id: EntId) -> Option<Entity> {
    let logic = world.get_resource::<Logic>()?;
    logic.props.iter().find(|(p, _)| *p == id).map(|(_, n)| *n)
}

/// Where a prop node is (its body's pose when it has one) and the centre
/// of its bounds.
fn prop_pose(world: &World, node: Entity) -> (Vec3, Quat, Vec3) {
    let (pos, rot) = match (world.get::<Position>(node), world.get::<Rotation>(node)) {
        (Some(p), Some(r)) => (p.0, r.0),
        _ => world
            .get::<GlobalTransform>(node)
            .map_or((Vec3::ZERO, Quat::IDENTITY), |g| {
                let (_, r, t) = g.to_scale_rotation_translation();
                (t, r)
            }),
    };
    let centre = std::iter::once(node)
        .chain(world.get::<Children>(node).map(|c| c.to_vec()).unwrap_or_default())
        .find_map(|e| world.get::<ColliderAabb>(e))
        .map_or(pos, |a| a.center());
    (pos, rot, centre)
}

/// A prop broke: its break sound where it is, its explosion from its
/// centre, its pieces from its pose and velocity.
fn prop_broke(world: &mut World, id: EntId, sound: Option<String>, explode: Option<PropExplosion>, scale: f32) {
    let Some(node) = prop_node(world, id) else { return };
    let (pos, rot, centre) = prop_pose(world, node);
    if let Some(s) = sound {
        world.write_message(PlaySound::at(s, pos));
    }
    if let Some(x) = explode {
        let attacker = match x.attacker {
            Some(Who::Player(e)) => Some(e),
            Some(Who::Ent(e)) => prop_node(world, e),
            None => None,
        };
        world.write_message(Explosion {
            origin: centre,
            damage: x.damage / 100.0,
            radius: x.radius * scale,
            attacker,
            inflictor: Some(node),
            sound: x.sound,
            weapon: None,
        });
    }
    if let Some(index) = world.get::<PropIndex>(node).map(|p| p.0) {
        let velocity = world.get::<LinearVelocity>(node).map_or(Vec3::ZERO, |v| v.0);
        let spin = world.get::<AngularVelocity>(node).map_or(Vec3::ZERO, |v| v.0);
        let skin = world.get::<PropLook>(node).map(|l| l.skin);
        world.write_message(BreakProp {
            prop: index,
            transform: Transform::from_translation(pos).with_rotation(rot),
            velocity,
            spin,
            skin,
        });
    }
}

/// Each moving body's velocity before this physics step (impact damage
/// needs the speed a collision took away).
#[derive(Resource, Default)]
struct PreStep(std::collections::HashMap<Entity, Vec3>);

fn record_velocities(mut pre: ResMut<PreStep>, bodies: Query<(Entity, &RigidBody, &LinearVelocity)>) {
    pre.0.clear();
    pre.0.extend(
        bodies
            .iter()
            .filter(|(_, rb, _)| rb.is_dynamic())
            .map(|(e, _, v)| (e, v.0)),
    );
}

/// A player's mass for impacts (kg; Source's player shadow).
const PLAYER_MASS: f32 = 85.0;
/// Damage (health points) that breaks any breakable: an impact on one
/// that breaks on the first physics impact (flag 512).
const BREAK_NOW: f32 = 1.0e6;
/// An impact is measured over this many physics steps from its first
/// contact: avian's contacts are speculative and soft, so a body is
/// stopped over a few steps where Source's stops it in one.
const IMPACT_STEPS: u32 = 4;

/// Who an impact damages.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Receiver {
    /// A logic prop: its table, energy scale and mass.
    Prop(super::prop_damage::ImpactTable, f32, f32),
    /// A breakable brush (static): its table, energy scale and mass, and
    /// whether the first impact breaks it.
    Breakable(super::prop_damage::ImpactTable, f32, f32, bool),
    Player,
}

/// An impact being measured: both bodies' velocities when it began and
/// the slowest each has been since (engine space).
#[derive(Clone, Debug)]
struct Impact {
    me: Entity,
    other: Entity,
    receiver: Receiver,
    before: [Vec3; 2],
    slowest: [Vec3; 2],
    steps: u32,
}

#[derive(Resource, Default)]
struct Impacts(Vec<Impact>);

/// Physics impacts (prop_damage.md 5, 6): a new contact of a logic prop's
/// body damages it by the speed the collision took away (its table and
/// energy scale), and a moving prop hitting a player damages the player
/// (the player table), as crush damage applied next tick.
#[allow(clippy::too_many_arguments)]
fn impacts(
    mut started: MessageReader<CollisionStart>,
    logic: Option<Res<Logic>>,
    pre: Res<PreStep>,
    mut pending: ResMut<Impacts>,
    bodies: Query<(
        &RigidBody,
        &LinearVelocity,
        Option<&ComputedMass>,
        Has<crate::map::PhysicsProp>,
    )>,
    characters: Query<&MovementState, With<Intent>>,
    shadows: Query<&crate::map::prop_physics::PhysicsShadow>,
    positions: Query<&Position>,
    mut damage: MessageWriter<Damage>,
) {
    let Some(logic) = logic else {
        started.clear();
        pending.0.clear();
        return;
    };
    let scale = logic.scale;
    let velocity = |e: Entity| match bodies.get(e) {
        Ok((rb, v, ..)) if rb.is_dynamic() => v.0,
        _ => Vec3::ZERO,
    };
    // A player's physics shadow touches props for its player.
    let player = |e: Entity| shadows.get(e).map_or(e, |s| s.owner);
    // New contacts start being measured.
    for s in started.read() {
        let (b1, b2) = (
            player(s.body1.unwrap_or(s.collider1)),
            player(s.body2.unwrap_or(s.collider2)),
        );
        if b1 == b2 {
            continue;
        }
        for (me, other) in [(b1, b2), (b2, b1)] {
            let receiver = if let Some((id, _)) = logic.props.iter().find(|(_, n)| *n == me)
                && let Some((table, energy, mass)) = logic.world.prop_impact(*id)
                // Players' contacts are left out (prop_damage.md Q13).
                && !characters.contains(other)
            {
                Receiver::Prop(table, energy, mass.max(0.1))
            } else if let Some((id, _)) = logic.nodes.iter().find(|(_, n)| *n == me)
                && let Some((table, energy, mass, instant)) = logic.world.breakable_impact(*id)
                // Only physics objects (players break them by touch).
                && bodies.get(other).is_ok_and(|(rb, ..)| rb.is_dynamic())
                && !characters.contains(other)
            {
                Receiver::Breakable(table, energy, mass, instant)
            } else if characters.get(me).is_ok_and(|s| s.ground != Some(other))
                && bodies.get(other).is_ok_and(|(rb, _, _, prop)| rb.is_dynamic() && prop)
            {
                Receiver::Player
            } else {
                continue;
            };
            if pending.0.iter().any(|i| i.me == me && i.other == other) {
                continue;
            }
            let before = [me, other].map(|e| pre.0.get(&e).copied().unwrap_or_else(|| velocity(e)));
            pending.0.push(Impact {
                me,
                other,
                receiver,
                before,
                slowest: before,
                steps: 0,
            });
        }
    }
    // Measure; done ones deal their damage.
    let mut done = Vec::new();
    pending.0.retain_mut(|i| {
        for (k, e) in [i.me, i.other].into_iter().enumerate() {
            let v = velocity(e);
            if v.length_squared() < i.slowest[k].length_squared() {
                i.slowest[k] = v;
            }
        }
        i.steps += 1;
        if i.steps >= IMPACT_STEPS {
            done.push(i.clone());
            false
        } else {
            true
        }
    });
    for i in done {
        let fixed = |e: Entity| !bodies.get(e).is_ok_and(|(rb, ..)| rb.is_dynamic());
        let mass = |e: Entity| {
            bodies
                .get(e)
                .ok()
                .and_then(|(_, _, m, _)| m.map(|m| m.value()))
                .unwrap_or(1.0)
        };
        let side = |k: usize, e: Entity, m: f32| super::prop_damage::Impactor {
            mass: m,
            before: engine_to_entity(i.before[k], scale),
            after: engine_to_entity(i.slowest[k], scale),
            fixed: fixed(e),
        };
        let theirs = side(1, i.other, mass(i.other));
        let point = positions.get(i.me).map_or(Vec3::ZERO, |p| p.0);
        let (amount, force) = match i.receiver {
            Receiver::Prop(table, energy, m) => {
                let mine = super::prop_damage::Impactor {
                    // A frozen prop still counts its own mass.
                    fixed: false,
                    ..side(0, i.me, m)
                };
                let amount = super::prop_damage::impact_damage(&table, energy, mine, theirs, false);
                (amount, velocity(i.me) * m)
            }
            Receiver::Breakable(table, energy, m, instant) => {
                // A static brush: its speed never changes.
                let mine = super::prop_damage::Impactor { mass: m, ..default() };
                let amount = super::prop_damage::impact_damage(&table, energy, mine, theirs, false);
                // "Break immediately on physics": any impact is lethal.
                let amount = if instant { amount.max(BREAK_NOW) } else { amount };
                (amount, i.slowest[1] * theirs.mass)
            }
            Receiver::Player => {
                let player = super::prop_damage::Impactor {
                    mass: PLAYER_MASS,
                    ..default()
                };
                let amount =
                    super::prop_damage::impact_damage(&super::prop_damage::PLAYER_TABLE, 1.0, player, theirs, true);
                (amount, Vec3::ZERO)
            }
        };
        if amount > 0.0 && bodies.contains(i.me) | characters.contains(i.me) {
            damage.write(Damage {
                target: i.me,
                attacker: (!theirs.fixed).then_some(i.other),
                amount: amount / 100.0,
                point,
                dir: i.before[1].normalize_or(Vec3::NEG_Y),
                hitgroup: Hitgroup::Generic,
                kind: DamageKind::Crush,
                weapon: None,
                force,
            });
        }
    }
}

/// Outputs game rules ask a map entity to fire (a bomb target's
/// `BombExplode`), with the player as activator.
fn fire_outputs(mut asks: MessageReader<crate::map::entities::FireEntityOutput>, logic: Option<ResMut<Logic>>) {
    let Some(mut logic) = logic else {
        asks.clear();
        return;
    };
    for a in asks.read() {
        let id = logic
            .world
            .ids()
            .into_iter()
            .find(|id| logic.world.get(*id).is_some_and(|e| e.map_index == Some(a.map_index)));
        if let Some(id) = id {
            let who = a.activator.map(super::world::Who::Player);
            logic.world.fire_output(id, &a.output, who, super::Value::Void);
            // A bomb target's legacy `target` is used when it explodes
            // (spec objectives.md 6.3, *hyp.*).
            let target = logic.world.get(id).and_then(|e| e.kv("target")).map(str::to_string);
            if a.output.eq_ignore_ascii_case("BombExplode")
                && let Some(t) = target.filter(|t| !t.is_empty())
            {
                logic.world.queue_input(&t, "Use", super::Value::Void, 0.0, who);
            }
        }
    }
}
