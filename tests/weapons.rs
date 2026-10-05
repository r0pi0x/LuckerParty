//! Scenario tests for the weapon frame with CS:S's knife and AK-47 on the
//! greybox map, at CS:S's tick (0.015 s). Expectations come from
//! specs/cs_source/weapons.md's test cases (T5, T6, T8, T10, T11).

use bevy::prelude::*;
use mashup::{
    core::{Health, Intent, MaxSpeed},
    games::cs_source::{
        TICK_INTERVAL,
        weapons::{AK47, CsWeaponsPlugin, KNIFE},
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    movement::placeholder,
    weapon::{Inventory, Magazine, Weapon},
};

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim
}

fn active(sim: &Sim, p: Entity) -> Entity {
    sim.app
        .world()
        .get::<Inventory>(p)
        .unwrap()
        .active
        .expect("no active weapon")
}

fn active_id(sim: &Sim, p: Entity) -> &'static str {
    sim.app.world().get::<Weapon>(active(sim, p)).unwrap().id
}

fn magazine(sim: &Sim, p: Entity) -> Magazine {
    sim.app.world().get::<Magazine>(active(sim, p)).unwrap().clone()
}

fn health(sim: &Sim, p: Entity) -> f32 {
    sim.app.world().get::<Health>(p).unwrap().current
}

/// Look from `p`'s eye at `target`.
fn aim_at(sim: &mut Sim, p: Entity, target: Vec3) {
    let eye = sim.position(p) + sim.state(p).eye_offset;
    let d = target - eye;
    let mut i = sim.intent(p);
    i.yaw = (-d.x).atan2(-d.z);
    i.pitch = d.y.atan2(d.xz().length());
}

/// Ticks until the clip changes, holding `fire`; None within `limit`.
fn ticks_until_shot(sim: &mut Sim, p: Entity, limit: u32) -> Option<u32> {
    let clip = magazine(sim, p).clip;
    for t in 1..=limit {
        sim.ticks(1);
        if magazine(sim, p).clip != clip {
            return Some(t);
        }
    }
    None
}

#[test]
fn characters_start_with_knife_and_ak47_drawn() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.ticks(2);
    let inv = sim.app.world().get::<Inventory>(p).unwrap();
    assert_eq!(inv.weapons.len(), 2);
    assert_eq!(active_id(&sim, p), AK47);
    // The AK-47's MaxPlayerSpeed, 221 units/s.
    let speed = sim.app.world().get::<MaxSpeed>(p).unwrap().0;
    assert!((speed - 221.0 * 0.0254).abs() < 1e-4, "{speed}");
}

#[test]
fn deploy_then_fire_every_seven_ticks() {
    // T5: the first shot 67 ticks after the draw (1.0 s); T6: held fire
    // every 7 ticks (0.1 s cycle, "set" rule).
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.ticks(1); // weapons given and the AK-47 drawn this tick
    sim.intent(p).fire = true;
    let first = ticks_until_shot(&mut sim, p, 200).expect("never fired");
    assert_eq!(first, 67, "first shot after the draw");
    for _ in 0..5 {
        assert_eq!(ticks_until_shot(&mut sim, p, 20), Some(7));
    }
    assert_eq!(magazine(&sim, p).clip, 24);
}

#[test]
fn reload_swaps_at_the_end_and_refuses_when_full() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.seconds(1.1);
    // T10: full clip: nothing happens.
    sim.intent(p).reload = true;
    sim.ticks(2);
    assert_eq!(magazine(&sim, p).clip, 30);
    sim.intent(p).reload = false;
    // T8: clip 10, reserve 90 -> after 163 ticks: 30 / 70, not before.
    let w = active(&sim, p);
    sim.app.world_mut().get_mut::<Magazine>(w).unwrap().clip = 10;
    sim.intent(p).reload = true;
    sim.ticks(1); // the reload starts on this tick
    sim.intent(p).reload = false;
    sim.ticks(162);
    assert_eq!(magazine(&sim, p).clip, 10, "swapped too early");
    sim.ticks(1);
    let m = magazine(&sim, p);
    assert_eq!((m.clip, m.reserve), (30, 70));
}

#[test]
fn switching_cancels_a_reload() {
    // T11: no ammo moves; the new weapon draws (1.0 s).
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.seconds(1.1);
    let ak = active(&sim, p);
    sim.app.world_mut().get_mut::<Magazine>(ak).unwrap().clip = 10;
    sim.intent(p).reload = true;
    sim.ticks(1);
    sim.intent(p).reload = false;
    sim.seconds(1.0);
    sim.intent(p).select = Some(2);
    sim.ticks(1);
    sim.intent(p).select = None;
    assert_eq!(active_id(&sim, p), KNIFE);
    sim.seconds(3.0);
    let m = sim.app.world().get::<Magazine>(ak).unwrap();
    assert_eq!((m.clip, m.reserve), (10, 90));
    // Knife speed.
    let speed = sim.app.world().get::<MaxSpeed>(p).unwrap().0;
    assert!((speed - 250.0 * 0.0254).abs() < 1e-4);
}

#[test]
fn ak47_shot_damages_a_target_once_per_shot() {
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target_at = greybox::SPAWNS[0] - Vec3::Z * 8.0;
    let target = sim.spawn_character(target_at, placeholder::ID);
    sim.seconds(1.1);
    // Chest height on the 1.8 m capsule (about 70 % up).
    let chest = sim.position(target) + Vec3::Y * (1.8 * 0.72 - 0.9);
    aim_at(&mut sim, shooter, chest);
    sim.intent(shooter).fire = true;
    sim.ticks(1);
    sim.intent(shooter).fire = false;
    sim.ticks(1);
    let lost = 1.0 - health(&sim, target);
    // 36 hp at ~8 m (315 units): 36 * 0.98^(315/500).
    let expected = 0.36 * 0.98f32.powf(8.0 / 0.0254 / 500.0);
    assert!((lost - expected).abs() < 0.01, "lost {lost}, expected {expected}");
}

#[test]
fn headshots_and_deaths() {
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 5.0, placeholder::ID);
    sim.seconds(1.1);
    let head = sim.position(target) + Vec3::Y * (1.8 * 0.92 - 0.9);
    aim_at(&mut sim, shooter, head);
    sim.intent(shooter).fire = true;
    sim.ticks(1);
    sim.intent(shooter).fire = false;
    sim.ticks(1);
    let lost = 1.0 - health(&sim, target);
    // UNMEASURED head multiplier 4: 144 hp kills.
    assert_eq!(health(&sim, target), 0.0, "lost {lost}");
}

#[test]
fn knife_slash_and_backstab() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 0.9, placeholder::ID);
    sim.ticks(1);
    sim.intent(p).select = Some(2);
    sim.ticks(1);
    sim.intent(p).select = None;
    sim.seconds(1.1);
    assert_eq!(active_id(&sim, p), KNIFE);
    let chest = sim.position(target) + Vec3::Y * 0.3;
    aim_at(&mut sim, p, chest);
    // The target faces the attacker: a plain slash.
    sim.intent(target).yaw = std::f32::consts::PI;
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    sim.ticks(1);
    assert!(
        (1.0 - health(&sim, target) - 0.20).abs() < 1e-4,
        "{}",
        health(&sim, target)
    );
    // Turned away: a stab from behind kills.
    sim.intent(target).yaw = 0.0;
    sim.seconds(0.6);
    sim.intent(p).secondary = true;
    sim.ticks(1);
    sim.intent(p).secondary = false;
    sim.ticks(1);
    assert_eq!(health(&sim, target), 0.0);
}

#[test]
fn empty_clip_reloads_itself() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.seconds(1.1);
    let w = active(&sim, p);
    sim.app.world_mut().get_mut::<Magazine>(w).unwrap().clip = 1;
    sim.intent(p).fire = true;
    sim.seconds(0.5);
    sim.intent(p).fire = false;
    assert_eq!(magazine(&sim, p).clip, 0);
    sim.seconds(2.6);
    let m = magazine(&sim, p);
    assert_eq!((m.clip, m.reserve), (30, 60));
    let _ = Intent::default();
}

#[test]
fn the_dead_respawn_with_fresh_weapons_and_scores_count() {
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 5.0, placeholder::ID);
    sim.seconds(1.1);
    let head = sim.position(target) + Vec3::Y * (1.8 * 0.92 - 0.9);
    aim_at(&mut sim, shooter, head);
    sim.intent(shooter).fire = true;
    sim.ticks(1);
    sim.intent(shooter).fire = false;
    sim.ticks(1);
    assert_eq!(health(&sim, target), 0.0);
    let score = |sim: &Sim, e| *sim.app.world().get::<mashup::rules::Score>(e).unwrap();
    assert_eq!(score(&sim, shooter).kills, 1);
    assert_eq!(score(&sim, target).deaths, 1);
    // Dead: no firing, and still dead before the delay.
    sim.seconds(1.5);
    assert_eq!(health(&sim, target), 0.0);
    sim.seconds(0.6);
    assert_eq!(health(&sim, target), 1.0);
    assert!(sim.app.world().get::<mashup::rules::Dead>(target).is_none());
    assert_eq!(active_id(&sim, target), AK47);
    assert_eq!(sim.app.world().get::<Inventory>(target).unwrap().weapons.len(), 2);
}

#[test]
fn a_bot_shoots_an_enemy_in_sight() {
    let mut sim = sim();
    sim.app.insert_resource(mashup::slots::Loadout {
        movement: placeholder::ID,
    });
    // In the open, 12 m from the bot's spawn (SPAWNS[1]).
    let player = sim.spawn_character(greybox::SPAWNS[1] + Vec3::X * 12.0, placeholder::ID);
    let bot = mashup::bot::add_bot(sim.app.world_mut(), mashup::core::Team(1)).expect("no bot");
    sim.app.world_mut().resource_mut::<mashup::bot::BotConfig>().stop = 1;
    // Reaction time, turning and the draw: hit within 2 s.
    let mut hit_at = None;
    for t in 0..(2.0 / TICK_INTERVAL) as u32 {
        sim.ticks(1);
        if health(&sim, player) < 1.0 {
            hit_at = Some(t);
            break;
        }
    }
    assert!(hit_at.is_some(), "bot never hit the player");
    assert_eq!(
        sim.app.world().get::<mashup::bot::Bot>(bot).unwrap().target,
        Some(player)
    );
}
