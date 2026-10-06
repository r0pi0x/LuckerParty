//! What first-person view models play, driven by weapon events
//! (specs/cs_source/weapons.md 3.3–3.8), on the greybox map with stand-in
//! view models (one bone, sequences named and timed like CS:S's). The real
//! models are checked in tests/map_de_dust2.rs.

use std::sync::Arc;

use bevy::prelude::*;
use mashup::{
    games::cs_source::{
        TICK_INTERVAL,
        view_anim::ViewAnimPlugin,
        weapons::{AK47, CsWeaponsPlugin, KNIFE},
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    map::{
        DriveAnimation, MapModel, MapViewModel, ViewAnimator, ViewModelEvent, ViewModelEventKind, ViewModels,
        anim::{AnimEvent, AnimSet, Animation, Sequence},
    },
    movement::placeholder,
    weapon::{Inventory, Magazine},
};

/// (name, activity, frames, fps, looping)
type Seq = (&'static str, &'static str, usize, f32, bool);

fn set(seqs: &[Seq]) -> AnimSet {
    let defaults = vec![(Quat::IDENTITY, Vec3::ZERO)];
    let mut set = AnimSet {
        bases: vec![defaults.clone()],
        defaults,
        ..default()
    };
    for (i, (name, activity, frames, fps, looping)) in seqs.iter().enumerate() {
        set.animations.push(Animation {
            name: format!("a_{name}"),
            fps: *fps,
            frames: *frames,
            ..default()
        });
        set.sequences.push(Sequence {
            name: name.to_string(),
            activity: activity.to_string(),
            activity_weight: 1,
            looping: *looping,
            fade_in: 0.2,
            fade_out: 0.2,
            grid: (1, 1),
            anims: vec![i],
            bone_weights: vec![1.0],
            // As in v_rif_ak47.mdl (dump --sequences).
            events: if name.starts_with("ak47_fire") {
                vec![
                    AnimEvent {
                        cycle: 0.0,
                        event: 5001,
                        name: String::new(),
                        options: "1".into(),
                    },
                    AnimEvent {
                        cycle: 0.0,
                        event: 0,
                        name: "AE_CLIENT_EFFECT_ATTACH".into(),
                        options: "EjectBrass_762Nato 2 150".into(),
                    },
                ]
            } else {
                Vec::new()
            },
            ..default()
        });
    }
    set
}

fn view_model(key: &str, seqs: &[Seq]) -> MapViewModel {
    MapViewModel {
        key: key.to_string(),
        model: MapModel::default(),
        bones: Vec::new(),
        root: Transform::default(),
        animations: Some(Arc::new(set(seqs))),
        right_handed: false,
        allow_flipping: true,
        attachments: Vec::new(),
        light_origin: Vec3::ZERO,
    }
}

/// View-model effects sent so far.
#[derive(Resource, Default)]
struct Effects(Vec<ViewModelEvent>);

fn collect(mut events: MessageReader<ViewModelEvent>, mut out: ResMut<Effects>) {
    out.0.extend(events.read().cloned());
}

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin, ViewAnimPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app
        .init_resource::<Effects>()
        .add_systems(Update, collect.after(DriveAnimation));
    // v_rif_ak47 and v_knife_t's sequences as in the install (dump
    // --sequences; durations match the spec's table).
    sim.app.insert_resource(ViewModels(Arc::new(vec![
        view_model(
            AK47,
            &[
                ("ak47_idle", "ACT_VM_IDLE", 16, 30.0, true),
                ("ak47_fire1", "ACT_VM_PRIMARYATTACK", 16, 20.0, false),
                ("ak47_fire2", "ACT_VM_PRIMARYATTACK", 16, 20.0, false),
                ("ak47_fire3", "ACT_VM_PRIMARYATTACK", 16, 20.0, false),
                ("ak47_draw", "ACT_VM_DRAW", 31, 30.0, false),
                ("ak47_reload", "ACT_VM_RELOAD", 91, 37.0, false),
            ],
        ),
        view_model(
            KNIFE,
            &[
                ("idle", "ACT_VM_IDLE", 151, 12.0, false),
                ("draw", "ACT_VM_DRAW", 46, 45.0, false),
                ("stab", "ACT_VM_HITCENTER", 46, 35.0, false),
                ("stab_miss", "ACT_VM_MISSCENTER", 61, 40.0, false),
                ("midslash1", "ACT_VM_HITCENTER", 66, 55.0, false),
                ("midslash2", "ACT_VM_HITCENTER", 66, 55.0, false),
            ],
        ),
    ])));
    sim
}

fn view(sim: &Sim, p: Entity) -> (Option<String>, Option<String>, Option<String>) {
    let v = sim.app.world().get::<ViewAnimator>(p).expect("a view animator");
    (v.key.clone(), v.activity().map(str::to_string), v.sequence().map(str::to_string))
}

fn activity(sim: &Sim, p: Entity) -> String {
    view(sim, p).1.unwrap_or_default()
}

#[test]
fn deploy_fire_reload_and_idle() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.ticks(3);
    // The AK-47 is drawn first.
    let (key, act, _) = view(&sim, p);
    assert_eq!(key.as_deref(), Some(AK47));
    assert_eq!(act.as_deref(), Some("ACT_VM_DRAW"));
    // Draw (1.0 s) plays out, then idle.
    sim.seconds(0.9);
    assert_eq!(activity(&sim, p), "ACT_VM_DRAW");
    sim.seconds(0.2);
    assert_eq!(activity(&sim, p), "ACT_VM_IDLE");

    // Firing switches to a fire sequence, restarted by each shot.
    let clip = sim.app.world().get::<Magazine>(active(&sim, p)).unwrap().clip;
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    assert_eq!(sim.app.world().get::<Magazine>(active(&sim, p)).unwrap().clip, clip - 1);
    let (_, act, seq) = view(&sim, p);
    assert_eq!(act.as_deref(), Some("ACT_VM_PRIMARYATTACK"));
    assert!(seq.unwrap().starts_with("ak47_fire"));
    // It holds its last frame after 0.75 s and idles TimeToIdle (1.9 s)
    // after the shot.
    sim.seconds(1.5);
    let v = sim.app.world().get::<ViewAnimator>(p).unwrap();
    assert_eq!(v.activity(), Some("ACT_VM_PRIMARYATTACK"));
    assert_eq!(v.animator.as_ref().unwrap().cycle, 1.0);
    sim.seconds(0.5);
    assert_eq!(activity(&sim, p), "ACT_VM_IDLE");

    // Several shots pick among the three fire sequences.
    let mut seen = std::collections::HashSet::new();
    for _ in 0..12 {
        sim.intent(p).fire = true;
        sim.ticks(1);
        sim.intent(p).fire = false;
        seen.insert(view(&sim, p).2.unwrap());
        sim.seconds(0.2);
    }
    assert!(seen.len() >= 2, "{seen:?}");

    // Reload.
    sim.seconds(2.0);
    sim.intent(p).reload = true;
    sim.ticks(1);
    sim.intent(p).reload = false;
    assert_eq!(activity(&sim, p), "ACT_VM_RELOAD");
    sim.seconds(2.0);
    assert_eq!(activity(&sim, p), "ACT_VM_RELOAD");
    sim.seconds(2.5);
    assert_eq!(activity(&sim, p), "ACT_VM_IDLE");
}

#[test]
fn knife_draws_slashes_and_stabs() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.ticks(2);
    sim.intent(p).select = Some(2);
    sim.ticks(1);
    sim.intent(p).select = None;
    sim.ticks(1);
    let (key, act, _) = view(&sim, p);
    assert_eq!(key.as_deref(), Some(KNIFE));
    assert_eq!(act.as_deref(), Some("ACT_VM_DRAW"));
    sim.seconds(1.1);
    assert_eq!(activity(&sim, p), "ACT_VM_IDLE");
    // A slash at nothing: CS:S has no slash-miss sequence, so a slash.
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    assert!(view(&sim, p).2.unwrap().starts_with("midslash"));
    sim.seconds(1.5);
    assert_eq!(activity(&sim, p), "ACT_VM_IDLE");
    // A stab at nothing.
    sim.intent(p).secondary = true;
    sim.ticks(1);
    sim.intent(p).secondary = false;
    assert_eq!(view(&sim, p).2.as_deref(), Some("stab_miss"));
}

#[test]
fn the_dead_show_no_view_model() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.ticks(3);
    assert!(view(&sim, p).0.is_some());
    sim.app.world_mut().get_mut::<mashup::core::Health>(p).unwrap().current = 0.0;
    sim.ticks(1);
    assert_eq!(view(&sim, p).0, None);
}

/// Spec view_models.md E1: every shot restarts a fire sequence and re-arms
/// its events, so the muzzle flash and brass fire on both of two quick
/// shots, even when the same fire sequence is picked twice.
#[test]
fn every_shot_fires_the_flash_and_brass_events() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.seconds(1.2);
    assert_eq!(activity(&sim, p), "ACT_VM_IDLE");
    sim.app.world_mut().resource_mut::<Effects>().0.clear();
    for _ in 0..2 {
        sim.intent(p).fire = true;
        sim.ticks(1);
        sim.intent(p).fire = false;
        sim.seconds(0.1);
    }
    let effects = &sim.app.world().resource::<Effects>().0;
    let count = |kind: ViewModelEventKind| effects.iter().filter(|e| e.owner == p && e.kind == kind).count();
    let flashes = count(ViewModelEventKind::MuzzleFlash { attachment: 0 });
    let brass = count(ViewModelEventKind::EjectBrass {
        attachment: 1,
        shell: "762Nato".into(),
        speed: 150.0,
    });
    assert_eq!((flashes, brass), (2, 2), "{effects:?}");
    // Idling fires nothing.
    sim.app.world_mut().resource_mut::<Effects>().0.clear();
    sim.seconds(3.0);
    assert!(sim.app.world().resource::<Effects>().0.is_empty());
}

/// Events between two cycles: the start of a (re)started sequence counts.
#[test]
fn events_fire_when_the_cycle_passes_them() {
    let seq = Sequence {
        events: [0.0, 0.5, 1.0]
            .map(|cycle| AnimEvent { cycle, ..default() })
            .to_vec(),
        ..default()
    };
    let cycles = |from, to| seq.events_between(from, to).map(|e| e.cycle).collect::<Vec<f32>>();
    assert_eq!(cycles(None, 0.1), [0.0]);
    assert!(cycles(Some(0.0), 0.1).is_empty());
    assert_eq!(cycles(Some(0.4), 0.6), [0.5]);
    assert_eq!(cycles(Some(0.6), 1.0), [1.0]);
    // A loop wrapped from 0.9 to 0.1.
    assert_eq!(cycles(Some(0.9), 0.1), [0.0, 1.0]);
}

fn active(sim: &Sim, p: Entity) -> Entity {
    sim.app.world().get::<Inventory>(p).unwrap().active.unwrap()
}
