//! Scenario tests for bots and grenades on the greybox map at CS:S's tick:
//! a bot lobs an HE over a crate at an enemy it remembers but can't see,
//! and it goes off near them; a bot throwing a flash turns away from it;
//! bots buy grenades with money left after a gun and armour.

use bevy::prelude::*;
use mashup::{
    bot::{Bot, BotConfig},
    core::{Health, Team},
    games::cs_source::{
        TICK_INTERVAL,
        grenades::{DRAW_TIME, FLASHBANG, HEGRENADE, SMOKEGRENADE},
        weapons::{AK47, CsWeaponsPlugin},
    },
    greybox::GreyboxMapPlugin,
    harness::Sim,
    movement::placeholder,
    objectives::RoundOpen,
    weapon::{
        Armor, Inventory, Weapon,
        economy::{Money, autobuy_rolling},
        give,
        grenade::{Projectile, Throwable},
    },
};

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    // No throws until a test says so (bots hear the enemy deploy its gun).
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 0;
    sim
}

fn active_id(sim: &Sim, p: Entity) -> Option<&'static str> {
    let w = sim.app.world().get::<Inventory>(p)?.active?;
    sim.app.world().get::<Weapon>(w).map(|w| w.id)
}

fn grenade_of(sim: &Sim, p: Entity, id: &str) -> Option<Entity> {
    let inv = sim.app.world().get::<Inventory>(p)?;
    inv.weapons
        .iter()
        .copied()
        .find(|w| sim.app.world().get::<Weapon>(*w).is_some_and(|x| x.id == id))
}

/// A bot at `at` (team 1) holding an AK-47 and carrying `grenade`, and an
/// enemy (team 2) at `enemy_at` it remembers seeing.
fn setup(sim: &mut Sim, at: Vec3, enemy_at: Vec3, grenade: &str) -> (Entity, Entity) {
    let bot = sim.spawn_character(at, placeholder::ID);
    let enemy = sim.spawn_character(enemy_at, placeholder::ID);
    sim.app.world_mut().entity_mut(bot).insert((Team(1), Bot::default()));
    sim.app.world_mut().entity_mut(enemy).insert(Team(2));
    sim.ticks(1);
    give(sim.app.world_mut(), bot, AK47).unwrap();
    sim.ticks(1);
    give(sim.app.world_mut(), bot, grenade).unwrap();
    // `give` draws what it gives: back to the rifle, as after a buy.
    let ak = grenade_of(sim, bot, AK47).unwrap();
    sim.app.world_mut().get_mut::<Inventory>(bot).unwrap().wanted = Some(ak);
    sim.seconds(DRAW_TIME as f64 + 0.3);
    assert_eq!(active_id(sim, bot), Some(AK47));
    assert_eq!(sim.app.world().get::<Bot>(bot).unwrap().target, None, "hidden");
    let now = sim.app.world().resource::<Time>().elapsed_secs_f64();
    let feet = enemy_at - Vec3::Y * 0.9;
    sim.app.world_mut().get_mut::<Bot>(bot).unwrap().lead = Some((feet, now));
    // Whenever an arc works.
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 2;
    (bot, enemy)
}

#[test]
fn a_bot_lobs_an_he_over_cover_at_a_remembered_enemy() {
    let mut sim = sim();
    // The long crate (x -3..3, z -20.5..-19.5, 3 m tall) hides the enemy
    // 25 m away: an air burst over it.
    let (bot, enemy) = setup(
        &mut sim,
        Vec3::new(1.0, 0.9, 1.0),
        Vec3::new(0.0, 0.9, -24.0),
        HEGRENADE,
    );
    let before = sim.app.world().get::<Health>(enemy).unwrap().current;
    let mut planned = None;
    let mut last: Option<(Entity, Vec3)> = None;
    let mut popped = None;
    for _ in 0..(6.0 / TICK_INTERVAL) as usize {
        sim.ticks(1);
        if planned.is_none() {
            planned = sim.app.world().get::<Bot>(bot).unwrap().grenade_plan().cloned();
        }
        let w = sim.app.world_mut();
        let now: Vec<(Entity, Vec3, Option<Entity>)> = w
            .query::<(Entity, &Projectile, &Transform)>()
            .iter(w)
            .map(|(e, p, t)| (e, t.translation, p.thrower))
            .collect();
        if let Some((g, at)) = last
            && !now.iter().any(|(e, ..)| *e == g)
        {
            popped = Some(at);
            break;
        }
        if let Some((e, at, thrower)) = now.first() {
            assert_eq!(*thrower, Some(bot));
            last = Some((*e, *at));
        }
    }
    let plan = planned.expect("the bot planned a throw");
    assert!(plan.error < 2.0, "{plan:?}");
    assert!(plan.points.iter().any(|p| p.y > 3.0), "over the crate");
    let at = popped.expect("the grenade went off");
    let body = Vec3::new(0.0, 0.9, -24.0);
    assert!(
        at.distance(body) < 2.5,
        "went off at {at}, {} m away (planned {})",
        at.distance(body),
        plan.pop
    );
    let after = sim.app.world().get::<Health>(enemy).unwrap().current;
    assert!(after < before - 0.3, "hurt: {before} -> {after}");
    // The rifle back in hand, the spent grenade gone.
    sim.seconds(1.5);
    assert!(grenade_of(&sim, bot, HEGRENADE).is_none());
    assert_eq!(active_id(&sim, bot), Some(AK47));
    assert!(!sim.app.world().get::<Bot>(bot).unwrap().throwing());
}

#[test]
fn a_bot_turns_away_from_its_own_flash() {
    let mut sim = sim();
    let (bot, _) = setup(
        &mut sim,
        Vec3::new(1.0, 0.9, 1.0),
        Vec3::new(0.0, 0.9, -24.0),
        FLASHBANG,
    );
    let mut averted = false;
    for _ in 0..(4.0 / TICK_INTERVAL) as usize {
        sim.ticks(1);
        let b = sim.app.world().get::<Bot>(bot).unwrap();
        if let Some(from) = b.averting() {
            averted = true;
            // Facing away after the turn (yaw 0 looks down -Z: at the flash).
            let yaw = sim.intent(bot).yaw;
            let look = Vec2::new(-yaw.sin(), -yaw.cos());
            let to = (from - sim.position(bot)).xz().normalize();
            if sim.app.world().get::<Bot>(bot).unwrap().throwing() {
                continue;
            }
            sim.seconds(0.6);
            if sim.app.world().get::<Bot>(bot).unwrap().averting().is_some() {
                let yaw = sim.intent(bot).yaw;
                let look2 = Vec2::new(-yaw.sin(), -yaw.cos());
                assert!(look2.dot(to) < -0.5, "turned away: {look} -> {look2}, flash at {to}");
            }
            break;
        }
    }
    assert!(averted, "looked away from the flash");
}

/// Whether `bot` starts a throw (plans one, or a grenade flies) in
/// `secs`.
fn throws_within(sim: &mut Sim, bot: Entity, secs: f64) -> bool {
    for _ in 0..(secs / TICK_INTERVAL) as usize {
        sim.ticks(1);
        let w = sim.app.world_mut();
        if w.get::<Bot>(bot).unwrap().throwing() || w.query::<&Projectile>().iter(w).next().is_some() {
            return true;
        }
    }
    false
}

/// Playtest: bots threw at where the last enemy died, at the round's end.
/// A dead enemy is forgotten, an old sighting isn't thrown at, and no
/// throw starts while the round is closed (frozen, or won).
#[test]
fn bots_throw_nothing_at_dead_or_old_sightings_nor_after_the_round() {
    let (at, enemy_at) = (Vec3::new(1.0, 0.9, 1.0), Vec3::new(0.0, 0.9, -24.0));
    let now = |sim: &Sim| sim.app.world().resource::<Time>().elapsed_secs_f64();

    // Seen, shot dead: forgotten, nothing thrown where it fell.
    let mut sim = sim();
    sim.app
        .world_mut()
        .resource_mut::<mashup::console::Console>()
        .submit("mp_respawn_delay 100");
    let (bot, enemy) = setup(&mut sim, at, enemy_at, HEGRENADE);
    sim.app.world_mut().get_mut::<Bot>(bot).unwrap().lead = None;
    // In the open, 12 m off.
    sim.app.world_mut().get_mut::<Transform>(enemy).unwrap().translation = Vec3::new(1.0, 0.9, -11.0);
    let mut died = None;
    for _ in 0..(4.0 / TICK_INTERVAL) as usize {
        assert!(!throws_within(&mut sim, bot, TICK_INTERVAL), "threw while fighting");
        if sim.app.world().get::<Health>(enemy).unwrap().current <= 0.0 {
            died = Some(now(&sim));
            break;
        }
    }
    assert!(died.is_some(), "the bot shot the enemy");
    sim.ticks(1);
    assert_eq!(sim.app.world().get::<Bot>(bot).unwrap().lead, None, "forgot the dead");
    assert!(!throws_within(&mut sim, bot, 3.0), "threw at a dead enemy");

    // Seen 6 s ago: too old to throw at.
    let mut sim = self::sim();
    let (bot, _) = setup(&mut sim, at, enemy_at, HEGRENADE);
    let t = now(&sim);
    sim.app.world_mut().get_mut::<Bot>(bot).unwrap().lead = Some((enemy_at - Vec3::Y * 0.9, t - 6.0));
    assert!(!throws_within(&mut sim, bot, 3.0), "threw at an old sighting");

    // The round closed (frozen or over): no throw; open again: a throw.
    let mut sim = self::sim();
    let (bot, _) = setup(&mut sim, at, enemy_at, HEGRENADE);
    sim.app.insert_resource(RoundOpen(false));
    assert!(!throws_within(&mut sim, bot, 3.0), "threw with the round closed");
    sim.app.insert_resource(RoundOpen(true));
    let t = now(&sim);
    sim.app.world_mut().get_mut::<Bot>(bot).unwrap().lead = Some((enemy_at - Vec3::Y * 0.9, t));
    assert!(throws_within(&mut sim, bot, 3.0), "throws once the round is open");
}

#[test]
fn bots_buy_grenades_with_money_left() {
    let mut sim = sim();
    let p = sim.spawn_character(Vec3::new(0.0, 0.9, 12.0), placeholder::ID);
    sim.ticks(1);
    give(sim.app.world_mut(), p, AK47).unwrap();
    sim.seconds(1.0);
    let count =
        |sim: &Sim, id| grenade_of(sim, p, id).map_or(0, |w| sim.app.world().get::<Throwable>(w).unwrap().count);
    // Every roll a yes: armour, then an HE, two flashes and a smoke.
    sim.app.world_mut().entity_mut(p).insert(Money(16000));
    autobuy_rolling(sim.app.world_mut(), p, &mut || 0.0);
    assert_eq!(sim.app.world().get::<Armor>(p).map(|a| a.helmet), Some(true));
    assert_eq!(
        [
            count(&sim, HEGRENADE),
            count(&sim, FLASHBANG),
            count(&sim, SMOKEGRENADE)
        ],
        [1, 2, 1]
    );
    assert_eq!(
        sim.app.world().get::<Money>(p),
        Some(&Money(16000 - 1000 - 300 - 400 - 300))
    );
    sim.ticks(2);
    assert_eq!(active_id(&sim, p), Some(AK47), "the rifle stays in hand");
    // At the carry limits nothing more is bought.
    autobuy_rolling(sim.app.world_mut(), p, &mut || 0.0);
    assert_eq!(sim.app.world().get::<Money>(p), Some(&Money(14000)));

    // Rolls of 0.55: an HE (0.6) but no flash (0.5) and so on; with only
    // what armour leaves plus $350, the HE and no more.
    let q = sim.spawn_character(Vec3::new(2.0, 0.9, 12.0), placeholder::ID);
    sim.ticks(1);
    give(sim.app.world_mut(), q, AK47).unwrap();
    sim.app.world_mut().entity_mut(q).insert(Money(1350));
    autobuy_rolling(sim.app.world_mut(), q, &mut || 0.55);
    assert!(grenade_of(&sim, q, HEGRENADE).is_some());
    assert!(grenade_of(&sim, q, FLASHBANG).is_none());
    assert_eq!(sim.app.world().get::<Money>(q), Some(&Money(50)));
    // Never a yes: nothing.
    let r = sim.spawn_character(Vec3::new(4.0, 0.9, 12.0), placeholder::ID);
    sim.ticks(1);
    give(sim.app.world_mut(), r, AK47).unwrap();
    sim.app.world_mut().entity_mut(r).insert(Money(5000));
    autobuy_rolling(sim.app.world_mut(), r, &mut || 0.99);
    for id in [HEGRENADE, FLASHBANG, SMOKEGRENADE] {
        assert!(grenade_of(&sim, r, id).is_none());
    }
}

/// Every grenade a bot throws calls "Fire in the hole" to its team, as
/// a player's throw does (`weapon::grenade`).
#[test]
fn a_bots_throw_calls_fire_in_the_hole() {
    let mut sim = sim();
    let (bot, _) = setup(
        &mut sim,
        Vec3::new(1.0, 0.9, 1.0),
        Vec3::new(0.0, 0.9, -24.0),
        HEGRENADE,
    );
    let mut cursor = sim
        .app
        .world()
        .resource::<Messages<mashup::core::Radio>>()
        .get_cursor_current();
    let mut calls = 0;
    let mut thrown = false;
    for _ in 0..(4.0 / TICK_INTERVAL) as usize {
        sim.ticks(1);
        let w = sim.app.world_mut();
        thrown |= w.query::<&Projectile>().iter(w).next().is_some();
        let radio = w.resource::<Messages<mashup::core::Radio>>();
        calls += cursor
            .read(radio)
            .filter(|c| c.sender == bot && c.command == "fireinhole")
            .count();
    }
    assert!(thrown, "threw");
    assert_eq!(calls, 1, "one \"Fire in the hole\" for one throw");
}

/// A flash that would pop in a teammate's face isn't thrown (playtest:
/// bots flashed their own team). The teammate stands between the bot and
/// a remembered enemy in the open, looking that way; another throw with
/// the teammate behind the bot, looking away, still happens.
#[test]
fn bots_never_flash_a_teammate() {
    use mashup::core::Blinded;
    let at = Vec3::new(1.0, 0.9, 1.0);
    // Where an enemy was last heard, in the open 21 m off.
    let lead = Vec3::new(10.0, 0.0, -18.0);
    let run = |mate_at: Vec3, mate_yaw: f32| -> (f32, bool) {
        let mut sim = sim();
        let (bot, _) = setup(&mut sim, at, Vec3::new(0.0, 0.9, -24.0), FLASHBANG);
        let mate = sim.spawn_character(mate_at, placeholder::ID);
        sim.app.world_mut().entity_mut(mate).insert(Team(1));
        sim.intent(mate).yaw = mate_yaw;
        // (Before a tick: `setup` left it a lead behind the crate.)
        let now = sim.app.world().resource::<Time>().elapsed_secs_f64();
        sim.app.world_mut().get_mut::<Bot>(bot).unwrap().lead = Some((lead, now));
        let (mut worst, mut thrown) = (0.0f32, false);
        for _ in 0..(5.0 / TICK_INTERVAL) as usize {
            sim.ticks(1);
            let now = sim.app.world().resource::<Time>().elapsed_secs_f64();
            if let Some(b) = sim.app.world().get::<Blinded>(mate) {
                worst = worst.max((b.end - now) as f32);
            }
            let w = sim.app.world_mut();
            thrown |= w.query::<&Projectile>().iter(w).next().is_some();
            // Keep the lead fresh.
            let now = w.resource::<Time>().elapsed_secs_f64();
            if let Some(mut b) = w.get_mut::<Bot>(bot) {
                b.lead = Some((lead, now));
            }
        }
        (worst, thrown)
    };
    // In front, looking at where it would pop (yaw 0 looks down -Z).
    let (blind, thrown) = run(Vec3::new(6.0, 0.9, -6.0), 0.0);
    eprintln!("teammate in front: blinded for {blind:.2} s, flash thrown {thrown}");
    assert!(
        blind <= TEAM_FLASH_TIME,
        "the teammate was blinded {blind:.2} s"
    );
    // Behind the bot, looking away: thrown (the bot isn't afraid of every
    // teammate).
    let (blind, thrown) = run(Vec3::new(-2.0, 0.9, 20.0), std::f32::consts::PI);
    eprintln!("teammate behind, looking away: blinded for {blind:.2} s, flash thrown {thrown}");
    assert!(thrown, "thrown with the teammate out of harm's way");
    assert!(blind <= TEAM_FLASH_TIME, "{blind:.2} s");
}

/// `bot::grenades::TEAM_FLASH_TIME`: blindness a teammate may get, s.
const TEAM_FLASH_TIME: f32 = 1.5;
