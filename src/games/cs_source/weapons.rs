//! CS:S weapons built from the shared weapon parts, with values from the
//! CS:S weapon scripts and view models (specs/cs_source/weapons.md, "Weapon
//! data"). Rules CS:S keeps in code we can't read are marked UNMEASURED
//! until the probe measurements (the spec's M1–M17) replace them.

use bevy::prelude::*;

use crate::weapon::{
    DamageEffect, FireTiming, HitgroupScale, Hitscan, Magazine, Melee, RegisterWeapons, StartingWeapons, Swing,
    Trigger, Weapon, WeaponSounds,
};

const UNIT: f32 = 0.0254;
/// Script damage is in hit points; ours is normalized (100 hp = 1.0).
const HP: f32 = 0.01;

pub const KNIFE: &str = "cs_source:weapon_knife";
pub const AK47: &str = "cs_source:weapon_ak47";

/// Sound entries the weapons use, for precaching with the map's sounds.
pub const SOUNDS: &[&str] = &[
    "Weapon_Knife.Deploy",
    "Weapon_Knife.Hit",
    "Weapon_Knife.HitWall",
    "Weapon_Knife.Slash",
    "Weapon_Knife.Stab",
    "Weapon_AK47.Single",
    "Weapon_AK47.Clipout",
    "Weapon_AK47.Clipin",
    "Default.ClipEmpty_Rifle",
];

/// UNMEASURED (spec 7.5, M6): community hitgroup multipliers.
const HITGROUPS: HitgroupScale = HitgroupScale {
    head: 4.0,
    chest: 1.0,
    stomach: 1.25,
    arm: 1.0,
    leg: 0.75,
};

pub struct CsWeaponsPlugin;

impl Plugin for CsWeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.register_weapon(KNIFE, knife).register_weapon(AK47, ak47);
        let mut start = app.world_mut().get_resource_or_init::<StartingWeapons>();
        if start.0.is_empty() {
            // The best weapon is drawn: given last.
            start.0 = vec![KNIFE, AK47];
        }
    }
}

fn knife(e: &mut EntityWorldMut) {
    // UNMEASURED (spec 6.2, M11): damages, ranges, refire times and the
    // backstab rule are community figures; the hull fallback is the SDK's.
    let swing = |range: f32, damage: f32, hit: f32, miss: f32| Swing {
        range: range * UNIT,
        hull: Some(16.0 * UNIT),
        facing_cos: 0.70721,
        damage: damage * HP,
        backstab: None,
        hit_refire: hit,
        miss_refire: miss,
        // SDK melee push: 300 kg·in/s per hit point.
        force: 300.0 * UNIT / HP,
        sound_hit: Some("Weapon_Knife.Hit".into()),
        sound_hit_world: Some("Weapon_Knife.HitWall".into()),
        sound_miss: Some("Weapon_Knife.Slash".into()),
    };
    let slash = swing(48.0, 20.0, 0.4, 0.5);
    let stab = Swing {
        backstab: Some(195.0 * HP),
        sound_hit: Some("Weapon_Knife.Stab".into()),
        ..swing(32.0, 65.0, 1.0, 1.1)
    };
    e.insert((
        Weapon {
            id: KNIFE,
            slot: 2,
            owner: None,
            draw_time: 1.0,
            max_speed: Some(250.0 * UNIT),
        },
        Melee {
            primary: slash,
            secondary: Some(stab),
        },
        WeaponSounds {
            deploy: Some("Weapon_Knife.Deploy".into()),
            ..default()
        },
    ));
}

fn ak47(e: &mut EntityWorldMut) {
    e.insert((
        Weapon {
            id: AK47,
            slot: 0,
            owner: None,
            draw_time: 1.0,
            max_speed: Some(221.0 * UNIT),
        },
        // UNMEASURED (M4): the "set" refire rule (every 7 ticks).
        Trigger {
            automatic: true,
            cycle: 0.1,
            timing: FireTiming::Set,
        },
        Magazine {
            clip: 30,
            size: 30,
            // UNMEASURED (M17): 7.62 mm max carry.
            reserve: 90,
            reload_time: 2.4324,
        },
        Hitscan {
            range: 8192.0 * UNIT,
            pellets: 1,
            // UNMEASURED (M1, M2): a fixed cone of Spread + InaccuracyStand
            // stands in for CS:S's inaccuracy model; no recoil yet (M3).
            spread: 0.0006 + 0.00916,
        },
        DamageEffect {
            amount: 36.0 * HP,
            // UNMEASURED (M5): RangeModifier per 500 units.
            falloff: 0.98,
            falloff_step: 500.0 * UNIT,
            hitgroups: HITGROUPS,
            // UNMEASURED (Q8): the template's .50 AE impulse, kg·in/s.
            impulse: 2400.0 * UNIT,
        },
        WeaponSounds {
            fire: Some("Weapon_AK47.Single".into()),
            empty: Some("Default.ClipEmpty_Rifle".into()),
            deploy: None,
            // View-model animation events (spec 3.8).
            reload: vec![
                (0.35, "Weapon_AK47.Clipout".into()),
                (1.54, "Weapon_AK47.Clipin".into()),
            ],
        },
    ));
}
