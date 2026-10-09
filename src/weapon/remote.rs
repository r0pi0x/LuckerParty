//! Others' weapons on a network client (docs/plans/active/multiplayer.md,
//! slice 4): what a client sees of characters it doesn't play.
//!
//! - **What they hold** (`show_held`): a character drawn from snapshots
//!   carries one weapon, built from the registry like any weapon, so its
//!   body holds the right model and plays the right animations. It never
//!   runs the weapon frame (the weapon systems leave out
//!   `map::interp::NetDrawn` characters).
//! - **Their shots** (`RemoteShot`): the server sends each round someone
//!   else fires with its origin, angles, spread seed and spread
//!   (`WeaponEventKind::Fired`); the client traces the same pellets
//!   (`pellet_dirs`) through its own world with no damage (a client deals
//!   none), so their tracers, impacts and decals show and the fire sound
//!   plays, with the same spread pattern the server traced.
//! - **What else they do** (`RemoteAction`): reloads, swings and throws,
//!   for their bodies' animations.

use bevy::prelude::*;

use super::{
    AltModes, DamageEffect, Hitscan, Inventory, Penetration, SpreadShape, Weapon, WeaponEvent, WeaponEventKind,
    WeaponSounds, deliver, grenade::Throwable,
};
use crate::core::{NetRole, SimClock};

/// A round someone else fired, as the server traced it: draw it here.
#[derive(Message, Clone, Debug)]
pub struct RemoteShot {
    /// The character (this client's copy).
    pub shooter: Entity,
    pub weapon: &'static str,
    pub origin: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub seed: u32,
    pub spread: SpreadShape,
    pub mode: u8,
}

/// Something else someone else's weapon did (for its body's animation).
#[derive(Message, Clone, Debug)]
pub struct RemoteAction {
    pub shooter: Entity,
    pub kind: WeaponEventKind,
}

pub(super) fn plugin(app: &mut App) {
    app.add_message::<RemoteShot>()
        .add_message::<RemoteAction>()
        .add_systems(
            Update,
            (remote_shots, remote_actions).run_if(resource_equals(NetRole::Client)),
        );
}

/// Make `character` hold weapon `id` (None: nothing) in mode `mode`, its
/// grenade's pin out when `primed`: what a client draws of a character
/// it doesn't play.
pub fn show_held(world: &mut World, character: Entity, id: Option<&str>, mode: u8, primed: bool) {
    if world.get_entity(character).is_err() {
        return;
    }
    let held = world
        .get::<Inventory>(character)
        .and_then(|i| i.active)
        .filter(|w| world.get::<Weapon>(*w).map(|x| x.id) == id.and_then(|id| world.resource::<super::WeaponRegistry>().find(id).map(|d| d.id)));
    let weapon = match held {
        Some(w) => Some(w),
        None => {
            let old = world
                .get::<Inventory>(character)
                .map(|i| i.weapons.clone())
                .unwrap_or_default();
            for w in old {
                if let Ok(e) = world.get_entity_mut(w) {
                    e.despawn();
                }
            }
            let w = id.and_then(|id| super::spawn_weapon(world, id, character));
            world.entity_mut(character).insert(Inventory {
                weapons: w.into_iter().collect(),
                active: w,
                ..default()
            });
            w
        }
    };
    let Some(w) = weapon else { return };
    if let Some(mut m) = world.get_mut::<AltModes>(w)
        && m.current != mode
    {
        m.current = mode;
    }
    if let Some(mut t) = world.get_mut::<Throwable>(w)
        && t.pin != primed
    {
        t.pin = primed;
    }
}

/// Others' rounds traced again here: tracers, impacts and decals from the
/// `Shot` events, and the fire sound.
fn remote_shots(
    mut shots: MessageReader<RemoteShot>,
    owners: Query<&Inventory>,
    weapons: Query<(
        &Weapon,
        Option<&Hitscan>,
        Option<&DamageEffect>,
        Option<&Penetration>,
        Option<&WeaponSounds>,
    )>,
    mut world: deliver::World,
    clock: Res<SimClock>,
) {
    for s in shots.read() {
        let Some(active) = owners.get(s.shooter).ok().and_then(|i| i.active) else {
            continue;
        };
        let Ok((w, Some(scan), Some(effect), pen, sounds)) = weapons.get(active) else {
            continue;
        };
        if w.id != s.weapon {
            continue;
        }
        let scan = Hitscan {
            spread: s.spread,
            ..scan.clone()
        };
        let mut ctx = deliver::Shot {
            owner: s.shooter,
            weapon: active,
            eye: s.origin,
            aim: deliver::aim_of(s.yaw, s.pitch),
            yaw: s.yaw,
            pitch: s.pitch,
            seed: s.seed,
            now: clock.now,
            rewound: Vec::new(),
            w: &mut world,
        };
        ctx.fire(&scan, effect, pen);
        let sound = sounds.and_then(|x| match (s.mode > 0, &x.fire_alt) {
            (true, Some(alt)) => Some(alt.clone()),
            _ => x.fire.clone(),
        });
        if let Some(entry) = sound {
            world.sound(super::owner_sound(entry, s.shooter, s.origin, super::CHAN_WEAPON));
        }
    }
}

/// Others' reloads, swings and throws as weapon events of what they hold.
fn remote_actions(
    mut actions: MessageReader<RemoteAction>,
    owners: Query<&Inventory>,
    mut events: MessageWriter<WeaponEvent>,
) {
    for a in actions.read() {
        let Some(weapon) = owners.get(a.shooter).ok().and_then(|i| i.active) else {
            continue;
        };
        events.write(WeaponEvent {
            owner: a.shooter,
            weapon,
            kind: a.kind.clone(),
            replay: false,
        });
    }
}
