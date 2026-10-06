//! CS:S weapons built from the shared weapon parts, with values from the
//! CS:S weapon scripts and view models (specs/cs_source/weapons.md, "Weapon
//! data") and the rules measured on the game ("CS:S values (measured)").
//! CS:S's feel that doesn't decompose into parts lives here as components:
//! `Inaccuracy` (the accuracy penalty) and `Recoil` (view punch per shot).
//! Bullet penetration is the shared `Penetration` part with CS:S's numbers
//! (`PASS_MATERIALS`, measured M13).

use bevy::prelude::*;

use crate::{
    core::{Intent, MovementState, SimSet, Velocity},
    weapon::{
        CharacterPass, DamageEffect, FireTiming, HitgroupScale, Hitscan, Inventory, Magazine, Melee, PassMaterial,
        PassMaterials, Penetration, RegisterWeapons, SpreadShape, StartingWeapons, Swing, Trigger, ViewPunch, Weapon,
        WeaponEvent, WeaponEventKind, WeaponFrame, WeaponSounds,
    },
};

const UNIT: f32 = 0.0254;
/// Script damage is in hit points; ours is normalized (100 hp = 1.0).
pub(super) const HP: f32 = 0.01;

pub const KNIFE: &str = "cs_source:weapon_knife";
pub const AK47: &str = "cs_source:weapon_ak47";

/// World models (the script's `playermodel`), held by characters.
pub const WORLD_MODELS: &[(&str, &str)] = &[
    (KNIFE, "models/weapons/w_knife_ct.mdl"),
    (AK47, "models/weapons/w_rif_ak47.mdl"),
];

/// View models (the script's `viewmodel`), seen by the local player, and
/// whether each is built right-handed (the script's `BuiltRightHanded`;
/// spec view_models.md 2). CS:S ships one knife view model for both teams.
/// The scripts are encrypted, so handedness comes from the models: the AK
/// is held left of the eye (left-handed), the knife right of it
/// (`tests/map_de_dust2.rs::view_model_handedness_in_the_files`). With the
/// default `cl_righthand 1` only the AK is mirrored, so both end up in the
/// right hand, as CS:S shows them.
pub const VIEW_MODELS: &[(&str, &str, bool)] = &[
    (KNIFE, "models/weapons/v_knife_t.mdl", true),
    (AK47, "models/weapons/v_rif_ak47.mdl", false),
];

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
    "Bounce.PistolShell",
    "Bounce.RifleShell",
    "Bounce.ShotgunShell",
];

/// Hitgroup multipliers (measured M6).
const HITGROUPS: HitgroupScale = HitgroupScale {
    head: 4.0,
    chest: 1.0,
    stomach: 1.25,
    arm: 1.0,
    leg: 0.75,
};

/// How bullets pass things (measured M13). A wall passes when its
/// thickness along the path is at most the ammo's power x the material's
/// scale; each object passed scales the damage carried on.
pub fn pass_materials() -> PassMaterials {
    PassMaterials {
        by_class: vec![
            // Wood: scale and damage measured.
            (
                'W',
                PassMaterial {
                    scale: 2.0,
                    damage: 0.6,
                },
            ),
            // Metal, sand (dirt): scales measured; damage factors are not
            // (UNMEASURED: halfway between wood and concrete, as players).
            (
                'M',
                PassMaterial {
                    scale: 1.0,
                    damage: 0.5,
                },
            ),
            (
                'D',
                PassMaterial {
                    scale: 0.5,
                    damage: 0.5,
                },
            ),
            // Concrete: both measured.
            (
                'C',
                PassMaterial {
                    scale: 0.4,
                    damage: 0.25,
                },
            ),
        ],
        // UNMEASURED (tile, grate, glass, plastic, ...): metal's scale and a
        // halved damage.
        default: PassMaterial {
            scale: 1.0,
            damage: 0.5,
        },
        character: CharacterPass {
            // A fit to the measured stop cases (21-22 units per player).
            cost: 21.5 * UNIT,
            damage: 0.5,
            // The range left beyond a player's exit is halved.
            range_scale: 0.5,
        },
    }
}

pub struct CsWeaponsPlugin;

impl Plugin for CsWeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.register_weapon(KNIFE, knife)
            .register_weapon(AK47, ak47)
            .add_message::<WeaponEvent>()
            .add_systems(
                FixedUpdate,
                (before_shots.before(WeaponFrame), after_shots.after(WeaponFrame)).in_set(SimSet::Weapons),
            )
            .add_plugins((
                super::impacts::ImpactSoundsPlugin,
                super::impact_effects::ImpactEffectsPlugin,
            ))
            .insert_resource(pass_materials());
        let mut start = app.world_mut().get_resource_or_init::<StartingWeapons>();
        if start.0.is_empty() {
            // The best weapon is drawn: given last.
            start.0 = vec![KNIFE, AK47];
        }
    }
}

fn knife(e: &mut EntityWorldMut) {
    // Measured (M11): slash 20, or 15 within 0.9 s of the last slash, no
    // backstab bonus; stab 65, 195 from behind. Hits are generic. Reach: a
    // +-16 box swept 48 (slash) / 32 (stab) units from the eye.
    let swing = |range: f32, damage: f32| Swing {
        range: range * UNIT,
        hull: Some(16.0 * UNIT),
        hull_reach: range * UNIT,
        // Not measured: the SDK's.
        facing_cos: 0.70721,
        damage: damage * HP,
        follow_up: None,
        backstab: None,
        hitgroups: false,
        hit_refire: 0.0,
        miss_refire: 0.0,
        miss_refire_other: 0.0,
        // SDK melee push: 300 kg·in/s per hit point.
        force: 300.0 * UNIT / HP,
        // WeaponArmorRatio 1.7 (M7: knife hits are generic, always covered).
        armor_ratio: Some(1.7),
        quantum: HP,
        sound_hit: Some("Weapon_Knife.Hit".into()),
        sound_hit_world: Some("Weapon_Knife.HitWall".into()),
        sound_miss: Some("Weapon_Knife.Slash".into()),
    };
    let slash = Swing {
        follow_up: Some((0.9, 15.0 * HP)),
        hit_refire: 0.5,
        miss_refire: 0.4,
        miss_refire_other: 0.5,
        ..swing(48.0, 20.0)
    };
    let stab = Swing {
        backstab: Some(195.0 * HP),
        hit_refire: 1.1,
        miss_refire: 1.0,
        miss_refire_other: 1.0,
        sound_hit: Some("Weapon_Knife.Stab".into()),
        ..swing(32.0, 65.0)
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
    let accuracy = Inaccuracy {
        spread: 0.0006,
        stand: 0.00916,
        crouch: 0.00687,
        jump: 0.43044,
        land: 0.08609,
        fire: 0.01158,
        movement: 0.09222,
        recovery_stand: 0.48815,
        recovery_crouch: 0.34868,
        value: 0.00916,
        on_ground: true,
        fall_speed: 0.0,
    };
    e.insert((
        Weapon {
            id: AK47,
            slot: 0,
            owner: None,
            draw_time: 1.0,
            max_speed: Some(221.0 * UNIT),
        },
        Trigger {
            automatic: true,
            cycle: 0.1,
            timing: FireTiming::CarryOver,
        },
        Magazine {
            clip: 30,
            size: 30,
            // Max carry: ammo_762mm_max (M17).
            reserve: 90,
            reserve_max: 90,
            reload_time: 2.4324,
            reload_while_held: false,
        },
        Hitscan {
            range: 8192.0 * UNIT,
            pellets: 1,
            spread: SpreadShape::Disc {
                inaccuracy: accuracy.value,
                spread: accuracy.spread,
            },
            // Bullets go along view + 2 x punch (M3).
            punch_scale: 2.0,
        },
        DamageEffect {
            amount: 36.0 * HP,
            // RangeModifier per 500 units (M5).
            falloff: 0.98,
            falloff_step: 500.0 * UNIT,
            hitgroups: HITGROUPS,
            // UNMEASURED (Q8): the template's .50 AE impulse, kg·in/s.
            impulse: 2400.0 * UNIT,
            // Damage is truncated to whole hit points (M5).
            quantum: HP,
            armor_ratio: Some(1.55),
        },
        Penetration {
            // 762MM: power 39 units, two objects (M13). It still passes at
            // 4000 units; no farther limit was found.
            power: 39.0 * UNIT,
            objects: 2,
            max_distance: f32::INFINITY,
        },
        accuracy,
        Recoil {
            standing: Kick::new((1.0, 0.175), (0.375, 0.0375), (5.75, 1.75)),
            // Caps not reached when measured: the standing ones.
            crouched: Kick::new((0.9, 0.15), (0.35, 0.025), (5.75, 1.75)),
            moving: Kick::new((1.5, 0.225), (0.45, 0.05), (6.5, 2.5)),
            airborne: Kick::new((2.0, 0.5), (1.0, 0.35), (9.0, 6.0)),
            ..default()
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

/// CS:S's accuracy penalty (measured M1/M2; values are the weapon script's
/// `Inaccuracy*`, `RecoveryTime*` and `Spread`, in tangent units). Rests
/// at `stand` (`crouch` when ducked); the excess falls to 10 % in the
/// recovery time; a shot adds `fire`, a jump `jump`, a landing `land` x
/// the fall speed / 301.99 u/s. Moving adds `movement` x
/// clamp((v - vmax/3) / (2 vmax/3)) at shot time.
#[derive(Component, Clone, Debug)]
pub struct Inaccuracy {
    pub spread: f32,
    pub stand: f32,
    pub crouch: f32,
    pub jump: f32,
    pub land: f32,
    pub fire: f32,
    pub movement: f32,
    pub recovery_stand: f32,
    pub recovery_crouch: f32,
    /// The current penalty.
    pub value: f32,
    on_ground: bool,
    /// Vertical speed on the previous tick, u/s (for landings).
    fall_speed: f32,
}

/// Airborne decay of the penalty's excess per 0.015 s tick (measured M1:
/// toward the crouched rest after a jump, 10 % left after 1.05 s; a fall
/// without a jump stays at the standing rest, so the rest is a floor).
const AIR_DECAY_PER_TICK: f32 = 0.96752;

/// A landing adds `land` x |fall speed| / this (u/s; measured M1, about the
/// jump speed).
const LAND_SPEED: f32 = 301.99;

/// One recoil kick set (measured M3), degrees: up `up` for the first shot
/// and `up + up_step·n` after (n = shots so far), sideways likewise; the
/// punch is clamped at `up_cap` (pitch) and `side_cap` (yaw).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Kick {
    pub up: f32,
    pub up_step: f32,
    pub side: f32,
    pub side_step: f32,
    pub up_cap: f32,
    pub side_cap: f32,
}

impl Kick {
    pub const fn new(up: (f32, f32), side: (f32, f32), caps: (f32, f32)) -> Self {
        Self {
            up: up.0,
            up_step: up.1,
            side: side.0,
            side_step: side.1,
            up_cap: caps.0,
            side_cap: caps.1,
        }
    }
}

/// CS:S's recoil (measured M3): view punch per shot from the kick set for
/// the shooter's state on the shot tick: airborne, else moving (any
/// horizontal speed), else crouched, else standing. The sideways kick
/// starts in a random direction and flips with a 1/8 chance per shot.
#[derive(Component, Clone, Debug, Default)]
pub struct Recoil {
    pub standing: Kick,
    pub crouched: Kick,
    pub moving: Kick,
    pub airborne: Kick,
    shots: u32,
    last_shot: f64,
    direction: f32,
    rng: u64,
}

/// Faster than this (u/s) counts as moving for recoil (measured: 5 u/s
/// moves; this only keeps float noise at rest out).
const MOVING_SPEED: f32 = 0.1;

/// The shot count resets after this long without a shot (measured: it
/// holds about 0.45 s, then falls within 0.1 s).
const SHOTS_RESET: f64 = 0.5;

/// Punch decay per second: |p| -= (10 + 0.5 |p|) degrees/s (M3).
fn decay_punch(p: Vec2, dt: f32) -> Vec2 {
    let len = p.length().to_degrees();
    if len <= 0.0 {
        return Vec2::ZERO;
    }
    let next = (len - (10.0 + 0.5 * len) * dt).max(0.0);
    p * (next / len)
}

/// Before the weapon frame: punch decays, the penalty recovers, and the
/// active weapon's spread is set from the penalty and the movement term.
fn before_shots(
    mut owners: Query<(&Inventory, &MovementState, &Velocity, &Intent, Option<&mut ViewPunch>)>,
    mut weapons: Query<(&Weapon, &mut Inaccuracy, &mut Hitscan)>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    for (inv, state, vel, _intent, punch) in &mut owners {
        if let Some(mut p) = punch {
            let next = decay_punch(p.0, dt);
            if p.0 != next {
                p.0 = next;
            }
        }
        let Some(active) = inv.active else { continue };
        let Ok((weapon, mut acc, mut scan)) = weapons.get_mut(active) else {
            continue;
        };
        let rest = if state.crouching { acc.crouch } else { acc.stand };
        let recovery = if state.crouching {
            acc.recovery_crouch
        } else {
            acc.recovery_stand
        };
        if acc.on_ground && !state.on_ground && vel.0.y > 0.0 {
            acc.value += acc.jump;
        } else if !acc.on_ground && state.on_ground {
            // Scaled by the vertical speed on the tick before (M1).
            acc.value += acc.land * acc.fall_speed.abs() / LAND_SPEED;
        }
        acc.on_ground = state.on_ground;
        acc.fall_speed = vel.0.y / UNIT;
        acc.value = if !state.on_ground {
            let target = acc.crouch;
            (target + (acc.value - target) * AIR_DECAY_PER_TICK.powf(dt / 0.015)).max(rest.min(acc.value))
        } else if acc.value > rest {
            rest + (acc.value - rest) * 0.1f32.powf(dt / recovery)
        } else {
            rest
        };
        let speed = vel.0.xz().length() / UNIT;
        let vmax = weapon.max_speed.unwrap_or(250.0 * UNIT) / UNIT;
        let moving = ((speed - vmax / 3.0) / (vmax * 2.0 / 3.0)).clamp(0.0, 1.0) * acc.movement;
        scan.spread = SpreadShape::Disc {
            inaccuracy: acc.value + moving,
            spread: acc.spread,
        };
    }
}

/// After the weapon frame: each shot adds to the penalty and kicks the
/// view.
fn after_shots(
    mut events: MessageReader<WeaponEvent>,
    mut weapons: Query<(Option<&mut Inaccuracy>, Option<&mut Recoil>)>,
    mut owners: Query<(&MovementState, &Velocity, Option<&mut ViewPunch>)>,
    mut commands: Commands,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    for e in events.read() {
        if !matches!(e.kind, WeaponEventKind::Shot { .. }) {
            continue;
        }
        let Ok((acc, recoil)) = weapons.get_mut(e.weapon) else {
            continue;
        };
        if let Some(mut acc) = acc {
            acc.value += acc.fire;
        }
        let Some(mut r) = recoil else { continue };
        let Ok((state, vel, punch)) = owners.get_mut(e.owner) else {
            continue;
        };
        if now - r.last_shot > SHOTS_RESET {
            r.shots = 0;
        }
        r.last_shot = now;
        r.shots += 1;
        // xorshift for the sideways direction.
        r.rng = if r.rng == 0 { e.weapon.to_bits() | 1 } else { r.rng };
        r.rng ^= r.rng << 13;
        r.rng ^= r.rng >> 7;
        r.rng ^= r.rng << 17;
        let roll = (r.rng >> 40) as f32 / (1u64 << 24) as f32;
        if r.shots == 1 {
            r.direction = if roll < 0.5 { -1.0 } else { 1.0 };
        } else if roll < 0.125 {
            r.direction = -r.direction;
        }
        let n = if r.shots == 1 { 0.0 } else { r.shots as f32 };
        let kick = if !state.on_ground {
            r.airborne
        } else if vel.0.xz().length() / UNIT > MOVING_SPEED {
            // Crouched and moving isn't measured: the moving set.
            r.moving
        } else if state.crouching {
            r.crouched
        } else {
            r.standing
        };
        let (up, side) = (kick.up + kick.up_step * n, kick.side + kick.side_step * n);
        let old = punch.as_ref().map_or(Vec2::ZERO, |p| p.0);
        let kicked = Vec2::new(
            (old.x + up.to_radians()).min(kick.up_cap.to_radians()),
            (old.y + side.to_radians() * r.direction).clamp(-kick.side_cap.to_radians(), kick.side_cap.to_radians()),
        );
        match punch {
            Some(mut p) => p.0 = kicked,
            None => {
                commands.entity(e.owner).insert(ViewPunch(kicked));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punch_decays_linearly_plus_proportionally() {
        // |p| 5 degrees: -(10 + 2.5) x 0.015 = -0.1875 per tick.
        let p = Vec2::new(5f32.to_radians(), 0.0);
        let next = decay_punch(p, 0.015).x.to_degrees();
        assert!((next - 4.8125).abs() < 1e-4, "{next}");
        assert_eq!(decay_punch(Vec2::new(0.001, 0.0), 0.015), Vec2::ZERO);
    }
}
