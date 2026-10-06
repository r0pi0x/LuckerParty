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
    // T5: the first shot 67 ticks after the draw (1.0 s).
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.ticks(1); // weapons given and the AK-47 drawn this tick
    sim.intent(p).fire = true;
    let first = ticks_until_shot(&mut sim, p, 200).expect("never fired");
    assert_eq!(first, 67, "first shot after the draw");
    // Held: next = previous next + 0.1 s, one shot a tick at most, so the
    // AK-47 fires 7, 7, 7, 6 ticks apart (measured M4: exactly 600 rpm).
    let gaps: Vec<u32> = (0..6).map(|_| ticks_until_shot(&mut sim, p, 20).unwrap()).collect();
    assert_eq!(gaps, [7, 7, 6, 7, 7, 6]);
    assert_eq!(magazine(&sim, p).clip, 23);
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
    // 36 hp at ~8 m (about 315 units): int(36 * 0.98^(d/500)) = 35 (M5).
    assert!((lost - 0.35).abs() < 1e-4, "lost {lost}");
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

#[test]
fn knife_follow_up_slash_and_miss_refires() {
    // Measured M11: 20, then 15 within 0.9 s of the last slash; a slash
    // hit blocks both attacks 0.5 s.
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 0.9, placeholder::ID);
    sim.ticks(1);
    sim.intent(p).select = Some(2);
    sim.ticks(1);
    sim.intent(p).select = None;
    sim.seconds(1.1);
    let chest = sim.position(target) + Vec3::Y * 0.3;
    aim_at(&mut sim, p, chest);
    sim.intent(target).yaw = std::f32::consts::PI;
    sim.intent(p).fire = true;
    // Held: hits every 34 ticks (0.5 s), 20 then 15s.
    sim.ticks(70);
    sim.intent(p).fire = false;
    sim.ticks(1);
    let lost = ((1.0 - health(&sim, target)) * 100.0).round() as i32;
    assert_eq!(lost, 20 + 15 + 15, "three slashes");
}

#[test]
fn ak47_recoil_kicks_and_inaccuracy_grows() {
    use mashup::{games::cs_source::weapons::Inaccuracy, weapon::ViewPunch};
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    // The spawn drop lands (+InaccuracyLand); let that recover too.
    sim.seconds(3.0);
    let w = active(&sim, p);
    let rest = sim.app.world().get::<Inaccuracy>(w).unwrap().value;
    assert!((rest - 0.00916).abs() < 1e-5);
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    // First shot: 1 degree up, 0.375 sideways (M3), plus the fire penalty.
    let punch = sim.app.world().get::<ViewPunch>(p).unwrap().0;
    assert!((punch.x.to_degrees() - 1.0).abs() < 1e-3, "{punch}");
    assert!((punch.y.to_degrees().abs() - 0.375).abs() < 1e-3, "{punch}");
    let after = sim.app.world().get::<Inaccuracy>(w).unwrap().value;
    assert!((after - (0.00916 + 0.01158)).abs() < 1e-5, "{after}");
    // Both recover: the punch is gone within a second, the penalty is
    // back near rest after its recovery time.
    sim.seconds(1.0);
    assert_eq!(sim.app.world().get::<ViewPunch>(p).unwrap().0, Vec2::ZERO);
    let v = sim.app.world().get::<Inaccuracy>(w).unwrap().value;
    assert!(v - 0.00916 < 0.0002, "{v}");
}

#[test]
fn empty_clip_held_dry_fires_once_then_reloads_on_release() {
    // Measured M10: no reload while attack stays held.
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.seconds(1.1);
    let w = active(&sim, p);
    sim.app.world_mut().get_mut::<Magazine>(w).unwrap().clip = 1;
    sim.intent(p).fire = true;
    sim.seconds(3.0);
    assert_eq!(magazine(&sim, p).clip, 0, "reloaded while held");
    sim.intent(p).fire = false;
    sim.seconds(2.6);
    assert_eq!(magazine(&sim, p).clip, 30);
}

#[test]
fn armour_takes_its_share() {
    use mashup::weapon::Armor;
    // Measured M7/M11: a 20-damage slash on 100 armour: 17 health, 1 armour.
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 0.9, placeholder::ID);
    sim.app.world_mut().entity_mut(target).insert(Armor {
        amount: 1.0,
        helmet: false,
    });
    sim.ticks(1);
    sim.intent(p).select = Some(2);
    sim.ticks(1);
    sim.intent(p).select = None;
    sim.seconds(1.1);
    let chest = sim.position(target) + Vec3::Y * 0.3;
    aim_at(&mut sim, p, chest);
    sim.intent(target).yaw = std::f32::consts::PI;
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    sim.ticks(1);
    let lost = ((1.0 - health(&sim, target)) * 100.0).round() as i32;
    let armor = (sim.app.world().get::<Armor>(target).unwrap().amount * 100.0).round() as i32;
    assert_eq!((lost, armor), (17, 99));
}

#[test]
fn armour_without_helmet_leaves_headshots_alone() {
    use mashup::weapon::Armor;
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 5.0, placeholder::ID);
    sim.app.world_mut().entity_mut(target).insert(Armor {
        amount: 1.0,
        helmet: false,
    });
    sim.seconds(1.1);
    let head = sim.position(target) + Vec3::Y * (1.8 * 0.92 - 0.9);
    aim_at(&mut sim, shooter, head);
    sim.intent(shooter).fire = true;
    sim.ticks(1);
    sim.intent(shooter).fire = false;
    sim.ticks(1);
    // 36 x 4 at ~200 units kills; the armour is untouched.
    assert_eq!(health(&sim, target), 0.0);
    assert_eq!(sim.app.world().get::<Armor>(target).unwrap().amount, 1.0);
}

// ---------------------------------------------------------------------------
// Penetration and collaterals (measured M13)

const UNIT: f32 = 0.0254;

/// Damage lost, in whole hit points.
fn lost_hp(sim: &Sim, p: Entity) -> i32 {
    ((1.0 - health(sim, p)) * 100.0).round() as i32
}

/// One AK-47 shot from `shooter` at `target`'s chest (72 % up the capsule),
/// once the rifle is drawn.
fn shoot_chest(sim: &mut Sim, shooter: Entity, target: Entity) {
    sim.seconds(1.1);
    let chest = sim.position(target) + Vec3::Y * (1.8 * 0.72 - 0.9);
    aim_at(sim, shooter, chest);
    sim.intent(shooter).fire = true;
    sim.ticks(1);
    sim.intent(shooter).fire = false;
    sim.ticks(1);
}

#[test]
fn ak47_collateral_carries_half_the_damage() {
    // Spec M13 (through players): first at ~300 units, second at ~400,
    // chest: 17 on the second (35 alone); the first takes its own 35.
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let first = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 8.0, placeholder::ID);
    let second = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 10.6, placeholder::ID);
    shoot_chest(&mut sim, shooter, second);
    assert_eq!((lost_hp(&sim, first), lost_hp(&sim, second)), (35, 17));
}

#[test]
fn ak47_does_not_pass_two_players() {
    // Two players use 2 x 21.5 of the AK-47's 39: the second is hit but
    // not passed.
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let first = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 8.0, placeholder::ID);
    let second = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 10.6, placeholder::ID);
    let third = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 13.0, placeholder::ID);
    shoot_chest(&mut sim, shooter, third);
    assert_eq!(
        (lost_hp(&sim, first), lost_hp(&sim, second), lost_hp(&sim, third)),
        (35, 17, 0)
    );
}

#[test]
fn awp_penetration_passes_two_players() {
    // The AWP isn't in yet: the AK-47 with 338MAG's power 45 and three
    // objects (M13) passes two players (2 x 21.5) and hits a third, with
    // x 0.5 carried per player and falloff again at every hit:
    // 35.57 x 0.5 x 0.98^(401/500) x 0.5 x 0.98^(495/500) = 8.6.
    use mashup::weapon::Penetration;
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let first = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 8.0, placeholder::ID);
    let second = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 10.6, placeholder::ID);
    let third = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 13.0, placeholder::ID);
    sim.ticks(1);
    let w = active(&sim, shooter);
    sim.app.world_mut().entity_mut(w).insert(Penetration {
        power: 45.0 * UNIT,
        objects: 3,
        max_distance: f32::INFINITY,
    });
    shoot_chest(&mut sim, shooter, third);
    assert_eq!(
        (lost_hp(&sim, first), lost_hp(&sim, second), lost_hp(&sim, third)),
        (35, 17, 8)
    );
}

#[test]
fn knife_never_passes_a_player() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let first = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 0.9, placeholder::ID);
    let second = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 1.75, placeholder::ID);
    sim.ticks(1);
    sim.intent(p).select = Some(2);
    sim.ticks(1);
    sim.intent(p).select = None;
    sim.seconds(1.1);
    let chest = sim.position(first) + Vec3::Y * 0.3;
    aim_at(&mut sim, p, chest);
    for e in [first, second] {
        sim.intent(e).yaw = std::f32::consts::PI;
    }
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    sim.ticks(1);
    assert_eq!((lost_hp(&sim, first), lost_hp(&sim, second)), (20, 0));
}

/// A wall of `surface` across the line from SPAWNS[0] to -Z: front face
/// `at` meters ahead, `thick` units thick. The surface scripts give wood
/// and concrete their CS:S material classes.
fn wall(sim: &mut Sim, at: f32, thick: f32, surface: &str) {
    use avian3d::prelude::*;
    use mashup::map::{MapSounds, MapSurface, PropSurface, sound::SoundBank};
    let mut sounds = MapSounds::default();
    for (name, class) in [("wood", 'W'), ("concrete", 'C')] {
        sounds.surfaces.insert(
            name.into(),
            MapSurface {
                game_material: class,
                ..default()
            },
        );
    }
    sim.app.insert_resource(SoundBank(std::sync::Arc::new(sounds)));
    let t = thick * UNIT;
    let z = greybox::SPAWNS[0].z - at - t / 2.0;
    sim.app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(4.0, 3.0, t),
        Transform::from_xyz(0.0, 1.5, z),
        PropSurface(surface.into()),
    ));
}

/// Damage through a wall `thick` units of `surface` 3 m ahead to a target
/// 6 m ahead.
fn through_wall(thick: f32, surface: &str) -> i32 {
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 6.0, placeholder::ID);
    wall(&mut sim, 3.0, thick, surface);
    shoot_chest(&mut sim, shooter, target);
    lost_hp(&sim, target)
}

#[test]
fn ak47_passes_walls_thinner_than_its_limit() {
    // Limits 39 x material scale: wood 78 units, concrete 15.6 (M13).
    // Through: 36 x 0.98^(118/500) x factor x 0.98^(220/500), chest:
    // wood x 0.6 -> 21, concrete x 0.25 -> 8 (35 with no wall).
    assert_eq!(through_wall(70.0, "wood"), 21);
    assert_eq!(through_wall(15.0, "concrete"), 8);
}

#[test]
fn ak47_stops_in_walls_thicker_than_its_limit() {
    assert_eq!(through_wall(85.0, "wood"), 0);
    assert_eq!(through_wall(17.0, "concrete"), 0);
}

#[test]
fn ak47_passes_two_doors_then_hits() {
    // Two 8-unit wooden doors use 2 x 4 of 39 and both object passes: the
    // target behind still takes damage, x 0.6 per door with falloff again
    // at every hit: 36 x 0.98^(79/500) x 0.6 x 0.98^(158/500) x 0.6 x
    // 0.98^(220/500) = 12.8.
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 6.0, placeholder::ID);
    wall(&mut sim, 2.0, 8.0, "wood");
    wall(&mut sim, 4.0, 8.0, "wood");
    shoot_chest(&mut sim, shooter, target);
    assert_eq!(lost_hp(&sim, target), 12);
}

#[test]
fn ak47_recoil_sets_for_moving_and_airborne() {
    use mashup::weapon::ViewPunch;
    // Measured M3: first-shot kick up 1.5 moving, 2.0 airborne.
    let first_kick = |setup: fn(&mut Sim, Entity)| {
        let mut sim = sim();
        let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
        sim.seconds(1.1);
        setup(&mut sim, p);
        sim.intent(p).fire = true;
        sim.ticks(1);
        sim.intent(p).fire = false;
        sim.app.world().get::<ViewPunch>(p).unwrap().0.x.to_degrees()
    };
    let moving = first_kick(|sim, p| {
        sim.intent(p).move_axis = Vec2::new(0.0, 1.0);
        sim.ticks(10);
    });
    assert!((moving - 1.5).abs() < 1e-3, "moving {moving}");
    let airborne = first_kick(|sim, p| {
        sim.intent(p).jump = true;
        sim.ticks(3);
        sim.intent(p).jump = false;
        assert!(!sim.state(p).on_ground);
    });
    assert!((airborne - 2.0).abs() < 1e-3, "airborne {airborne}");
}

#[test]
fn landing_adds_inaccuracy_by_fall_speed() {
    use mashup::{core::Velocity, games::cs_source::weapons::Inaccuracy};
    // Measured M1: I += InaccuracyLand x |v_z| / 301.99 on the landing tick
    // (v_z from the tick before), then that tick's ground recovery.
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.seconds(3.0);
    let w = active(&sim, p);
    sim.intent(p).jump = true;
    sim.ticks(2);
    sim.intent(p).jump = false;
    let (mut value, mut vz) = (0.0f32, 0.0f32);
    for _ in 0..200 {
        let was_airborne = !sim.state(p).on_ground;
        sim.ticks(1);
        if was_airborne && sim.state(p).on_ground {
            let now = sim.app.world().get::<Inaccuracy>(w).unwrap().value;
            let raised = value + 0.08609 * vz.abs() / 301.99;
            let expected = 0.00916 + (raised - 0.00916) * 0.1f32.powf(TICK_INTERVAL as f32 / 0.48815);
            assert!(vz < -100.0, "fall speed {vz}");
            assert!((now - expected).abs() < 1e-5, "{now} vs {expected}");
            return;
        }
        value = sim.app.world().get::<Inaccuracy>(w).unwrap().value;
        vz = sim.app.world().get::<Velocity>(p).unwrap().0.y / UNIT;
    }
    panic!("never landed");
}
