//! What players sound like when hurt and killed (`games::cs_source::pain`,
//! the install's `game_sounds.txt` entries): a body shot with and without
//! kevlar, a headshot with and without a helmet, a death, a bot's hits.
//! Each plays from the victim: centred for the victim (its own sound),
//! panned by place for a bystander. Falls: tests/it/source_movement.rs.

use std::sync::Arc;

use bevy::prelude::*;
use mashup::{
    bot::{BotConfig, add_bot},
    core::{Health, Team},
    games::cs_source::{
        TICK_INTERVAL,
        weapons::{AK47, CsWeaponsPlugin, DEAGLE},
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    map::{
        MapSounds, MapSurface, PlaySound, SoundLevel,
        sound::{Ear, SoundBank},
    },
    movement::placeholder,
    weapon::{Armor, Inventory, give},
};

#[derive(Resource, Default)]
struct Heard(Vec<PlaySound>);

#[derive(Resource, Default)]
struct Impacts(Vec<String>);

fn hear(mut sounds: MessageReader<PlaySound>, mut heard: ResMut<Heard>, mut impacts: ResMut<Impacts>) {
    for s in sounds.read() {
        if s.entry.starts_with("Player.") {
            heard.0.push(s.clone());
        } else {
            impacts.0.push(s.entry.clone());
        }
    }
}

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    // Surfaces as the install's: players' hitboxes are `flesh`; `player`
    // (the movement box) has no impact sound.
    let mut sounds = MapSounds::default();
    for (name, impact) in [
        ("flesh", Some("Flesh.BulletImpact")),
        ("player", None),
        ("default", Some("Default.BulletImpact")),
    ] {
        sounds.surfaces.insert(
            name.into(),
            MapSurface {
                bullet_impact: impact.map(String::from),
                ..default()
            },
        );
    }
    sim.app.insert_resource(SoundBank(Arc::new(sounds)));
    sim.app
        .init_resource::<Heard>()
        .init_resource::<Impacts>()
        .add_systems(Last, hear);
    sim
}

fn entries(sim: &Sim) -> Vec<String> {
    sim.app
        .world()
        .resource::<Heard>()
        .0
        .iter()
        .map(|s| s.entry.clone())
        .collect()
}

/// One Desert Eagle shot from 8 m at the target's chest or head; the
/// target has `health` (normalized) and `armor`. Returns the sim, the
/// target and a bystander standing 3 m to the target's side.
fn shot(head: bool, armor: Option<Armor>, health: f32) -> (Sim, Entity, Entity) {
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 8.0, placeholder::ID);
    let bystander = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 8.0 + Vec3::X * 3.0, placeholder::ID);
    sim.app.world_mut().get_mut::<Health>(target).unwrap().current = health;
    if let Some(a) = armor {
        sim.app.world_mut().entity_mut(target).insert(a);
    }
    sim.ticks(1);
    give(sim.app.world_mut(), shooter, DEAGLE).unwrap();
    sim.seconds(1.2);
    assert!(sim.app.world().get::<Inventory>(shooter).unwrap().active.is_some());
    let height = if head { 1.8 * 0.92 - 0.9 } else { 1.8 * 0.72 - 0.9 };
    let point = sim.position(target) + Vec3::Y * height;
    let eye = sim.position(shooter) + sim.state(shooter).eye_offset;
    let d = point - eye;
    {
        let mut i = sim.intent(shooter);
        i.yaw = (-d.x).atan2(-d.z);
        i.pitch = d.y.atan2(d.xz().length());
    }
    sim.app.world_mut().resource_mut::<Heard>().0.clear();
    sim.intent(shooter).fire = true;
    sim.ticks(1);
    sim.intent(shooter).fire = false;
    sim.seconds(0.2);
    assert!(
        sim.app.world().get::<Health>(target).unwrap().current < health,
        "the shot missed"
    );
    (sim, target, bystander)
}

const KEVLAR: Armor = Armor {
    amount: 1.0,
    helmet: false,
};
const HELMET: Armor = Armor {
    amount: 1.0,
    helmet: true,
};

/// The one sound named `entry`: made by `victim` where it stands, heard
/// centred by the victim and panned toward it by the bystander.
fn check_heard(sim: &Sim, entry: &str, victim: Entity, bystander: Entity) {
    let heard = &sim.app.world().resource::<Heard>().0;
    let all: Vec<_> = heard.iter().filter(|s| s.entry == entry).collect();
    assert_eq!(all.len(), 1, "{entry} once: {:?}", entries(sim));
    let s = all[0];
    assert_eq!(s.source, Some(victim));
    let at = s.at.expect("positional");
    assert!(at.distance(sim.position(victim)) < 0.5, "at the victim");
    let ear = |who: Entity| Ear {
        at: GlobalTransform::from(
            Transform::from_translation(sim.position(who) + sim.state(who).eye_offset).looking_to(Vec3::NEG_Z, Vec3::Y),
        ),
        owner: Some(who),
    };
    let level = SoundLevel::Attenuation(1.0);
    let (l, r, _) = ear(victim).gains(at, s.source, level);
    assert!(l > 0.99 && (l - r).abs() < 1e-6, "victim: centred ({l}, {r})");
    // The bystander is at +x of the victim, facing -z: the victim is on
    // its left.
    let (l, r, _) = ear(bystander).gains(at, s.source, level);
    assert!(l > r && l < 1.0, "bystander: positional ({l}, {r})");
}

#[test]
fn body_shot_without_armour_is_only_the_flesh_impact() {
    let (sim, ..) = shot(false, None, 10.0);
    assert!(entries(&sim).is_empty(), "{:?}", entries(&sim));
    let impacts = &sim.app.world().resource::<Impacts>().0;
    assert_eq!(
        impacts.iter().filter(|e| *e == "Flesh.BulletImpact").count(),
        1,
        "{impacts:?}"
    );
}

#[test]
fn body_shot_on_kevlar_plays_the_kevlar_hit() {
    let (sim, target, bystander) = shot(false, Some(KEVLAR), 10.0);
    check_heard(&sim, "Player.DamageKevlar", target, bystander);
    assert_eq!(entries(&sim), ["Player.DamageKevlar"]);
}

#[test]
fn headshot_without_helmet_plays_the_headshot() {
    let (sim, target, bystander) = shot(true, Some(KEVLAR), 10.0);
    check_heard(&sim, "Player.DamageHeadShot", target, bystander);
    assert_eq!(entries(&sim), ["Player.DamageHeadShot"]);
}

#[test]
fn headshot_on_a_helmet_plays_the_helmet() {
    let (sim, target, bystander) = shot(true, Some(HELMET), 10.0);
    check_heard(&sim, "Player.DamageHelmet", target, bystander);
    assert_eq!(entries(&sim), ["Player.DamageHelmet"]);
}

#[test]
fn deaths_play_the_death_sound() {
    // A body shot that kills: kevlar's hit and the death cry.
    let (sim, target, bystander) = shot(false, Some(KEVLAR), 0.05);
    check_heard(&sim, "Player.Death", target, bystander);
    assert_eq!(entries(&sim), ["Player.DamageKevlar", "Player.Death"]);
    // A killing headshot: the headshot death, not the headshot hurt too.
    let (sim, target, bystander) = shot(true, None, 0.05);
    check_heard(&sim, "Player.DeathHeadShot", target, bystander);
    assert_eq!(entries(&sim), ["Player.DeathHeadShot"]);
    // Through a helmet: its ding and the headshot death.
    let (sim, ..) = shot(true, Some(HELMET), 0.05);
    assert_eq!(entries(&sim), ["Player.DamageHelmet", "Player.DeathHeadShot"]);
}

#[test]
fn teammates_hits_refused_by_friendly_fire_are_silent() {
    let mut sim = sim();
    let shooter = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * 8.0, placeholder::ID);
    for e in [shooter, target] {
        sim.app.world_mut().entity_mut(e).insert(Team(1));
    }
    sim.app.world_mut().entity_mut(target).insert(KEVLAR);
    sim.ticks(1);
    give(sim.app.world_mut(), shooter, DEAGLE).unwrap();
    sim.seconds(1.2);
    let point = sim.position(target) + Vec3::Y * (1.8 * 0.72 - 0.9);
    let d = point - (sim.position(shooter) + sim.state(shooter).eye_offset);
    {
        let mut i = sim.intent(shooter);
        i.yaw = (-d.x).atan2(-d.z);
        i.pitch = d.y.atan2(d.xz().length());
        i.fire = true;
    }
    sim.ticks(1);
    sim.intent(shooter).fire = false;
    sim.seconds(0.2);
    assert_eq!(sim.app.world().get::<Health>(target).unwrap().current, 1.0);
    assert!(entries(&sim).is_empty(), "{:?}", entries(&sim));
}

/// Bots' victims sound the same: a bot sprays an AK-47 at a player in
/// kevlar.
#[test]
fn a_bots_hits_play_the_victims_sounds() {
    let mut sim = sim();
    sim.app.insert_resource(mashup::slots::Loadout {
        movement: placeholder::ID,
    });
    {
        let mut cfg = sim.app.world_mut().resource_mut::<BotConfig>();
        cfg.grenades = 0;
        cfg.radio = 0;
        cfg.stop = 1;
    }
    let bot = add_bot(sim.app.world_mut(), Team(1)).unwrap();
    sim.app.world_mut().get_mut::<Transform>(bot).unwrap().translation = Vec3::new(0.0, 1.0, 30.0);
    let enemy = sim.spawn_character(Vec3::new(0.0, 1.0, 15.0), placeholder::ID);
    sim.app.world_mut().entity_mut(enemy).insert((
        Team(2),
        Health {
            current: 1000.0,
            max: 1000.0,
        },
        Armor {
            amount: 1000.0,
            helmet: true,
        },
    ));
    sim.ticks(1);
    give(sim.app.world_mut(), bot, AK47).unwrap();
    sim.seconds(5.0);
    let heard = &sim.app.world().resource::<Heard>().0;
    let hurt = heard
        .iter()
        .filter(|s| s.source == Some(enemy) && (s.entry == "Player.DamageKevlar" || s.entry == "Player.DamageHelmet"))
        .count();
    assert!(hurt >= 5, "the bot's hits: {:?}", entries(&sim));
}
