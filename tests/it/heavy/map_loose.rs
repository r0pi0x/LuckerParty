//! Dropped weapons on real CS:S maps, headless: a weapon thrown in a
//! crowded spawn leaves its dropper standing where they were and lands in
//! front of them; a shot weapon skids meters over a displacement instead
//! of catching on its triangles' edges. Skipped without an install.

use avian3d::prelude::*;
use bevy::prelude::*;
use mashup::{
    core::{SpawnPoint, Team},
    games::{
        self, cs_source,
        cs_source::{movement::SourceMovementPlugin, weapons::CsWeaponsPlugin},
    },
    harness::Sim,
    map::{MapData, MapPlugin, loose::LooseItem},
    mount::config::LocalConfig,
    movement::placeholder,
    weapon::{
        Inventory, Weapon,
        drop::{Loose, drop_weapon},
    },
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

/// The map's spawn points for `team` (a character's centre there).
fn spawn_points(sim: &mut Sim, team: u8) -> Vec<Transform> {
    let world = sim.app.world_mut();
    world
        .query::<(&Transform, &SpawnPoint)>()
        .iter(world)
        .filter(|(_, s)| s.team == Some(Team(team)))
        .map(|(t, _)| *t)
        .collect()
}

/// A character moving by `movement` whose centre is at `at` (engine
/// space).
fn stand_with(sim: &mut Sim, movement: &'static str, at: Vec3, team: u8, yaw: f32) -> Entity {
    let p = sim.spawn_character(at, movement);
    sim.app.world_mut().entity_mut(p).insert(Team(team));
    sim.intent(p).yaw = yaw;
    p
}

fn stand(sim: &mut Sim, at: Vec3, team: u8, yaw: f32) -> Entity {
    stand_with(sim, cs_source::movement::ID, at, team, yaw)
}

#[test]
fn thrown_weapons_leave_the_dropper_standing() {
    for movement in [cs_source::movement::ID, placeholder::ID] {
        crowd_throws(movement);
    }
}

fn crowd_throws(movement: &'static str) {
    let Some(map) = load("de_dust2") else { return };
    let mut sim = sim(map);
    // Every terrorist spawn: a crowd, as at a round start, each facing
    // the way their spawn does.
    let spawns = spawn_points(&mut sim, 1);
    let players: Vec<Entity> = spawns
        .iter()
        .map(|t| stand_with(&mut sim, movement, t.translation, 1, t.rotation.to_euler(EulerRot::YXZ).0))
        .collect();
    sim.seconds(1.5);
    for &p in &players {
        let inv = sim.app.world().get::<Inventory>(p).unwrap();
        assert!(inv.active.is_some(), "armed");
    }
    let before: Vec<Vec3> = players.iter().map(|p| sim.position(*p)).collect();
    let mut loose = Vec::new();
    for &p in &players {
        let item = drop_weapon(sim.app.world_mut(), p, true).expect("dropped");
        let weapon = sim.app.world().get::<Loose>(item).unwrap().weapon;
        loose.push((item, weapon));
    }
    // Landed, and still inside the 1 s touch delay (at 1 s one lying at
    // its dropper's feet may rightly be taken back: the drop is stamped
    // on the tick's clock now, not a frame ahead of it).
    sim.seconds(0.9);
    let mut thrown = 0;
    for (k, &p) in players.iter().enumerate() {
        let moved = sim.position(p) - before[k];
        assert!(
            moved.length() < 0.02,
            "{movement}: player {k} at {} was pushed {moved} by their own weapon",
            before[k]
        );
        let (item, weapon) = loose[k];
        let owner = sim.app.world().get::<Weapon>(weapon).unwrap().owner;
        assert_ne!(owner, Some(p), "player {k} took their weapon back at once");
        // Lying in front of them, or a teammate standing there took it.
        if sim.app.world().get_entity(item).is_err() || (sim.position(item) - before[k]).xz().length() > 0.5 {
            thrown += 1;
        }
    }
    // Some throws hit a teammate or a wall; most land well in front.
    assert!(thrown * 4 >= players.len() * 3, "{thrown} of {} thrown clear", players.len());
}

/// A point of the map's collision triangles (displacements) whose
/// surroundings (`radius` meters) are flat enough to slide over, with a
/// direction along the slope's level line.
fn flat_terrain(map: &MapData, radius: f32) -> Option<(Vec3, Vec3)> {
    let tris: Vec<[Vec3; 3]> = map
        .collision_indices
        .iter()
        .map(|t| t.map(|i| Vec3::from(map.collision_positions[i as usize])))
        .collect();
    let normal = |t: &[Vec3; 3]| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
    tris.iter().find_map(|t| {
        let c = (t[0] + t[1] + t[2]) / 3.0;
        let n = normal(t);
        if n.y < 0.99 {
            return None;
        }
        let near: Vec<&[Vec3; 3]> = tris
            .iter()
            .filter(|o| {
                let oc = (o[0] + o[1] + o[2]) / 3.0;
                (oc - c).xz().length() < radius && (oc.y - c.y).abs() < 1.0
            })
            .collect();
        // Nothing solid above it (a wall, a ledge) within reach.
        let (lo, hi) = (c - Vec3::new(radius, -0.05, radius), c + Vec3::new(radius, 2.0, radius));
        let clear = map.collision_hulls.iter().all(|h| {
            let (a, b) = h
                .iter()
                .map(|p| Vec3::from(*p))
                .fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(p), b.max(p)));
            a.cmpgt(hi).any() || b.cmplt(lo).any()
        });
        (clear && near.len() >= 30 && near.iter().all(|o| normal(o).y > 0.97)).then_some((c, Vec3::X))
    })
}

/// Lay a loose weapon at `at`, let it settle, give it a push along `dir`
/// (kg.m/s) and return how far it went.
fn slide(sim: &mut Sim, at: Vec3, dir: Vec3, push: f32) -> f32 {
    let spawn = spawn_points(sim, 1)[0].translation;
    let owner = stand(sim, spawn, 1, 0.0);
    sim.seconds(1.5);
    let item = drop_weapon(sim.app.world_mut(), owner, false).expect("dropped");
    sim.app.world_mut().despawn(owner);
    {
        let mut e = sim.app.world_mut().entity_mut(item);
        e.get_mut::<Transform>().unwrap().translation = at + Vec3::Y * 0.8;
    }
    sim.seconds(1.5);
    let start = sim.position(item);
    {
        let world = sim.app.world_mut();
        assert!(world.get::<RigidBody>(item).is_some(), "a body headless");
        let mut v = world.get_mut::<LinearVelocity>(item).unwrap();
        v.0 += dir * push / 3.0;
        world.entity_mut(item).remove::<Sleeping>();
    }
    sim.seconds(3.0);
    let end = sim.position(item);
    assert!(sim.app.world().get::<LooseItem>(item).is_some());
    (end - start).xz().length()
}

#[test]
fn shot_weapons_slide_over_displacements() {
    let Some(map) = load("de_dust2") else { return };
    let (open, dir) = flat_terrain(&map, 2.5).expect("a flat displacement");
    // The terrorists' spawn is a sand displacement too.
    let spawn = map.spawns.iter().find(|(_, t)| *t == Some(Team(1))).unwrap().0;
    let mut sim = sim(map);
    // A shot from a standing player a few meters off: along the ground
    // and down into it, at the guns' impulse (2400 kg.in/s, 20 m/s on a
    // 3 kg weapon). Over Source's displacements such a weapon skids
    // several meters; caught on the triangles' edges it went 0.2 m.
    let shot = |d: Vec3| (d - Vec3::Y * 0.5).normalize();
    for (at, d) in [(open, dir), (spawn, Vec3::X)] {
        let went = slide(&mut sim, at, shot(d), 61.0);
        eprintln!("slid {went:.2} m from {at}");
        assert!(went > 2.0, "caught on the displacement at {at}: {went} m");
    }
}

/// Walk `p` toward `to` until `done`; the ticks it took, None within
/// `limit`.
fn walk_to(sim: &mut Sim, p: Entity, to: Vec3, limit: u32, done: impl Fn(&Sim) -> bool) -> Option<u32> {
    for t in 0..limit {
        if done(sim) {
            sim.intent(p).move_axis = Vec2::ZERO;
            return Some(t);
        }
        let d = to - sim.position(p);
        let mut i = sim.intent(p);
        i.yaw = (-d.x).atan2(-d.z);
        i.move_axis = if d.xz().length() > 0.05 { Vec2::Y } else { Vec2::ZERO };
        drop(i);
        sim.ticks(1);
    }
    sim.intent(p).move_axis = Vec2::ZERO;
    None
}

/// On de_dust2's sand slopes, at every terrorist spawn: a rifle thrown
/// standing, and a pistol thrown running, come to rest and are taken
/// again by walking back over them; a dead player's rifle is taken by a
/// teammate who walks over it. With the map loaded after time spent
/// elsewhere (the main menu), as in the game.
#[test]
fn guns_dropped_on_dust2_are_walked_over_and_taken() {
    use mashup::{core::Health, games::cs_source::weapons::USP};
    let Some(map) = load("de_dust2") else { return };
    let mut sim = sim(load("de_dust2").unwrap());
    sim.seconds(20.0);
    mashup::swap_map(
        sim.app.world_mut(),
        "de_dust2",
        Some(map),
        std::time::Duration::from_secs_f64(cs_source::TICK_INTERVAL),
    );
    sim.ticks(2);
    let spawns = spawn_points(&mut sim, 1);
    let carries = |sim: &Sim, p: Entity, w: Entity| sim.app.world().get::<Inventory>(p).unwrap().weapons.contains(&w);
    let ticks = |s: f64| (s / cs_source::TICK_INTERVAL).round() as u32;
    for (k, t) in spawns.iter().enumerate().take(6) {
        let p = stand(&mut sim, t.translation, 1, t.rotation.to_euler(EulerRot::YXZ).0);
        sim.seconds(1.5);
        for (id, running) in [("rifle", false), (USP, true)] {
            if id == USP {
                let usp = sim
                    .app
                    .world()
                    .get::<Inventory>(p)
                    .unwrap()
                    .weapons
                    .iter()
                    .copied()
                    .find(|w| sim.app.world().get::<Weapon>(*w).unwrap().id == USP);
                let Some(usp) = usp else { continue };
                sim.app.world_mut().get_mut::<Inventory>(p).unwrap().wanted = Some(usp);
                sim.seconds(1.0);
                // Running when it leaves the hand.
                sim.intent(p).move_axis = Vec2::Y;
                sim.seconds(0.5);
            }
            let item = drop_weapon(sim.app.world_mut(), p, true).expect("dropped");
            let weapon = sim.app.world().get::<Loose>(item).unwrap().weapon;
            sim.intent(p).move_axis = if running { Vec2::Y } else { Vec2::new(0.0, -1.0) };
            sim.seconds(0.5);
            sim.intent(p).move_axis = Vec2::ZERO;
            sim.seconds(1.5);
            let at = sim.position(item);
            let took = walk_to(&mut sim, p, at, ticks(5.0), |s| carries(s, p, weapon));
            assert!(
                took.is_some(),
                "spawn {k}: a {id} lying at {at} not taken by walking over it (player at {})",
                sim.position(p)
            );
        }
        sim.app.world_mut().despawn(p);
    }
    // A dead player's rifle (as the dead drop it in rounds: let fall where
    // they lie), taken by a teammate walking over from another spawn.
    let (a, b) = (spawns[0].translation, spawns[spawns.len() - 1].translation);
    let dead = stand(&mut sim, a, 1, 0.0);
    let taker = stand(&mut sim, b, 1, 0.0);
    sim.seconds(1.5);
    // The taker has no primary.
    let theirs = sim.app.world().get::<Inventory>(taker).unwrap().active.unwrap();
    sim.app
        .world_mut()
        .get_mut::<Inventory>(taker)
        .unwrap()
        .weapons
        .retain(|w| *w != theirs);
    sim.app.world_mut().despawn(theirs);
    let rifle = sim.app.world().get::<Inventory>(dead).unwrap().active.unwrap();
    sim.app.world_mut().get_mut::<Health>(dead).unwrap().current = 0.0;
    drop_weapon(sim.app.world_mut(), dead, false).expect("the dead drop it");
    sim.seconds(2.0);
    let w = sim.app.world_mut();
    let lying = w
        .query::<(&Loose, &Transform)>()
        .iter(w)
        .find(|(l, _)| l.weapon == rifle)
        .map(|(_, t)| t.translation)
        .expect("lying");
    let took = walk_to(&mut sim, taker, lying, ticks(8.0), |s| carries(s, taker, rifle));
    assert!(took.is_some(), "the dead's rifle at {lying} not taken");
}
