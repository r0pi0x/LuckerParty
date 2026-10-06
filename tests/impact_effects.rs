//! Scenario tests for CS:S impact effects (specs/cs_source/impact_effects.md)
//! on the greybox map: shots on a wall, into water and into a player.
//! The greybox has no surface data, so walls count as `default`
//! (concrete, C) and surfaces as unresolved (white).

use bevy::prelude::*;
use mashup::{
    games::cs_source::{
        TICK_INTERVAL,
        impact_effects::{BLOOD_COUNTS, ImpactEffectSettings, MATERIALS},
        weapons::CsWeaponsPlugin,
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    map::particles::{MapParticles, ParticleMaterial, ParticleMaterials, Particles},
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
