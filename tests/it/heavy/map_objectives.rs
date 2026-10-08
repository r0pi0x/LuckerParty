//! Objectives on stock maps (specs/cs_source/objectives.md): planting at
//! de_dust2's A site and the site's BombExplode outputs, cs_office's
//! hostages and rescue. Skipped without a CS:S install.

use avian3d::prelude::Position;
use bevy::prelude::*;
use mashup::{
    console::Console,
    core::{SimTick, Team},
    games::{
        self, cs_source,
        cs_source::{
            TICK_INTERVAL,
            movement::{self, SourceMovementPlugin, to_engine},
            objectives::C4,
            weapons::CsWeaponsPlugin,
        },
    },
    harness::Sim,
    logic::Logic,
    map::{MapData, MapPlugin},
    mount::config::LocalConfig,
    objectives::{
        MapKind, MapObjectives,
        bomb::{BombOutcome, BombState, PlantedBomb},
        hostages::{Hostage, HostageTally},
    },
    weapon::{Inventory, give},
};

const UNIT: f32 = 0.0254;
/// The standing hull's centre above the feet, m (36 units).
const LIFT: f32 = 36.0 * UNIT;

fn load(name: &str) -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map(&format!("cs_source:{name}")).expect("load map"))
}

fn sim(map: MapData) -> Sim {
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.ticks(2);
    sim
}

fn character(sim: &mut Sim, feet_units: Vec3, team: u8) -> Entity {
    let e = sim.spawn_character(to_engine(feet_units) + Vec3::Y * (LIFT + 0.05), movement::ID);
    sim.app.world_mut().entity_mut(e).insert(Team(team));
    e
}

fn teleport(sim: &mut Sim, e: Entity, feet_units: Vec3) {
    let at = to_engine(feet_units) + Vec3::Y * (LIFT + 0.05);
    let w = sim.app.world_mut();
    w.get_mut::<Transform>(e).unwrap().translation = at;
    if let Some(mut p) = w.get_mut::<Position>(e) {
        p.0 = at;
    }
}

/// O4/O5 and a plant at A: the site's BombExplode fires when it blows.
#[test]
fn dust2_plants_at_a_and_the_site_explodes() {
    let Some(map) = load("de_dust2") else { return };
    let mut sim = sim(map);
    let objectives = sim.app.world().resource::<MapObjectives>().clone();
    assert_eq!(objectives.kind(), MapKind::Bomb);
    assert_eq!(objectives.bomb_targets.len(), 2);
    assert_eq!(objectives.bomb_radius, None, "dust2 has no bombradius: 500");
    // O4 / O5: the A box (1072, 2336, 96)-(1248, 2624, 192).
    let hull = |x: f32, y: f32| {
        let feet = to_engine(Vec3::new(x, y, 96.03));
        (
            feet + Vec3::new(-16.0, 0.0, -16.0) * UNIT,
            feet + Vec3::new(16.0, 72.0, 16.0) * UNIT,
        )
    };
    let (lo, hi) = hull(1160.0, 2480.0);
    let a = objectives.bomb_target_at(lo, hi).expect("in A");
    let (lo, hi) = hull(1000.0, 2480.0);
    assert_eq!(objectives.bomb_target_at(lo, hi), None);

    let t = character(&mut sim, Vec3::new(1160.0, 2480.0, 100.0), 1);
    sim.seconds(0.5);
    give(sim.app.world_mut(), t, C4).unwrap();
    sim.seconds(1.2);
    assert!(sim.state(t).on_ground);
    sim.app.world_mut().resource_mut::<Console>().submit("mp_c4timer 10");
    sim.ticks(1);
    sim.intent(t).fire = true;
    sim.seconds(3.1);
    sim.intent(t).fire = false;
    let (bomb, at) = {
        let w = sim.app.world_mut();
        let v: Vec<(PlantedBomb, Vec3)> = w
            .query::<(&PlantedBomb, &Transform)>()
            .iter(w)
            .map(|(b, t)| (b.clone(), t.translation))
            .collect();
        assert_eq!(v.len(), 1, "planted");
        v[0].clone()
    };
    assert_eq!(bomb.site, Some(a));
    assert!(
        (bomb.explode_at - bomb.planted_at - 10.0).abs() < 1e-6,
        "mp_c4timer read at the plant"
    );
    let feet = sim.position(t).y - LIFT;
    assert!((at.y - feet).abs() < 0.05, "on the floor: {} vs {feet}", at.y);
    sim.app.world_mut().resource_mut::<Logic>().world.record = true;
    let planted_tick = sim.app.world().resource::<SimTick>().0;
    sim.seconds(10.2);
    assert_eq!(
        sim.app.world().resource::<BombState>().outcome,
        Some(BombOutcome::Exploded)
    );
    let logic = sim.app.world().resource::<Logic>();
    let site_index = objectives.bomb_targets[a].map_index.unwrap();
    let fired = logic.world.fired.iter().any(|(_, id, out)| {
        out == "BombExplode" && logic.world.get(*id).is_some_and(|e| e.map_index == Some(site_index))
    });
    assert!(fired, "A's BombExplode fired");
    // The planter stood on it: dead.
    assert!(sim.app.world().get::<mashup::core::Health>(t).unwrap().current <= 0.0);
    let _ = planted_tick;
}

/// O2: cs_office's four hostages stand where the map puts them; one
/// follows a CT over the nav mesh and is rescued in a rescue zone.
#[test]
fn office_hostages_follow_and_are_rescued() {
    let Some(map) = load("cs_office") else { return };
    let mut sim = sim(map);
    let objectives = sim.app.world().resource::<MapObjectives>().clone();
    assert_eq!(objectives.kind(), MapKind::Hostage);
    let want = [
        Vec3::new(1784.0, 734.0, -124.0),
        Vec3::new(1744.0, 802.0, -124.0),
        Vec3::new(2048.0, -344.0, -156.0),
        Vec3::new(1984.0, -344.0, -156.0),
    ];
    assert_eq!(objectives.hostages.len(), 4);
    for (h, w) in objectives.hostages.iter().zip(want) {
        assert!((h.feet - to_engine(w)).length() < 0.05, "{} vs {w}", h.feet / UNIT);
    }
    assert!(!objectives.rescue_zones.is_empty());
    let t = character(&mut sim, Vec3::new(1500.0, 300.0, -150.0), 1);
    let ct = character(&mut sim, Vec3::new(1500.0, 200.0, -150.0), 2);
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_freezetime 0; mp_roundtime 5; mashup_rounds 1");
    sim.seconds(1.5);
    let hostages: Vec<(Entity, Vec3)> = {
        let w = sim.app.world_mut();
        w.query_filtered::<(Entity, &Transform), With<Hostage>>()
            .iter(w)
            .map(|(e, t)| (e, t.translation))
            .collect()
    };
    assert_eq!(hostages.len(), 4);
    for (h, at) in &hostages {
        assert!(sim.state(*h).on_ground, "hostage at {} not standing", at / UNIT);
    }
    let _ = t;
    // The CT walks up to the first hostage and uses it.
    let (h, at) = hostages[0];
    let hs = sim.position(h);
    teleport(&mut sim, ct, Vec3::new(1784.0, 734.0 - 50.0, -124.0));
    sim.ticks(3);
    let eye = sim.position(ct) + sim.state(ct).eye_offset;
    let d = hs - eye;
    sim.intent(ct).yaw = (-d.x).atan2(-d.z);
    sim.intent(ct).pitch = d.y.atan2(d.xz().length());
    sim.intent(ct).use_key = true;
    sim.ticks(2);
    sim.intent(ct).use_key = false;
    sim.ticks(1);
    assert_eq!(sim.app.world().get::<Hostage>(h).unwrap().leader, Some(ct));
    // He walks off 400 units; it comes after him.
    teleport(&mut sim, ct, Vec3::new(1784.0 - 400.0, 734.0 - 50.0, -124.0));
    sim.seconds(4.0);
    let moved = (sim.position(h) - at).xz().length() / UNIT;
    assert!(moved > 100.0, "it moved only {moved} units");
    // Into a rescue zone: rescued.
    let zone = objectives.rescue_zones[0].floor_centre();
    {
        let w = sim.app.world_mut();
        let to = zone + Vec3::Y * (LIFT + 0.1);
        w.get_mut::<Transform>(h).unwrap().translation = to;
        if let Some(mut p) = w.get_mut::<Position>(h) {
            p.0 = to;
        }
    }
    sim.ticks(3);
    assert!(sim.app.world().get_entity(h).is_err(), "rescued");
    assert_eq!(sim.app.world().resource::<HostageTally>().rescued, 1);
    let _ = Inventory::default();
}

/// A following hostage animates: idle standing, then a walk or run
/// sequence (HL2 citizen animations from `.ani` blocks) while it comes
/// after its leader, its body turned toward where it goes, and a nod when
/// it starts following.
#[test]
fn office_hostage_walks_after_its_leader() {
    use mashup::{
        games::cs_source::{
            hostage_anim::{FOLLOW_GESTURE, HostageAnim},
            player_anim::PlayerAnimPlugin,
        },
        map::anim::Animator,
    };
    let Some(map) = load("cs_office") else { return };
    let mut sim = Sim::new((
        MapPlugin::new(map),
        SourceMovementPlugin,
        CsWeaponsPlugin,
        PlayerAnimPlugin,
    ));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.ticks(2);
    let ct = character(&mut sim, Vec3::new(1500.0, 200.0, -150.0), 2);
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_freezetime 0; mp_roundtime 5; mashup_rounds 1");
    sim.seconds(1.5);
    let h = {
        let w = sim.app.world_mut();
        w.query::<(Entity, &Hostage)>()
            .iter(w)
            .find(|(_, h)| h.index == 0)
            .map(|(e, _)| e)
            .expect("hostage 1")
    };
    let main = |sim: &Sim| {
        let a = sim.app.world().get::<Animator>(h).expect("hostages get an animator");
        a.main.map(|s| a.set.sequences[s].name.to_lowercase())
    };
    assert_eq!(main(&sim).as_deref(), Some("idle_subtle"), "standing idle");
    // The CT uses it and walks off; it follows.
    let hs = sim.position(h);
    teleport(&mut sim, ct, Vec3::new(1784.0, 734.0 - 50.0, -124.0));
    sim.ticks(3);
    let eye = sim.position(ct) + sim.state(ct).eye_offset;
    let d = hs - eye;
    sim.intent(ct).yaw = (-d.x).atan2(-d.z);
    sim.intent(ct).pitch = d.y.atan2(d.xz().length());
    sim.intent(ct).use_key = true;
    sim.ticks(2);
    sim.intent(ct).use_key = false;
    sim.ticks(2);
    assert_eq!(sim.app.world().get::<Hostage>(h).unwrap().leader, Some(ct));
    {
        let a = sim.app.world().get::<Animator>(h).unwrap();
        let gesture = a
            .layers
            .first()
            .copied()
            .flatten()
            .map(|l| a.set.sequences[l.sequence].name.clone());
        assert_eq!(gesture.as_deref(), Some(FOLLOW_GESTURE), "a nod as it starts following");
    }
    teleport(&mut sim, ct, Vec3::new(1784.0 - 400.0, 734.0 - 50.0, -124.0));
    let mut seen = Vec::new();
    for _ in 0..30 {
        sim.seconds(0.1);
        if let Some(m) = main(&sim) {
            seen.push(m);
        }
        let v = sim.velocity(h).xz();
        let state = sim.app.world().get::<HostageAnim>(h).unwrap().clone();
        if v.length() / UNIT > 60.0 && state.activity != "ACT_IDLE" {
            // Facing where it goes (within the turn it is still making).
            let heading = (-v.x).atan2(-v.y);
            let yaw = sim.app.world().get::<Animator>(h).unwrap().yaw.unwrap();
            let off = (heading - yaw).rem_euclid(std::f32::consts::TAU);
            let off = off.min(std::f32::consts::TAU - off).to_degrees();
            assert!(off < 60.0, "body {off}° off its heading");
        }
    }
    assert!(
        seen.iter().any(|s| s == "walk_all" || s == "run_all"),
        "it never walked or ran: {seen:?}"
    );
    eprintln!("hostage played {seen:?}");
}
