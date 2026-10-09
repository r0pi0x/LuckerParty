//! What a player sounds like when hurt and when killed: the install's
//! `game_sounds.txt` entries, played from the victim (its own sounds are
//! centred for the victim, `map::sound::Ear`; positional for everyone
//! else). docs/plans/active/sound.md "Damage and death"; the choices
//! the scripts and specs leave open are in docs/tech-debt.md.
//!
//! - A bullet in the head: `Player.DamageHelmet` (CHAN_BODY) when a
//!   helmet covers it, else `Player.DamageHeadShot` (CHAN_VOICE) unless
//!   the shot kills (the death sound plays instead).
//! - A bullet anywhere armour covers (chest, stomach, arms, generic):
//!   `Player.DamageKevlar` (CHAN_BODY). Bare flesh gets only the shot's
//!   own impact (`Flesh.BulletImpact`, the hit surface's; `impacts.rs`).
//! - A landing that takes health: `Player.FallDamage` (CHAN_BODY;
//!   sounds.md, "Landing").
//! - A death: `Player.DeathHeadShot` for a bullet in the head, else
//!   `Player.Death` (both CHAN_VOICE).
//!
//! Only damage that is taken counts (not refused by team rules, map
//! filters or god mode). Hurt sounds are the server's (`map::GameSound`:
//! every client hears them once, the victim's own included, since no
//! client predicts being hit); death sounds play wherever the death is
//! seen (`core::Died`, which a network client gets from the server's
//! kill), so they are not sent again.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::{
    core::{Damage, DamageKind, DamageRules, Died, God, Health, Hitgroup, SimSet, Team},
    map::{GameSound, PlaySound},
    objectives::hostages::Hostage,
    weapon::Armor,
};

pub const DAMAGE_KEVLAR: &str = "Player.DamageKevlar";
pub const DAMAGE_HELMET: &str = "Player.DamageHelmet";
pub const DAMAGE_HEADSHOT: &str = "Player.DamageHeadShot";
pub const FALL_DAMAGE: &str = "Player.FallDamage";
pub const DEATH: &str = "Player.Death";
pub const DEATH_HEADSHOT: &str = "Player.DeathHeadShot";

/// The entries this module plays (precached with every map).
pub const SOUNDS: &[&str] = &[
    DAMAGE_KEVLAR,
    DAMAGE_HELMET,
    DAMAGE_HEADSHOT,
    FALL_DAMAGE,
    DEATH,
    DEATH_HEADSHOT,
];

pub struct PainSoundsPlugin;

impl Plugin for PainSoundsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArmorBeforeShots>()
            .add_message::<Damage>()
            .add_message::<Died>()
            .add_message::<GameSound>()
            .add_message::<PlaySound>()
            .add_systems(
                FixedUpdate,
                (
                    remember_armor.after(SimSet::Movement).before(SimSet::Weapons),
                    hurt_sounds.after(SimSet::Weapons).before(crate::core::apply_damage),
                )
                    .run_if(crate::core::authoritative),
            )
            .add_systems(Update, death_sounds);
    }
}

/// Every character's armour as the tick's shots start: whether armour
/// covered a hit is decided by what it had before the hit took its share.
#[derive(Resource, Default)]
struct ArmorBeforeShots(HashMap<Entity, Armor>);

fn remember_armor(armor: Query<(Entity, &Armor)>, mut before: ResMut<ArmorBeforeShots>) {
    before.0.clear();
    before.0.extend(armor.iter().map(|(e, a)| (e, *a)));
}

/// The hurt sound for one hit that is taken; `lethal`: it kills.
pub fn hurt_sound(kind: DamageKind, group: Hitgroup, armor: Option<Armor>, lethal: bool) -> Option<&'static str> {
    let covered = armor.is_some_and(|a| a.covers(group));
    match kind {
        DamageKind::Fall => Some(FALL_DAMAGE),
        DamageKind::Bullet if group == Hitgroup::Head => match (covered, lethal) {
            (true, _) => Some(DAMAGE_HELMET),
            (false, false) => Some(DAMAGE_HEADSHOT),
            (false, true) => None,
        },
        DamageKind::Bullet if covered => Some(DAMAGE_KEVLAR),
        _ => None,
    }
}

/// The death sound for the hit that killed.
pub fn death_sound(kind: DamageKind, group: Hitgroup) -> &'static str {
    if kind == DamageKind::Bullet && group == Hitgroup::Head {
        DEATH_HEADSHOT
    } else {
        DEATH
    }
}

fn hurt_sounds(
    mut damage: MessageReader<Damage>,
    before: Res<ArmorBeforeShots>,
    victims: Query<(&Health, &Transform, Has<God>), (With<Team>, Without<Hostage>)>,
    rules: DamageRules,
    mut out: MessageWriter<GameSound>,
) {
    // Health left as this tick's hits land (several can hit one victim).
    let mut left: HashMap<Entity, f32> = HashMap::new();
    for d in damage.read() {
        let Ok((health, at, god)) = victims.get(d.target) else {
            continue;
        };
        if god || rules.refuse(d) {
            continue;
        }
        let h = left.entry(d.target).or_insert(health.current);
        if *h <= 0.0 {
            continue;
        }
        *h -= d.amount;
        let Some(entry) = hurt_sound(d.kind, d.hitgroup, before.0.get(&d.target).copied(), *h <= 0.0) else {
            continue;
        };
        out.write(GameSound(PlaySound {
            source: Some(d.target),
            ..PlaySound::at(entry, at.translation)
        }));
    }
}

fn death_sounds(
    mut died: MessageReader<Died>,
    victims: Query<&GlobalTransform, (With<Team>, Without<Hostage>)>,
    mut play: MessageWriter<PlaySound>,
) {
    for d in died.read() {
        let Ok(at) = victims.get(d.entity) else { continue };
        play.write(PlaySound {
            source: Some(d.entity),
            ..PlaySound::at(death_sound(d.damage.kind, d.damage.hitgroup), at.translation())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEVLAR: Armor = Armor {
        amount: 1.0,
        helmet: false,
    };
    const HELMET: Armor = Armor {
        amount: 1.0,
        helmet: true,
    };

    #[test]
    fn bullets_by_armour_and_hitgroup() {
        let b = DamageKind::Bullet;
        assert_eq!(hurt_sound(b, Hitgroup::Chest, Some(KEVLAR), false), Some(DAMAGE_KEVLAR));
        assert_eq!(
            hurt_sound(b, Hitgroup::LeftArm, Some(KEVLAR), true),
            Some(DAMAGE_KEVLAR)
        );
        assert_eq!(hurt_sound(b, Hitgroup::Chest, None, false), None);
        // Legs aren't covered.
        assert_eq!(hurt_sound(b, Hitgroup::RightLeg, Some(HELMET), false), None);
        // Spent armour covers nothing.
        let spent = Armor {
            amount: 0.0,
            helmet: true,
        };
        assert_eq!(hurt_sound(b, Hitgroup::Stomach, Some(spent), false), None);
        assert_eq!(hurt_sound(b, Hitgroup::Head, Some(HELMET), false), Some(DAMAGE_HELMET));
        assert_eq!(hurt_sound(b, Hitgroup::Head, Some(HELMET), true), Some(DAMAGE_HELMET));
        assert_eq!(
            hurt_sound(b, Hitgroup::Head, Some(KEVLAR), false),
            Some(DAMAGE_HEADSHOT)
        );
        assert_eq!(hurt_sound(b, Hitgroup::Head, None, true), None);
    }

    #[test]
    fn other_kinds() {
        assert_eq!(
            hurt_sound(DamageKind::Fall, Hitgroup::Generic, Some(HELMET), false),
            Some(FALL_DAMAGE)
        );
        assert_eq!(
            hurt_sound(DamageKind::Melee, Hitgroup::Chest, Some(HELMET), false),
            None
        );
        assert_eq!(
            hurt_sound(DamageKind::Blast, Hitgroup::Generic, Some(HELMET), false),
            None
        );
        assert_eq!(hurt_sound(DamageKind::Burn, Hitgroup::Generic, None, false), None);
        assert_eq!(death_sound(DamageKind::Bullet, Hitgroup::Head), DEATH_HEADSHOT);
        assert_eq!(death_sound(DamageKind::Melee, Hitgroup::Head), DEATH);
        assert_eq!(death_sound(DamageKind::Fall, Hitgroup::Generic), DEATH);
    }
}
