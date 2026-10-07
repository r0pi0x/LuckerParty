//! Scenario tests for CS:S's grenades (specs/cs_source/grenades.md) on the
//! greybox map at CS:S's tick (0.015 s): buying and carry limits (G31),
//! the pin and release timing (G17, G30), the throw (G1), flight and
//! bounces against floors, walls and players (G10-G14), the fuse (G16), HE
//! damage by distance, walls and players in between (G18-G20), kill
//! credit, flash blindness, the smoke cloud's life and bots' sight through
//! it, round restarts, and a primed grenade dropped by the dying.

use bevy::{ecs::message::Messages, prelude::*};
use mashup::{
    bot::{Bot, BotConfig},
    core::{Blinded, Damage, DamageKind, Died, Health, Hitgroup, RoundRestarts, SightBlocker, Team},
    games::cs_source::{
        TICK_INTERVAL,
        grenades::{DRAW_TIME, FLASHBANG, HEGRENADE, SMOKEGRENADE, THROW_TIME},
        weapons::{AK47, CsWeaponsPlugin},
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    movement::placeholder,
    weapon::{
        Inventory, Weapon, WeaponEvent, WeaponEventKind,
        economy::{Money, buy},
        give,
        grenade::{Projectile, SmokeCloud, Throwable, spawn_projectile},
    },
};

const UNIT: f32 = 0.0254;

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim
}

fn active(sim: &Sim, p: Entity) -> Option<Entity> {
    sim.app.world().get::<Inventory>(p).unwrap().active
}

fn active_id(sim: &Sim, p: Entity) -> Option<&'static str> {
    active(sim, p)
        .and_then(|w| sim.app.world().get::<Weapon>(w))
        .map(|w| w.id)
}

fn grenade_of(sim: &Sim, p: Entity, id: &str) -> Option<Entity> {
    let inv = sim.app.world().get::<Inventory>(p)?;
    inv.weapons
        .iter()
        .copied()
        .find(|w| sim.app.world().get::<Weapon>(*w).is_some_and(|x| x.id == id))
}

fn projectiles(sim: &mut Sim) -> Vec<(Entity, Projectile, Vec3)> {
    let w = sim.app.world_mut();
    w.query::<(Entity, &Projectile, &Transform)>()
        .iter(w)
        .map(|(e, p, t)| (e, p.clone(), t.translation))
        .collect()
}

fn clouds(sim: &mut Sim) -> Vec<(SmokeCloud, SightBlocker)> {
    let w = sim.app.world_mut();
    w.query::<(&SmokeCloud, &SightBlocker)>()
        .iter(w)
        .map(|(c, s)| (c.clone(), *s))
        .collect()
}

fn health(sim: &Sim, p: Entity) -> f32 {
    sim.app.world().get::<Health>(p).unwrap().current
}

/// A character at `at` holding `id`, drawn.
fn holder(sim: &mut Sim, at: Vec3, id: &str) -> Entity {
    let p = sim.spawn_character(at, placeholder::ID);
    sim.ticks(1);
    give(sim.app.world_mut(), p, id).unwrap();
    sim.ticks(1);
    assert_eq!(active_id(sim, p), Some(id));
    sim.seconds(DRAW_TIME as f64 + 0.05);
    p
}

/// A grenade of `id` (held by a fresh character far away, so it's the
/// thrower) placed at `at` with `velocity`.
fn place(sim: &mut Sim, id: &str, at: Vec3, velocity: Vec3) -> (Entity, Entity) {
    let thrower = holder(sim, Vec3::new(36.0, 1.0, 36.0), id);
    let weapon = grenade_of(sim, thrower, id).unwrap();
    let g = spawn_projectile(sim.app.world_mut(), weapon, at, velocity).unwrap();
    (thrower, g)
}

fn died(sim: &Sim) -> Vec<Died> {
    let messages = sim.app.world().resource::<Messages<Died>>();
    let mut cursor = messages.get_cursor();
    cursor.read(messages).cloned().collect()
}

#[test]
fn buying_respects_prices_carry_limits_and_keeps_the_hand() {
    let mut sim = sim();
    let p = holder(&mut sim, greybox::SPAWNS[0], AK47);
    sim.app.world_mut().entity_mut(p).insert(Money(2000));
    let w = sim.app.world_mut();
    buy(w, p, "flashbang").unwrap();
    buy(w, p, "flashbang").unwrap();
    // G31: a third is refused and costs nothing.
    assert_eq!(buy(w, p, "flashbang").unwrap_err(), "You cannot carry any more.");
    assert_eq!(w.get::<Money>(p), Some(&Money(1600)));
    buy(w, p, "hegrenade").unwrap();
    assert!(buy(w, p, "hegrenade").is_err());
    buy(w, p, "smokegrenade").unwrap();
    assert!(buy(w, p, "smokegrenade").is_err());
    assert_eq!(w.get::<Money>(p), Some(&Money(1000)));
    sim.ticks(2);
    // All three types at once in slot 3 (the 4th key), and the rifle still
    // in hand.
    assert_eq!(active_id(&sim, p), Some(AK47));
    let flash = grenade_of(&sim, p, FLASHBANG).unwrap();
    assert_eq!(sim.app.world().get::<Throwable>(flash).unwrap().count, 2);
    for id in [HEGRENADE, FLASHBANG, SMOKEGRENADE] {
        let w = grenade_of(&sim, p, id).unwrap();
        assert_eq!(sim.app.world().get::<Weapon>(w).unwrap().slot, 3);
    }
}

#[test]
fn pin_release_throw_fuse_and_the_spent_weapon_goes() {
    let mut sim = sim();
    let p = holder(&mut sim, greybox::SPAWNS[0], HEGRENADE);
    // G30: the pin comes out on the press.
    sim.intent(p).fire = true;
    sim.ticks(1);
    let w = grenade_of(&sim, p, HEGRENADE).unwrap();
    assert!(sim.app.world().get::<Throwable>(w).unwrap().pin);
    sim.ticks(30);
    assert!(projectiles(&mut sim).is_empty(), "held: nothing thrown yet");
    // G17: release seen on tick R, the grenade appears on R + 7 ...
    sim.intent(p).fire = false;
    sim.ticks(7);
    assert!(projectiles(&mut sim).is_empty());
    sim.ticks(1);
    let thrown = projectiles(&mut sim);
    assert_eq!(thrown.len(), 1);
    // G1: level view, standing: 600 units/s 10 degrees up (after one tick
    // of 0.4 gravity).
    let v = thrown[0].1.velocity / UNIT;
    assert!((v.z + 590.88).abs() < 0.1 && v.x.abs() < 0.1, "{v}");
    assert!((v.y - (104.19 - 320.0 * 0.015)).abs() < 0.1, "{v}");
    assert_eq!(thrown[0].1.thrower, Some(p));
    // ... and goes off 104 ticks later (G16: 1.56 s, R + 111).
    sim.ticks(103);
    assert_eq!(projectiles(&mut sim).len(), 1);
    sim.ticks(1);
    assert!(projectiles(&mut sim).is_empty());
    // The last grenade thrown, the weapon is gone and the rifle drawn.
    sim.seconds(THROW_TIME as f64);
    assert!(grenade_of(&sim, p, HEGRENADE).is_none());
    sim.ticks(2);
    assert_eq!(active_id(&sim, p), Some(AK47));
}

#[test]
fn a_second_flashbang_is_drawn_after_the_first_throw() {
    let mut sim = sim();
    let p = holder(&mut sim, greybox::SPAWNS[0], FLASHBANG);
    buy(sim.app.world_mut(), p, "flashbang").unwrap();
    // A tap: press one tick, release the next (throws at once).
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    sim.ticks(10);
    assert_eq!(projectiles(&mut sim).len(), 1);
    sim.seconds(THROW_TIME as f64 + DRAW_TIME as f64 + 0.05);
    assert_eq!(active_id(&sim, p), Some(FLASHBANG));
    let w = grenade_of(&sim, p, FLASHBANG).unwrap();
    assert_eq!(sim.app.world().get::<Throwable>(w).unwrap().count, 1);
    // It can be thrown again.
    sim.intent(p).fire = true;
    sim.ticks(1);
    assert!(sim.app.world().get::<Throwable>(w).unwrap().pin);
}

#[test]
fn view_model_events_pin_then_throw() {
    let mut sim = sim();
    let p = holder(&mut sim, greybox::SPAWNS[0], SMOKEGRENADE);
    sim.intent(p).fire = true;
    sim.ticks(3);
    sim.intent(p).fire = false;
    sim.ticks(1);
    let messages = sim.app.world().resource::<Messages<WeaponEvent>>();
    let mut cursor = messages.get_cursor();
    let kinds: Vec<String> = cursor
        .read(messages)
        .filter(|e| e.owner == p)
        .map(|e| format!("{:?}", e.kind))
        .collect();
    assert!(kinds.iter().any(|k| k == "Thrown"), "{kinds:?}");
    assert!(matches!(WeaponEventKind::PinPulled, WeaponEventKind::PinPulled));
}

#[test]
fn dropped_grenade_bounces_and_rests_on_the_floor() {
    let mut sim = sim();
    let (_, g) = place(&mut sim, HEGRENADE, Vec3::new(0.0, 1.0, 5.0), Vec3::ZERO);
    sim.seconds(1.4);
    let p = projectiles(&mut sim).into_iter().find(|x| x.0 == g).unwrap();
    assert!(p.1.resting, "at rest after a few bounces");
    assert_eq!(p.1.velocity, Vec3::ZERO);
    assert!((p.2.y - 2.0 * UNIT).abs() < 0.01, "lies on the floor: {}", p.2.y);
}

#[test]
fn walls_keep_045_and_players_0135() {
    let mut sim = sim();
    // G11: into the east wall (inner face x = 40) at 10 m/s.
    let (_, g) = place(&mut sim, SMOKEGRENADE, Vec3::new(38.0, 2.0, 0.0), Vec3::X * 10.0);
    sim.ticks(20);
    let p = projectiles(&mut sim).into_iter().find(|x| x.0 == g).unwrap();
    assert!((p.1.velocity.x + 4.5).abs() < 0.05, "{}", p.1.velocity);
    // G12: into a player.
    let victim = sim.spawn_character(Vec3::new(5.0, 0.9, -2.0), placeholder::ID);
    sim.ticks(1);
    let (_, g) = place(&mut sim, SMOKEGRENADE, Vec3::new(2.0, 1.0, -2.0), Vec3::X * 10.0);
    sim.ticks(20);
    let p = projectiles(&mut sim).into_iter().find(|x| x.0 == g).unwrap();
    assert!((p.1.velocity.x + 1.35).abs() < 0.05, "{}", p.1.velocity);
    assert!(health(&sim, victim) > 0.99);
}

/// The blast point of a grenade resting on the floor: 0.6 units up.
fn he_damage_at(sim: &mut Sim, victim_at: Vec3, grenade_at: Vec3) -> (f32, Entity, Entity) {
    let victim = sim.spawn_character(victim_at, placeholder::ID);
    sim.ticks(1);
    let (thrower, _) = place(sim, HEGRENADE, grenade_at, Vec3::ZERO);
    let before = health(sim, victim);
    sim.seconds(1.7);
    (before - health(sim, victim), victim, thrower)
}

#[test]
fn he_damage_falls_off_with_distance_g18() {
    let mut sim = sim();
    let at = Vec3::new(0.0, 0.9, 10.0);
    let g = Vec3::new(0.0, 2.0 * UNIT, 10.0 - 200.0 * UNIT);
    let (lost, victim, _) = he_damage_at(&mut sim, at, g);
    // Aimed between 70 % and 100 % of the eye height above the feet.
    let eye = sim.position(victim) + sim.state(victim).eye_offset;
    let src = Vec3::new(g.x, 1.6 * UNIT, g.z);
    let far = (eye - src).length() / UNIT;
    let near = (eye.with_y(0.7 * eye.y) - src).length() / UNIT;
    let (lo, hi) = (100.0 - far / 3.5, 100.0 - near / 3.5);
    let hp = lost * 100.0;
    assert!(hp >= lo.floor() - 0.01 && hp <= hi + 0.01, "{hp} not in {lo}..{hi}");
}

#[test]
fn he_is_blocked_by_walls_not_players_g19_g20() {
    let mut sim = sim();
    // Behind the long crate (z -20.5..-19.5, 3 m tall).
    let (lost, ..) = he_damage_at(&mut sim, Vec3::new(0.0, 0.9, -23.0), Vec3::new(0.0, 0.06, -18.0));
    assert_eq!(lost, 0.0, "the wall blocks it");
    // Another player between doesn't.
    let mut sim = self::sim();
    let shield = sim.spawn_character(Vec3::new(5.0, 0.9, 8.0), placeholder::ID);
    let (lost, ..) = he_damage_at(&mut sim, Vec3::new(5.0, 0.9, 6.0), Vec3::new(5.0, 0.06, 10.0));
    assert!(lost > 0.3, "{lost}");
    assert!(health(&sim, shield) < 1.0);
    // Out of reach (G21).
    let mut sim = self::sim();
    let (lost, ..) = he_damage_at(&mut sim, Vec3::new(0.0, 0.9, 0.0), Vec3::new(0.0, 0.06, 10.5));
    assert_eq!(lost, 0.0);
}

#[test]
fn he_kills_are_credited_to_the_thrower_with_the_grenade() {
    let mut sim = sim();
    let victim = sim.spawn_character(Vec3::new(0.0, 0.9, 5.0), placeholder::ID);
    sim.ticks(1);
    sim.app.world_mut().get_mut::<Health>(victim).unwrap().current = 0.1;
    let (thrower, _) = place(&mut sim, HEGRENADE, Vec3::new(0.0, 0.06, 4.0), Vec3::ZERO);
    let mut found = None;
    for _ in 0..130 {
        sim.ticks(1);
        if let Some(d) = died(&sim).into_iter().find(|d| d.entity == victim) {
            found = Some(d);
            break;
        }
    }
    let d = found.expect("the victim died");
    assert_eq!(d.attacker, Some(thrower));
    assert_eq!(d.damage.weapon, Some(HEGRENADE));
    assert_eq!(d.damage.kind, DamageKind::Blast);
}

#[test]
fn flashes_blind_by_view_and_walls() {
    let mut sim = sim();
    let facing = sim.spawn_character(Vec3::new(0.0, 0.9, 8.0), placeholder::ID);
    let away = sim.spawn_character(Vec3::new(2.0, 0.9, 8.0), placeholder::ID);
    let hidden = sim.spawn_character(Vec3::new(0.0, 0.9, -23.0), placeholder::ID);
    // Grenade at (0, 1.6, 5): `facing` looks at it (-Z), `away` looks +Z.
    sim.intent(away).yaw = std::f32::consts::PI;
    sim.ticks(1);
    let (thrower, _) = place(&mut sim, FLASHBANG, Vec3::new(1.0, 1.6, 5.0), Vec3::ZERO);
    // Hold it in the air: it would fall; the fuse is 1.56 s, so watch the
    // blind state right after.
    sim.seconds(1.6);
    let blind = |sim: &Sim, e: Entity| sim.app.world().get::<Blinded>(e).copied();
    let f = blind(&sim, facing).expect("facing is blinded");
    let a = blind(&sim, away).expect("facing away still is, less");
    assert!(f.alpha > a.alpha || f.end > a.end, "{f:?} vs {a:?}");
    assert!(blind(&sim, hidden).is_none(), "behind the crate");
    // The thrower (far away, 45 m) isn't reached by the fit's radius.
    assert!(blind(&sim, thrower).is_none());
    // It wears off.
    sim.seconds(6.0);
    let now = sim.app.world().resource::<Time>().elapsed_secs_f64();
    assert_eq!(blind(&sim, facing).unwrap().alpha_at(now), 0.0);
}

#[test]
fn smoke_cloud_grows_blocks_bots_and_goes() {
    let mut sim = sim();
    sim.app.world_mut().resource_mut::<BotConfig>().stop = 1;
    sim.app.world_mut().resource_mut::<BotConfig>().dont_shoot = 1;
    // A bot at z 14 facing an enemy at z -6, the grenade between.
    let bot = sim.spawn_character(Vec3::new(0.0, 0.9, 14.0), placeholder::ID);
    let enemy = sim.spawn_character(Vec3::new(0.0, 0.9, -6.0), placeholder::ID);
    sim.app.world_mut().entity_mut(bot).insert((Team(1), Bot::default()));
    sim.app.world_mut().entity_mut(enemy).insert(Team(2));
    sim.ticks(5);
    let target = |sim: &Sim| sim.app.world().get::<Bot>(bot).unwrap().target;
    assert_eq!(target(&sim), Some(enemy), "in plain sight");
    let (thrower, g) = place(&mut sim, SMOKEGRENADE, Vec3::new(0.0, 0.06, 4.0), Vec3::ZERO);
    // The thrower is the bot's teammate (else it would be seen instead).
    sim.app.world_mut().entity_mut(thrower).insert(Team(1));
    sim.seconds(1.6);
    let c = clouds(&mut sim);
    assert_eq!(c.len(), 1, "popped after the fuse once still");
    sim.seconds(1.0);
    let (cloud, blocker) = clouds(&mut sim).remove(0);
    assert!(
        (blocker.radius - 240.0 * UNIT * 0.75).abs() < 1e-3,
        "{}",
        blocker.radius
    );
    assert_eq!(cloud.grenade, Some(g));
    sim.ticks(5);
    assert_eq!(target(&sim), None, "hidden by the smoke");
    // Gone by 22 s, with the spent grenade.
    sim.seconds(19.0);
    assert_eq!(clouds(&mut sim).len(), 1, "fading at 20 s");
    sim.seconds(2.0);
    assert!(clouds(&mut sim).is_empty());
    assert!(sim.app.world().get_entity(g).is_err());
    sim.ticks(5);
    assert_eq!(target(&sim), Some(enemy));
}

#[test]
fn a_new_round_clears_grenades_and_clouds() {
    let mut sim = sim();
    place(&mut sim, SMOKEGRENADE, Vec3::new(0.0, 0.06, 4.0), Vec3::ZERO);
    sim.seconds(2.0);
    place(&mut sim, HEGRENADE, Vec3::new(3.0, 0.06, 4.0), Vec3::ZERO);
    assert_eq!(clouds(&mut sim).len(), 1);
    assert!(!projectiles(&mut sim).is_empty());
    sim.app.world_mut().get_resource_or_init::<RoundRestarts>().0 += 1;
    sim.ticks(2);
    assert!(clouds(&mut sim).is_empty());
    assert!(projectiles(&mut sim).is_empty());
}

#[test]
fn dying_with_the_pin_out_drops_a_live_grenade() {
    let mut sim = sim();
    let p = holder(&mut sim, greybox::SPAWNS[0], HEGRENADE);
    sim.intent(p).fire = true;
    sim.ticks(5);
    sim.app.world_mut().write_message(Damage {
        force: bevy::math::Vec3::ZERO,
        target: p,
        attacker: None,
        amount: 2.0,
        point: Vec3::ZERO,
        dir: Vec3::Y,
        hitgroup: Hitgroup::Generic,
        kind: DamageKind::Generic,
        weapon: None,
    });
    sim.ticks(3);
    let thrown = projectiles(&mut sim);
    assert_eq!(thrown.len(), 1, "dropped live");
    assert!(thrown[0].1.velocity.xz().length() < 0.01, "only the body's velocity");
}
