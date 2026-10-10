//! Scenario tests for bots' manners with teammates in narrow places, on
//! the greybox map with a wall and a 1 m door added (and a navigation
//! mesh of three areas for it): two bots crossing the door from either
//! side both get through quickly, and a bot meeting a person in the door
//! gives way (`bot::give_way`).

use avian3d::prelude::*;
use bevy::prelude::*;
use mashup::{
    bot::{Bot, BotConfig},
    core::Team,
    games::cs_source::{TICK_INTERVAL, weapons::CsWeaponsPlugin},
    greybox::GreyboxMapPlugin,
    harness::Sim,
    map::nav::{NavArea, NavMesh, Side, Via},
    movement::placeholder,
};

/// The wall along z = 0 east of the ramps, its door at x 31.5..32.5.
const DOOR: (f32, f32) = (31.5, 32.5);
const WEST: f32 = 24.0;
const EAST: f32 = 39.9;

fn area(id: u32, x0: f32, z0: f32, x1: f32, z1: f32) -> NavArea {
    NavArea {
        id,
        flags: 0,
        min: Vec2::new(x0, z0),
        max: Vec2::new(x1, z1),
        heights: [0.0; 4],
        center: Vec3::new((x0 + x1) / 2.0, 0.0, (z0 + z1) / 2.0),
        links: Vec::new(),
        place: None,
        hiding: Vec::new(),
        visible: Vec::new(),
        encounters: Vec::new(),
    }
}

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 0;
    sim.ticks(1);
    // The wall, either side of the door.
    for (x0, x1) in [(WEST, DOOR.0), (DOOR.1, EAST)] {
        sim.app.world_mut().spawn((
            Transform::from_xyz((x0 + x1) / 2.0, 1.5, 0.0),
            RigidBody::Static,
            Collider::cuboid(x1 - x0, 3.0, 0.4),
        ));
    }
    // South room, the door, north room.
    let mut m = NavMesh {
        areas: vec![
            area(1, WEST, 0.2, EAST, 10.0),
            area(2, DOOR.0, -0.2, DOOR.1, 0.2),
            area(3, WEST, -10.0, EAST, -0.2),
        ],
        ..default()
    };
    let mut link = |a: usize, b: usize, side: Side| {
        m.areas[a].links.push((b, Via::Walk(side)));
        m.areas[b].links.push((a, Via::Walk(side.opposite())));
    };
    link(1, 0, Side::MaxZ);
    link(2, 1, Side::MaxZ);
    sim.app.insert_resource(m);
    sim.ticks(2);
    sim
}

fn bot(sim: &mut Sim, at: Vec3, to: Vec3) -> Entity {
    let b = sim.spawn_character(at, placeholder::ID);
    sim.app.world_mut().entity_mut(b).insert((Team(1), Bot::default()));
    sim.ticks(1);
    sim.app
        .world_mut()
        .get_mut::<Bot>(b)
        .unwrap()
        .send_to(Some(to - Vec3::Y * 0.9));
    b
}

/// Seconds until each of `who` is within 1 m (flat) of its goal, the
/// first `limit` seconds (None: never); `walk` moves anyone not a bot
/// (forward along -Z) each tick.
fn crossing(sim: &mut Sim, who: &[(Entity, Vec3)], limit: f64, people: &[Entity]) -> Vec<Option<f64>> {
    let mut done = vec![None; who.len()];
    let mut t = 0.0;
    while t < limit && done.iter().any(|d| d.is_none()) {
        for &p in people {
            let mut i = sim.intent(p);
            i.move_axis = Vec2::Y;
            i.yaw = 0.0;
        }
        sim.ticks(1);
        t += TICK_INTERVAL as f64;
        for (k, (e, goal)) in who.iter().enumerate() {
            if done[k].is_none() && (sim.position(*e) - *goal).xz().length() < 1.0 {
                done[k] = Some(t);
            }
        }
    }
    done
}

#[test]
fn bots_crossing_a_door_both_get_through() {
    let mut sim = sim();
    let (south, north) = (Vec3::new(32.0, 0.9, 6.0), Vec3::new(32.0, 0.9, -6.0));
    let a = bot(&mut sim, south, north);
    let b = bot(&mut sim, north + Vec3::X * 0.1, south + Vec3::X * 0.1);
    let done = crossing(&mut sim, &[(a, north), (b, south + Vec3::X * 0.1)], 20.0, &[]);
    eprintln!("two bots through a door from either side: {done:.2?} s");
    assert!(done.iter().all(|d| d.is_some_and(|t| t < 8.0)), "{done:.2?}");
}

#[test]
fn a_bot_gives_way_to_a_person_in_a_door() {
    let mut sim = sim();
    let (south, north) = (Vec3::new(32.0, 0.9, 6.0), Vec3::new(32.0, 0.9, -6.0));
    // The person walks north (-Z) through the door; the bot comes south.
    let p = sim.spawn_character(south, placeholder::ID);
    sim.app.world_mut().entity_mut(p).insert(Team(1));
    let b = bot(&mut sim, north, south);
    let done = crossing(&mut sim, &[(p, north), (b, south)], 20.0, &[p]);
    eprintln!(
        "a person and a bot through a door: person {:.2?} s, bot {:.2?} s",
        done[0], done[1]
    );
    // Walking straight at 12 m: unhindered it takes about 12 m / speed.
    assert!(done[0].is_some_and(|t| t < 6.0), "the person was held up: {done:.2?}");
    assert!(
        done[1].is_some_and(|t| t < 12.0),
        "the bot got through after: {done:.2?}"
    );
}
