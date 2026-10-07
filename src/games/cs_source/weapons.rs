//! CS:S weapons built from the shared weapon parts, with values from the
//! CS:S weapon scripts and view models (specs/cs_source/weapons.md, "Weapon
//! data") and the rules measured on the game ("CS:S values (measured)").
//! CS:S's feel that doesn't decompose into parts lives here as components:
//! `Inaccuracy` (the accuracy penalty) and `Recoil` (view punch per shot).
//! Bullet penetration is the shared `Penetration` part with CS:S's numbers
//! (`PASS_MATERIALS`, measured M13).
//!
//! Guns are rows of script values (`Gun`, one const per weapon; each field
//! names its script key) built by `gun`. The scripts are encrypted; the
//! values are the spec's tables, which were read from the user's install.
//! Sound entries and model paths are the install's own names (its sound
//! scripts and model files), checked by `tests/map_de_dust2.rs`.

use bevy::prelude::*;

use crate::{
    core::{Intent, MovementState, SimSet, Velocity},
    weapon::{
        AltModes, Burst, CharacterPass, DamageEffect, FireTiming, HitgroupScale, Hitscan, Inventory, Magazine, Melee,
        PassMaterial, PassMaterials, Penetration, RegisterWeapons, ShellReload, SpreadShape, StartingWeapons, Swing,
        Trigger, ViewPunch, Weapon, WeaponEvent, WeaponEventKind, WeaponFrame, WeaponSounds, Zoom,
    },
};

const UNIT: f32 = 0.0254;
/// Script damage is in hit points; ours is normalized (100 hp = 1.0).
pub(super) const HP: f32 = 0.01;

pub const KNIFE: &str = "cs_source:weapon_knife";
pub const AK47: &str = "cs_source:weapon_ak47";
pub const M4A1: &str = "cs_source:weapon_m4a1";
pub const AWP: &str = "cs_source:weapon_awp";
pub const USP: &str = "cs_source:weapon_usp";
pub const GLOCK: &str = "cs_source:weapon_glock";
pub const DEAGLE: &str = "cs_source:weapon_deagle";
pub const FAMAS: &str = "cs_source:weapon_famas";
pub const GALIL: &str = "cs_source:weapon_galil";
pub const AUG: &str = "cs_source:weapon_aug";
pub const SG552: &str = "cs_source:weapon_sg552";
pub const SCOUT: &str = "cs_source:weapon_scout";
pub const SG550: &str = "cs_source:weapon_sg550";
pub const G3SG1: &str = "cs_source:weapon_g3sg1";
pub const MAC10: &str = "cs_source:weapon_mac10";
pub const TMP: &str = "cs_source:weapon_tmp";
pub const MP5NAVY: &str = "cs_source:weapon_mp5navy";
pub const UMP45: &str = "cs_source:weapon_ump45";
pub const P90: &str = "cs_source:weapon_p90";
pub const M249: &str = "cs_source:weapon_m249";
pub const P228: &str = "cs_source:weapon_p228";
pub const FIVESEVEN: &str = "cs_source:weapon_fiveseven";
pub const ELITE: &str = "cs_source:weapon_elite";
pub const M3: &str = "cs_source:weapon_m3";
pub const XM1014: &str = "cs_source:weapon_xm1014";

/// World models (the script's `playermodel`), held by characters.
pub const WORLD_MODELS: &[(&str, &str)] = &[
    (KNIFE, "models/weapons/w_knife_ct.mdl"),
    (AK47, "models/weapons/w_rif_ak47.mdl"),
    (M4A1, "models/weapons/w_rif_m4a1.mdl"),
    (AWP, "models/weapons/w_snip_awp.mdl"),
    (USP, "models/weapons/w_pist_usp.mdl"),
    (GLOCK, "models/weapons/w_pist_glock18.mdl"),
    (DEAGLE, "models/weapons/w_pist_deagle.mdl"),
    (FAMAS, "models/weapons/w_rif_famas.mdl"),
    (GALIL, "models/weapons/w_rif_galil.mdl"),
    (AUG, "models/weapons/w_rif_aug.mdl"),
    (SG552, "models/weapons/w_rif_sg552.mdl"),
    (SCOUT, "models/weapons/w_snip_scout.mdl"),
    (SG550, "models/weapons/w_snip_sg550.mdl"),
    (G3SG1, "models/weapons/w_snip_g3sg1.mdl"),
    (MAC10, "models/weapons/w_smg_mac10.mdl"),
    (TMP, "models/weapons/w_smg_tmp.mdl"),
    (MP5NAVY, "models/weapons/w_smg_mp5.mdl"),
    (UMP45, "models/weapons/w_smg_ump45.mdl"),
    (P90, "models/weapons/w_smg_p90.mdl"),
    (M249, "models/weapons/w_mach_m249para.mdl"),
    (P228, "models/weapons/w_pist_p228.mdl"),
    (FIVESEVEN, "models/weapons/w_pist_fiveseven.mdl"),
    (ELITE, "models/weapons/w_pist_elite.mdl"),
    (M3, "models/weapons/w_shot_m3super90.mdl"),
    (XM1014, "models/weapons/w_shot_xm1014.mdl"),
];

/// World models with the silencer on (the scripts' `SilencerModel`), held
/// under `silenced_key(id)` while the silencer is on.
pub const SILENCED_WORLD_MODELS: &[(&str, &str)] = &[
    (M4A1, "models/weapons/w_rif_m4a1_silencer.mdl"),
    (USP, "models/weapons/w_pist_usp_silencer.mdl"),
];

/// The held-model key of a weapon with its silencer on.
pub fn silenced_key(id: &str) -> String {
    format!("{id}#silenced")
}

/// View models (the script's `viewmodel`), seen by the local player, and
/// whether each is built right-handed (the script's `BuiltRightHanded`;
/// spec view_models.md 2). CS:S ships one knife view model for both teams.
/// The scripts are encrypted, so handedness comes from the models: the AK
/// is held left of the eye (left-handed), the knife right of it
/// (`tests/map_de_dust2.rs::view_model_handedness_in_the_files`). With the
/// default `cl_righthand 1` the left-handed ones are mirrored, so all end
/// up in the right hand, as CS:S shows them. Built right-handed (muzzle
/// right of the eye): the knife, FAMAS, Galil, M249 and the Elites (whose
/// first muzzle is the right pistol's).
pub const VIEW_MODELS: &[(&str, &str, bool)] = &[
    (KNIFE, "models/weapons/v_knife_t.mdl", true),
    (AK47, "models/weapons/v_rif_ak47.mdl", false),
    (M4A1, "models/weapons/v_rif_m4a1.mdl", false),
    (AWP, "models/weapons/v_snip_awp.mdl", false),
    (USP, "models/weapons/v_pist_usp.mdl", false),
    (GLOCK, "models/weapons/v_pist_glock18.mdl", false),
    (DEAGLE, "models/weapons/v_pist_deagle.mdl", false),
    (FAMAS, "models/weapons/v_rif_famas.mdl", true),
    (GALIL, "models/weapons/v_rif_galil.mdl", true),
    (AUG, "models/weapons/v_rif_aug.mdl", false),
    (SG552, "models/weapons/v_rif_sg552.mdl", false),
    (SCOUT, "models/weapons/v_snip_scout.mdl", false),
    (SG550, "models/weapons/v_snip_sg550.mdl", false),
    (G3SG1, "models/weapons/v_snip_g3sg1.mdl", false),
    (MAC10, "models/weapons/v_smg_mac10.mdl", false),
    (TMP, "models/weapons/v_smg_tmp.mdl", false),
    (MP5NAVY, "models/weapons/v_smg_mp5.mdl", false),
    (UMP45, "models/weapons/v_smg_ump45.mdl", false),
    (P90, "models/weapons/v_smg_p90.mdl", false),
    (M249, "models/weapons/v_mach_m249para.mdl", true),
    (P228, "models/weapons/v_pist_p228.mdl", false),
    (FIVESEVEN, "models/weapons/v_pist_fiveseven.mdl", false),
    (ELITE, "models/weapons/v_pist_elite.mdl", true),
    (M3, "models/weapons/v_shot_m3super90.mdl", false),
    (XM1014, "models/weapons/v_shot_xm1014.mdl", false),
];

/// Sound entries the weapons use, for precaching with the map's sounds:
/// `SOUNDS` and every gun's.
pub fn sounds() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = SOUNDS.to_vec();
    out.extend(super::grenades::SOUNDS);
    out.extend(super::objectives::SOUNDS);
    for g in GUNS {
        for s in [g.fire, g.empty]
            .into_iter()
            .chain(g.fire_alt)
            .chain(g.model_sounds.entries())
        {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Sound entries other than the guns' own.
pub const SOUNDS: &[&str] = &[
    "Weapon_Knife.Deploy",
    "Weapon_Knife.Hit",
    "Weapon_Knife.HitWall",
    "Weapon_Knife.Slash",
    "Weapon_Knife.Stab",
    "Default.Zoom",
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
            .register_weapon(AK47, |e| gun(e, &AK47_GUN))
            .register_weapon(M4A1, |e| gun(e, &M4A1_GUN))
            .register_weapon(AWP, |e| gun(e, &AWP_GUN))
            .register_weapon(USP, |e| gun(e, &USP_GUN))
            .register_weapon(GLOCK, |e| gun(e, &GLOCK_GUN))
            .register_weapon(DEAGLE, |e| gun(e, &DEAGLE_GUN))
            .register_weapon(FAMAS, |e| gun(e, &FAMAS_GUN))
            .register_weapon(GALIL, |e| gun(e, &GALIL_GUN))
            .register_weapon(AUG, |e| gun(e, &AUG_GUN))
            .register_weapon(SG552, |e| gun(e, &SG552_GUN))
            .register_weapon(SCOUT, |e| gun(e, &SCOUT_GUN))
            .register_weapon(SG550, |e| gun(e, &SG550_GUN))
            .register_weapon(G3SG1, |e| gun(e, &G3SG1_GUN))
            .register_weapon(MAC10, |e| gun(e, &MAC10_GUN))
            .register_weapon(TMP, |e| gun(e, &TMP_GUN))
            .register_weapon(MP5NAVY, |e| gun(e, &MP5NAVY_GUN))
            .register_weapon(UMP45, |e| gun(e, &UMP45_GUN))
            .register_weapon(P90, |e| gun(e, &P90_GUN))
            .register_weapon(M249, |e| gun(e, &M249_GUN))
            .register_weapon(P228, |e| gun(e, &P228_GUN))
            .register_weapon(FIVESEVEN, |e| gun(e, &FIVESEVEN_GUN))
            .register_weapon(ELITE, |e| gun(e, &ELITE_GUN))
            .register_weapon(M3, |e| gun(e, &M3_GUN))
            .register_weapon(XM1014, |e| gun(e, &XM1014_GUN))
            .insert_resource(prices())
            .add_message::<WeaponEvent>()
            .add_systems(
                FixedUpdate,
                (before_shots.before(WeaponFrame), after_shots.after(WeaponFrame)).in_set(SimSet::Weapons),
            )
            .add_plugins((
                super::impacts::ImpactSoundsPlugin,
                super::impact_effects::ImpactEffectsPlugin,
                super::grenades::GrenadesPlugin,
                super::fire::FirePlugin,
                super::objectives::CsObjectivesPlugin,
            ))
            .insert_resource(pass_materials());
        let mut start = app.world_mut().get_resource_or_init::<StartingWeapons>();
        if start.is_empty() {
            // CS:S's spawn kit: the knife and the team's pistol (Terrorists,
            // team 1, the Glock; everyone else the USP). Deathmatch adds the
            // AK-47 for all; the best weapon is drawn: given last.
            *start = StartingWeapons {
                team: vec![(Some(1), vec![KNIFE, GLOCK]), (None, vec![KNIFE, USP])],
                all: vec![AK47],
            };
        }
    }
}

/// Prices (spec weapons.md, "Economy": the scripts' `WeaponPrice`; armour
/// from the game's buy menu: kevlar 650, kevlar and helmet 1000, the
/// helmet alone 350).
fn prices() -> crate::weapon::economy::Prices {
    let mut p = crate::weapon::economy::Prices {
        vest: 650,
        vest_helmet: 1000,
        helmet: 350,
        ..default()
    };
    for (id, price, team) in BUY {
        p.weapons.insert(id, *price);
        if let Some(team) = team {
            p.team_only.insert(id, *team);
        }
    }
    for (id, price, _) in super::grenades::GRENADES {
        p.weapons.insert(id, *price);
    }
    // The defusal kit, counter-terrorists only (spec 5, "Kit").
    p.defuser = super::objectives::DEFUSER_PRICE;
    p.team_only.insert("defuser", 2);
    p.menu = menu();
    // Ammo by the box per ammo type; the spawn pistols carry two more
    // clips (Glock 40, USP 24: UNMEASURED, the well-known values).
    for g in GUNS {
        p.ammo.insert(
            g.id,
            crate::weapon::economy::AmmoBox {
                price: g.ammo.box_price,
                rounds: g.ammo.box_rounds,
            },
        );
    }
    p.starting_reserve.insert(GLOCK, 40);
    p.starting_reserve.insert(USP, 24);
    p.ammo_sound = Some(AMMO_SOUND.into());
    // Computer players mostly buy the team rifles (ours; CS:S's bot
    // profiles aren't in the spec).
    for (id, weight) in BOT_WEIGHTS {
        p.bot_weights.insert(id, *weight);
    }
    p
}

/// Every gun's `WeaponPrice` and `Team` (spec weapons.md, "Economy": our
/// team 1 terrorists, 2 CTs; None: anyone).
pub const BUY: &[(&str, u32, Option<u8>)] = &[
    (GLOCK, 400, None),
    (USP, 500, None),
    (P228, 600, None),
    (DEAGLE, 650, None),
    (ELITE, 800, Some(1)),
    (FIVESEVEN, 750, Some(2)),
    (M3, 1700, None),
    (XM1014, 3000, None),
    (MAC10, 1400, Some(1)),
    (TMP, 1250, Some(2)),
    (MP5NAVY, 1500, None),
    (UMP45, 1700, None),
    (P90, 2350, None),
    (GALIL, 2000, Some(1)),
    (FAMAS, 2250, Some(2)),
    (AK47, 2500, Some(1)),
    (M4A1, 3100, Some(2)),
    (SCOUT, 2750, None),
    (SG552, 3500, Some(1)),
    (AUG, 3500, Some(2)),
    (AWP, 4750, None),
    (G3SG1, 5000, Some(1)),
    (SG550, 4200, Some(2)),
    (M249, 5750, None),
];

/// How much bots like each primary when buying (`Prices::bot_weights`).
const BOT_WEIGHTS: &[(&str, f32)] = &[
    (AK47, 8.0),
    (M4A1, 8.0),
    (GALIL, 3.0),
    (FAMAS, 3.0),
    (SG552, 3.0),
    (AUG, 3.0),
    (AWP, 2.0),
    (MP5NAVY, 2.0),
    (P90, 2.0),
    (UMP45, 1.0),
    (MAC10, 1.0),
    (TMP, 1.0),
    (M3, 1.0),
    (XM1014, 1.0),
    (SCOUT, 1.0),
    (G3SG1, 0.5),
    (SG550, 0.5),
    (M249, 0.5),
];

/// The sound of buying ammo (Source's generic ammo pickup entry).
pub const AMMO_SOUND: &str = "BaseCombatCharacter.AmmoPickup";

/// CS:S's buy menu: its categories on their number keys (6 and 7 buy
/// primary and secondary ammo at once) and items in its order; each team
/// sees its own.
fn menu() -> Vec<crate::weapon::economy::BuyCategory> {
    use crate::weapon::economy::{BuyCategory, BuyItem};
    let category = |key: u8, name: &'static str, items: &[(&'static str, &'static str)]| BuyCategory {
        key,
        name,
        items: items.iter().map(|(buy, label)| BuyItem { buy, label }).collect(),
        direct: None,
    };
    let direct = |key: u8, name: &'static str, buy: &'static str| BuyCategory {
        direct: Some(buy),
        ..category(key, name, &[])
    };
    use super::grenades::{FLASHBANG, HEGRENADE, SMOKEGRENADE};
    vec![
        category(
            1,
            "Pistols",
            &[
                (GLOCK, "Glock"),
                (USP, "USP"),
                (P228, "P228"),
                (DEAGLE, "Desert Eagle"),
                (ELITE, "Dual Elites"),
                (FIVESEVEN, "Five-SeveN"),
            ],
        ),
        category(2, "Shotguns", &[(M3, "M3"), (XM1014, "XM1014")]),
        category(
            3,
            "Sub-Machine Guns",
            &[
                (MAC10, "MAC-10"),
                (TMP, "TMP"),
                (MP5NAVY, "MP5"),
                (UMP45, "UMP45"),
                (P90, "P90"),
            ],
        ),
        category(
            4,
            "Rifles",
            &[
                (GALIL, "Galil"),
                (FAMAS, "FAMAS"),
                (AK47, "AK-47"),
                (M4A1, "M4A1"),
                (SCOUT, "Scout"),
                (SG552, "SG 552"),
                (AUG, "AUG"),
                (AWP, "AWP"),
                (G3SG1, "G3SG1"),
                (SG550, "SG 550"),
            ],
        ),
        category(5, "Machine Guns", &[(M249, "M249")]),
        direct(6, "Primary Ammo", "primammo"),
        direct(7, "Secondary Ammo", "secammo"),
        category(
            8,
            "Equipment",
            &[
                ("vest", "Kevlar"),
                ("vesthelm", "Kevlar + Helmet"),
                (FLASHBANG, "Flashbang"),
                (HEGRENADE, "HE Grenade"),
                (SMOKEGRENADE, "Smoke Grenade"),
                ("defuser", "Defusal Kit"),
            ],
        ),
    ]
}

fn knife(e: &mut EntityWorldMut) {
    e.insert(crate::weapon::drop::Undroppable);
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

/// An ammo type: penetration power and reach (measured M13), most rounds
/// carried (`ammo_<type>_max`, M17), and the box it's bought in
/// (UNMEASURED: the spec has no ammo prices; these are the well-known
/// Counter-Strike values, docs/tech-debt.md).
#[derive(Clone, Copy, Debug)]
pub struct Ammo {
    /// Units of a scale-1 material.
    pub power: f32,
    /// Objects this far (units) or farther aren't passed.
    pub max_distance: f32,
    pub max: u32,
    /// A box's price and rounds.
    pub box_price: u32,
    pub box_rounds: u32,
}

/// 762MM: power 39 (M13); it still passes at 4000 units, no farther limit
/// was found.
pub const AMMO_762MM: Ammo = Ammo {
    power: 39.0,
    max_distance: f32::INFINITY,
    max: 90,
    box_price: 80,
    box_rounds: 30,
};
/// 338MAG: power 45, still passes at 4000 (M13).
pub const AMMO_338MAG: Ammo = Ammo {
    power: 45.0,
    max_distance: f32::INFINITY,
    max: 30,
    box_price: 125,
    box_rounds: 10,
};
/// 50AE: power 30, passes a player at 1006 units, not at 1015 (M13).
pub const AMMO_50AE: Ammo = Ammo {
    power: 30.0,
    max_distance: 1010.0,
    max: 35,
    box_price: 40,
    box_rounds: 7,
};
/// 45ACP: power 15 (M13). UNMEASURED distance: the community value 500
/// (the USP passed doors at 100).
pub const AMMO_45ACP: Ammo = Ammo {
    power: 15.0,
    max_distance: 500.0,
    max: 100,
    box_price: 25,
    box_rounds: 12,
};
/// 556MM: UNMEASURED power and distance: the community values 35 / 4000
/// (the same table's 45ACP, 50AE, 762MM and 338MAG powers match M13).
pub const AMMO_556MM: Ammo = Ammo {
    power: 35.0,
    max_distance: 4000.0,
    max: 90,
    box_price: 60,
    box_rounds: 30,
};
/// 9MM: UNMEASURED power and distance: the community values 21 / 800.
pub const AMMO_9MM: Ammo = Ammo {
    power: 21.0,
    max_distance: 800.0,
    max: 120,
    box_price: 20,
    box_rounds: 30,
};

/// 357SIG: UNMEASURED power and distance: 45ACP's (the measured pistol
/// round).
pub const AMMO_357SIG: Ammo = Ammo {
    max: 52,
    box_price: 50,
    box_rounds: 13,
    ..AMMO_45ACP
};
/// 57MM: UNMEASURED power and distance: 45ACP's.
pub const AMMO_57MM: Ammo = Ammo {
    max: 100,
    box_price: 50,
    box_rounds: 50,
    ..AMMO_45ACP
};
/// BUCKSHOT: UNMEASURED power and distance (per pellet): 45ACP's.
pub const AMMO_BUCKSHOT: Ammo = Ammo {
    max: 32,
    box_price: 65,
    box_rounds: 8,
    ..AMMO_45ACP
};
/// 556MM_BOX (the M249's): UNMEASURED power and distance: 556MM's.
pub const AMMO_556MM_BOX: Ammo = Ammo {
    max: 200,
    box_price: 60,
    box_rounds: 30,
    ..AMMO_556MM
};

/// Which hand of the dual Elites fires the shot that leaves `clip` rounds:
/// they alternate (spec weapons.md, view-model table), the right one on
/// even counts (ours: the full 30 fires left first).
pub fn elite_right_hand(clip: u32) -> bool {
    clip.is_multiple_of(2)
}

/// The script's accuracy keys for one mode (`Spread`, `Inaccuracy*`; the
/// alternate mode's are the `*Alt` keys), tangent units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccuracyKeys {
    pub spread: f32,
    pub crouch: f32,
    pub stand: f32,
    pub jump: f32,
    pub land: f32,
    pub fire: f32,
    pub movement: f32,
}

impl AccuracyKeys {
    /// In the spec table's column order: Spread, InaccuracyCrouch,
    /// InaccuracyStand, InaccuracyJump, InaccuracyLand, InaccuracyFire,
    /// InaccuracyMove (InaccuracyLadder is not used yet).
    pub const fn new(v: [f32; 7]) -> Self {
        Self {
            spread: v[0],
            crouch: v[1],
            stand: v[2],
            jump: v[3],
            land: v[4],
            fire: v[5],
            movement: v[6],
        }
    }
}

/// What attack2 does on a gun.
#[derive(Clone, Copy, Debug)]
pub enum Alt {
    None,
    /// Zoom levels (FOV per level) and the zoomed max speed (M15; None:
    /// unchanged); `unzoom`: a shot unzooms until the next one may fire
    /// (AWP, scout); `overlay`: a sniper scope (no view model).
    Scope {
        fov: &'static [f32],
        speed: Option<f32>,
        unzoom: bool,
        overlay: bool,
    },
    /// Screw the silencer on or off; both attacks wait `time` (M16).
    Silencer {
        time: f32,
    },
    /// Toggle 3-round bursts, rounds `interval` apart (M16).
    Burst {
        interval: f32,
        refire: f32,
    },
}

/// One CS:S gun: its script values (key names in the comments), its
/// view-model durations (spec table "View-model sequence durations") and
/// the measured recoil.
pub struct Gun {
    pub id: &'static str,
    /// `bucket`.
    pub slot: u8,
    /// The view model's draw duration, s.
    pub draw: f32,
    /// `MaxPlayerSpeed`, units/s.
    pub max_speed: f32,
    /// `FullAuto`.
    pub automatic: bool,
    /// `CycleTime`, s.
    pub cycle: f32,
    /// `clip_size`.
    pub clip: u32,
    /// `primary_ammo`.
    pub ammo: Ammo,
    /// The view model's reload duration, s (a shotgun's: one shell's).
    pub reload: f32,
    /// Shell by shell (shotguns): the reload's start and each shell, s.
    pub shells: Option<(f32, f32)>,
    /// `TimeToIdle`, s (0: none).
    pub idle: f32,
    /// `Damage`, hit points.
    pub damage: f32,
    /// `Range`, units.
    pub range: f32,
    /// `RangeModifier` (per 500 units, M5).
    pub range_modifier: f32,
    /// `Penetration`: objects passed.
    pub penetration: u32,
    /// `Bullets`: pellets per shot.
    pub pellets: u32,
    /// `WeaponArmorRatio`.
    pub armor_ratio: f32,
    pub accuracy: AccuracyKeys,
    /// The `*Alt` keys, used in the alternate mode.
    pub accuracy_alt: Option<AccuracyKeys>,
    /// `RecoveryTimeCrouch`, `RecoveryTimeStand`, s.
    pub recovery: (f32, f32),
    /// Standing, crouched, moving, airborne kick sets (M3); None: no punch.
    pub recoil: Option<[Kick; 4]>,
    pub alt: Alt,
    /// `SoundData`: `single_shot`, the alternate mode's, `empty`.
    pub fire: &'static str,
    pub fire_alt: Option<&'static str>,
    pub empty: &'static str,
    /// Sounds the view model's sequences play.
    pub model_sounds: ModelSounds,
}

/// Sounds a view model's sequences play, in seconds from the sequence's
/// start: its animation events (event 5004, at cycle x duration), read
/// from the install's model (`dump cs_source --sequences`).
pub struct ModelSounds {
    /// From the reload's start (a shotgun's: from each shell's insert).
    pub reload: &'static [(f32, &'static str)],
    pub draw: &'static [(f32, &'static str)],
    /// The silencer going on (mode 1) and off (mode 0).
    pub modes: &'static [(u8, f32, &'static str)],
    /// From each shot (the fire sequence's, e.g. the scout's bolt).
    pub fire: &'static [(f32, &'static str)],
    /// From the end of a shell-by-shell reload (its finish sequence).
    pub finish: &'static [(f32, &'static str)],
}

impl ModelSounds {
    const NONE: Self = Self {
        reload: &[],
        draw: &[],
        modes: &[],
        fire: &[],
        finish: &[],
    };

    fn entries(&self) -> impl Iterator<Item = &'static str> {
        let timed = [self.reload, self.draw, self.fire, self.finish];
        timed
            .into_iter()
            .flatten()
            .map(|(_, s)| *s)
            .chain(self.modes.iter().map(|(_, _, s)| *s))
    }
}

/// No cap on the punch was seen for the semi-automatics (UNMEASURED).
const NO_CAP: f32 = 90.0;

/// Semi-automatics (USP, Deagle, AWP unscoped; M3): every shot kicks
/// straight up 2 degrees. Moving, airborne, silenced and scoped shots were
/// not measured (UNMEASURED: the same).
const SEMI_AUTO_KICK: [Kick; 4] = [Kick::new((2.0, 0.0), (0.0, 0.0), (NO_CAP, NO_CAP)); 4];

/// The AK-47's standing, crouched, moving and airborne sets (M3).
const AK47_KICK: [Kick; 4] = [
    Kick::new((1.0, 0.175), (0.375, 0.0375), (5.75, 1.75)),
    // Caps not reached when measured: the standing ones.
    Kick::new((0.9, 0.15), (0.35, 0.025), (5.75, 1.75)),
    Kick::new((1.5, 0.225), (0.45, 0.05), (6.5, 2.5)),
    Kick::new((2.0, 0.5), (1.0, 0.35), (9.0, 6.0)),
];

/// The M4A1's standing and crouched sets (M3); UNMEASURED: moving and
/// airborne as standing.
const M4A1_KICK: [Kick; 4] = [
    Kick::new((0.65, 0.25), (0.35, 0.015), (3.5, 2.25)),
    Kick::new((0.6, 0.2), (0.3, 0.0125), (3.25, 2.0)),
    Kick::new((0.65, 0.25), (0.35, 0.015), (3.5, 2.25)),
    Kick::new((0.65, 0.25), (0.35, 0.015), (3.5, 2.25)),
];

/// Shotguns: UNMEASURED; the SDK template's shotgun (spec weapons.md,
/// constants): a whole 4-6 degrees up on the ground, 8-11 in the air, no
/// sideways part.
const SHOTGUN_KICK: [Kick; 4] = [
    Kick::random_up(4.0, 6.0),
    Kick::random_up(4.0, 6.0),
    Kick::random_up(4.0, 6.0),
    Kick::random_up(8.0, 11.0),
];

pub const AK47_GUN: Gun = Gun {
    id: AK47,
    slot: 0,
    draw: 1.0,
    max_speed: 221.0,
    automatic: true,
    cycle: 0.1,
    clip: 30,
    ammo: AMMO_762MM,
    reload: 2.4324,
    shells: None,
    idle: 1.9,
    damage: 36.0,
    range: 8192.0,
    range_modifier: 0.98,
    penetration: 2,
    pellets: 1,
    armor_ratio: 1.55,
    accuracy: AccuracyKeys::new([0.0006, 0.00687, 0.00916, 0.43044, 0.08609, 0.01158, 0.09222]),
    accuracy_alt: None,
    recovery: (0.34868, 0.48815),
    recoil: Some(AK47_KICK),
    alt: Alt::None,
    fire: "Weapon_AK47.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: ModelSounds {
        reload: &[(0.35, "Weapon_AK47.Clipout"), (1.54, "Weapon_AK47.Clipin")],
        draw: &[(0.3667, "Weapon_AK47.BoltPull")],
        modes: &[],
        fire: &[],
        finish: &[],
    },
};

pub const M4A1_GUN: Gun = Gun {
    id: M4A1,
    slot: 0,
    draw: 0.975,
    max_speed: 230.0,
    automatic: true,
    cycle: 0.09,
    clip: 30,
    ammo: AMMO_556MM,
    reload: 3.0541,
    shells: None,
    idle: 1.5,
    damage: 33.0,
    range: 8192.0,
    range_modifier: 0.97,
    penetration: 2,
    pellets: 1,
    armor_ratio: 1.4,
    accuracy: AccuracyKeys::new([0.0006, 0.00525, 0.007, 0.34151, 0.0683, 0.01266, 0.06872]),
    accuracy_alt: Some(AccuracyKeys::new([
        0.00054, 0.00525, 0.007, 0.34846, 0.06969, 0.01165, 0.07039,
    ])),
    recovery: (0.26973, 0.37762),
    recoil: Some(M4A1_KICK),
    alt: Alt::Silencer { time: 2.0 },
    fire: "Weapon_M4A1.Single",
    fire_alt: Some("Weapon_M4A1.Silenced"),
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: M4A1_SOUNDS,
};

pub const AWP_GUN: Gun = Gun {
    id: AWP,
    slot: 0,
    draw: 1.0,
    max_speed: 210.0,
    automatic: false,
    cycle: 1.5,
    clip: 10,
    ammo: AMMO_338MAG,
    reload: 3.6667,
    shells: None,
    idle: 2.0,
    damage: 115.0,
    range: 8192.0,
    range_modifier: 0.99,
    penetration: 3,
    pellets: 1,
    armor_ratio: 1.95,
    accuracy: AccuracyKeys::new([0.0002, 0.0606, 0.0808, 0.546, 0.0546, 0.14, 0.273]),
    accuracy_alt: Some(AccuracyKeys::new([0.0002, 0.0015, 0.002, 0.546, 0.0546, 0.14, 0.273])),
    recovery: (0.24671, 0.34539),
    recoil: Some(SEMI_AUTO_KICK),
    alt: Alt::Scope {
        fov: &[40.0, 10.0],
        speed: Some(150.0),
        unzoom: true,
        overlay: true,
    },
    fire: "Weapon_AWP.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    // v_snip_awp.mdl is version 48, which `dump` doesn't read yet.
    model_sounds: ModelSounds::NONE,
};

pub const USP_GUN: Gun = Gun {
    id: USP,
    slot: 1,
    draw: 1.0,
    max_speed: 250.0,
    automatic: false,
    cycle: 0.15,
    clip: 12,
    ammo: AMMO_45ACP,
    reload: 2.6757,
    shells: None,
    idle: 0.0,
    damage: 34.0,
    range: 4096.0,
    range_modifier: 0.79,
    penetration: 1,
    pellets: 1,
    armor_ratio: 1.0,
    accuracy: AccuracyKeys::new([0.004, 0.006, 0.008, 0.28725, 0.05745, 0.03495, 0.01724]),
    accuracy_alt: Some(AccuracyKeys::new([
        0.003, 0.006, 0.008, 0.29625, 0.05925, 0.02504, 0.01778,
    ])),
    recovery: (0.23371, 0.28045),
    recoil: Some(SEMI_AUTO_KICK),
    alt: Alt::Silencer { time: 3.0 },
    fire: "Weapon_USP.Single",
    fire_alt: Some("Weapon_USP.SilencedShot"),
    empty: "Default.ClipEmpty_Pistol",
    model_sounds: USP_SOUNDS,
};

pub const GLOCK_GUN: Gun = Gun {
    id: GLOCK,
    slot: 1,
    draw: 1.0667,
    max_speed: 250.0,
    automatic: false,
    cycle: 0.15,
    clip: 20,
    ammo: AMMO_9MM,
    reload: 2.1429,
    shells: None,
    idle: 0.0,
    damage: 25.0,
    range: 4096.0,
    range_modifier: 0.75,
    penetration: 1,
    pellets: 1,
    armor_ratio: 1.05,
    accuracy: AccuracyKeys::new([0.004, 0.0075, 0.01, 0.2775, 0.0555, 0.03167, 0.01665]),
    accuracy_alt: Some(AccuracyKeys::new([
        0.004, 0.0075, 0.01, 0.2775, 0.0555, 0.02217, 0.01665,
    ])),
    recovery: (0.21875, 0.26249),
    // Measured M3: the Glock doesn't punch the view at all.
    recoil: None,
    // Measured M16: rounds on ticks 0, 4 and 8. UNMEASURED: the next pull
    // 0.5 s after the first round.
    alt: Alt::Burst {
        interval: 0.06,
        refire: 0.5,
    },
    fire: "Weapon_Glock.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Pistol",
    model_sounds: GLOCK_SOUNDS,
};

pub const DEAGLE_GUN: Gun = Gun {
    id: DEAGLE,
    slot: 1,
    draw: 1.0,
    max_speed: 250.0,
    automatic: false,
    cycle: 0.225,
    clip: 7,
    ammo: AMMO_50AE,
    reload: 2.1667,
    shells: None,
    idle: 0.0,
    damage: 54.0,
    range: 4096.0,
    range_modifier: 0.81,
    penetration: 2,
    pellets: 1,
    armor_ratio: 1.5,
    accuracy: AccuracyKeys::new([0.004, 0.00975, 0.013, 0.345, 0.069, 0.055, 0.0207]),
    accuracy_alt: None,
    recovery: (0.32236, 0.38683),
    recoil: Some(SEMI_AUTO_KICK),
    alt: Alt::None,
    fire: "Weapon_DEagle.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Pistol",
    model_sounds: DEAGLE_SOUNDS,
};

pub const FAMAS_GUN: Gun = Gun {
    id: FAMAS,
    slot: 0,
    draw: 1.0,
    max_speed: 220.0,
    automatic: true,
    cycle: 0.09,
    clip: 25,
    ammo: AMMO_556MM,
    reload: 3.3333,
    shells: None,
    idle: 1.1,
    damage: 30.0,
    range: 8192.0,
    range_modifier: 0.96,
    penetration: 2,
    pellets: 1,
    armor_ratio: 1.4,
    accuracy: AccuracyKeys::new([0.0006, 0.00412, 0.00549, 0.36527, 0.07305, 0.01186, 0.0698]),
    accuracy_alt: Some(AccuracyKeys::new([
        0.0006, 0.00412, 0.00549, 0.36527, 0.07305, 0.00593, 0.0698,
    ])),
    recovery: (0.30328, 0.4246),
    // UNMEASURED: the M4A1's sets (both 5.56 CT rifles at 0.09 s).
    recoil: Some(M4A1_KICK),
    // Measured M16: 3 rounds 5-6 ticks apart, held a new burst every 36-37 ticks.
    alt: Alt::Burst {
        interval: 0.08,
        refire: 0.55,
    },
    fire: "Weapon_FAMAS.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: FAMAS_SOUNDS,
};

pub const GALIL_GUN: Gun = Gun {
    id: GALIL,
    slot: 0,
    draw: 0.7812,
    max_speed: 215.0,
    automatic: true,
    cycle: 0.09,
    clip: 35,
    ammo: AMMO_556MM,
    reload: 2.9524,
    shells: None,
    idle: 1.28,
    damage: 30.0,
    range: 8192.0,
    range_modifier: 0.98,
    penetration: 2,
    pellets: 1,
    armor_ratio: 1.55,
    accuracy: AccuracyKeys::new([0.0006, 0.00939, 0.01253, 0.45434, 0.09087, 0.00984, 0.10561]),
    accuracy_alt: None,
    recovery: (0.35197, 0.49275),
    // UNMEASURED: the M4A1's sets (a 5.56 rifle at 0.09 s).
    recoil: Some(M4A1_KICK),
    alt: Alt::None,
    fire: "Weapon_Galil.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: GALIL_SOUNDS,
};

pub const AUG_GUN: Gun = Gun {
    id: AUG,
    slot: 0,
    draw: 1.0,
    max_speed: 221.0,
    automatic: true,
    cycle: 0.09,
    clip: 30,
    ammo: AMMO_762MM,
    reload: 3.7714,
    shells: None,
    idle: 1.9,
    damage: 32.0,
    range: 8192.0,
    range_modifier: 0.96,
    penetration: 2,
    pellets: 1,
    armor_ratio: 1.4,
    accuracy: AccuracyKeys::new([0.0006, 0.00412, 0.00549, 0.36936, 0.07387, 0.0109, 0.07268]),
    accuracy_alt: Some(AccuracyKeys::new([
        0.0006, 0.00288, 0.00385, 0.36936, 0.07387, 0.0109, 0.07268,
    ])),
    recovery: (0.30263, 0.42368),
    // UNMEASURED: the M4A1's sets (scoped too).
    recoil: Some(M4A1_KICK),
    // Measured M15: 55, off; stays zoomed after a shot; zoomed speed 221 (unchanged).
    alt: Alt::Scope {
        fov: &[55.0],
        speed: None,
        unzoom: false,
        overlay: false,
    },
    fire: "Weapon_AUG.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: AUG_SOUNDS,
};

pub const SG552_GUN: Gun = Gun {
    id: SG552,
    slot: 0,
    draw: 0.8108,
    max_speed: 235.0,
    automatic: true,
    cycle: 0.09,
    clip: 30,
    ammo: AMMO_556MM,
    reload: 2.7568,
    shells: None,
    idle: 2.0,
    damage: 33.0,
    range: 8192.0,
    range_modifier: 0.955,
    penetration: 2,
    pellets: 1,
    armor_ratio: 1.4,
    accuracy: AccuracyKeys::new([0.0006, 0.00405, 0.0054, 0.33464, 0.06693, 0.01227, 0.06132]),
    accuracy_alt: Some(AccuracyKeys::new([
        0.0006, 0.00284, 0.00378, 0.33464, 0.06693, 0.00859, 0.06132,
    ])),
    recovery: (0.27631, 0.38683),
    // UNMEASURED: the M4A1's sets (scoped too).
    recoil: Some(M4A1_KICK),
    // Measured M15: 55, off, stays zoomed (as the AUG); UNMEASURED: the zoomed speed (unchanged, as the AUG's).
    alt: Alt::Scope {
        fov: &[55.0],
        speed: None,
        unzoom: false,
        overlay: false,
    },
    fire: "Weapon_SG552.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: SG552_SOUNDS,
};

pub const SCOUT_GUN: Gun = Gun {
    id: SCOUT,
    slot: 0,
    draw: 1.0,
    max_speed: 260.0,
    automatic: false,
    cycle: 1.25,
    clip: 10,
    ammo: AMMO_762MM,
    reload: 2.9,
    shells: None,
    idle: 1.8,
    damage: 75.0,
    range: 8192.0,
    range_modifier: 0.98,
    penetration: 3,
    pellets: 1,
    armor_ratio: 1.7,
    accuracy: AccuracyKeys::new([0.0003, 0.02378, 0.0317, 0.38195, 0.03819, 0.06667, 0.19097]),
    accuracy_alt: Some(AccuracyKeys::new([
        0.0003, 0.003, 0.004, 0.38195, 0.03819, 0.06667, 0.19097,
    ])),
    recovery: (0.17681, 0.24753),
    // UNMEASURED: the AWP's (semi-automatic) kick.
    recoil: Some(SEMI_AUTO_KICK),
    // Measured M15: 40, 15, off; a shot unzooms until it may fire again; zoomed speed 220.
    alt: Alt::Scope {
        fov: &[40.0, 15.0],
        speed: Some(220.0),
        unzoom: true,
        overlay: true,
    },
    fire: "Weapon_Scout.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: SCOUT_SOUNDS,
};

pub const SG550_GUN: Gun = Gun {
    id: SG550,
    slot: 0,
    draw: 1.0,
    max_speed: 210.0,
    automatic: true,
    cycle: 0.25,
    clip: 30,
    ammo: AMMO_556MM,
    reload: 3.75,
    shells: None,
    idle: 1.8,
    damage: 70.0,
    range: 8192.0,
    range_modifier: 0.98,
    penetration: 2,
    pellets: 1,
    armor_ratio: 1.45,
    accuracy: AccuracyKeys::new([0.0003, 0.01928, 0.0257, 0.43727, 0.04373, 0.03829, 0.21864]),
    accuracy_alt: Some(AccuracyKeys::new([
        0.0003, 0.0015, 0.002, 0.43727, 0.04373, 0.03829, 0.21864,
    ])),
    recovery: (0.2097, 0.29358),
    // UNMEASURED: the AWP's kick.
    recoil: Some(SEMI_AUTO_KICK),
    // Measured M15: 40, 15, off; stays zoomed; zoomed speed 150.
    alt: Alt::Scope {
        fov: &[40.0, 15.0],
        speed: Some(150.0),
        unzoom: false,
        overlay: true,
    },
    fire: "Weapon_SG550.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: SG550_SOUNDS,
};

pub const G3SG1_GUN: Gun = Gun {
    id: G3SG1,
    slot: 0,
    draw: 1.0,
    max_speed: 210.0,
    automatic: true,
    cycle: 0.25,
    clip: 20,
    ammo: AMMO_762MM,
    reload: 4.6667,
    shells: None,
    idle: 1.8,
    damage: 80.0,
    range: 8192.0,
    range_modifier: 0.98,
    penetration: 3,
    pellets: 1,
    armor_ratio: 1.65,
    accuracy: AccuracyKeys::new([0.0003, 0.01935, 0.0258, 0.46557, 0.04656, 0.04989, 0.23279]),
    accuracy_alt: Some(AccuracyKeys::new([
        0.0003, 0.0015, 0.002, 0.46557, 0.04656, 0.04989, 0.23279,
    ])),
    recovery: (0.22245, 0.31142),
    // UNMEASURED: the AWP's kick.
    recoil: Some(SEMI_AUTO_KICK),
    // Measured M15: 40, 15, off; stays zoomed. UNMEASURED: the zoomed speed (the SG550's 150).
    alt: Alt::Scope {
        fov: &[40.0, 15.0],
        speed: Some(150.0),
        unzoom: false,
        overlay: true,
    },
    fire: "Weapon_G3SG1.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: G3SG1_SOUNDS,
};

pub const MAC10_GUN: Gun = Gun {
    id: MAC10,
    slot: 0,
    draw: 1.0,
    max_speed: 250.0,
    automatic: true,
    cycle: 0.075,
    clip: 30,
    ammo: AMMO_45ACP,
    reload: 3.1429,
    shells: None,
    idle: 2.0,
    damage: 29.0,
    range: 4096.0,
    range_modifier: 0.82,
    penetration: 1,
    pellets: 1,
    armor_ratio: 0.95,
    accuracy: AccuracyKeys::new([0.001, 0.01425, 0.019, 0.13704, 0.02741, 0.00845, 0.0062]),
    accuracy_alt: None,
    recovery: (0.25263, 0.35368),
    // UNMEASURED: the M4A1's sets (the lightest measured automatic).
    recoil: Some(M4A1_KICK),
    alt: Alt::None,
    fire: "Weapon_MAC10.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: MAC10_SOUNDS,
};

pub const TMP_GUN: Gun = Gun {
    id: TMP,
    slot: 0,
    draw: 0.8333,
    max_speed: 250.0,
    automatic: true,
    cycle: 0.07,
    clip: 30,
    ammo: AMMO_9MM,
    reload: 2.12,
    shells: None,
    idle: 2.0,
    damage: 26.0,
    range: 4096.0,
    range_modifier: 0.84,
    penetration: 1,
    pellets: 1,
    armor_ratio: 1.0,
    accuracy: AccuracyKeys::new([0.001, 0.015, 0.02, 0.1118, 0.02236, 0.01594, 0.00389]),
    accuracy_alt: None,
    recovery: (0.15131, 0.21184),
    // UNMEASURED: the M4A1's sets. Always silenced (its own fire sound), no toggle.
    recoil: Some(M4A1_KICK),
    alt: Alt::None,
    fire: "Weapon_TMP.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: TMP_SOUNDS,
};

pub const MP5NAVY_GUN: Gun = Gun {
    id: MP5NAVY,
    slot: 0,
    draw: 0.8571,
    max_speed: 250.0,
    automatic: true,
    cycle: 0.08,
    clip: 30,
    ammo: AMMO_9MM,
    reload: 3.0526,
    shells: None,
    idle: 2.0,
    damage: 26.0,
    range: 4096.0,
    range_modifier: 0.84,
    penetration: 1,
    pellets: 1,
    armor_ratio: 1.0,
    accuracy: AccuracyKeys::new([0.001, 0.01289, 0.01718, 0.23025, 0.04605, 0.00638, 0.01785]),
    accuracy_alt: None,
    recovery: (0.2796, 0.39144),
    // UNMEASURED: the M4A1's sets.
    recoil: Some(M4A1_KICK),
    alt: Alt::None,
    fire: "Weapon_MP5Navy.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: MP5NAVY_SOUNDS,
};

pub const UMP45_GUN: Gun = Gun {
    id: UMP45,
    slot: 0,
    draw: 1.0,
    max_speed: 250.0,
    automatic: true,
    cycle: 0.105,
    clip: 25,
    ammo: AMMO_45ACP,
    reload: 3.4545,
    shells: None,
    idle: 2.0,
    damage: 30.0,
    range: 4096.0,
    range_modifier: 0.82,
    penetration: 1,
    pellets: 1,
    armor_ratio: 1.0,
    accuracy: AccuracyKeys::new([0.001, 0.01439, 0.01919, 0.16941, 0.03388, 0.01129, 0.01366]),
    accuracy_alt: None,
    recovery: (0.2171, 0.30394),
    // UNMEASURED: the M4A1's sets.
    recoil: Some(M4A1_KICK),
    alt: Alt::None,
    fire: "Weapon_UMP45.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: UMP45_SOUNDS,
};

pub const P90_GUN: Gun = Gun {
    id: P90,
    slot: 0,
    draw: 1.0,
    max_speed: 245.0,
    automatic: true,
    cycle: 0.07,
    clip: 50,
    ammo: AMMO_57MM,
    reload: 3.375,
    shells: None,
    idle: 2.0,
    damage: 26.0,
    range: 4096.0,
    range_modifier: 0.84,
    penetration: 1,
    pellets: 1,
    armor_ratio: 1.5,
    accuracy: AccuracyKeys::new([0.001, 0.01463, 0.01951, 0.16494, 0.03299, 0.00732, 0.01062]),
    accuracy_alt: None,
    recovery: (0.23289, 0.32605),
    // UNMEASURED: the M4A1's sets.
    recoil: Some(M4A1_KICK),
    alt: Alt::None,
    fire: "Weapon_P90.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: P90_SOUNDS,
};

pub const M249_GUN: Gun = Gun {
    id: M249,
    slot: 0,
    draw: 0.96,
    max_speed: 220.0,
    automatic: true,
    cycle: 0.08,
    clip: 100,
    ammo: AMMO_556MM_BOX,
    reload: 5.7,
    shells: None,
    idle: 1.6,
    damage: 35.0,
    range: 8192.0,
    range_modifier: 0.97,
    penetration: 2,
    pellets: 1,
    armor_ratio: 1.6,
    accuracy: AccuracyKeys::new([0.002, 0.00763, 0.01017, 0.7083, 0.14166, 0.00427, 0.10618]),
    accuracy_alt: None,
    recovery: (0.5592, 0.78288),
    // UNMEASURED: the AK-47's sets (the hardest-kicking measured automatic).
    recoil: Some(AK47_KICK),
    alt: Alt::None,
    fire: "Weapon_M249.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: M249_SOUNDS,
};

pub const P228_GUN: Gun = Gun {
    id: P228,
    slot: 1,
    draw: 1.0,
    max_speed: 250.0,
    automatic: false,
    cycle: 0.15,
    clip: 13,
    ammo: AMMO_357SIG,
    reload: 2.7143,
    shells: None,
    idle: 0.0,
    damage: 40.0,
    range: 4096.0,
    range_modifier: 0.8,
    penetration: 1,
    pellets: 1,
    armor_ratio: 1.25,
    accuracy: AccuracyKeys::new([0.004, 0.00825, 0.011, 0.285, 0.057, 0.03318, 0.0171]),
    accuracy_alt: None,
    recovery: (0.23026, 0.27631),
    // UNMEASURED: the USP's and Deagle's 2 degrees.
    recoil: Some(SEMI_AUTO_KICK),
    alt: Alt::None,
    fire: "Weapon_P228.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Pistol",
    model_sounds: P228_SOUNDS,
};

pub const FIVESEVEN_GUN: Gun = Gun {
    id: FIVESEVEN,
    slot: 1,
    draw: 1.0,
    max_speed: 250.0,
    automatic: false,
    cycle: 0.15,
    clip: 20,
    ammo: AMMO_57MM,
    reload: 3.2,
    shells: None,
    idle: 0.0,
    damage: 25.0,
    range: 4096.0,
    range_modifier: 0.885,
    penetration: 1,
    pellets: 1,
    armor_ratio: 1.5,
    accuracy: AccuracyKeys::new([0.004, 0.006, 0.01, 0.25635, 0.05127, 0.05883, 0.01538]),
    accuracy_alt: None,
    recovery: (0.18628, 0.22353),
    // UNMEASURED: the USP's and Deagle's 2 degrees.
    recoil: Some(SEMI_AUTO_KICK),
    alt: Alt::None,
    fire: "Weapon_FiveSeven.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Pistol",
    model_sounds: FIVESEVEN_SOUNDS,
};

pub const ELITE_GUN: Gun = Gun {
    id: ELITE,
    slot: 1,
    draw: 1.3333,
    max_speed: 250.0,
    automatic: false,
    cycle: 0.12,
    clip: 30,
    ammo: AMMO_9MM,
    reload: 3.76,
    shells: None,
    idle: 0.0,
    damage: 45.0,
    range: 4096.0,
    range_modifier: 0.75,
    penetration: 1,
    pellets: 1,
    armor_ratio: 1.05,
    accuracy: AccuracyKeys::new([0.004, 0.006, 0.008, 0.29625, 0.05925, 0.03162, 0.01778]),
    accuracy_alt: None,
    recovery: (0.24753, 0.29703),
    // UNMEASURED: the USP's and Deagle's 2 degrees. Hands alternate (`elite_right_hand`).
    recoil: Some(SEMI_AUTO_KICK),
    alt: Alt::None,
    fire: "Weapon_Elite.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Pistol",
    model_sounds: ELITE_SOUNDS,
};

pub const M3_GUN: Gun = Gun {
    id: M3,
    slot: 0,
    draw: 1.0,
    max_speed: 220.0,
    automatic: true,
    cycle: 0.88,
    clip: 8,
    ammo: AMMO_BUCKSHOT,
    reload: 0.4909,
    // UNMEASURED (M9): the SDK template's start 0.5 s and 0.45 s a shell (spec T25); the model's start/insert/finish are 0.375/0.4909/0.875 s.
    shells: Some((0.5, 0.45)),
    idle: 0.0,
    damage: 26.0,
    range: 3000.0,
    range_modifier: 0.7,
    penetration: 1,
    pellets: 9,
    armor_ratio: 1.0,
    accuracy: AccuracyKeys::new([0.04, 0.0075, 0.01, 0.42, 0.084, 0.04164, 0.0432]),
    accuracy_alt: None,
    recovery: (0.29605, 0.41447),
    recoil: Some(SHOTGUN_KICK),
    alt: Alt::None,
    fire: "Weapon_M3.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: M3_SOUNDS,
};

pub const XM1014_GUN: Gun = Gun {
    id: XM1014,
    slot: 0,
    draw: 1.0,
    max_speed: 240.0,
    automatic: true,
    cycle: 0.25,
    clip: 7,
    ammo: AMMO_BUCKSHOT,
    reload: 0.3889,
    // UNMEASURED (M9): as the M3 (the template's); the model's are 0.6667/0.3889/0.4 s.
    shells: Some((0.5, 0.45)),
    idle: 0.0,
    damage: 22.0,
    range: 3000.0,
    range_modifier: 0.7,
    penetration: 1,
    pellets: 6,
    armor_ratio: 1.0,
    accuracy: AccuracyKeys::new([0.04, 0.0075, 0.01, 0.41176, 0.08235, 0.03644, 0.03544]),
    accuracy_alt: None,
    recovery: (0.32894, 0.46052),
    recoil: Some(SHOTGUN_KICK),
    alt: Alt::None,
    fire: "Weapon_XM1014.Single",
    fire_alt: None,
    empty: "Default.ClipEmpty_Rifle",
    model_sounds: XM1014_SOUNDS,
};

// The silenced and unsilenced sequences play the same sounds.
const M4A1_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.6756, "Weapon_M4A1.Clipout"),
        (1.4324, "Weapon_M4A1.Clipin"),
        (2.3785, "Weapon_M4A1.Boltpull"),
    ],
    draw: &[(0.025, "Weapon_M4A1.Deploy"), (0.425, "Weapon_M4A1.Boltpull")],
    modes: &[
        (1, 0.9333, "Weapon_M4A1.Silencer_On"),
        (0, 0.7, "Weapon_M4A1.Silencer_Off"),
    ],
    fire: &[],
    finish: &[],
};
const USP_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.0, "Weapon_USP.Slideback2"),
        (0.4594, "Weapon_USP.Clipout"),
        (1.081, "Weapon_USP.Clipin"),
        (2.2163, "Weapon_USP.Sliderelease"),
    ],
    draw: &[(0.5417, "Weapon_USP.Slideback")],
    modes: &[
        (1, 1.027, "Weapon_USP.AttachSilencer"),
        (0, 0.7838, "Weapon_USP.DetachSilencer"),
    ],
    fire: &[],
    finish: &[],
};
const GLOCK_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.0, "Weapon_Glock.Slideback"),
        (0.4001, "Weapon_Glock.Clipout"),
        (1.0858, "Weapon_Glock.Clipin"),
        (1.8285, "Weapon_Glock.Sliderelease"),
    ],
    draw: &[(0.3778, "Weapon_Glock.Sliderelease")],
    modes: &[],
    fire: &[],
    finish: &[],
};
const DEAGLE_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.0, "Weapon_DEagle.Slideback"),
        (0.4667, "Weapon_DEagle.Clipout"),
        (1.1334, "Weapon_DEagle.Clipin"),
    ],
    draw: &[(0.0333, "Weapon_DEagle.Deploy")],
    modes: &[],
    fire: &[],
    finish: &[],
};

const FAMAS_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.5187, "Weapon_FAMAS.Clipout"),
        (1.4813, "Weapon_FAMAS.Clipin"),
        (2.3333, "Weapon_FAMAS.Forearm"),
    ],
    draw: &[],
    modes: &[],
    fire: &[],
    finish: &[],
};
const GALIL_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.3809, "Weapon_Galil.Clipout"),
        (1.3094, "Weapon_Galil.Clipin"),
        (2.1668, "Weapon_Galil.Boltpull"),
    ],
    draw: &[],
    modes: &[],
    fire: &[],
    finish: &[],
};
const AUG_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.2859, "Weapon_AUG.Boltpull"),
        (1.4286, "Weapon_AUG.Clipout"),
        (2.5143, "Weapon_AUG.Clipin"),
        (3.2, "Weapon_AUG.Boltslap"),
    ],
    draw: &[(0.3, "Weapon_AUG.Forearm")],
    modes: &[],
    fire: &[],
    finish: &[],
};
const SG552_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.4325, "Weapon_SG552.Clipout"),
        (1.6486, "Weapon_SG552.Clipin"),
        (2.4326, "Weapon_SG552.Boltpull"),
    ],
    draw: &[(0.3243, "Weapon_SG552.Boltpull")],
    modes: &[],
    fire: &[],
    finish: &[],
};
const SCOUT_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.4333, "Weapon_Scout.Clipout"),
        (1.2334, "Weapon_Scout.Clipin"),
        (1.7333, "Weapon_Scout.Bolt"),
    ],
    draw: &[],
    modes: &[],
    fire: &[(0.3714, "Weapon_Scout.Bolt")],
    finish: &[],
};
const SG550_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.7856, "Weapon_SG550.Clipout"),
        (1.6429, "Weapon_SG550.Clipin"),
        (2.9288, "Weapon_SG550.Boltpull"),
    ],
    draw: &[],
    modes: &[],
    fire: &[],
    finish: &[],
};
const G3SG1_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.5334, "Weapon_G3SG1.Slide"),
        (1.8, "Weapon_G3SG1.Clipout"),
        (2.8332, "Weapon_G3SG1.Clipin"),
        (3.8668, "Weapon_G3SG1.Slide"),
    ],
    draw: &[],
    modes: &[],
    fire: &[],
    finish: &[],
};
const MAC10_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.6286, "Weapon_MAC10.Clipout"),
        (1.5715, "Weapon_MAC10.Clipin"),
        (2.4857, "Weapon_MAC10.Boltpull"),
    ],
    draw: &[],
    modes: &[],
    fire: &[],
    finish: &[],
};
const TMP_SOUNDS: ModelSounds = ModelSounds {
    reload: &[(0.48, "Weapon_TMP.Clipout"), (1.28, "Weapon_TMP.Clipin")],
    // The draw's event names Weapon_TMP.Deploy, which no sound script
    // defines (the game plays nothing).
    draw: &[],
    modes: &[],
    fire: &[],
    finish: &[],
};
const MP5NAVY_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.3156, "Weapon_MP5Navy.Clipout"),
        (1.1578, "Weapon_MP5Navy.Clipin"),
        (2.2632, "Weapon_MP5Navy.Slideback"),
    ],
    draw: &[(0.3714, "Weapon_MP5Navy.Slideback")],
    modes: &[],
    fire: &[],
    finish: &[],
};
const UMP45_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.6971, "Weapon_UMP45.Clipout"),
        (1.7877, "Weapon_UMP45.Clipin"),
        (2.6061, "Weapon_UMP45.Boltslap"),
    ],
    draw: &[],
    modes: &[],
    fire: &[],
    finish: &[],
};
const P90_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.4249, "Weapon_P90.Cliprelease"),
        (0.8751, "Weapon_P90.Clipout"),
        (1.8752, "Weapon_P90.Clipin"),
        (2.7, "Weapon_P90.Boltpull"),
    ],
    draw: &[(0.3, "Weapon_P90.Boltpull")],
    modes: &[],
    fire: &[],
    finish: &[],
};
const M249_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.4668, "Weapon_M249.Boxout"),
        (1.5002, "Weapon_M249.Coverup"),
        (2.6665, "Weapon_M249.Boxin"),
        (3.2997, "Weapon_M249.Chain"),
        (4.5002, "Weapon_M249.Coverdown"),
    ],
    draw: &[],
    modes: &[],
    fire: &[],
    finish: &[],
};
const P228_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.0, "Weapon_P228.Slideback"),
        (0.6856, "Weapon_P228.Clipout"),
        (1.4, "Weapon_P228.Clipin"),
        (2.3142, "Weapon_P228.Sliderelease"),
    ],
    draw: &[(0.5, "Weapon_P228.Slidepull")],
    modes: &[],
    fire: &[],
    finish: &[],
};
const FIVESEVEN_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.0, "Weapon_FiveSeven.Slideback"),
        (0.4998, "Weapon_FiveSeven.Clipout"),
        (1.3667, "Weapon_FiveSeven.Clipin"),
        (2.4998, "Weapon_FiveSeven.Sliderelease"),
    ],
    draw: &[(0.4333, "Weapon_FiveSeven.Slidepull")],
    modes: &[],
    fire: &[],
    finish: &[],
};
const ELITE_SOUNDS: ModelSounds = ModelSounds {
    reload: &[
        (0.0, "Weapon_ELITE.Reloadstart"),
        (0.4802, "Weapon_ELITE.Clipout"),
        (1.4401, "Weapon_ELITE.Rclipin"),
        (2.3602, "Weapon_ELITE.Lclipin"),
        (3.24, "Weapon_ELITE.Sliderelease"),
    ],
    draw: &[(0.0333, "Weapon_ELITE.Deploy")],
    modes: &[],
    fire: &[],
    finish: &[],
};
const M3_SOUNDS: ModelSounds = ModelSounds {
    reload: &[(0.0, "Weapon_M3.Insertshell")],
    draw: &[(0.3667, "Weapon_M3.Pump")],
    modes: &[],
    fire: &[],
    finish: &[(0.3, "Weapon_M3.Pump")],
};
const XM1014_SOUNDS: ModelSounds = ModelSounds {
    reload: &[(0.0222, "Weapon_XM1014.InsertShell")],
    draw: &[(0.0333, "Weapon_DEagle.Deploy")],
    modes: &[],
    fire: &[],
    finish: &[],
};

/// Every gun, for tables and tests.
pub const GUNS: &[&Gun] = &[
    &AK47_GUN,
    &M4A1_GUN,
    &AWP_GUN,
    &USP_GUN,
    &GLOCK_GUN,
    &DEAGLE_GUN,
    &FAMAS_GUN,
    &GALIL_GUN,
    &AUG_GUN,
    &SG552_GUN,
    &SCOUT_GUN,
    &SG550_GUN,
    &G3SG1_GUN,
    &MAC10_GUN,
    &TMP_GUN,
    &MP5NAVY_GUN,
    &UMP45_GUN,
    &P90_GUN,
    &M249_GUN,
    &P228_GUN,
    &FIVESEVEN_GUN,
    &ELITE_GUN,
    &M3_GUN,
    &XM1014_GUN,
];

fn timed(sounds: &[(f32, &str)]) -> Vec<(f32, String)> {
    sounds.iter().map(|(t, s)| (*t, s.to_string())).collect()
}

/// Build a gun's parts on `e`.
pub fn gun(e: &mut EntityWorldMut, g: &Gun) {
    let accuracy = Inaccuracy::new(g.accuracy, g.accuracy_alt, g.recovery);
    e.insert((
        Weapon {
            id: g.id,
            slot: g.slot,
            owner: None,
            draw_time: g.draw,
            max_speed: Some(g.max_speed * UNIT),
        },
        Trigger {
            automatic: g.automatic,
            cycle: g.cycle,
            // Held: next = previous next + cycle; a fresh press: now + cycle
            // (M4), semi-automatics alike.
            timing: FireTiming::CarryOver,
        },
        Magazine {
            clip: g.clip,
            size: g.clip,
            // Deathmatch: the reserve starts full.
            reserve: g.ammo.max,
            reserve_max: g.ammo.max,
            reload_time: g.reload,
            reload_while_held: false,
        },
        Hitscan {
            range: g.range * UNIT,
            pellets: g.pellets,
            spread: SpreadShape::Disc {
                inaccuracy: accuracy.value,
                spread: g.accuracy.spread,
            },
            // Bullets go along view + 2 x punch (M3).
            punch_scale: 2.0,
        },
        DamageEffect {
            amount: g.damage * HP,
            falloff: g.range_modifier,
            falloff_step: 500.0 * UNIT,
            hitgroups: HITGROUPS,
            // UNMEASURED (Q8): the template's .50 AE impulse, kg·in/s.
            impulse: 2400.0 * UNIT,
            // Damage is truncated to whole hit points (M5).
            quantum: HP,
            armor_ratio: Some(g.armor_ratio),
        },
        Penetration {
            power: g.ammo.power * UNIT,
            objects: g.penetration,
            max_distance: g.ammo.max_distance * UNIT,
        },
        accuracy,
        WeaponSounds {
            fire: Some(g.fire.into()),
            fire_alt: g.fire_alt.map(Into::into),
            empty: Some(g.empty.into()),
            deploy: None,
            reload: timed(g.model_sounds.reload),
            draw: timed(g.model_sounds.draw),
            shot: timed(g.model_sounds.fire),
            finish: timed(g.model_sounds.finish),
            modes: g
                .model_sounds
                .modes
                .iter()
                .map(|(m, t, s)| (*m, *t, s.to_string()))
                .collect(),
        },
    ));
    if let Some((start, insert)) = g.shells {
        e.insert(ShellReload { start, insert });
    }
    if let Some([standing, crouched, moving, airborne]) = g.recoil {
        e.insert(Recoil {
            standing,
            crouched,
            moving,
            airborne,
            ..default()
        });
    }
    match g.alt {
        Alt::None => {}
        Alt::Scope {
            fov,
            speed,
            unzoom,
            overlay,
        } => {
            // Each step: next secondary in 0.3 s (M15).
            let mut modes = AltModes::new(fov.len() as u8 + 1, 0.3, false);
            modes.sound = Some("Default.Zoom".into());
            e.insert((
                modes,
                Zoom {
                    fov: fov.to_vec(),
                    max_speed: speed.map(|s| s * UNIT),
                    unzoom_after_shot: unzoom,
                    scope: overlay,
                },
            ));
        }
        Alt::Silencer { time } => {
            e.insert(AltModes::new(2, time, true));
        }
        Alt::Burst { interval, refire } => {
            // 0.3 s secondary delay (M16).
            e.insert((
                AltModes::new(2, 0.3, false),
                Burst {
                    mode: 1,
                    count: 3,
                    interval,
                    refire,
                },
            ));
        }
    }
}

/// CS:S's accuracy penalty (measured M1/M2; values are the weapon script's
/// `Inaccuracy*`, `RecoveryTime*` and `Spread`, in tangent units). Rests
/// at `stand` (`crouch` when ducked); the excess falls to 10 % in the
/// recovery time; a shot adds `fire`, a jump `jump`, a landing `land` x
/// the fall speed / 301.99 u/s. Moving adds `movement` x
/// clamp((v - vmax/3) / (2 vmax/3)) at shot time. In an alternate mode
/// (`AltModes`: silenced, burst, scoped) the `*Alt` keys apply, with the
/// same recovery times (the scripts have no Alt ones).
#[derive(Component, Clone, Debug)]
pub struct Inaccuracy {
    pub keys: AccuracyKeys,
    pub alt: Option<AccuracyKeys>,
    pub recovery_stand: f32,
    pub recovery_crouch: f32,
    /// The current penalty.
    pub value: f32,
    on_ground: bool,
    /// Vertical speed on the previous tick, u/s (for landings).
    fall_speed: f32,
}

impl Inaccuracy {
    /// From the keys and (`RecoveryTimeCrouch`, `RecoveryTimeStand`); starts
    /// at rest standing.
    pub fn new(keys: AccuracyKeys, alt: Option<AccuracyKeys>, recovery: (f32, f32)) -> Self {
        Self {
            keys,
            alt,
            recovery_crouch: recovery.0,
            recovery_stand: recovery.1,
            value: keys.stand,
            on_ground: true,
            fall_speed: 0.0,
        }
    }

    /// The keys for `mode` (0 = normal).
    pub fn keys(&self, mode: u8) -> AccuracyKeys {
        match (mode, self.alt) {
            (1.., Some(alt)) => alt,
            _ => self.keys,
        }
    }
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
    /// Above `up`: each shot kicks up a whole number of degrees from `up`
    /// to this at random instead (the SDK template's shotgun).
    pub up_max: f32,
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
            up_max: 0.0,
        }
    }

    /// Straight up by a random whole number of degrees in `lo..=hi`.
    pub const fn random_up(lo: f32, hi: f32) -> Self {
        let mut k = Self::new((lo, 0.0), (0.0, 0.0), (NO_CAP, NO_CAP));
        k.up_max = hi;
        k
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
    mut weapons: Query<(&Weapon, &mut Inaccuracy, &mut Hitscan, Option<&AltModes>)>,
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
        let Ok((weapon, mut acc, mut scan, modes)) = weapons.get_mut(active) else {
            continue;
        };
        let keys = acc.keys(modes.map_or(0, |m| m.current));
        let rest = if state.crouching { keys.crouch } else { keys.stand };
        let recovery = if state.crouching {
            acc.recovery_crouch
        } else {
            acc.recovery_stand
        };
        if acc.on_ground && !state.on_ground && vel.0.y > 0.0 {
            acc.value += keys.jump;
        } else if !acc.on_ground && state.on_ground {
            // Scaled by the vertical speed on the tick before (M1).
            acc.value += keys.land * acc.fall_speed.abs() / LAND_SPEED;
        }
        acc.on_ground = state.on_ground;
        acc.fall_speed = vel.0.y / UNIT;
        acc.value = if !state.on_ground {
            let target = keys.crouch;
            (target + (acc.value - target) * AIR_DECAY_PER_TICK.powf(dt / 0.015)).max(rest.min(acc.value))
        } else if acc.value > rest {
            rest + (acc.value - rest) * 0.1f32.powf(dt / recovery)
        } else {
            rest
        };
        let speed = vel.0.xz().length() / UNIT;
        let vmax = weapon.max_speed.unwrap_or(250.0 * UNIT) / UNIT;
        let moving = ((speed - vmax / 3.0) / (vmax * 2.0 / 3.0)).clamp(0.0, 1.0) * keys.movement;
        scan.spread = SpreadShape::Disc {
            inaccuracy: acc.value + moving,
            spread: keys.spread,
        };
    }
}

/// After the weapon frame: each shot adds to the penalty and kicks the
/// view.
fn after_shots(
    mut events: MessageReader<WeaponEvent>,
    mut weapons: Query<(Option<&mut Inaccuracy>, Option<&mut Recoil>, Option<&AltModes>)>,
    mut owners: Query<(&MovementState, &Velocity, Option<&mut ViewPunch>, Option<&crate::core::Seed>)>,
    mut commands: Commands,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    for e in events.read() {
        if !matches!(e.kind, WeaponEventKind::Shot { .. }) {
            continue;
        }
        let Ok((acc, recoil, modes)) = weapons.get_mut(e.weapon) else {
            continue;
        };
        if let Some(mut acc) = acc {
            // (A sniper shot has already unzoomed: the AWP's fire key is
            // the same in both modes.)
            acc.value += acc.keys(modes.map_or(0, |m| m.current)).fire;
        }
        let Some(mut r) = recoil else { continue };
        let Ok((state, vel, punch, seed)) = owners.get_mut(e.owner) else {
            continue;
        };
        if now - r.last_shot > SHOTS_RESET {
            r.shots = 0;
        }
        r.last_shot = now;
        r.shots += 1;
        // xorshift for the sideways direction.
        // From the owner's `Seed`: entity ids shift with any spawn.
        r.rng = if r.rng == 0 {
            seed.map_or(e.weapon.to_bits(), |s| s.0.wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1
        } else {
            r.rng
        };
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
        let (mut up, side) = (kick.up + kick.up_step * n, kick.side + kick.side_step * n);
        if kick.up_max > kick.up {
            // A second roll from the same dice.
            r.rng ^= r.rng << 13;
            r.rng ^= r.rng >> 7;
            r.rng ^= r.rng << 17;
            let u = (r.rng >> 40) as f32 / (1u64 << 24) as f32;
            up = (kick.up + (u * (kick.up_max - kick.up + 1.0)).floor()).min(kick.up_max);
        }
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
