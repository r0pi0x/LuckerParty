//! Props as logic entities on real CS:S maps, headless: cs_assault's model
//! doors (prop_door_rotating, specs/source/doors_buttons.md) open away
//! from the player who uses them, block and stop blocking shots as they
//! turn, close after their return delay and at a round restart; de_port's
//! and cs_compound's load as doors too. Shooting a prop fires its outputs:
//! de_nuke's fire extinguishers start their steam sound, cs_office's
//! projector stops its hum. Skipped without an install.

use avian3d::prelude::*;
use bevy::{ecs::system::SystemState, prelude::*};
use mashup::{
    core::{MovingSolid, RoundRestarts},
    games::{
        self, cs_source,
        cs_source::{
            movement::{self, SourceMovementPlugin, to_engine, to_source},
            weapons::{AK47, CsWeaponsPlugin},
        },
    },
    harness::Sim,
    logic::{EntId, Logic, movers::DoorState},
    map::{LiveSounds, MapBrushEntity, MapData, MapEntity, MapPlugin, PropEntity},
    mount::config::LocalConfig,
    movement::noclip,
    weapon::{Inventory, Weapon, give},
};

fn load(name: &str) -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map(&format!("cs_source:{name}")).expect("load map"))
}

fn index_of(map: &MapData, name: &str) -> usize {
    map.entities
        .iter()
        .position(|e| e.get("targetname") == Some(name))
        .unwrap_or_else(|| panic!("no entity {name}"))
}

/// The logic id of map entity `index`.
fn id_of(sim: &Sim, index: usize) -> Option<EntId> {
    let logic = sim.app.world().resource::<Logic>();
    logic
        .world
        .ids()
        .into_iter()
        .find(|id| logic.world.get(*id).unwrap().map_index == Some(index))
}

fn door_state(sim: &Sim, index: usize) -> (DoorState, f32) {
    let id = id_of(sim, index).expect("door exists");
    let d = sim.app.world().resource::<Logic>().world.prop_door(id).expect("a model door").clone();
    (d.state, d.push.angles.y)
}

/// World bounds (Source units) of a brush-like entity where it spawns.
fn bounds(e: &MapEntity) -> (Vec3, Vec3) {
    let rot = mashup::map::entities::entity_rotation(e.angles());
    let pts = e.hulls.iter().flat_map(|h| &h.points).map(|p| rot * *p + e.origin());
    pts.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)))
}

fn node(sim: &mut Sim, index: usize) -> Entity {
    let world = sim.app.world_mut();
    world
        .query::<(Entity, &MapBrushEntity)>()
        .iter(world)
        .find(|(_, n)| n.0 == index)
        .map(|(e, _)| e)
        .expect("the door's node")
}

/// How far (Source units) a ray from `from` toward `to` gets, `skip` left
/// out.
fn ray(sim: &mut Sim, from: Vec3, to: Vec3, skip: Entity) -> f32 {
    let world = sim.app.world_mut();
    let mut state = SystemState::<SpatialQuery>::new(world);
    let q = state.get(world).unwrap();
    let (a, b) = (to_engine(from), to_engine(to));
    let dir = Dir3::new(b - a).unwrap();
    let filter = SpatialQueryFilter::from_excluded_entities([skip]);
    q.cast_ray(a, dir, 100.0, true, &filter)
        .map_or(f32::MAX, |h| h.distance / 0.0254)
}

#[test]
fn cs_assault_model_doors_open_away_on_use() {
    let Some(map) = load("cs_assault") else { return };
    let doors: Vec<usize> = map
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.classname() == "prop_door_rotating")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(doors.len(), 4, "cs_assault's model doors");
    for &i in &doors {
        let e = &map.entities[i];
        assert!(e.mover && !e.hulls.is_empty(), "door {i} has a node and volumes");
        assert_eq!(e.get(mashup::map::entities::DOOR_MOVE_KEY), Some("Doors.Move1"), "skin 5's door_options");
        // Its hardware's handle sounds ("hardwareN" in door_options).
        let hardware = e.get("hardware").unwrap_or("0");
        let locked = e.get(mashup::map::entities::DOOR_LOCKED_KEY);
        eprintln!("door {i}: hardware {hardware}, locked sound {locked:?}");
        let want = match hardware {
            "0" => "DoorSound.Null".to_string(),
            n => format!("DoorHandles.Locked{n}"),
        };
        assert_eq!(locked, Some(want.as_str()));
        // The door model rides its own node.
        assert!(map.props.iter().any(|p| p.entity == Some(i) && p.parent == Some(i)));
    }
    let lower = index_of(&map, "door_lower");
    let e = map.entities[lower].clone();
    let (lo, hi) = bounds(&e);
    let centre = (lo + hi) / 2.0;
    eprintln!("door_lower: origin {}, leaf bounds {lo} .. {hi}", e.origin());
    // In front of it (its +X, angles 0), 40 units out from the leaf,
    // looking at the leaf's middle.
    let feet = Vec3::new(hi.x + 40.0, centre.y, lo.z);
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    let p = sim.spawn_character(to_engine(feet + Vec3::Z * 37.0), movement::ID);
    sim.intent(p).yaw = (180.0f32 - 90.0).to_radians();
    sim.ticks(10);
    let n = node(&mut sim, lower);
    let eye = to_source(sim.position(p) + sim.state(p).eye_offset);
    let target = Vec3::new(centre.x, centre.y, eye.z);
    let blocked = ray(&mut sim, eye, target, p);
    assert!(blocked < eye.distance(target) + 4.0, "the closed door stops the ray ({blocked})");
    assert_eq!(door_state(&sim, lower), (DoorState::Closed, 0.0));

    sim.intent(p).use_key = true;
    sim.ticks(1);
    sim.intent(p).use_key = false;
    sim.ticks(59);
    let (state, yaw) = door_state(&sim, lower);
    assert_eq!(state, DoorState::Opening, "still turning after 59 ticks");
    assert!(yaw.abs() > 80.0 && yaw.abs() < 90.0, "{yaw}");
    sim.ticks(1);
    let (state, yaw) = door_state(&sim, lower);
    assert_eq!((state, yaw.abs()), (DoorState::Open, 90.0), "open after 0.9 s");
    // Away from the user: the leaf's middle is now farther from them.
    let solid = sim.app.world().get::<MovingSolid>(n).unwrap().clone();
    assert!(solid.solid && !solid.brushes.is_empty());
    let (a, b) = solid
        .brushes
        .iter()
        .fold((Vec3::MAX, Vec3::MIN), |(a, b), br| (a.min(br.min), b.max(br.max)));
    let leaf = to_source((a + b) / 2.0);
    assert!(leaf.x < centre.x - 10.0, "swung away from the user: leaf at {leaf}, was {centre}");
    // The node turned (and the door model with it); shots pass the
    // doorway now.
    let turned = sim.app.world().get::<Transform>(n).unwrap().rotation;
    assert!(turned.angle_between(Quat::IDENTITY).to_degrees() > 89.0);
    assert!(ray(&mut sim, eye, target, p) > blocked + 8.0, "the doorway is open");

    // Closes 20.1 s after fully open, then 0.9 s turning.
    sim.ticks(1339);
    assert_eq!(door_state(&sim, lower).0, DoorState::Open);
    sim.ticks(1);
    assert_eq!(door_state(&sim, lower).0, DoorState::Closing);
    sim.ticks(60);
    assert_eq!(door_state(&sim, lower), (DoorState::Closed, 0.0));
    assert!((ray(&mut sim, eye, target, p) - blocked).abs() < 1.0, "blocks the ray again");

    // Open it again; a new round starts with it closed.
    sim.intent(p).use_key = true;
    sim.ticks(1);
    sim.intent(p).use_key = false;
    sim.ticks(30);
    assert_eq!(door_state(&sim, lower).0, DoorState::Opening);
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    sim.ticks(1);
    assert_eq!(door_state(&sim, lower), (DoorState::Closed, 0.0));
}

#[test]
fn de_port_and_cs_compound_model_doors_load() {
    let Some(map) = load("de_port") else { return };
    let doors: Vec<usize> = map
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.classname() == "prop_door_rotating")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(doors.len(), 4);
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);
    // Two start open (flag 1, spawnpos 1), turned 110 degrees.
    let open: Vec<(DoorState, f32)> = doors.iter().map(|&i| door_state(&sim, i)).collect();
    eprintln!("de_port doors: {open:?}");
    assert_eq!(open.iter().filter(|(s, _)| *s == DoorState::Open).count(), 2);
    assert_eq!(open.iter().filter(|(s, _)| *s == DoorState::Closed).count(), 2);

    let Some(map) = load("cs_compound") else { return };
    let i = index_of(&map, "door_hostageshack1_1");
    assert!(map.entities[i].mover && !map.entities[i].hulls.is_empty());
}

/// A shooter standing `dist` units in front of prop entity `index` (along
/// its forward), aiming at the middle of its model with an AK-47.
fn shoot_prop(sim: &mut Sim, map: &MapData, index: usize, dist: f32) -> Entity {
    let prop = map.props.iter().find(|p| p.entity == Some(index)).expect("its prop");
    let (lo, hi) = map.models[prop.model].bounds;
    let centre = to_source(prop.translation + prop.rotation * ((lo + hi) / 2.0));
    // A side the prop can be shot from: around it and from above, the
    // first whose ray hits the prop itself.
    let world = sim.app.world_mut();
    let node = world
        .query::<(Entity, &PropEntity)>()
        .iter(world)
        .find(|(_, p)| p.0 == index)
        .map(|(e, _)| e)
        .expect("its node");
    let eye = (0..8)
        .flat_map(|k| [0.0f32, 0.6].map(|up| (k, up)))
        .map(|(k, up)| {
            let a = (k as f32 * 45.0).to_radians();
            centre + Vec3::new(a.cos(), a.sin(), up).normalize() * dist
        })
        .find(|eye| hits(sim, *eye, centre) == Some(node))
        .expect("an open side");
    let p = sim.spawn_character(to_engine(eye), noclip::ID);
    sim.ticks(1);
    let offset = sim.state(p).eye_offset;
    sim.app.world_mut().get_mut::<Transform>(p).unwrap().translation = to_engine(eye) - offset;
    give(sim.app.world_mut(), p, AK47).unwrap();
    sim.seconds(1.5);
    let active = sim.app.world().get::<Inventory>(p).unwrap().active.unwrap();
    assert_eq!(sim.app.world().get::<Weapon>(active).unwrap().id, AK47);
    let d = to_engine(centre) - (sim.position(p) + sim.state(p).eye_offset);
    let mut i = sim.intent(p);
    i.yaw = (-d.x).atan2(-d.z);
    i.pitch = d.y.atan2(d.xz().length());
    sim.ticks(1);
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    sim.ticks(2);
    p
}

/// The body a ray from `from` toward `to` (Source units) hits first.
fn hits(sim: &mut Sim, from: Vec3, to: Vec3) -> Option<Entity> {
    let world = sim.app.world_mut();
    let mut state = SystemState::<(SpatialQuery, Query<&ColliderOf>)>::new(world);
    let (q, of) = state.get(world).unwrap();
    let (a, b) = (to_engine(from), to_engine(to));
    let hit = q.cast_ray(a, Dir3::new(b - a).unwrap(), 100.0, true, &SpatialQueryFilter::default())?;
    Some(of.get(hit.entity).map_or(hit.entity, |c| c.body))
}

fn playing(sim: &Sim, entry: &str) -> bool {
    sim.app
        .world()
        .resource::<LiveSounds>()
        .sounds
        .values()
        .any(|s| s.entry == entry)
}

#[test]
fn de_nuke_fire_extinguisher_steams_when_shot() {
    let Some(map) = load("de_nuke") else { return };
    let extinguishers: Vec<usize> = map
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.get("model")
                .is_some_and(|m| m.eq_ignore_ascii_case("models/props/cs_office/Fire_Extinguisher.mdl"))
                && e.keyvalues.iter().any(|(k, _)| k.eq_ignore_ascii_case("OnHealthChanged"))
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(extinguishers.len(), 6);
    let ext = extinguishers[0];
    eprintln!(
        "fire extinguisher health from its model: {:?}",
        map.entities[ext].get(mashup::map::entities::PROP_HEALTH_KEY)
    );
    let steam = "ambient/gas/cannister_loop.wav";
    let snapshot = map.clone();
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);
    assert!(!playing(&sim, steam), "silent until shot");
    // The prop node takes damage.
    let world = sim.app.world_mut();
    let prop = world
        .query::<(Entity, &PropEntity)>()
        .iter(world)
        .find(|(_, p)| p.0 == ext)
        .map(|(e, _)| e)
        .expect("its node");
    assert!(sim.app.world().get::<mashup::core::Damageable>(prop).is_some());

    sim.app.world_mut().resource_mut::<Logic>().world.record = true;
    shoot_prop(&mut sim, &snapshot, ext, 30.0);
    let id = id_of(&sim, ext).unwrap();
    let logic = &sim.app.world().resource::<Logic>().world;
    let outputs: Vec<&String> = logic.fired.iter().filter(|f| f.1 == id).map(|f| &f.2).collect();
    assert!(outputs.iter().any(|o| *o == "OnHealthChanged"), "the shot reaches the logic: {outputs:?}");
    assert!(playing(&sim, steam), "the shot starts the steam");
    // It stops 4 s later (then fades out over 1 s).
    sim.seconds(4.0 + 1.5);
    assert!(!playing(&sim, steam), "and stops");
}

#[test]
fn cs_office_projector_stops_when_shot() {
    let Some(map) = load("cs_office") else { return };
    let projector = map
        .entities
        .iter()
        .position(|e| {
            e.get("model")
                .is_some_and(|m| m.eq_ignore_ascii_case("models/props/cs_office/projector.mdl"))
        })
        .expect("the projector");
    let glow = index_of(&map, "projectorglow");
    let hum = "ambient/tones/projector.wav";
    let snapshot = map.clone();
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);
    assert!(playing(&sim, hum));
    assert!(id_of(&sim, glow).is_some());
    shoot_prop(&mut sim, &snapshot, projector, 60.0);
    assert!(!playing(&sim, hum), "the shot stops the projector's hum");
    assert!(id_of(&sim, glow).is_none(), "and kills its glow");
    // A new round: humming again.
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    sim.ticks(2);
    assert!(playing(&sim, hum));
    assert!(id_of(&sim, glow).is_some());
}
