//! Impact decals on brush entities (de_nuke from a real CS:S install,
//! headless with the render assets decals need): a shot into a door
//! leaves a bullet hole on the door's own brushes, as a child of its node,
//! so it turns with the door and goes when the door is removed; a decal on
//! a breakable goes when it breaks and doesn't come back with it at a
//! round restart. Skipped without an install.

use avian3d::prelude::*;
use bevy::{ecs::system::SystemState, prelude::*};
use mashup::{
    core::RoundRestarts,
    games::{
        self, cs_source,
        cs_source::{
            movement::{SourceMovementPlugin, to_engine},
            weapons::{AK47, CsWeaponsPlugin},
        },
    },
    harness::Sim,
    logic::{Logic, Value},
    map::{
        BrushEntityBounds, MapBrushEntity, MapData, MapEntity, MapPlugin,
        decal::{DecalGroup, DecalMaterial, PlaceDecal, RuntimeDecal},
        prop_material::PropMaterial,
        rope_material::RopeMaterial,
        sprite_material::SpriteMaterial,
        world_material::WorldMaterial,
    },
    mount::config::LocalConfig,
    movement::noclip,
    weapon::{Inventory, Weapon, give},
};

fn nuke() -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map("cs_source:de_nuke").expect("load de_nuke"))
}

/// The render assets the map spawns its meshes and decals with.
fn render_assets(app: &mut App) {
    app.init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<WorldMaterial>()
        .init_asset::<PropMaterial>()
        .init_asset::<SpriteMaterial>()
        .init_asset::<RopeMaterial>()
        .init_asset::<DecalMaterial>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
}

fn sim(map: MapData) -> Sim {
    let mut sim = Sim::new((
        render_assets,
        MapPlugin::new(map),
        SourceMovementPlugin,
        CsWeaponsPlugin,
    ));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim
}

/// World bounds (Source units) of a brush entity where it spawns.
fn bounds(e: &MapEntity) -> (Vec3, Vec3) {
    let rot = mashup::map::entities::entity_rotation(e.angles());
    let pts = e.hulls.iter().flat_map(|h| &h.points).map(|p| rot * *p + e.origin());
    pts.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)))
}

/// Eye points in front of each face of a thin brush, `dist` units out.
fn approaches(e: &MapEntity, dist: f32) -> (Vec3, [Vec3; 2]) {
    let (lo, hi) = bounds(e);
    let (size, centre) = (hi - lo, (lo + hi) / 2.0);
    let thin = if size.x < size.y { Vec3::X } else { Vec3::Y };
    let out = thin * (size.dot(thin) / 2.0 + dist);
    (centre, [centre + out, centre - out])
}

fn node(sim: &mut Sim, index: usize) -> Entity {
    let world = sim.app.world_mut();
    world
        .query::<(Entity, &MapBrushEntity)>()
        .iter(world)
        .find(|(_, n)| n.0 == index)
        .map(|(e, _)| e)
        .expect("a node")
}

/// What a ray from `from` toward `to` (Source units) hits first: the
/// entity, point and normal (engine space).
fn ray(sim: &mut Sim, from: Vec3, to: Vec3, skip: Option<Entity>) -> Option<(Entity, Vec3, Vec3)> {
    let world = sim.app.world_mut();
    let mut state = SystemState::<SpatialQuery>::new(world);
    let q = state.get(world).unwrap();
    let (a, b) = (to_engine(from), to_engine(to));
    let dir = Dir3::new(b - a).unwrap();
    let filter = SpatialQueryFilter::from_excluded_entities(skip);
    q.cast_ray(a, dir, 100.0, true, &filter)
        .map(|h| (h.entity, a + *dir * h.distance, h.normal))
}

/// Runtime decals under `node`.
fn decals_on(sim: &mut Sim, node: Entity) -> Vec<Entity> {
    let world = sim.app.world_mut();
    world
        .query_filtered::<(Entity, &ChildOf), With<RuntimeDecal>>()
        .iter(world)
        .filter(|(_, c)| c.parent() == node)
        .map(|(e, _)| e)
        .collect()
}

/// A floating shooter whose eye is at `eye` (Source units) holding an
/// AK-47, looking at `target`.
fn shooter(sim: &mut Sim, eye: Vec3, target: Vec3) -> Entity {
    let p = sim.spawn_character(to_engine(eye), noclip::ID);
    sim.ticks(1);
    let offset = sim.state(p).eye_offset;
    sim.app.world_mut().get_mut::<Transform>(p).unwrap().translation = to_engine(eye) - offset;
    give(sim.app.world_mut(), p, AK47).unwrap();
    sim.seconds(1.5);
    let active = sim.app.world().get::<Inventory>(p).unwrap().active.unwrap();
    assert_eq!(sim.app.world().get::<Weapon>(active).unwrap().id, AK47);
    let eye_at = sim.position(p) + sim.state(p).eye_offset;
    let d = to_engine(target) - eye_at;
    let mut i = sim.intent(p);
    i.yaw = (-d.x).atan2(-d.z);
    i.pitch = d.y.atan2(d.xz().length());
    p
}

fn input(sim: &mut Sim, target: &str, input: &str) {
    sim.app
        .world_mut()
        .resource_mut::<Logic>()
        .world
        .queue_input(target, input, Value::Void, 0.0, None);
}

#[test]
fn a_shot_door_carries_its_bullet_hole_until_removed() {
    let Some(map) = nuke() else { return };
    let doors: Vec<usize> = map
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.classname() == "func_door_rotating" && e.get("targetname").is_some_and(|n| !n.is_empty()))
        .map(|(i, _)| i)
        .collect();
    assert!(!doors.is_empty(), "de_nuke has named rotating doors");
    let entities = map.entities.clone();
    let mut sim = sim(map);
    for door in doors {
        let n = node(&mut sim, door);
        // Shoot it from a side where nothing else is in the way, a little
        // above the middle (the handle rides the door).
        let (centre, sides) = approaches(&entities[door], 40.0);
        let target = centre + Vec3::Z * 12.0;
        let Some(eye) = sides
            .into_iter()
            .find(|eye| ray(&mut sim, *eye, target, None).is_some_and(|(e, _, _)| e == n))
        else {
            continue;
        };
        let p = shooter(&mut sim, eye, target);
        sim.intent(p).fire = true;
        sim.ticks(1);
        sim.intent(p).fire = false;
        sim.ticks(3);
        let decals = decals_on(&mut sim, n);
        assert_eq!(decals.len(), 1, "one bullet hole on the door's node");
        let decal = decals[0];
        // Lying on the door: its mesh within the door's bounds.
        let corners = |sim: &mut Sim| {
            let world = sim.app.world_mut();
            let mesh = world.get::<Mesh3d>(decal).unwrap().0.clone();
            let mesh = world.resource::<Assets<Mesh>>().get(&mesh).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
                panic!("decal positions")
            };
            v.iter().map(|p| Vec3::from(*p)).collect::<Vec<_>>()
        };
        let local = corners(&mut sim);
        let b = sim
            .app
            .world()
            .get::<BrushEntityBounds>(n)
            .map(|b| (b.min, b.max))
            .unwrap();
        let near = |p: Vec3| p.cmpge(b.0 - Vec3::splat(0.02)).all() && p.cmple(b.1 + Vec3::splat(0.02)).all();
        assert!(
            local.iter().all(|p| near(*p)),
            "the decal lies on the door (node space)"
        );

        // The door opens: the hole turns with it.
        let name = entities[door].get("targetname").unwrap().to_string();
        let before = *sim.app.world().get::<GlobalTransform>(n).unwrap();
        input(&mut sim, &name, "Open");
        sim.seconds(1.0);
        let after = *sim.app.world().get::<GlobalTransform>(n).unwrap();
        assert!(
            before.rotation().angle_between(after.rotation()) > 1.0,
            "the door turned"
        );
        assert_eq!(decals_on(&mut sim, n), vec![decal], "the hole stays on the door");
        let at = *sim.app.world().get::<GlobalTransform>(decal).unwrap();
        assert!(
            at.rotation().angle_between(after.rotation()) < 1e-3,
            "the hole turned with the door"
        );

        // Removed: the hole goes with it.
        input(&mut sim, &name, "Kill");
        sim.ticks(3);
        assert!(
            sim.app.world().get_entity(decal).is_err(),
            "the hole is gone with the door"
        );
        return;
    }
    panic!("no door could be shot");
}

#[test]
fn a_breakables_decal_goes_when_it_breaks() {
    let Some(map) = nuke() else { return };
    // The vents (func_breakable, metal, health 1).
    let vents: Vec<usize> = map
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.classname() == "func_breakable" && e.get("material") == Some("2"))
        .map(|(i, _)| i)
        .collect();
    let entities = map.entities.clone();
    let mut sim = sim(map);
    sim.ticks(2);
    for vent in vents {
        let n = node(&mut sim, vent);
        let (centre, sides) = approaches(&entities[vent], 24.0);
        let Some((point, normal)) = sides.into_iter().find_map(|eye| {
            ray(&mut sim, eye, centre, None)
                .filter(|(e, _, _)| *e == n)
                .map(|(_, p, nrm)| (p, nrm))
        }) else {
            continue;
        };
        sim.app.world_mut().write_message(PlaceDecal {
            target: Some(n),
            group: DecalGroup::Material('M'),
            point,
            normal,
            dir: -normal,
            spin: false,
        });
        sim.ticks(1);
        let decals = decals_on(&mut sim, n);
        assert_eq!(decals.len(), 1, "a metal impact on the vent's node");

        let name = entities[vent].get("targetname").filter(|n| !n.is_empty());
        input(&mut sim, name.unwrap_or("func_breakable"), "Break");
        sim.seconds(0.5);
        assert!(sim.app.world().get_entity(decals[0]).is_err(), "gone with the vent");
        // Decals asked for on a broken vent don't appear.
        sim.app.world_mut().write_message(PlaceDecal {
            target: Some(n),
            group: DecalGroup::Material('M'),
            point,
            normal,
            dir: -normal,
            spin: false,
        });
        sim.ticks(1);
        assert!(decals_on(&mut sim, n).is_empty());

        // A new round: the vent is back, unmarked.
        sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
        sim.ticks(3);
        assert_eq!(sim.app.world().get::<Visibility>(n), Some(&Visibility::Inherited));
        assert!(decals_on(&mut sim, n).is_empty(), "whole again without the old hole");
        // Marked again, then another round: brush entities are made anew,
        // without their decals.
        sim.app.world_mut().write_message(PlaceDecal {
            target: Some(n),
            group: DecalGroup::Material('M'),
            point,
            normal,
            dir: -normal,
            spin: false,
        });
        sim.ticks(1);
        assert_eq!(decals_on(&mut sim, n).len(), 1);
        sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
        sim.ticks(3);
        assert!(decals_on(&mut sim, n).is_empty(), "a round restart clears it");
        return;
    }
    panic!("no vent to mark");
}
