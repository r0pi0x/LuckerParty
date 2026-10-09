//! Network weapons on de_dust2 with the install's player models
//! (docs/plans/active/multiplayer.md, slice 4): the server poses hitboxes
//! per tick from its own animation of each body (`map::SimAnimator`), and
//! lag compensation rewinds those poses: a head shot at a player as it
//! was seen standing hits the head though the player has ducked since.
//! Skipped without a CS:S install.

use std::{sync::Arc, time::Duration};

use avian3d::prelude::Position;
use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    core::{God, Hitgroup, Intent, MovementState},
    games::{
        self,
        cs_source::{
            self,
            movement::{self as source, SourceMovementPlugin},
            weapons::CsWeaponsPlugin,
        },
    },
    harness::NetSim,
    map::{MapPlugin, SimAnimator},
    mount::config::LocalConfig,
    net::{interp::InterpClock, memory::LinkConditions},
    slots::Loadout,
    weapon::{
        WeaponEvent, WeaponEventKind,
        lagcomp::{HitFrame, HitHistory, LagCompSettings},
    },
};

fn dust2() -> Option<Arc<mashup::map::MapData>> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(Arc::new(games::load_map("cs_source:de_dust2").expect("load de_dust2")))
}

/// The head box's centre in the world for one kept tick.
fn head(f: &HitFrame) -> Vec3 {
    let boxes = f.hitboxes.as_ref().expect("hitboxes");
    let b = boxes
        .iter()
        .filter(|b| b.group == Hitgroup::Head)
        .max_by(|a, b| a.center.y.total_cmp(&b.center.y))
        .unwrap();
    f.origin.with_y(f.origin.y + f.min.y) + Quat::from_rotation_y(f.yaw) * b.center
}

/// The head at fractional server tick `at`, from the history.
fn head_at(h: &HitHistory, at: f64) -> Option<Vec3> {
    let i = h.frames.iter().rposition(|f| f.tick as f64 <= at)?;
    let a = &h.frames[i];
    let b = h.frames.get(i + 1).unwrap_or(a);
    let f = if b.tick > a.tick {
        ((at - a.tick as f64) / (b.tick - a.tick) as f64) as f32
    } else {
        0.0
    };
    Some(head(a).lerp(head(b), f))
}

/// Client 0 shoots client 1 in the head as it saw it, just as client 1
/// ducks: the head shots the server counts, of six.
fn head_shots(unlag: bool) -> Option<(usize, usize)> {
    let map = dust2()?;
    // CT spawns 0 and 2: same height, 4.9 m apart in the open.
    let (a, b) = (map.spawns[2].0 + Vec3::Y * 0.9, map.spawns[0].0 + Vec3::Y * 0.9);
    let setup = move |app: &mut App| {
        app.add_plugins((MapPlugin::new((*map).clone()), SourceMovementPlugin, CsWeaponsPlugin))
            .insert_resource(Loadout { movement: source::ID });
    };
    let link = LinkConditions {
        latency: Duration::from_millis(100),
        jitter: Duration::ZERO,
        loss: 0.0,
    };
    let mut sim = NetSim::new(link, 61, 2, setup);
    sim.until_joined(900);
    sim.ticks(400);
    sim.server.app.world_mut().resource_mut::<LagCompSettings>().unlag = unlag as u8;
    let (shooter, target) = (sim.character_of(0).unwrap(), sim.character_of(1).unwrap());
    sim.server.app.world_mut().entity_mut(target).insert(God);
    for (e, at) in [(shooter, a), (target, b)] {
        let w = sim.server.app.world_mut();
        w.get_mut::<Transform>(e).unwrap().translation = at;
        if let Some(mut p) = w.get_mut::<Position>(e) {
            p.0 = at;
        }
    }
    sim.ticks(200);
    assert!(
        sim.server.app.world().get::<SimAnimator>(target).is_some(),
        "the server animates the body per tick"
    );
    let world = sim.server.app.world();
    let mut events: MessageCursor<WeaponEvent> = world.resource::<Messages<WeaponEvent>>().get_cursor_current();
    let mut heads = 0;
    let mut shots = 0;
    for round in 0..6 {
        // Stand, then duck: fire once the server's head is well below
        // where the shooter sees it (the duck seen late by the round
        // trip and the interpolation delay).
        let mut fired = false;
        for k in 0..200u32 {
            {
                let me = sim.local_player(1).unwrap();
                sim.clients[1].app.world_mut().get_mut::<Intent>(me).unwrap().crouch = k >= 60;
            }
            let render = sim.clients[0].app.world().resource::<InterpClock>().render_tick;
            let history = sim.server.app.world().get::<HitHistory>(target).unwrap();
            let seen = head_at(history, render);
            let now = history.frames.back().map(head);
            let me = sim.local_player(0).unwrap();
            let w = sim.clients[0].app.world();
            let eye = w.get::<Transform>(me).unwrap().translation + w.get::<MovementState>(me).unwrap().eye_offset;
            let fire = !fired && matches!((seen, now), (Some(s), Some(n)) if s.y - n.y > 0.25);
            {
                let w = sim.clients[0].app.world_mut();
                let mut i = w.get_mut::<Intent>(me).unwrap();
                if let Some(s) = seen {
                    let d = s - eye;
                    i.yaw = (-d.x).atan2(-d.z).rem_euclid(std::f32::consts::TAU);
                    i.pitch = d.y.atan2(d.xz().length());
                }
                i.fire = fire;
            }
            fired |= fire;
            sim.step();
            let world = sim.server.app.world();
            for e in events.read(world.resource::<Messages<WeaponEvent>>()) {
                if e.owner != shooter {
                    continue;
                }
                match e.kind {
                    WeaponEventKind::Fired { .. } => shots += 1,
                    WeaponEventKind::Hit { target: t, hitgroup, .. } if t == target && hitgroup == Hitgroup::Head => {
                        heads += 1
                    }
                    _ => {}
                }
            }
        }
        assert!(fired, "round {round}: the duck never showed");
    }
    Some((heads, shots))
}

#[test]
fn lag_compensation_rewinds_per_tick_hitbox_poses() {
    let Some((on, shots_on)) = head_shots(true) else { return };
    let Some((off, shots_off)) = head_shots(false) else { return };
    println!("de_dust2, 100 ms: head shots {on} of {shots_on} with lag compensation, {off} of {shots_off} without");
    assert_eq!(shots_on, 6);
    assert!(on >= 5, "{on} of 6");
    assert!(off <= 1, "{off} of 6 without");
}
