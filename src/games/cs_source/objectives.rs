//! CS:S's bomb and hostages (specs/cs_source/objectives.md): the C4 as a
//! weapon, its and the kit's models, the sound entries, the defusal kit's
//! price, the hostage models. The rules themselves are game-neutral
//! (`crate::objectives`); this fills in CS:S's data.

use bevy::prelude::*;

use crate::{
    core::Team,
    objectives::{
        bomb::{BombRules, BombSounds, C4 as C4Part},
        hostages::{HostageRules, HostageSounds},
    },
    weapon::{RegisterWeapons, Weapon, drop::PickupTeam},
};

const UNIT: f32 = 0.0254;

pub const C4: &str = "cs_source:weapon_c4";
/// Held-model keys of the planted bomb and a dropped defusal kit.
pub const C4_PLANTED: &str = "cs_source:c4_planted";
pub const DEFUSER: &str = "cs_source:item_defuser";

/// World models (held, lying, planted).
pub const WORLD_MODELS: &[(&str, &str)] = &[
    (C4, "models/weapons/w_c4.mdl"),
    (C4_PLANTED, "models/weapons/w_c4_planted.mdl"),
    (DEFUSER, "models/weapons/w_defuser.mdl"),
];

/// The C4's view model (spec Constants: `v_c4`, press-button 2.7667 s).
pub const VIEW_MODELS: &[(&str, &str, bool)] = &[(C4, "models/weapons/v_c4.mdl", false)];

/// The only hostage models CS:S ships (spec Q9).
pub const HOSTAGE_MODELS: &[&str] = &[
    "models/characters/hostage_01.mdl",
    "models/characters/hostage_02.mdl",
    "models/characters/hostage_03.mdl",
    "models/characters/hostage_04.mdl",
];

/// Sound entries (spec Sounds), for precaching with the map's sounds.
pub const SOUNDS: &[&str] = &[
    "c4.click",
    "c4.plant",
    "C4.PlantSound",
    "c4.disarmstart",
    "c4.disarmfinish",
    "c4.explode",
    "defuser.equip",
    "Event.BombPlanted",
    "Event.BombDefused",
    "Event.HostageTouched",
    "Event.HostageRescued",
    "Event.HostageKilled",
    "Hostage.Pain",
    "Hostage.StartFollowCT",
    "Hostage.StopFollowCT",
];

/// The defusal kit's price (localization "DEFUSAL KIT: $200"; CTs only).
pub const DEFUSER_PRICE: u32 = 200;

pub struct CsObjectivesPlugin;

impl Plugin for CsObjectivesPlugin {
    fn build(&self, app: &mut App) {
        app.register_weapon(C4, c4);
        let s = |x: &str| Some(x.to_string());
        let mut bomb = app.world_mut().get_resource_or_init::<BombRules>();
        bomb.weapon = Some(C4);
        bomb.sounds = BombSounds {
            click: s("c4.click"),
            plant: s("c4.plant"),
            beep: s("C4.PlantSound"),
            defuse_start: s("c4.disarmstart"),
            defuse_finish: s("c4.disarmfinish"),
            explode: s("c4.explode"),
            kit: s("defuser.equip"),
            announce_planted: s("Event.BombPlanted"),
            announce_defused: s("Event.BombDefused"),
        };
        bomb.planted_model = s(C4_PLANTED);
        bomb.kit_model = s(DEFUSER);
        // w_c4_planted stands on end in the held-model frame: lay it down,
        // keypad up.
        bomb.planted_turn =
            Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let mut hostages = app.world_mut().get_resource_or_init::<HostageRules>();
        hostages.models = HOSTAGE_MODELS.iter().map(|m| m.to_string()).collect();
        hostages.sounds = HostageSounds {
            pain: s("Hostage.Pain"),
            start_follow: s("Hostage.StartFollowCT"),
            stop_follow: s("Hostage.StopFollowCT"),
            announce_touched: s("Event.HostageTouched"),
            announce_rescued: s("Event.HostageRescued"),
            announce_killed: s("Event.HostageKilled"),
        };
    }
}

/// weapon_c4: slot 5 (key 5), drawn in 1 s, the knife's 250 speed;
/// terrorists only pick it up.
fn c4(e: &mut EntityWorldMut) {
    e.insert((
        Weapon {
            id: C4,
            slot: 4,
            owner: None,
            draw_time: 1.0,
            max_speed: Some(250.0 * UNIT),
        },
        C4Part,
        PickupTeam(Team(1)),
    ));
}
