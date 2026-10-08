//! Scenario tests for CS:S impact effects (specs/cs_source/impact_effects.md)
//! on the greybox map: shots on a wall, into water and into a player,
//! ricochets, a knife into water, blood and team rules.
//! The greybox has no surface data, so walls count as `default`
//! (concrete, C) and surfaces as unresolved (white).

use bevy::prelude::*;
use mashup::{
    games::cs_source::{
        TICK_INTERVAL,
        impact_effects::{BLOOD_COUNTS, ImpactEffectSettings, MATERIALS, RICOCHET},
        weapons::CsWeaponsPlugin,
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    map::{
        PlaySound,
        particles::{MapParticles, ParticleMaterial, ParticleMaterials, Particles},
    },
    movement::{noclip, placeholder},
};

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    // Stand-in materials under the effects' names (no install needed).
    let materials = MATERIALS
        .iter()
        .map(|n| ParticleMaterial {
            name: n.to_string(),
            ..default()
        })
        .collect();
    sim.app.insert_resource(ParticleMaterials(MapParticles { materials }));
    sim
}

fn aim_at(sim: &mut Sim, p: Entity, target: Vec3) {
    let eye = sim.position(p) + sim.state(p).eye_offset;
    let d = target - eye;
    let mut i = sim.intent(p);
    i.yaw = (-d.x).atan2(-d.z);
    i.pitch = d.y.atan2(d.xz().length());
}

/// Fire one shot at `target` after the draw; returns the live particles
/// (capped ones, uncapped ones).
fn shoot(sim: &mut Sim, p: Entity, target: Vec3) -> (usize, usize) {
    sim.seconds(1.1);
    sim.app.world_mut().resource_mut::<Particles>().groups.clear();
    aim_at(sim, p, target);
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    sim.ticks(1);
    let pool = sim.app.world().resource::<Particles>();
    (pool.capped_count(), pool.count() - pool.capped_count())
}

#[test]
fn a_shot_on_a_concrete_wall_makes_flecks_and_dust() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let (capped, free) = shoot(&mut sim, p, Vec3::new(0.0, 1.5, greybox::NORTH_WALL_Z));
    // 4-16 flecks + 2 trail + 4 grit + 1 cap sprites.
    assert!((4 + 7..=16 + 7).contains(&capped), "{capped}");
    assert_eq!(free, 0);
}

#[test]
fn r_drawflecks_0_leaves_only_the_dust() {
    let mut sim = sim();
    sim.app.world_mut().resource_mut::<ImpactEffectSettings>().r_drawflecks = 0;
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let (capped, _) = shoot(&mut sim, p, Vec3::new(0.0, 1.5, greybox::NORTH_WALL_Z));
    assert_eq!(capped, 7);
}

#[test]
fn a_bullet_into_water_splashes_and_makes_no_impact() {
    let mut sim = sim();
    let centre = (greybox::TANK_MIN + greybox::TANK_MAX) / 2.0;
    let p = sim.spawn_character(Vec3::new(centre.x, 6.0, centre.y - 3.0), noclip::ID);
    let (capped, _) = shoot(&mut sim, p, Vec3::new(centre.x, 0.5, centre.y));
    // 16 drops + 8 gouts + 1 ripple, no flecks or dust.
    assert_eq!(capped, 25);
    // cl_show_splashes 0: nothing at all.
    sim.app
        .world_mut()
        .resource_mut::<ImpactEffectSettings>()
        .cl_show_splashes = 0;
    let (capped, _) = shoot(&mut sim, p, Vec3::new(centre.x, 0.5, centre.y));
    assert_eq!(capped, 0);
}

#[test]
fn a_hit_player_bleeds_unless_violence_hblood_is_0() {
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 8.0, placeholder::ID);
    sim.ticks(2);
    // Keep the bullet in the player (the AK-47 would pass it and mark the
    // wall behind).
    let rifle = sim
        .app
        .world()
        .get::<mashup::weapon::Inventory>(shooter)
        .unwrap()
        .active
        .unwrap();
    sim.app
        .world_mut()
        .entity_mut(rifle)
        .remove::<mashup::weapon::Penetration>();
    let chest = sim.position(target) + Vec3::Y * (1.8 * 0.72 - 0.9);
    let (capped, free) = shoot(&mut sim, shooter, chest);
    // The blood system only: no surface effect on players.
    assert_eq!(capped, 0);
    assert_eq!(free, BLOOD_COUNTS.iter().sum::<usize>());
    sim.app
        .world_mut()
        .resource_mut::<ImpactEffectSettings>()
        .violence_hblood = 0;
    let chest = sim.position(target) + Vec3::Y * (1.8 * 0.72 - 0.9);
    let (capped, free) = shoot(&mut sim, shooter, chest);
    assert_eq!((capped, free), (0, 0));
}

/// Messages of type `M` written over `ticks` ticks.
fn collect<M: Message + Clone>(sim: &mut Sim, ticks: usize) -> Vec<M> {
    let mut cursor = sim.app.world().resource::<Messages<M>>().get_cursor_current();
    let mut out = Vec::new();
    for _ in 0..ticks {
        sim.ticks(1);
        out.extend(cursor.read(sim.app.world().resource::<Messages<M>>()).cloned());
    }
    out
}

/// Spec section 2 step 6: 3 in 10 bullet impacts play `Bounce.Shrapnel`
/// at the hit (the effect seed is fixed, so the count is too).
#[test]
fn about_three_in_ten_bullet_impacts_ricochet() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.seconds(1.1);
    // (The first surface on that line is a greybox obstacle before the wall.)
    let wall = Vec3::new(0.0, 1.5, greybox::NORTH_WALL_Z);
    let mut ricochets = 0;
    let shots = 60;
    for _ in 0..shots {
        aim_at(&mut sim, p, wall);
        sim.intent(p).fire = true;
        let sounds = collect::<PlaySound>(&mut sim, 1);
        sim.intent(p).fire = false;
        let more = collect::<PlaySound>(&mut sim, 30);
        ricochets += sounds
            .iter()
            .chain(&more)
            .filter(|s| s.entry == RICOCHET)
            .inspect(|s| assert!(s.at.is_some(), "at the hit"))
            .count();
        // Never empty.
        let gun = sim.app.world().get::<mashup::weapon::Inventory>(p).unwrap().active.unwrap();
        sim.app.world_mut().get_mut::<mashup::weapon::Magazine>(gun).unwrap().clip = 30;
    }
    eprintln!("{ricochets} ricochets in {shots} impacts");
    assert!((8..=28).contains(&ricochets), "{ricochets} of {shots}");
}

/// Spec section 12: a knife swing from the dry into water splashes with a
/// fixed scale (8) and makes nothing else (no flecks, no slash decal).
#[test]
fn a_knife_swing_into_water_splashes() {
    use mashup::{games::cs_source::weapons::KNIFE, map::decal::PlaceDecal, weapon::Weapon};
    let mut sim = sim();
    let centre = (greybox::TANK_MIN + greybox::TANK_MAX) / 2.0;
    let p = sim.spawn_character(Vec3::new(centre.x, 4.0, centre.y), noclip::ID);
    sim.ticks(2);
    // The eye 0.4 m above the water: the slash's 48 units reach into it.
    let eye_offset = sim.state(p).eye_offset;
    let at = Vec3::new(centre.x, greybox::TANK_WATER_DEPTH + 0.4, centre.y) - eye_offset;
    sim.app.world_mut().get_mut::<Transform>(p).unwrap().translation = at;
    let knife = {
        let world = sim.app.world();
        let inv = world.get::<mashup::weapon::Inventory>(p).unwrap();
        inv.weapons
            .iter()
            .copied()
            .find(|w| world.get::<Weapon>(*w).unwrap().id == KNIFE)
            .unwrap()
    };
    sim.app.world_mut().get_mut::<mashup::weapon::Inventory>(p).unwrap().wanted = Some(knife);
    sim.seconds(1.5);
    sim.app.world_mut().resource_mut::<Particles>().groups.clear();
    {
        let mut i = sim.intent(p);
        i.pitch = -std::f32::consts::FRAC_PI_2 + 0.01;
        i.fire = true;
    }
    let decals = collect::<PlaceDecal>(&mut sim, 1);
    sim.intent(p).fire = false;
    let sounds = collect::<PlaySound>(&mut sim, 1);
    let pool = sim.app.world().resource::<Particles>();
    // 16 drops + 8 gouts + 1 ripple, nothing else.
    assert_eq!(pool.capped_count(), 25);
    assert!(decals.is_empty(), "no slash mark under water: {decals:?}");
    assert!(sounds.iter().all(|s| s.entry != RICOCHET));
}

/// Spec section 10: blood (the burst and the stains on the wall behind)
/// only where the damage is taken: not from a teammate with friendly fire
/// off.
#[test]
fn blood_stains_the_wall_behind_unless_team_rules_refuse_the_hit() {
    use mashup::{
        core::Team,
        map::decal::{DecalGroup, PlaceDecal},
    };
    for (same_team, friendly_fire, bleeds) in [(false, 0, true), (true, 0, false), (true, 1, true)] {
        let mut sim = sim();
        // 2 m in front of the north wall, shot from 8 m.
        let target = sim.spawn_character(Vec3::new(0.0, 1.0, greybox::NORTH_WALL_Z + 2.0), placeholder::ID);
        let shooter = sim.spawn_character(Vec3::new(0.0, 1.0, greybox::NORTH_WALL_Z + 10.0), placeholder::ID);
        sim.app.world_mut().entity_mut(shooter).insert(Team(1));
        sim.app
            .world_mut()
            .entity_mut(target)
            .insert(Team(if same_team { 1 } else { 2 }));
        sim.app
            .world_mut()
            .resource_mut::<mashup::console::Console>()
            .submit(&format!("mp_friendlyfire {friendly_fire}"));
        sim.seconds(1.1);
        sim.app.world_mut().resource_mut::<Particles>().groups.clear();
        let chest = sim.position(target) + Vec3::Y * 0.4;
        aim_at(&mut sim, shooter, chest);
        sim.intent(shooter).fire = true;
        let decals = collect::<PlaceDecal>(&mut sim, 1);
        sim.intent(shooter).fire = false;
        let decals: Vec<PlaceDecal> = decals
            .into_iter()
            .chain(collect::<PlaceDecal>(&mut sim, 1))
            .filter(|d| matches!(&d.group, DecalGroup::Named(n) if n == "Blood"))
            .collect();
        let free = {
            let pool = sim.app.world().resource::<Particles>();
            pool.count() - pool.capped_count()
        };
        if bleeds {
            assert_eq!(free, BLOOD_COUNTS.iter().sum::<usize>(), "team {same_team} ff {friendly_fire}");
            assert!(!decals.is_empty(), "blood on the wall behind");
            for d in &decals {
                assert!((d.point.z - greybox::NORTH_WALL_Z).abs() < 0.3, "on the north wall: {}", d.point);
            }
        } else {
            assert_eq!(free, 0, "a refused hit doesn't bleed");
            assert!(decals.is_empty());
        }
    }
}
