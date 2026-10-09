//! Round restarts (Counter-Strike): the map's entities re-created as they
//! spawned, except the kept classes (`ROUND_KEEP`).

use super::*;
use crate::core::DamageKind;

fn map_entity(pairs: &[(&str, &str)], hulls: Vec<MapHull>) -> crate::map::MapEntity {
    crate::map::MapEntity {
        keyvalues: kv(pairs),
        mover: !hulls.is_empty(),
        physics: None,
        hulls,
    }
}

/// The id of map entity `index` (None: gone).
fn by_index(w: &LogicWorld, index: usize) -> Option<EntId> {
    w.ids().into_iter().find(|id| w.get(*id).unwrap().map_index == Some(index))
}

#[test]
fn round_restart_recreates_map_entities() {
    let door = map_entity(
        &[
            ("classname", "func_door"),
            ("targetname", "door"),
            ("origin", "100 0 64"),
            ("lip", "4"),
            ("speed", "100"),
            ("wait", "-1"),
            ("spawnflags", "256"),
        ],
        vec![hull(Vec3::new(-32.0, -4.0, -64.0), Vec3::new(32.0, 4.0, 64.0))],
    );
    let entities = vec![
        map_entity(
            &[
                ("classname", "logic_auto"),
                ("OnMapSpawn", "ctr,Add,1,0,-1"),
                ("OnMultiNewMap", "ctr,Add,10,0,-1"),
                ("OnMultiNewRound", "ctr,Add,100,0,-1"),
            ],
            vec![],
        ),
        map_entity(&[("classname", "math_counter"), ("targetname", "ctr")], vec![]),
        door,
        map_entity(
            &[("classname", "trigger_once"), ("spawnflags", "1"), ("OnTrigger", "door,Open,,0,-1")],
            vec![hull(Vec3::new(-400.0, -100.0, 0.0), Vec3::new(-200.0, 100.0, 100.0))],
        ),
        map_entity(
            &[("classname", "func_breakable"), ("material", "2"), ("health", "1"), ("origin", "0 300 32")],
            vec![hull(Vec3::new(-32.0, -4.0, -16.0), Vec3::new(32.0, 4.0, 16.0))],
        ),
        map_entity(
            &[("classname", "func_brush"), ("targetname", "wall"), ("origin", "0 -300 32")],
            vec![hull(Vec3::splat(-16.0), Vec3::splat(16.0))],
        ),
        // A message due in 30 s: dropped by the restart.
        map_entity(&[("classname", "logic_auto"), ("OnMapSpawn", "ctr,Add,1000,30,-1")], vec![]),
        map_entity(
            &[("classname", "game_text"), ("targetname", "msg"), ("message", "hi")],
            vec![],
        ),
    ];
    let mut w = world();
    let ids = w.load_map(&entities);
    run_to(&mut w, 20);
    assert_eq!(counter(&w, ids[1]), 11.0, "OnMapSpawn and OnMultiNewMap at map start");
    assert!(w.get(ids[0]).is_some(), "logic_auto without flag 1 stays");

    // Round 1: the trigger opens the door, the vent breaks, the wall goes
    // off, the counter climbs.
    player_at(&mut w, 0, Vec3::new(-300.0, 0.0, 0.0));
    run_to(&mut w, 200);
    assert!(w.get(ids[3]).is_none(), "trigger_once used up");
    assert_eq!(origin_of(&w, ids[2]).x, 158.0, "door opened");
    w.damage(ids[4], 26.0, DamageKind::Bullet, None, Vec3::new(0.0, 300.0, 32.0), Vec3::Y);
    w.queue_input("wall", "Disable", Value::Void, 0.0, None);
    w.queue_input("ctr", "Add", Value::Void, 0.0, None);
    run_to(&mut w, 220);
    assert!(w.get(ids[4]).is_none(), "vent broken and removed");
    let wall_enabled = |w: &LogicWorld| match &w.get(by_index(w, 5).unwrap()).unwrap().class {
        Class::Brush(t) => t.enabled,
        _ => panic!(),
    };
    assert!(!wall_enabled(&w));

    // The player leaves; the restart.
    w.players.clear();
    let tick = w.tick;
    let ids = w.round_restart(&entities);
    assert_eq!(w.tick, tick, "time goes on");
    assert_eq!(w.round, 1);
    let door = by_index(&w, 2).expect("door back");
    assert_eq!(origin_of(&w, door), Vec3::new(100.0, 0.0, 64.0), "closed again");
    assert!(by_index(&w, 3).is_some(), "trigger_once back");
    let vent = by_index(&w, 4).expect("vent back");
    assert!(matches!(&w.get(vent).unwrap().class, Class::Breakable(b) if !b.broken && b.health == 1));
    assert!(w.mover_solid(vent).is_some() && w.shootable(vent), "whole and solid");
    assert_eq!(counter(&w, ids[1]), 0.0, "counter from its start value");
    assert!(!wall_enabled(&w), "func_brush is kept as it was (ROUND_KEEP)");
    assert_eq!(ids[5], EntId { index: 5, generation: 0 });

    // logic_auto fires again 0.2 s later, as a new round; the old queue
    // (the +1000 due at 30 s) is gone.
    let fired_from = w.fired.len();
    run_to(&mut w, tick + 13);
    let outputs: Vec<&str> = w.fired[fired_from..]
        .iter()
        .filter(|(_, e, _)| *e == ids[0])
        .map(|(_, _, o)| o.as_str())
        .collect();
    assert_eq!(outputs, ["OnNewGame", "OnMapSpawn", "OnMultiNewRound"]);
    assert_eq!(counter(&w, ids[1]), 101.0);
    run_to(&mut w, tick + 3000);
    assert_eq!(counter(&w, ids[1]), 1101.0, "this round's own +1000 only");

    // Round 2: the trigger works again.
    player_at(&mut w, 0, Vec3::new(-300.0, 0.0, 0.0));
    run_to(&mut w, tick + 3200);
    assert!(by_index(&w, 3).is_none(), "trigger_once fired again");
    assert_eq!(origin_of(&w, door).x, 158.0, "and opened the door");
}

#[test]
fn round_restart_keeps_removed_kept_entities_removed() {
    let entities = vec![
        map_entity(&[("classname", "info_target"), ("targetname", "t")], vec![]),
        map_entity(&[("classname", "logic_relay"), ("targetname", "r")], vec![]),
    ];
    let mut w = world();
    w.load_map(&entities);
    w.queue_input("t", "Kill", Value::Void, 0.0, None);
    w.queue_input("r", "Kill", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    assert!(w.find("t").is_none() && w.find("r").is_none());
    let ids = w.round_restart(&entities);
    assert!(w.find("t").is_none(), "info_target is kept: stays removed");
    assert!(w.get(ids[0]).is_none());
    assert_eq!(w.find("r"), Some(ids[1]), "the relay is re-created");
    assert!(kept_on_restart("env_soundscape_proxy") && kept_on_restart("FUNC_BRUSH"));
    assert!(!kept_on_restart("func_breakable"));
}

#[test]
fn round_restart_closes_model_doors_and_brings_props_back() {
    let entities = vec![
        map_entity(
            &[
                ("classname", "prop_door_rotating"),
                ("targetname", "door"),
                ("spawnflags", "8192"),
                ("returndelay", "-1"),
            ],
            vec![hull(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 52.0, 100.0))],
        ),
        map_entity(
            &[
                ("classname", "prop_physics_multiplayer"),
                ("targetname", "projector"),
                (crate::map::entities::PROP_HEALTH_KEY, "20"),
                (crate::map::entities::PROP_PIECES_KEY, "10"),
            ],
            vec![],
        ),
    ];
    let mut w = world();
    let ids = w.load_map(&entities);
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    w.damage(ids[1], 60.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    run_to(&mut w, 100);
    assert_eq!(angles_of(&w, ids[0]).y, -90.0);
    assert!(w.get(ids[1]).is_none(), "broken");
    let ids = w.round_restart(&entities);
    assert_eq!(angles_of(&w, ids[0]).y, 0.0, "closed again");
    assert!(matches!(&w.get(ids[0]).unwrap().class, Class::PropDoor(d) if d.state == DoorState::Closed));
    assert!(w.mover_solid(ids[0]).is_some());
    assert_eq!(w.prop_health(ids[1]), Some((20, 20)), "whole again");
    assert_eq!(w.find("projector"), Some(ids[1]));
}
