//! Players and physics props on real CS:S maps, headless
//! (specs/cs_source/physics_props.md 4): a player walking into a light
//! `prop_physics` shoves it through their physics shadow and follows it;
//! a heavy one (over 350 kg) stops them like a wall; standing on a crate
//! is stable; cs_office's light multiplayer props get shoved by a player
//! walking slowly into them; a broken prop's pieces show its skin, spin
//! as it spun, land on the world and fade out. Skipped without an install.

use avian3d::prelude::*;
use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    core::{Damage, DamageKind, Hitgroup},
    games::{
        self, cs_source,
        cs_source::{
            movement::{self, SourceMovementPlugin, to_engine, to_source},
            pushaway::prop_box,
            weapons::CsWeaponsPlugin,
        },
    },
    harness::Sim,
    map::{BreakProp, MapData, MapPlugin, PhysicsProp, PropEntity, PushAway, SpawnGibs},
    mount::config::LocalConfig,
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

fn sim(map: MapData) -> Sim {
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);
    sim
}

fn node(sim: &mut Sim, index: usize) -> Entity {
    let world = sim.app.world_mut();
    world
        .query::<(Entity, &PropEntity)>()
        .iter(world)
        .find(|(_, p)| p.0 == index)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("no node for entity {index}"))
}

/// Every message of type `M` still held.
fn messages<M: Message + Clone>(sim: &Sim) -> Vec<M> {
    let m = sim.app.world().resource::<Messages<M>>();
    MessageCursor::<M>::default().read(m).cloned().collect()
}

/// A wood crate placed as a `prop_physics` (not multiplayer), neither
/// asleep nor frozen: (map, its entity index, its prop index).
fn crate_prop() -> Option<(MapData, usize, usize)> {
    for name in [
        "de_port",
        "cs_militia",
        "de_inferno",
        "cs_havana",
        "de_chateau",
        "cs_compound",
    ] {
        let map = load(name)?;
        let found = map.props.iter().enumerate().find_map(|(pi, p)| {
            let i = p.entity?;
            let e = &map.entities[i];
            let ok = e.classname() == "prop_physics"
                && e.get("model")
                    .is_some_and(|m| m.to_ascii_lowercase().contains("wood_crate"))
                && e.get("spawnflags")
                    .and_then(|s| s.parse::<u32>().ok())
                    .is_none_or(|f| f & 9 == 0)
                && p.physics
                    .as_ref()
                    .is_some_and(|ph| ph.push == PushAway::Collide && ph.mass <= 100.0);
            ok.then_some((i, pi))
        });
        if let Some((i, pi)) = found {
            eprintln!(
                "{name}: crate {i} at {} ({} kg)",
                map.entities[i].origin(),
                map.props[pi].physics.as_ref().unwrap().mass
            );
            return Some((map, i, pi));
        }
    }
    panic!("no loose prop_physics wood crate found");
}

/// The crate's world box (Source units) after settling.
fn crate_box(sim: &mut Sim, n: Entity) -> (Vec3, Vec3) {
    let world = sim.app.world();
    let (t, p) = (world.get::<Transform>(n).unwrap(), world.get::<PhysicsProp>(n).unwrap());
    prop_box(t, p.bounds)
}

/// Walk at the crate from 80 units away along each axis, from each side
/// that is open and reaches it; returns (how far the crate moved, how far
/// the player got past where the crate's near face was) in units per
/// such side.
fn walk_into(map: &MapData, index: usize, seconds: f64) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let sides = [
        (Vec3::X, 0.0f32),
        (Vec3::NEG_X, 180.0),
        (Vec3::Y, 90.0),
        (Vec3::NEG_Y, 270.0),
    ];
    for (dir, yaw) in sides {
        let mut sim = sim(map.clone());
        sim.seconds(2.0);
        let n = node(&mut sim, index);
        let (lo, hi) = crate_box(&mut sim, n);
        let centre = (lo + hi) / 2.0;
        let start_at = sim.app.world().get::<Transform>(n).unwrap().translation;
        // The near face along `dir`, and feet 80 units before it.
        let near = (centre - (hi - lo) / 2.0 * dir.abs() * dir.signum()).dot(dir);
        let feet = Vec3::new(centre.x, centre.y, lo.z + 1.0) - dir * ((hi - lo).dot(dir.abs()) / 2.0 + 16.0 + 80.0);
        let p = sim.spawn_character(to_engine(feet + Vec3::Z * 36.0), movement::ID);
        sim.seconds(0.5);
        let settled = to_source(sim.position(p));
        if (settled - (feet + Vec3::Z * 36.0)).length() > 4.0 {
            continue;
        }
        // Intent yaw 0 looks down -Z, which is Source yaw 90.
        sim.intent(p).yaw = (yaw - 90.0f32).to_radians();
        sim.intent(p).move_axis = Vec2::Y;
        // SHADOW_TRACE=1 prints the crate, player and shadow each 3 ticks.
        if std::env::var("SHADOW_TRACE").is_ok() {
            for _ in 0..40 {
                sim.ticks(3);
                let world = sim.app.world_mut();
                let cv = world.get::<LinearVelocity>(n).map_or(Vec3::ZERO, |v| v.0) / 0.0254;
                let sh: Vec<String> = world
                    .query::<(&mashup::map::prop_physics::PhysicsShadow, &LinearVelocity, &Position)>()
                    .iter(world)
                    .map(|(_, v, x)| format!("shadow v {:.1} at {:.1}", v.0 / 0.0254, to_source(x.0)))
                    .collect();
                let touch = sim
                    .app
                    .world()
                    .get::<mashup::games::cs_source::shadow::PhysicsTouch>(p)
                    .copied();
                eprintln!("{touch:?}");
                eprintln!(
                    "crate v {cv:.1} at {:.1}; player v {:.1} at {:.1}; {sh:?}",
                    to_source(sim.app.world().get::<Transform>(n).unwrap().translation),
                    to_source(sim.velocity(p)),
                    to_source(sim.position(p))
                );
            }
        }
        sim.seconds(seconds);
        let front = (to_source(sim.position(p)) + dir * 16.0).dot(dir);
        let moved = sim
            .app
            .world()
            .get::<Transform>(n)
            .unwrap()
            .translation
            .distance(start_at)
            / 0.0254;
        eprintln!(
            "from {dir}: crate moved {moved:.1} units, player front {:.1} past its face",
            front - near
        );
        // The run reached the crate (blocked by it, or pushing it).
        if front - near > -8.0 {
            out.push((moved, front - near));
        }
    }
    out
}

/// Spec 4.1: walking into a light prop_physics shoves it ahead (the
/// shadow pushes it) and the player follows.
#[test]
fn walking_into_a_light_crate_pushes_it() {
    let Some((map, crate_, _)) = crate_prop() else { return };
    let runs = walk_into(&map, crate_, 2.0);
    assert!(!runs.is_empty(), "an open side to walk from");
    // From some side the crate has room to slide (others may jam it
    // against a wall soon).
    assert!(
        runs.iter().any(|(moved, past)| *moved > 30.0 && *past > 25.0),
        "the crate wasn't shoved and followed far: {runs:?}"
    );
}

/// Spec 4.1 / test case: a prop over 350 kg is a wall.
#[test]
fn a_heavy_prop_stops_the_player() {
    let Some((mut map, crate_, _)) = crate_prop() else {
        return;
    };
    // Every prop_physics heavy (de_port stacks its crates: a light one
    // in front would be pushed into it).
    for p in map.props.iter_mut().filter_map(|p| p.physics.as_mut()) {
        if p.push == PushAway::Collide {
            p.mass = 400.0;
        }
    }
    let runs = walk_into(&map, crate_, 2.0);
    assert!(!runs.is_empty(), "an open side to walk from");
    for (moved, past) in runs {
        assert!(moved < 1.0, "the heavy crate moved {moved} units");
        assert!(past < 1.0, "the player got {past} units into it");
    }
}

/// Standing on a crate: neither the player nor the crate drifts.
#[test]
fn standing_on_a_crate_is_stable() {
    let Some((map, crate_, _)) = crate_prop() else { return };
    let mut sim = sim(map);
    sim.seconds(2.0);
    let n = node(&mut sim, crate_);
    let (lo, hi) = crate_box(&mut sim, n);
    let top = Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, hi.z);
    let p = sim.spawn_character(to_engine(top + Vec3::Z * (36.0 + 4.0)), movement::ID);
    sim.seconds(1.0);
    let start = to_source(sim.position(p));
    let crate_start = sim.app.world().get::<Transform>(n).unwrap().translation;
    assert!(sim.state(p).on_ground, "standing");
    assert!(
        (start.z - 36.0 - hi.z).abs() < 2.0,
        "on the crate: feet {} top {}",
        start.z - 36.0,
        hi.z
    );
    sim.seconds(3.0);
    let end = to_source(sim.position(p));
    let crate_moved = sim
        .app
        .world()
        .get::<Transform>(n)
        .unwrap()
        .translation
        .distance(crate_start)
        / 0.0254;
    assert!((end - start).length() < 1.0, "the player drifted {}", end - start);
    assert!(crate_moved < 1.0, "the crate drifted {crate_moved} units");
    assert!(sim.velocity(p).length() < 0.01, "still");
}

/// Spec 4.2.2: cs_office's light multiplayer props ("non-solid" mode) are
/// shoved by a player walking into them at any speed, and the player
/// walks through without launching them.
#[test]
fn cs_office_light_props_get_shoved_by_a_slow_walk() {
    let Some(map) = load("cs_office") else { return };
    let light: Vec<usize> = map
        .props
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.physics
                .as_ref()
                .is_some_and(|ph| ph.push == PushAway::NonSolid && !ph.frozen && !ph.asleep)
        })
        .map(|(i, _)| i)
        .collect();
    assert!(!light.is_empty(), "light props on cs_office");
    let mut shoved = None;
    for &i in light.iter().take(8) {
        let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
        sim.set_tick_interval(cs_source::TICK_INTERVAL);
        sim.seconds(2.0);
        let name = format!("Prop {i}");
        let find = |sim: &mut Sim| {
            let mut q = sim.app.world_mut().query::<(&Name, &Transform, &PhysicsProp)>();
            q.iter(sim.app.world())
                .find(|(n, ..)| n.as_str() == name)
                .map(|(_, t, p)| (*t, *p))
        };
        let Some((t, p)) = find(&mut sim) else { continue };
        let (lo, hi) = prop_box(&t, p.bounds);
        let centre = (lo + hi) / 2.0;
        for (dir, yaw) in [
            (Vec3::X, 0.0f32),
            (Vec3::NEG_X, 180.0),
            (Vec3::Y, 90.0),
            (Vec3::NEG_Y, 270.0),
        ] {
            let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
            sim.set_tick_interval(cs_source::TICK_INTERVAL);
            sim.seconds(2.0);
            let feet = Vec3::new(centre.x, centre.y, lo.z.min(centre.z - 8.0)) - dir * 70.0;
            let pl = sim.spawn_character(to_engine(feet + Vec3::Z * 37.0), movement::ID);
            sim.seconds(0.5);
            if (to_source(sim.position(pl)).truncate() - feet.truncate()).length() > 4.0 {
                continue;
            }
            sim.intent(pl).yaw = (yaw - 90.0f32).to_radians();
            sim.intent(pl).move_axis = Vec2::Y;
            // Walking (+speed): slower than the 75 u/s solid-mode rule.
            sim.intent(pl).walk = true;
            sim.seconds(2.5);
            let (t2, _) = find(&mut sim).unwrap();
            let moved = t2.translation.distance(t.translation) / 0.0254;
            let speed = sim
                .app
                .world_mut()
                .query::<(&Name, &LinearVelocity)>()
                .iter(sim.app.world())
                .find(|(n, _)| n.as_str() == name)
                .map(|(_, v)| v.0.length() / 0.0254)
                .unwrap();
            eprintln!("prop {i} from {dir}: moved {moved:.1} units, now {speed:.1} u/s");
            if moved > 4.0 {
                shoved = Some((moved, speed));
                break;
            }
        }
        if shoved.is_some() {
            break;
        }
    }
    let (moved, speed) = shoved.expect("a light prop shoved by a walking player");
    assert!(moved < 400.0, "launched {moved} units");
    assert!(speed < 300.0, "launched at {speed} u/s");
}

/// A bullet of `amount` health points on prop node `target`.
fn shoot(sim: &mut Sim, target: Entity, amount: f32) {
    let point = sim.app.world().get::<Transform>(target).unwrap().translation;
    sim.app.world_mut().write_message(Damage {
        target,
        attacker: None,
        amount: amount / 100.0,
        point,
        dir: Vec3::NEG_Z,
        hitgroup: Hitgroup::Generic,
        kind: DamageKind::Bullet,
        weapon: None,
        force: Vec3::ZERO,
    });
    sim.ticks(1);
}

/// prop_damage.md 7.3: a broken crate's pieces take its skin and its
/// spin, collide with the world, and fade out after their fade time.
#[test]
fn crate_pieces_take_its_skin_and_spin_and_fade() {
    let Some((mut map, crate_, prop)) = crate_prop() else {
        return;
    };
    let model = map.props[prop].model;
    let skins = map.models[model].skins.len();
    eprintln!("crate model has {skins} skins");
    let skin = if skins > 1 { 1 } else { 0 };
    map.props[prop].skin = skin;
    let kv = &mut map.entities[crate_].keyvalues;
    kv.retain(|(k, _)| k != "skin");
    kv.push(("skin".into(), skin.to_string()));
    let mut sim = sim(map);
    sim.seconds(1.0);
    let n = node(&mut sim, crate_);
    let floor = sim.app.world().get::<Transform>(n).unwrap().translation.y - 1.0;
    // Set it spinning, then break it.
    sim.app.world_mut().entity_mut(n).insert(AngularVelocity(Vec3::Y * 3.0));
    shoot(&mut sim, n, 100.0);
    let breaks = messages::<BreakProp>(&sim);
    assert_eq!(breaks.len(), 1);
    assert!(breaks[0].spin.length() > 0.5, "spin captured: {}", breaks[0].spin);
    let gibs = messages::<SpawnGibs>(&sim);
    let pieces: Vec<_> = gibs.iter().filter(|g| g.prop).flat_map(|g| &g.pieces).collect();
    assert!(!pieces.is_empty());
    for p in &pieces {
        assert_eq!(p.skin, skin, "the prop's skin");
        assert!(p.frozen || p.spin.length() > 0.5, "spinning as the prop spun");
    }
    let life = pieces.iter().map(|p| p.life).fold(0.0f32, f32::max);
    assert!(life.is_finite(), "they fade");
    let count = |sim: &mut Sim| {
        let world = sim.app.world_mut();
        world
            .query::<(&Name, &Transform)>()
            .iter(world)
            .filter(|(n, _)| n.as_str() == "Gib")
            .map(|(_, t)| t.translation)
            .collect::<Vec<_>>()
    };
    sim.seconds(4.0);
    let landed = count(&mut sim);
    assert_eq!(landed.len(), pieces.len());
    for p in &landed {
        assert!(p.y > floor - 1.0, "a piece at {p} fell through the world");
    }
    // Gone once faded (life, then a 1 s fade).
    sim.seconds((life + 1.5 - 4.0) as f64);
    assert!(count(&mut sim).is_empty(), "the pieces faded out");
}
