//! What CS:S view models play (specs/cs_source/weapons.md 3.3–3.8): the
//! draw activity when a weapon is deployed, a fire activity per shot or
//! swing, the reload activity, and idle once the last one has played out.
//! Drives each character's `map::ViewAnimator`; `map::view_model` draws it.

use bevy::prelude::*;

use crate::{
    core::Health,
    map::{DriveAnimation, ViewAnimator, ViewModels},
    weapon::{Inventory, Weapon, WeaponEvent, WeaponEventKind},
};

/// CS:S's `viewmodel_fov` default, horizontal degrees at 4:3 (open
/// question in docs/plans/active/view-models.md).
pub const VIEWMODEL_FOV: f32 = 54.0;

/// The view models' vertical field of view (radians): `VIEWMODEL_FOV` at
/// 4:3, kept on wider screens like the world camera's.
pub fn fov() -> f32 {
    2.0 * ((VIEWMODEL_FOV.to_radians() / 2.0).tan() * 3.0 / 4.0).atan()
}

/// Script `TimeToIdle` (s): idle no sooner than this after an attack
/// (spec weapons.md 3.7, legacy keys table). Weapons without one idle when
/// their sequence ends.
fn time_to_idle(weapon: &str) -> f32 {
    match weapon {
        super::weapons::AK47 => 1.9,
        _ => 0.0,
    }
}

pub const DRAW: &str = "ACT_VM_DRAW";
pub const IDLE: &str = "ACT_VM_IDLE";
pub const PRIMARY: &str = "ACT_VM_PRIMARYATTACK";
pub const SECONDARY: &str = "ACT_VM_SECONDARYATTACK";
pub const RELOAD: &str = "ACT_VM_RELOAD";
pub const HIT_CENTER: &str = "ACT_VM_HITCENTER";
pub const MISS_CENTER: &str = "ACT_VM_MISSCENTER";

/// `v_knife_t.mdl` tags the stab and both slashes `ACT_VM_HITCENTER` and
/// the stab miss `ACT_VM_MISSCENTER`, so its attacks are picked by
/// sequence name (spec 6.2 lists them; which the game plays when is open:
/// slashes are chosen at random here, hit or miss).
const KNIFE_SLASH: &[&str] = &["midslash1", "midslash2"];
const KNIFE_STAB: &[&str] = &["stab"];
const KNIFE_STAB_MISS: &[&str] = &["stab_miss"];

pub struct ViewAnimPlugin;

impl Plugin for ViewAnimPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<WeaponEvent>()
            .add_systems(Update, drive.in_set(DriveAnimation));
    }
}

/// Per character: when it last attacked and its sequence-choice dice.
#[derive(Component, Debug, Clone)]
pub struct ViewModelPlay {
    last_attack: f64,
    rng: u32,
}

impl ViewModelPlay {
    fn roll(&mut self) -> f32 {
        // xorshift32
        let mut x = self.rng.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Play the first available of `activities` from the start.
fn play(view: &mut ViewAnimator, dice: &mut ViewModelPlay, activities: &[&str], now: f64) -> bool {
    let Some(animator) = view.animator.as_mut() else { return false };
    let roll = dice.roll();
    let Some(s) = activities.iter().find_map(|a| animator.set.pick_activity(a, roll)) else {
        return false;
    };
    animator.restart(s, now);
    true
}

/// Play one of the sequences `names` (at random), else the first
/// available of `activities`.
fn play_named(view: &mut ViewAnimator, dice: &mut ViewModelPlay, names: &[&str], activities: &[&str], now: f64) {
    let Some(animator) = view.animator.as_mut() else { return };
    let found: Vec<usize> = names.iter().filter_map(|n| animator.set.sequence(n)).collect();
    if found.is_empty() {
        play(view, dice, activities, now);
        return;
    }
    let pick = found[((dice.roll() * found.len() as f32) as usize).min(found.len() - 1)];
    animator.restart(pick, now);
}

#[allow(clippy::type_complexity)]
fn drive(
    time: Res<Time>,
    models: Option<Res<ViewModels>>,
    mut characters: Query<(
        Entity,
        &Inventory,
        Option<&Health>,
        Option<&mut ViewAnimator>,
        Option<&mut ViewModelPlay>,
    )>,
    weapons: Query<&Weapon>,
    mut events: MessageReader<WeaponEvent>,
    mut commands: Commands,
) {
    let (dt, now) = (time.delta_secs(), time.elapsed_secs_f64());
    let events: Vec<&WeaponEvent> = events.read().collect();
    for (e, inventory, health, view, dice) in &mut characters {
        let (Some(mut view), Some(mut dice)) = (view, dice) else {
            commands.entity(e).insert((
                ViewAnimator::default(),
                ViewModelPlay {
                    last_attack: f64::MIN,
                    rng: e.to_bits() as u32 ^ 0x9e37_79b9,
                },
            ));
            continue;
        };
        let alive = health.is_none_or(|h| h.current > 0.0);
        let weapon = inventory
            .active
            .filter(|_| alive)
            .and_then(|w| weapons.get(w).ok())
            .map(|w| w.id);
        // A new weapon in hand draws it (spec 3.3).
        let mut drawn = false;
        if view.show(weapon, models.as_deref()) {
            drawn = play(&mut view, &mut dice, &[DRAW, IDLE], now);
        }
        let Some(weapon) = weapon else { continue };
        let mut fired = false;
        for ev in events.iter().filter(|ev| ev.owner == e) {
            match &ev.kind {
                WeaponEventKind::Deployed if !drawn => {
                    drawn = play(&mut view, &mut dice, &[DRAW, IDLE], now);
                }
                WeaponEventKind::Shot { .. } if !fired => {
                    fired = play(&mut view, &mut dice, &[PRIMARY], now);
                    dice.last_attack = now;
                }
                WeaponEventKind::Swing { hit, secondary, .. } => {
                    // Slashes have no miss sequence (spec 6.2): the hit
                    // ones play either way.
                    let (names, order): (&[&str], &[&str]) = match (secondary, hit) {
                        (false, _) => (KNIFE_SLASH, &[HIT_CENTER, PRIMARY]),
                        (true, true) => (KNIFE_STAB, &[HIT_CENTER, SECONDARY]),
                        (true, false) => (KNIFE_STAB_MISS, &[MISS_CENTER, SECONDARY]),
                    };
                    play_named(&mut view, &mut dice, names, order, now);
                    dice.last_attack = now;
                }
                WeaponEventKind::ReloadStarted => {
                    play(&mut view, &mut dice, &[RELOAD], now);
                    dice.last_attack = now;
                }
                _ => {}
            }
        }
        let Some(animator) = view.animator.as_mut() else { continue };
        animator.advance(dt, now);
        // Idle once the sequence has played out (spec 3.7); a non-looping
        // idle (the knife's) starts over.
        if animator.finished() && now - dice.last_attack >= time_to_idle(weapon) as f64 {
            play(&mut view, &mut dice, &[IDLE], now);
        }
    }
}
