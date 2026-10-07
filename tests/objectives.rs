//! Bomb and hostage objectives (specs/cs_source/objectives.md, test cases
//! O1-O42) on the greybox map with CS:S's weapons, at CS:S's tick.

use avian3d::prelude::Position;
use bevy::prelude::*;
use mashup::{
    console::Console,
    core::{Damage, Health, Hitgroup, SimTick, Team},
    games::cs_source::{TICK_INTERVAL, objectives::C4, weapons::CsWeaponsPlugin},
    greybox::GreyboxMapPlugin,
    harness::Sim,
    movement::placeholder,
    objectives::{
        HostageSpawn, MapObjectives, ObjectiveEvent, Zone,
        bomb::{BombOutcome, BombState, C4 as C4Part, LooseKit, PlantedBomb, carried_bomb},
        hostages::{Hostage, HostageTally},
    },
    rules::rounds::{Phase, RoundEndReason, RoundEnded, RoundState},
    weapon::{
        Inventory, Weapon,
        drop::Loose,
        economy::{BuyWindow, DefuseKit, Money, buy},
        give,
    },
};

/// Every objective event with the tick it happened on.
#[derive(Resource, Default)]
struct Log(Vec<(u64, ObjectiveEvent)>, Vec<RoundEnded>);

fn record(
    mut events: MessageReader<ObjectiveEvent>,
    mut ended: MessageReader<RoundEnded>,
    tick: Res<SimTick>,
    mut log: ResMut<Log>,
) {
    for e in events.read() {
        log.0.push((tick.0, e.clone()));
    }
    log.1.extend(ended.read().cloned());
}

/// The bomb target: a box around the greybox's first spawn.
const ZONE: (Vec3, Vec3) = (Vec3::new(-3.0, -1.0, 8.0), Vec3::new(3.0, 3.0, 16.0));

fn sim(objectives: MapObjectives) -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app.init_resource::<Log>();
    sim.app.add_systems(FixedLast, record);
    sim.app.insert_resource(objectives);
    sim
}

fn bomb_map() -> MapObjectives {
    MapObjectives {
        bomb_targets: vec![Zone::engine_box(ZONE.0, ZONE.1)],
        ..default()
    }
}

fn character(sim: &mut Sim, at: Vec3, team: u8) -> Entity {
    let e = sim.spawn_character(at, placeholder::ID);
    sim.app.world_mut().entity_mut(e).insert(Team(team));
    e
}

fn teleport(sim: &mut Sim, e: Entity, at: Vec3) {
    let w = sim.app.world_mut();
    w.get_mut::<Transform>(e).unwrap().translation = at;
    if let Some(mut p) = w.get_mut::<Position>(e) {
        p.0 = at;
    }
}

fn events(sim: &Sim) -> Vec<(u64, ObjectiveEvent)> {
    sim.app.world().resource::<Log>().0.clone()
}

fn first(sim: &Sim, f: impl Fn(&ObjectiveEvent) -> bool) -> Option<u64> {
    events(sim).into_iter().find(|(_, e)| f(e)).map(|(t, _)| t)
}

fn bombs(sim: &mut Sim) -> Vec<(Entity, PlantedBomb, Vec3)> {
    let w = sim.app.world_mut();
    w.query::<(Entity, &PlantedBomb, &Transform)>()
        .iter(w)
        .map(|(e, b, t)| (e, b.clone(), t.translation))
        .collect()
}

fn money(sim: &Sim, e: Entity) -> u32 {
    sim.app.world().get::<Money>(e).unwrap().0
}

/// A terrorist in the target with the bomb drawn.
fn carrier(sim: &mut Sim) -> Entity {
    let t = character(sim, Vec3::new(0.0, 1.0, 12.0), 1);
    sim.ticks(2);
    give(sim.app.world_mut(), t, C4).unwrap();
    sim.seconds(1.2);
    let active = sim.app.world().get::<Inventory>(t).unwrap().active.unwrap();
    assert!(sim.app.world().get::<C4Part>(active).is_some(), "the bomb is drawn");
    t
}

/// Hold fire until the bomb is planted; returns the tick it began.
fn plant(sim: &mut Sim, t: Entity) -> u64 {
    sim.intent(t).fire = true;
    sim.ticks(1);
    let begun = first(sim, |e| matches!(e, ObjectiveEvent::BeginPlant { .. })).expect("arming began");
    sim.ticks(205);
    sim.intent(t).fire = false;
    sim.ticks(1);
    begun
}

/// Aim `who` at a point from where it stands (its eye).
fn look_at(sim: &mut Sim, who: Entity, at: Vec3) {
    let eye = sim.position(who) + sim.state(who).eye_offset;
    let d = at - eye;
    let mut i = sim.intent(who);
    i.yaw = (-d.x).atan2(-d.z);
    i.pitch = d.y.atan2(d.xz().length());
}

#[test]
fn arming_takes_three_seconds_and_starts_over_when_let_go() {
    let mut sim = sim(bomb_map());
    let t = carrier(&mut sim);
    // O5: outside the target nothing starts.
    teleport(&mut sim, t, Vec3::new(10.0, 1.0, 12.0));
    sim.ticks(3);
    sim.intent(t).fire = true;
    sim.ticks(2);
    sim.intent(t).fire = false;
    sim.ticks(2);
    let refused = events(&sim).into_iter().any(|(_, e)| {
        matches!(e, ObjectiveEvent::PlantRefused { reason, .. } if reason == "C4 must be planted at a bomb site.")
    });
    assert!(refused);
    assert!(first(&sim, |e| matches!(e, ObjectiveEvent::BeginPlant { .. })).is_none());
    // O7: let go at tick 190: no bomb.
    teleport(&mut sim, t, Vec3::new(0.0, 1.0, 12.0));
    sim.ticks(3);
    sim.intent(t).fire = true;
    sim.ticks(190);
    sim.intent(t).fire = false;
    sim.ticks(10);
    assert!(first(&sim, |e| matches!(e, ObjectiveEvent::AbortPlant { .. })).is_some());
    assert!(bombs(&mut sim).is_empty());
    // O6: hold it: planted 200 ticks (3.000 s) after it began.
    sim.app.world_mut().resource_mut::<Log>().0.clear();
    let begun = plant(&mut sim, t);
    let planted = first(&sim, |e| matches!(e, ObjectiveEvent::Planted { .. })).expect("planted");
    assert_eq!(planted - begun, 200);
    assert!(carried_bomb(sim.app.world(), t).is_none(), "the bomb left his hands");
    let (_, b, at) = bombs(&mut sim).pop().unwrap();
    assert!((b.explode_at - b.planted_at - 45.0).abs() < 1e-6);
    assert!((at - sim.position(t)).xz().length() < 0.01, "at his feet");
    assert!(sim.app.world().resource::<BombState>().planted.is_some());
}

/// A planted bomb and a defender beside it, looking at it.
fn planted_with_defender(kit: bool) -> (Sim, Entity, Entity, u64) {
    let mut sim = sim(bomb_map());
    let t = carrier(&mut sim);
    plant(&mut sim, t);
    let planted = first(&sim, |e| matches!(e, ObjectiveEvent::Planted { .. })).unwrap();
    teleport(&mut sim, t, Vec3::new(20.0, 1.0, 30.0));
    let (_, _, at) = bombs(&mut sim).pop().unwrap();
    let ct = character(&mut sim, at + Vec3::new(0.0, 1.0, 1.0), 2);
    if kit {
        sim.app.world_mut().entity_mut(ct).insert(DefuseKit);
    }
    sim.ticks(2);
    look_at(&mut sim, ct, at + Vec3::Y * 0.05);
    (sim, t, ct, planted)
}

fn tick(sim: &Sim) -> u64 {
    sim.app.world().resource::<SimTick>().0
}

fn wait_until(sim: &mut Sim, tick_no: u64) {
    let now = tick(sim);
    assert!(tick_no >= now, "already past tick {tick_no}");
    sim.ticks(tick_no - now);
}

#[test]
fn defusing_with_a_kit_takes_five_seconds() {
    // O14: start at 30.0 s, defused on the first tick at or after 5 s.
    let (mut sim, _, ct, planted) = planted_with_defender(true);
    wait_until(&mut sim, planted + 2000 - 1);
    sim.intent(ct).use_key = true;
    sim.ticks(1);
    let begun = first(&sim, |e| matches!(e, ObjectiveEvent::BeginDefuse { kit: true, .. })).expect("defuse began");
    assert_eq!(begun - planted, 2000);
    sim.ticks(340);
    let done = first(&sim, |e| matches!(e, ObjectiveEvent::Defused { .. })).expect("defused");
    assert_eq!(done - begun, 334);
    assert_eq!(
        sim.app.world().resource::<BombState>().outcome,
        Some(BombOutcome::Defused)
    );
    // It no longer explodes.
    sim.seconds(12.0);
    assert!(first(&sim, |e| matches!(e, ObjectiveEvent::Exploded { .. })).is_none());
}

#[test]
fn defusing_without_a_kit_takes_ten_and_letting_go_starts_over() {
    // O15.
    let (mut sim, _, ct, planted) = planted_with_defender(false);
    wait_until(&mut sim, planted + 2000 - 1);
    sim.intent(ct).use_key = true;
    // O17's interrupt: let go after 4 s, again 0.5 s later.
    sim.seconds(4.0);
    sim.intent(ct).use_key = false;
    sim.seconds(0.5);
    assert!(first(&sim, |e| matches!(e, ObjectiveEvent::AbortDefuse { .. })).is_some());
    let again = tick(&sim);
    sim.intent(ct).use_key = true;
    sim.seconds(10.2);
    let done = first(&sim, |e| matches!(e, ObjectiveEvent::Defused { .. })).expect("defused");
    assert_eq!(done - (again + 1), 667, "the full 10 s again");
}

#[test]
fn a_defuse_ending_after_the_timer_loses_to_the_explosion() {
    // O16: no kit, starting 36 s in: it would end at 46.
    let (mut sim, _, ct, planted) = planted_with_defender(false);
    let far = character(&mut sim, Vec3::new(30.0, 1.0, -30.0), 2);
    wait_until(&mut sim, planted + 2400 - 1);
    sim.intent(ct).use_key = true;
    sim.seconds(9.5);
    let exploded = first(&sim, |e| matches!(e, ObjectiveEvent::Exploded { .. })).expect("exploded");
    assert_eq!(exploded - planted, 3000, "45 s after the plant");
    assert!(first(&sim, |e| matches!(e, ObjectiveEvent::Defused { .. })).is_none());
    sim.ticks(3);
    // The blast: the defender beside it dies; one 2000 units off (past
    // 3.5 x 500) takes nothing.
    assert!(sim.app.world().get::<Health>(ct).unwrap().current <= 0.0);
    let far_health = sim.app.world().get::<Health>(far).unwrap().current;
    assert!(far_health > 0.8, "far defender took {far_health}");
    assert!(bombs(&mut sim).is_empty(), "the bomb is gone");
}

#[test]
fn another_defender_is_told_it_is_taken() {
    // O18.
    let (mut sim, _, ct, _) = planted_with_defender(true);
    let (_, _, at) = bombs(&mut sim).pop().unwrap();
    let ct2 = character(&mut sim, at + Vec3::new(1.0, 1.0, 0.0), 2);
    sim.ticks(2);
    look_at(&mut sim, ct2, at + Vec3::Y * 0.05);
    sim.intent(ct).use_key = true;
    sim.ticks(5);
    sim.intent(ct2).use_key = true;
    sim.ticks(5);
    let told = events(&sim).into_iter().any(|(_, e)| {
        matches!(e, ObjectiveEvent::DefuseRefused { who, reason } if who == ct2 && reason == "The bomb is already being defused.")
    });
    assert!(told);
    sim.ticks(340);
    assert!(
        events(&sim)
            .iter()
            .any(|(_, e)| *e == ObjectiveEvent::Defused { who: ct })
    );
}

fn rounds(sim: &mut Sim) {
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_freezetime 1; mp_roundtime 1; mashup_rounds 1");
    sim.ticks(3);
}

fn phase(sim: &Sim) -> Phase {
    sim.app.world().resource::<RoundState>().phase
}

fn ended(sim: &Sim) -> Option<RoundEnded> {
    sim.app.world().resource::<Log>().1.last().cloned()
}

fn bomb_carriers(sim: &mut Sim) -> Vec<Entity> {
    let w = sim.app.world_mut();
    w.query::<(&Weapon, &C4Part)>()
        .iter(w)
        .filter_map(|(w, _)| w.owner)
        .collect()
}

#[test]
fn a_round_gives_one_terrorist_the_bomb_and_the_explosion_wins() {
    let mut sim = sim(bomb_map());
    let ts: Vec<Entity> = (0..3)
        .map(|i| character(&mut sim, Vec3::new(i as f32, 1.0, 12.0), 1))
        .collect();
    let ct = character(&mut sim, Vec3::new(5.0, 1.0, 12.0), 2);
    rounds(&mut sim);
    // O1: exactly one terrorist, no CT.
    let carriers = bomb_carriers(&mut sim);
    assert_eq!(carriers.len(), 1);
    let t = carriers[0];
    assert!(ts.contains(&t));
    assert!(first(&sim, |e| *e == ObjectiveEvent::GotBomb { who: t }).is_some());
    // Not drawn: a pistol stays in hand.
    let active = sim.app.world().get::<Inventory>(t).unwrap().active;
    sim.seconds(1.3);
    assert!(matches!(phase(&sim), Phase::Live { .. }));
    let active_now = sim.app.world().get::<Inventory>(t).unwrap().active;
    assert!(
        active_now.is_none_or(|a| sim.app.world().get::<C4Part>(a).is_none()),
        "{active:?}"
    );
    let start = money(&sim, t);
    // Into the target, draw it (slot 5), plant.
    teleport(&mut sim, t, Vec3::new(0.0, 1.0, 12.0));
    for &o in ts.iter().filter(|o| **o != t) {
        teleport(&mut sim, o, Vec3::new(20.0, 1.0, -30.0));
    }
    teleport(&mut sim, ct, Vec3::new(-30.0, 1.0, -30.0));
    sim.intent(t).select = Some(4);
    sim.seconds(1.2);
    sim.intent(t).select = None;
    plant(&mut sim, t);
    assert_eq!(money(&sim, t), start + 300, "planter's reward (hyp.)");
    // O19: every terrorist dead: the round goes on.
    for &o in &ts {
        sim.app.world_mut().write_message(Damage {
            force: Vec3::ZERO,
            target: o,
            attacker: Some(ct),
            amount: 10.0,
            point: Vec3::ZERO,
            dir: Vec3::X,
            hitgroup: Hitgroup::Chest,
            kind: default(),
            weapon: None,
        });
    }
    // O23: and the clock (1 min) runs out: still going.
    sim.seconds(44.0);
    assert!(matches!(phase(&sim), Phase::Live { .. }), "{:?}", phase(&sim));
    let ct_before = money(&sim, ct);
    let t_before = money(&sim, t);
    sim.seconds(1.5);
    let end = ended(&sim).expect("round over");
    assert_eq!(end.winner, Some(Team(1)));
    assert_eq!(end.reason, RoundEndReason::TargetBombed);
    // O13: terrorists +3500; the CT the loss bonus (and +300 per kill).
    assert_eq!(money(&sim, t), t_before + 3500);
    assert_eq!(money(&sim, ct), ct_before + 1400);
}

#[test]
fn a_defused_bomb_wins_for_the_defenders_and_pays_the_planters_too() {
    let mut sim = sim(bomb_map());
    let t = character(&mut sim, Vec3::new(0.0, 1.0, 12.0), 1);
    let ct = character(&mut sim, Vec3::new(5.0, 1.0, 12.0), 2);
    rounds(&mut sim);
    sim.seconds(1.3);
    teleport(&mut sim, t, Vec3::new(0.0, 1.0, 12.0));
    teleport(&mut sim, ct, Vec3::new(-30.0, 1.0, -30.0));
    sim.intent(t).select = Some(4);
    sim.seconds(1.2);
    sim.intent(t).select = None;
    plant(&mut sim, t);
    teleport(&mut sim, t, Vec3::new(20.0, 1.0, -30.0));
    let (_, _, at) = bombs(&mut sim).pop().unwrap();
    teleport(&mut sim, ct, at + Vec3::new(0.0, 1.0, 1.0));
    sim.ticks(2);
    look_at(&mut sim, ct, at + Vec3::Y * 0.05);
    let (t0, c0) = (money(&sim, t), money(&sim, ct));
    sim.intent(ct).use_key = true;
    sim.seconds(10.2);
    let end = ended(&sim).expect("round over");
    assert_eq!((end.winner, end.reason), (Some(Team(2)), RoundEndReason::BombDefused));
    // O14's money: CTs +3500, terrorists the loss bonus + 800.
    assert_eq!(money(&sim, ct), c0 + 3500);
    assert_eq!(money(&sim, t), t0 + 1400 + 800);
}

#[test]
fn bomb_maps_end_on_time_for_the_defenders_and_on_elimination() {
    // O21.
    let mut sim = sim(bomb_map());
    let t = character(&mut sim, Vec3::new(0.0, 1.0, 12.0), 1);
    let ct = character(&mut sim, Vec3::new(5.0, 1.0, 12.0), 2);
    rounds(&mut sim);
    let c0 = money(&sim, ct);
    sim.seconds(62.0);
    let end = ended(&sim).expect("round over");
    assert_eq!((end.winner, end.reason), (Some(Team(2)), RoundEndReason::TargetSaved));
    assert_eq!(money(&sim, ct), c0 + 3250);
    // O20: all CTs dead after a plant: terrorists at once.
    sim.seconds(5.5);
    sim.seconds(1.3);
    teleport(&mut sim, t, Vec3::new(0.0, 1.0, 12.0));
    teleport(&mut sim, ct, Vec3::new(-30.0, 1.0, -30.0));
    sim.intent(t).select = Some(4);
    sim.seconds(1.2);
    sim.intent(t).select = None;
    plant(&mut sim, t);
    sim.app.world_mut().write_message(Damage {
        force: Vec3::ZERO,
        target: ct,
        attacker: Some(t),
        amount: 10.0,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: Hitgroup::Chest,
        kind: default(),
        weapon: None,
    });
    sim.ticks(3);
    let end = ended(&sim).expect("round over");
    assert_eq!((end.winner, end.reason), (Some(Team(1)), RoundEndReason::Eliminated));
}

#[test]
fn the_carrier_drops_the_bomb_and_only_terrorists_take_it() {
    let mut sim = sim(bomb_map());
    let t = carrier(&mut sim);
    // O10: knife (or anything) in hand, killed: the bomb lies there.
    sim.app.world_mut().write_message(Damage {
        force: Vec3::ZERO,
        target: t,
        attacker: None,
        amount: 10.0,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: Hitgroup::Chest,
        kind: default(),
        weapon: None,
    });
    sim.ticks(3);
    let loose: Vec<(Entity, Vec3)> = {
        let w = sim.app.world_mut();
        w.query::<(&Loose, &Transform)>()
            .iter(w)
            .filter(|(l, _)| w.get::<C4Part>(l.weapon).is_some())
            .map(|(l, t)| (l.weapon, t.translation))
            .collect()
    };
    assert_eq!(loose.len(), 1, "the bomb on the ground");
    assert!(first(&sim, |e| *e == ObjectiveEvent::DroppedBomb { who: t }).is_some());
    let (bomb, at) = loose[0];
    // O11: a CT walks over it: nothing.
    let ct = character(&mut sim, at + Vec3::Y * 0.5, 2);
    sim.ticks(20);
    assert!(sim.app.world().get::<Weapon>(bomb).unwrap().owner.is_none());
    teleport(&mut sim, ct, Vec3::new(-20.0, 1.0, 0.0));
    // O12: a terrorist takes it, into its slot, his hand unchanged.
    let t2 = character(&mut sim, Vec3::new(-20.0, 1.0, 20.0), 1);
    sim.ticks(3);
    let held = sim.app.world().get::<Inventory>(t2).unwrap().active;
    teleport(&mut sim, t2, at + Vec3::Y * 0.5);
    sim.ticks(20);
    assert_eq!(sim.app.world().get::<Weapon>(bomb).unwrap().owner, Some(t2));
    assert_eq!(sim.app.world().get::<Inventory>(t2).unwrap().active, held);
    assert!(first(&sim, |e| *e == ObjectiveEvent::PickedUpBomb { who: t2 }).is_some());
}

#[test]
fn defusal_kits_are_bought_by_defenders_and_dropped_on_death() {
    let mut sim = sim(bomb_map());
    let ct = character(&mut sim, Vec3::new(0.0, 1.0, 12.0), 2);
    let t = character(&mut sim, Vec3::new(4.0, 1.0, 12.0), 1);
    sim.ticks(2);
    sim.app.world_mut().insert_resource(BuyWindow::default());
    // O32.
    sim.app.world_mut().entity_mut(ct).insert(Money(800));
    sim.app.world_mut().entity_mut(t).insert(Money(800));
    buy(sim.app.world_mut(), ct, "defuser").unwrap();
    assert_eq!(money(&sim, ct), 600);
    assert!(sim.app.world().get::<DefuseKit>(ct).is_some());
    assert!(buy(sim.app.world_mut(), ct, "defuser").is_err());
    // O33.
    assert!(buy(sim.app.world_mut(), t, "defuser").is_err());
    assert_eq!(money(&sim, t), 800);
    // O34: dies, the kit lies there; a CT with a kit leaves it, one
    // without takes it.
    sim.app.world_mut().write_message(Damage {
        force: Vec3::ZERO,
        target: ct,
        attacker: None,
        amount: 10.0,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: Hitgroup::Chest,
        kind: default(),
        weapon: None,
    });
    sim.ticks(3);
    let kit_at = {
        let w = sim.app.world_mut();
        let v: Vec<Vec3> = w
            .query_filtered::<&Transform, With<LooseKit>>()
            .iter(w)
            .map(|t| t.translation)
            .collect();
        assert_eq!(v.len(), 1);
        v[0]
    };
    assert!(sim.app.world().get::<DefuseKit>(ct).is_none());
    let with = character(&mut sim, Vec3::new(10.0, 1.0, 0.0), 2);
    sim.app.world_mut().entity_mut(with).insert(DefuseKit);
    let without = character(&mut sim, Vec3::new(14.0, 1.0, 0.0), 2);
    teleport(&mut sim, with, kit_at);
    sim.ticks(5);
    assert!(sim.app.world_mut().query::<&LooseKit>().iter(sim.app.world()).count() == 1);
    teleport(&mut sim, with, Vec3::new(10.0, 1.0, 0.0));
    teleport(&mut sim, without, kit_at);
    sim.ticks(5);
    assert!(sim.app.world().get::<DefuseKit>(without).is_some());
    assert_eq!(
        sim.app.world_mut().query::<&LooseKit>().iter(sim.app.world()).count(),
        0
    );
}

/// One hostage near the first spawn; a rescue zone off to the east.
fn hostage_map() -> MapObjectives {
    MapObjectives {
        rescue_zones: vec![Zone::engine_box(Vec3::new(8.0, -1.0, -2.0), Vec3::new(18.0, 3.0, 8.0))],
        hostages: vec![HostageSpawn {
            feet: Vec3::new(0.0, 0.0, 5.0),
            yaw: 0.0,
            kind: None,
        }],
        ..default()
    }
}

fn hostages(sim: &mut Sim) -> Vec<(Entity, Hostage, Vec3)> {
    let w = sim.app.world_mut();
    w.query::<(Entity, &Hostage, &Transform)>()
        .iter(w)
        .map(|(e, h, t)| (e, h.clone(), t.translation))
        .collect()
}

#[test]
fn a_hostage_follows_its_leader_to_the_rescue_zone() {
    let mut sim = sim(hostage_map());
    let t = character(&mut sim, Vec3::new(0.0, 1.0, 12.0), 1);
    let ct = character(&mut sim, Vec3::new(5.0, 1.0, 12.0), 2);
    rounds(&mut sim);
    sim.seconds(1.3);
    let (h, _, at) = hostages(&mut sim).pop().expect("hostage spawned");
    assert!(sim.app.world().get::<Team>(h).is_none(), "on no team");
    // O37: a terrorist can't move it.
    teleport(&mut sim, t, Vec3::new(0.0, 1.0, 6.5));
    teleport(&mut sim, ct, Vec3::new(-20.0, 1.0, 20.0));
    sim.ticks(2);
    look_at(&mut sim, t, at);
    sim.intent(t).use_key = true;
    sim.ticks(2);
    sim.intent(t).use_key = false;
    assert!(first(&sim, |e| *e == ObjectiveEvent::HostageRefused { who: t }).is_some());
    teleport(&mut sim, t, Vec3::new(-30.0, 1.0, -30.0));
    // O35: a CT's use: it follows him, +150 once.
    teleport(&mut sim, ct, Vec3::new(0.0, 1.0, 6.5));
    sim.ticks(2);
    look_at(&mut sim, ct, at);
    let c0 = money(&sim, ct);
    sim.intent(ct).use_key = true;
    sim.ticks(2);
    sim.intent(ct).use_key = false;
    sim.ticks(1);
    assert_eq!(hostages(&mut sim)[0].1.leader, Some(ct));
    assert_eq!(money(&sim, ct), c0 + 150);
    // O36: again: it stays.
    sim.intent(ct).use_key = true;
    sim.ticks(2);
    sim.intent(ct).use_key = false;
    sim.ticks(1);
    assert_eq!(hostages(&mut sim)[0].1.leader, None);
    sim.intent(ct).use_key = true;
    sim.ticks(2);
    sim.intent(ct).use_key = false;
    sim.ticks(1);
    assert_eq!(money(&sim, ct), c0 + 150, "only the first touch pays");
    // The CT walks into the zone; the hostage comes after him.
    let c1 = money(&sim, ct);
    teleport(&mut sim, ct, Vec3::new(13.0, 1.0, 3.0));
    let mut moved = false;
    for _ in 0..400 {
        sim.ticks(1);
        match hostages(&mut sim).pop() {
            Some((_, _, p)) => moved |= (p - at).xz().length() > 1.0,
            None => break,
        }
    }
    assert!(moved, "it walked");
    // O38/O39: rescued: gone, +1000, and with no hostage left: CT win.
    assert!(hostages(&mut sim).is_empty(), "rescued");
    assert_eq!(sim.app.world().resource::<HostageTally>().rescued, 1);
    sim.ticks(2);
    let end = ended(&sim).expect("round over");
    assert_eq!(
        (end.winner, end.reason),
        (Some(Team(2)), RoundEndReason::HostagesRescued)
    );
    assert_eq!(money(&sim, ct), c1 + 1000 + 3500);
}

#[test]
fn hostage_maps_go_to_the_terrorists_on_time() {
    // O40, and O2's shape: nobody gets a bomb on a hostage map.
    let mut sim = sim(hostage_map());
    let _t = character(&mut sim, Vec3::new(0.0, 1.0, 12.0), 1);
    let _ct = character(&mut sim, Vec3::new(5.0, 1.0, 12.0), 2);
    rounds(&mut sim);
    assert!(bomb_carriers(&mut sim).is_empty());
    assert_eq!(hostages(&mut sim).len(), 1);
    sim.seconds(62.0);
    let end = ended(&sim).expect("round over");
    assert_eq!(
        (end.winner, end.reason),
        (Some(Team(1)), RoundEndReason::HostagesNotRescued)
    );
    // The next round puts it back.
    sim.seconds(5.5);
    assert_eq!(hostages(&mut sim).len(), 1);
}

#[test]
fn killing_a_hostage_costs_money_and_too_many_get_you_removed() {
    // O41 with mp_hostagepenalty 3.
    let mut sim = sim(MapObjectives {
        hostages: (0..4)
            .map(|i| HostageSpawn {
                feet: Vec3::new(i as f32 * 2.0, 0.0, 5.0),
                yaw: 0.0,
                kind: None,
            })
            .collect(),
        ..hostage_map()
    });
    let t = character(&mut sim, Vec3::new(0.0, 1.0, 12.0), 1);
    let _ct = character(&mut sim, Vec3::new(5.0, 1.0, 12.0), 2);
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_hostagepenalty 3");
    rounds(&mut sim);
    sim.seconds(1.3);
    let m0 = money(&sim, t);
    let all: Vec<Entity> = hostages(&mut sim).iter().map(|h| h.0).collect();
    for (n, h) in all[..3].iter().enumerate() {
        if n == 2 {
            assert_eq!(money(&sim, t), m0.saturating_sub(2 * 1500));
        }
        sim.app.world_mut().write_message(Damage {
            force: Vec3::ZERO,
            target: *h,
            attacker: Some(t),
            amount: 10.0,
            point: Vec3::ZERO,
            dir: Vec3::X,
            hitgroup: Hitgroup::Chest,
            kind: default(),
            weapon: None,
        });
        sim.ticks(3);
    }
    assert_eq!(sim.app.world().resource::<HostageTally>().killed, 3);
    let kills = events(&sim)
        .iter()
        .filter(|(_, e)| matches!(e, ObjectiveEvent::HostageKilled { attacker: Some(a), .. } if *a == t))
        .count();
    assert_eq!(kills, 3);
    // A computer player would be gone; this one is (no LocalPlayer).
    assert!(sim.app.world().get_entity(t).is_err(), "removed after the third");
}

#[test]
fn hostages_hurt_report_their_attacker() {
    let mut sim = sim(hostage_map());
    let t = character(&mut sim, Vec3::new(0.0, 1.0, 12.0), 1);
    let _ct = character(&mut sim, Vec3::new(5.0, 1.0, 12.0), 2);
    rounds(&mut sim);
    let (h, ..) = hostages(&mut sim).pop().unwrap();
    sim.app.world_mut().write_message(Damage {
        force: Vec3::ZERO,
        target: h,
        attacker: Some(t),
        amount: 0.2,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: Hitgroup::Chest,
        kind: default(),
        weapon: None,
    });
    sim.ticks(2);
    assert!(
        events(&sim)
            .iter()
            .any(|(_, e)| matches!(e, ObjectiveEvent::HostageHurt { attacker: Some(a), .. } if *a == t))
    );
    assert!((sim.app.world().get::<Health>(h).unwrap().current - 0.8).abs() < 1e-4);
}
