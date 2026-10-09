//! Groundwork for network play (docs/plans/active/multiplayer.md, slice
//! 0): the predicted path (weapon selection, movement, the weapon frame)
//! replays a command stream from saved state bit for bit, without side
//! effects; clients don't run the server's systems.

use bevy::{ecs::message::Messages, prelude::*};
use mashup::{
    core::{
        Damage, DamageKind, FirstTimePredicted, Health, Hitgroup, Intent, MovementState, NetRole, PredictedComponents,
        SimClock, Velocity, predict,
    },
    games::cs_source::{
        TICK_INTERVAL,
        movement::{self as source, SourceMovement, SourceMovementPlugin},
        weapons::{AK47, CsWeaponsPlugin, Inaccuracy, Recoil},
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    map::{
        PlaySound,
        ragdoll::{PoseFrame, SkeletonPose},
    },
    weapon::{Hitscan, Inventory, Magazine, ViewPunch, WeaponEvent, WeaponEventKind, WeaponState, give},
};

/// What one tick's command did to the predicted player, in full float
/// precision (`{:?}` prints the shortest text that reads back to the same
/// bits, so equal text is equal bits).
fn outcome(world: &World, p: Entity) -> String {
    let inv = world.get::<Inventory>(p).unwrap();
    let w = inv.active.unwrap();
    format!(
        "{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}",
        world.get::<Transform>(p).unwrap(),
        world.get::<Velocity>(p).unwrap(),
        world.get::<MovementState>(p).unwrap(),
        world.get::<SourceMovement>(p).unwrap(),
        world.get::<ViewPunch>(p),
        inv,
        world.get::<WeaponState>(w).unwrap(),
        world.get::<Magazine>(w).map(|m| (m.clip, m.reserve)),
        world.get::<Inaccuracy>(w).map(|a| a.value),
        world.get::<Recoil>(w),
        world.get::<Hitscan>(w).map(|h| format!("{:?}", h.spread)),
        world.get::<mashup::core::MaxSpeed>(p),
    )
}

/// The command for tick `i` of the script: walk, strafe, jump, duck, turn,
/// spray the AK-47 (spread and recoil), switch to the pistol and back.
fn script(i: u32, intent: &mut Intent) {
    intent.move_axis = match i {
        0..=19 => Vec2::Y,
        20..=39 => Vec2::new(1.0, 1.0).normalize(),
        40..=59 => Vec2::X * -1.0,
        _ => Vec2::ZERO,
    };
    intent.yaw = 0.3 + i as f32 * 0.013;
    intent.pitch = -0.05 + (i % 7) as f32 * 0.002;
    intent.jump = (8..=10).contains(&i) || (45..=46).contains(&i);
    intent.crouch = (25..=34).contains(&i);
    intent.fire = (5..=70).contains(&i);
    intent.select = match i {
        75..=76 => Some(1),
        85..=86 => Some(0),
        _ => None,
    };
}

#[test]
fn replaying_commands_from_saved_state_is_bit_identical() {
    let mut sim = Sim::new((GreyboxMapPlugin, SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    let p = sim.spawn_character(greybox::SPAWNS[0], source::ID);
    sim.ticks(1);
    give(sim.app.world_mut(), p, AK47).unwrap();
    // Draw it and land.
    sim.seconds(1.5);

    // Save the predicted state of the player and what it carries.
    let entities: Vec<Entity> = std::iter::once(p)
        .chain(sim.app.world().get::<Inventory>(p).unwrap().weapons.iter().copied())
        .collect();
    let world = sim.app.world();
    let saved = world.resource::<PredictedComponents>().save(world, &entities);
    let start = outcome(world, p);

    // First run: the normal fixed ticks.
    const TICKS: u32 = 100;
    let mut shots_cursor = world.resource::<Messages<WeaponEvent>>().get_cursor_current();
    let mut commands: Vec<(Intent, SimClock)> = Vec::new();
    let mut first: Vec<(String, Vec<String>)> = Vec::new();
    let shots = |world: &World, cursor: &mut bevy::ecs::message::MessageCursor<WeaponEvent>| {
        cursor
            .read(world.resource::<Messages<WeaponEvent>>())
            .filter_map(|e| match e.kind {
                WeaponEventKind::Shot { from, to, .. } => Some(format!("{from:?} {to:?}")),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for i in 0..TICKS {
        script(i, &mut sim.intent(p));
        sim.ticks(1);
        let world = sim.app.world();
        commands.push((world.get::<Intent>(p).unwrap().clone(), *world.resource::<SimClock>()));
        first.push((outcome(world, p), shots(world, &mut shots_cursor)));
    }
    let fired: usize = first.iter().map(|(_, s)| s.len()).sum();
    assert!(fired >= 5, "the script should fire a burst ({fired} shots)");
    assert!(
        first.iter().any(|(o, _)| o.contains("crouching: true")),
        "the script should duck"
    );

    // Back to the saved state, then the same commands through the
    // prediction schedules (`core::predict`), as a client re-running them
    // after a correction.
    let world = sim.app.world_mut();
    world.resource_scope(|world, registry: Mut<PredictedComponents>| registry.restore(world, &saved));
    assert_eq!(outcome(world, p), start, "restored state");
    world.insert_resource(FirstTimePredicted(false));
    let mut sounds = world.resource::<Messages<PlaySound>>().get_cursor_current();
    let mut damage = world.resource::<Messages<Damage>>().get_cursor_current();
    let mut shots_cursor = world.resource::<Messages<WeaponEvent>>().get_cursor_current();
    for (i, (intent, clock)) in commands.iter().enumerate() {
        *world.get_mut::<Intent>(p).unwrap() = intent.clone();
        world.insert_resource(*clock);
        predict(world);
        let again = (outcome(world, p), shots(world, &mut shots_cursor));
        assert_eq!(again, first[i], "tick {i} of the replay differs");
    }
    world.insert_resource(FirstTimePredicted(true));
    assert_eq!(
        sounds.read(world.resource::<Messages<PlaySound>>()).count(),
        0,
        "a replay plays no sounds"
    );
    assert_eq!(
        damage.read(world.resource::<Messages<Damage>>()).count(),
        0,
        "a replay deals no damage"
    );
}

/// Damage for `target` from nobody.
fn hurt(target: Entity) -> Damage {
    Damage {
        target,
        attacker: None,
        amount: 0.25,
        point: Vec3::ZERO,
        dir: Vec3::NEG_Y,
        hitgroup: Hitgroup::Generic,
        kind: DamageKind::Generic,
        weapon: None,
        force: Vec3::ZERO,
    }
}

#[test]
fn a_client_leaves_damage_and_bots_to_the_server() {
    let run = |role: NetRole| {
        let mut sim = Sim::new(GreyboxMapPlugin);
        sim.app.insert_resource(role);
        let p = sim.spawn_character(greybox::SPAWNS[1] + Vec3::X * 12.0, mashup::movement::placeholder::ID);
        sim.app.insert_resource(mashup::slots::Loadout {
            movement: mashup::movement::placeholder::ID,
        });
        sim.app.world_mut().entity_mut(p).insert(mashup::core::Team(2));
        sim.app.world_mut().resource_mut::<mashup::bot::BotConfig>().dont_shoot = 1;
        let bot = mashup::bot::add_bot(sim.app.world_mut(), mashup::core::Team(1)).unwrap();
        sim.ticks(1);
        sim.app.world_mut().write_message(hurt(p));
        sim.seconds(1.0);
        let health = sim.app.world().get::<Health>(p).unwrap().current;
        let target = sim.app.world().get::<mashup::bot::Bot>(bot).unwrap().target;
        (health, target.is_some())
    };
    let (health, targeted) = run(NetRole::Standalone);
    assert_eq!(health, 0.75, "standalone takes the damage");
    assert!(targeted, "a standalone bot sees its enemy");
    let (health, targeted) = run(NetRole::Client);
    assert_eq!(health, 1.0, "a client doesn't apply damage itself");
    assert!(!targeted, "a client doesn't run bots");
}

#[test]
fn pose_history_is_kept_by_tick() {
    let frame = |tick: u64| PoseFrame {
        time: tick as f64 * TICK_INTERVAL,
        tick,
        root: Transform::default(),
        bones: Vec::new(),
    };
    let pose = SkeletonPose {
        frames: [10, 11, 13].into_iter().map(frame).collect(),
    };
    assert_eq!(pose.at_tick(11).map(|f| f.tick), Some(11));
    // A tick that wasn't kept: the one before it.
    assert_eq!(pose.at_tick(12).map(|f| f.tick), Some(11));
    assert_eq!(pose.at_tick(99).map(|f| f.tick), Some(13));
    assert!(pose.at_tick(9).is_none(), "older than the history");
}
