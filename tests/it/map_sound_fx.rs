//! Sounds that come from the physics and the map's entities, on small maps
//! in Source units (specs/cs_source/sounds.md 4 and 6): a crate sliding on
//! the floor scrapes (a looping sound that follows it, louder and higher
//! the harder it slides, and stops once it rests); env_soundscape points
//! the logic enables and disables, OnPlay, and the room settings a
//! soundscape sets.

use std::sync::Arc;

use avian3d::prelude::{LinearVelocity, Position};
use bevy::prelude::*;
use mashup::{
    games::cs_source::{
        self,
        movement::{SourceMovementPlugin, to_engine},
        scrapes::Scrapes,
        weapons::CsWeaponsPlugin,
    },
    harness::Sim,
    logic::{Logic, Value},
    map::{
        LiveSounds, MapBrush, MapCollision, MapConvex, MapData, MapEntity, MapModel, MapPhysics, MapPlugin, MapProp,
        MapSoundClip, MapSoundEntry, MapSounds, MapSurface, PropEntity, PropSolid, PushAway, SoundLevel,
        room::RoomDsp,
        sound::{Interval, SoundListener, Soundscape, SoundscapeEmitter},
        soundscape::{ScapeState, Source},
    },
};

const SCALE: f32 = 0.0254;

fn entity(pairs: &[(&str, &str)]) -> MapEntity {
    MapEntity {
        keyvalues: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        hulls: Vec::new(),
        mover: false,
        physics: None,
    }
}

/// A floor at z = 0 plus `entities`, with `sounds`.
fn map(entities: Vec<MapEntity>, sounds: MapSounds) -> MapData {
    let mut data = MapData {
        name: "test:sound_fx".into(),
        entities,
        entity_scale: SCALE,
        sounds: Arc::new(sounds),
        ..default()
    };
    let (a, b) = (
        to_engine(Vec3::new(-4096.0, -4096.0, -64.0)),
        to_engine(Vec3::new(4096.0, 4096.0, 0.0)),
    );
    let (a, b) = (a.min(b), a.max(b));
    data.collision_brushes.push(MapBrush::from_box(a, b));
    data.collision_hulls.push(
        (0..8)
            .map(|k| [[a.x, b.x][k & 1], [a.y, b.y][(k >> 1) & 1], [a.z, b.z][(k >> 2) & 1]])
            .collect(),
    );
    data
}

fn clip() -> MapSoundClip {
    MapSoundClip {
        rate: 11025,
        channels: 1,
        samples: vec![0i16; 11025].into(),
        loop_start: None,
    }
}

/// A wooden crate surface that scrapes ("Crate.ScrapeRough": script
/// volume 0.8, pitch 90-110) on a default floor.
fn scrape_sounds() -> MapSounds {
    let mut s = MapSounds::default();
    s.clips.push(clip());
    s.entries.insert(
        "crate.scraperough".into(),
        MapSoundEntry {
            waves: vec![0],
            volume: Interval::fixed(0.8),
            pitch: Interval {
                start: 90.0,
                range: 20.0,
            },
            level: SoundLevel::Db(75.0),
            channel: 4,
            dry: false,
        },
    );
    s.surfaces.insert(
        "wood_crate".into(),
        MapSurface {
            game_material: 'W',
            scrape_rough: Some("Crate.ScrapeRough".into()),
            roughness: 1.0,
            rough_threshold: 0.5,
            ..default()
        },
    );
    s.surfaces.insert(
        "default".into(),
        MapSurface {
            game_material: 'C',
            roughness: 1.0,
            rough_threshold: 0.5,
            ..default()
        },
    );
    s
}

/// A 50 kg, 32-unit wooden crate (map entity `index`) resting on the floor.
fn add_crate(data: &mut MapData, index: usize) {
    let half = 16.0 * SCALE;
    let (lo, hi) = (Vec3::splat(-half), Vec3::splat(half));
    let corners = (0..8)
        .map(|k| {
            Vec3::new(
                [lo.x, hi.x][k & 1],
                [lo.y, hi.y][(k >> 1) & 1],
                [lo.z, hi.z][(k >> 2) & 1],
            )
        })
        .collect();
    data.models.push(MapModel {
        bounds: (lo, hi),
        surfaceprop: Some("wood_crate".into()),
        collision: Some(MapCollision {
            pieces: vec![MapConvex {
                points: corners,
                planes: Vec::new(),
            }],
            mass: 50.0,
            ..default()
        }),
        ..default()
    });
    data.props.push(MapProp {
        pose: None,
        ragdoll: None,
        model: data.models.len() - 1,
        translation: to_engine(Vec3::new(0.0, 0.0, 16.5)),
        rotation: Quat::IDENTITY,
        solid: PropSolid::Mesh,
        skybox: false,
        lighting: None,
        vertex_light: None,
        casts_shadow: false,
        physics: Some(MapPhysics {
            mass: 50.0,
            friction: 0.5,
            elasticity: 0.0,
            damping: 0.0,
            rotdamping: 0.0,
            push: PushAway::Collide,
            frozen: false,
            asleep: false,
        }),
        fade: None,
        parent: None,
        entity: Some(index),
        skin: 0,
        body: 0,
        detail: false,
    });
}

fn crate_node(sim: &mut Sim) -> Entity {
    let world = sim.app.world_mut();
    world.query::<(Entity, &PropEntity)>().iter(world).next().unwrap().0
}

/// The scrape sound playing now: (volume, pitch).
fn scrape(sim: &Sim) -> Option<(f32, f32)> {
    let live = sim.app.world().resource::<LiveSounds>();
    live.sounds
        .values()
        .find(|s| s.entry == "Crate.ScrapeRough")
        .map(|s| (s.volume, s.pitch))
}

#[test]
fn a_sliding_crate_scrapes_until_it_stops() {
    let mut data = map(
        vec![entity(&[("classname", "prop_physics"), ("origin", "0 0 16.5")])],
        scrape_sounds(),
    );
    add_crate(&mut data, 0);
    let mut sim = Sim::new((MapPlugin::new(data), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.seconds(0.5);
    assert_eq!(scrape(&sim), None, "resting: silent");
    // Shove it along the floor at 300 u/s.
    let node = crate_node(&mut sim);
    let start = sim.app.world().get::<Position>(node).unwrap().0;
    sim.app.world_mut().get_mut::<LinearVelocity>(node).unwrap().0 = Vec3::X * 300.0 * SCALE;
    sim.ticks(6);
    let (volume, pitch) = scrape(&sim).expect("sliding: scrapes");
    // Started at script volume × amplitude², above the start threshold;
    // pitch across 90-110 by volume.
    assert!(volume > 0.1 && volume <= 0.8, "{volume}");
    assert!((90.0..=110.0).contains(&pitch), "{pitch}");
    {
        let scrapes = sim.app.world().resource::<Scrapes>();
        assert_eq!(scrapes.playing.len(), 1, "one loop per body");
        assert_eq!(scrapes.playing[0].body, node);
        assert!(!scrapes.playing[0].client, "a prop: the server's slots");
    }
    // It follows the crate.
    let live = sim.app.world().resource::<LiveSounds>();
    let s = live.sounds.values().find(|s| s.entry == "Crate.ScrapeRough").unwrap();
    assert_eq!(s.follow, Some(node));
    assert!(s.looping);
    // Friction stops it within a second or so; the scrape stops with it.
    sim.seconds(2.0);
    let moved = sim.app.world().get::<Position>(node).unwrap().0 - start;
    assert!(moved.x > 0.3, "it slid: {moved:?}");
    assert_eq!(scrape(&sim), None, "stopped with the crate");
    assert!(sim.app.world().resource::<Scrapes>().playing.is_empty());
}

/// Two soundscapes ("a" sets dsp 2, dsp_volume 0.5, a mixer and a player
/// preset; "b" nothing) and two env_soundscapes: "a" at the origin, "b"
/// (StartDisabled, OnPlay kills "marker") nearer the listener.
fn soundscape_map() -> MapData {
    let mut sounds = MapSounds::default();
    sounds.soundscapes = vec![
        Soundscape {
            name: "a".into(),
            dsp: Some(2),
            dsp_player: Some(1),
            dsp_volume: Some(0.5),
            mixer: Some("Default_Mix".into()),
            ..default()
        },
        Soundscape {
            name: "b".into(),
            dsp: Some(3),
            ..default()
        },
    ];
    let emitter = |at: Vec3, scape: usize, entity: usize, start_disabled: bool| SoundscapeEmitter {
        at: to_engine(at),
        radius: None,
        scape,
        positions: Vec::new(),
        entity: Some(entity),
        start_disabled,
    };
    sounds.soundscape_emitters = vec![
        emitter(Vec3::ZERO, 0, 0, false),
        emitter(Vec3::new(500.0, 0.0, 64.0), 1, 1, true),
    ];
    map(
        vec![
            entity(&[
                ("classname", "env_soundscape"),
                ("targetname", "a"),
                ("origin", "0 0 0"),
            ]),
            entity(&[
                ("classname", "env_soundscape"),
                ("targetname", "b"),
                ("origin", "500 0 64"),
                ("StartDisabled", "1"),
                ("OnPlay", "marker,Kill,,0,-1"),
            ]),
            entity(&[("classname", "info_target"), ("targetname", "marker")]),
        ],
        sounds,
    )
}

fn current(sim: &Sim) -> Option<usize> {
    match sim.app.world().resource::<ScapeState>().current {
        Some((_, Source::Emitter(i))) => Some(i),
        _ => None,
    }
}

fn input(sim: &mut Sim, target: &str, input: &str) {
    let mut logic = sim.app.world_mut().resource_mut::<Logic>();
    logic.world.queue_input(target, input, Value::Void, 0.0, None);
}

#[test]
fn soundscapes_follow_enable_and_disable() {
    let mut sim = Sim::new((MapPlugin::new(soundscape_map()), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.app.world_mut().spawn((
        SoundListener,
        Transform::from_translation(to_engine(Vec3::new(450.0, 0.0, 64.0))),
        GlobalTransform::from_translation(to_engine(Vec3::new(450.0, 0.0, 64.0))),
    ));
    sim.ticks(4);
    // "b" is nearer but disabled: "a" plays, with its room settings.
    assert_eq!(current(&sim), Some(0));
    {
        let room = sim.app.world().resource::<RoomDsp>();
        assert_eq!(room.preset, 2);
        assert_eq!(room.scape_volume, Some(0.5));
        assert!((room.level() - 0.5).abs() < 1e-6, "dsp_volume wins over the user's");
        assert_eq!(room.player, Some(1));
        let state = sim.app.world().resource::<ScapeState>();
        assert_eq!(state.mixer.as_deref(), Some("Default_Mix"));
        let readout = mashup::map::soundscape::readout(state, room);
        assert!(readout.contains("mixer Default_Mix"), "{readout}");
    }
    assert!(sim.app.world().resource::<Logic>().world.find("marker").is_some());
    // Enabled, the nearer one takes over and fires OnPlay.
    input(&mut sim, "b", "Enable");
    sim.ticks(4);
    assert_eq!(current(&sim), Some(1));
    {
        let room = sim.app.world().resource::<RoomDsp>();
        assert_eq!(room.preset, 3);
        assert_eq!(room.scape_volume, None, "no dsp_volume: the user's again");
        assert_eq!(room.player, Some(1), "no dsp_player: unchanged");
    }
    assert!(sim.app.world().resource::<ScapeState>().mixer.is_none());
    assert!(
        sim.app.world().resource::<Logic>().world.find("marker").is_none(),
        "OnPlay fired"
    );
    // Disabled while current: dropped, and the other qualifies at once.
    input(&mut sim, "b", "Disable");
    sim.ticks(4);
    assert_eq!(current(&sim), Some(0));
    // Disabling the only qualifying one keeps it playing (nothing else
    // qualifies).
    input(&mut sim, "a", "Disable");
    sim.ticks(4);
    assert_eq!(current(&sim), Some(0));
    // A Toggle brings "b" back.
    input(&mut sim, "b", "ToggleEnabled");
    sim.ticks(4);
    assert_eq!(current(&sim), Some(1));
}

/// Several sounds for one source's channel in one frame (a network
/// client gets them in a burst after a stall): the last one plays, each
/// one before it is stopped once. They were despawned twice, a Bevy
/// warning each time (hundreds in a 17-minute live soak; multiplayer.md,
/// "Soak").
#[test]
fn a_burst_on_one_channel_replaces_cleanly() {
    let mut s = MapSounds::default();
    s.clips.push(clip());
    s.entries.insert(
        "weapon.shot".into(),
        MapSoundEntry {
            waves: vec![0],
            volume: Interval::fixed(1.0),
            pitch: Interval::fixed(100.0),
            level: SoundLevel::Db(75.0),
            channel: 1,
            dry: false,
        },
    );
    let mut sim = Sim::new((MapPlugin::new(map(Vec::new(), s)), SourceMovementPlugin));
    sim.app.set_error_handler(bevy::ecs::error::panic);
    sim.app.init_asset::<mashup::map::live_sound::LiveClip>();
    sim.ticks(2);
    let shooter = sim.app.world_mut().spawn(Transform::default()).id();
    let shot = || mashup::map::PlaySound {
        entry: "Weapon.Shot".into(),
        at: Some(Vec3::ZERO),
        volume: None,
        pitch: None,
        source: Some(shooter),
        channel: Some(1),
    };
    let playing = |sim: &mut Sim| {
        let w = sim.app.world_mut();
        w.query::<&AudioPlayer<mashup::map::live_sound::LiveClip>>().iter(w).count()
    };
    for burst in [1, 3, 2, 4] {
        for _ in 0..burst {
            sim.app.world_mut().write_message(shot());
        }
        sim.app.update();
        assert_eq!(playing(&mut sim), 1, "one sound on the channel after a burst of {burst}");
    }
}
