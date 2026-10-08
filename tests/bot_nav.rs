//! Bots walking real maps' navigation meshes: places where they used to
//! get stuck (ladder tops, drops, crouch spots), each a start and a goal
//! a lone bot must reach in time, and every ladder on de_nuke up and
//! down (`ladders_climb_both_ways`, ignored: any maps). Skipped without a
//! CS:S install.
//!
//! `nav_near` (ignored) prints the nav areas, links and ladders around a
//! point, for working out why a bot stops somewhere:
//! `MASHUP_NAV_MAP=de_nuke MASHUP_NAV_AT=11.4,-15.2,34.4 cargo test
//! --features dev --test bot_nav -- --ignored --nocapture nav_near`
//! (engine meters, as `bot_rounds`' stuck report prints them).

use bevy::prelude::*;
use mashup::{
    bot::{Bot, BotConfig},
    core::{Team, Velocity},
    games::{
        self,
        cs_source::{
            self, TICK_INTERVAL,
            movement::{self, SourceMovementPlugin},
            weapons::CsWeaponsPlugin,
        },
    },
    harness::Sim,
    map::{
        MapPlugin,
        nav::{NavMesh, Via},
    },
    mount::config::LocalConfig,
};

fn installed() -> bool {
    let ok = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !ok {
        eprintln!("skipping: no CS:S install configured");
    }
    ok
}

fn sim(map: &str) -> Sim {
    let map = games::load_map(&format!("cs_source:{map}")).expect("load map");
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    sim.ticks(2);
    sim
}

/// Half the standing hull: the origin above the feet, m.
const HALF: f32 = 0.9;

fn feet(sim: &Sim, e: Entity) -> Vec3 {
    sim.position(e) - Vec3::Y * HALF
}

fn trace_every() -> u64 {
    std::env::var("MASHUP_BOT_TRACE").ok().and_then(|v| v.parse().ok()).unwrap_or(5)
}

/// The floor at `p` (feet), if above it: a mesh area's height can be
/// below the real floor, inside a curved train roof.
fn floor_under(sim: &mut Sim, p: Vec3, skip: Entity) -> Vec3 {
    use avian3d::prelude::{SpatialQuery, SpatialQueryFilter};
    let world = sim.app.world_mut();
    let mut state = bevy::ecs::system::SystemState::<SpatialQuery>::new(world);
    let q = state.get(world).unwrap();
    let filter = SpatialQueryFilter::from_excluded_entities([skip]).with_mask(mashup::core::SOLID_LAYERS);
    [0.6, 0.4, 0.2]
        .into_iter()
        .find_map(|up| {
            let hit = q.cast_ray(p + Vec3::Y * up, Dir3::NEG_Y, up + 0.3, true, &filter)?;
            (hit.distance > 0.0).then(|| p + Vec3::Y * (up - hit.distance))
        })
        // (Only ever up: physics' world colliders can sit a little below
        // the brushes movement walks on.)
        .map_or(p, |f| f.max(p))
}

/// The nearest spot to `p` (feet) where a standing box isn't inside a
/// brush, within 0.5 m (else `p`): a mesh area can be narrower than a
/// body against a wall (de_piranesi's ledge at a ladder's top).
fn clear_spot(sim: &Sim, p: Vec3) -> Vec3 {
    let Some(brushes) = sim.app.world().get_resource::<mashup::core::MapBrushes>() else {
        return p;
    };
    let half = Vec3::new(16.0, 36.0, 16.0) * 0.0254;
    let solid = |feet: Vec3| {
        let c = feet + Vec3::Y * (half.y + 0.06);
        brushes.0.iter().any(|b| {
            b.max.cmpgt(c - half).all()
                && b.min.cmplt(c + half).all()
                && b.planes.iter().all(|(n, d)| n.dot(c) - (d + n.abs().dot(half)) < -0.01)
        })
    };
    if !solid(p) {
        return p;
    }
    (1..=10)
        .flat_map(|r| (0..16).map(move |k| (r, k)))
        .map(|(r, k)| {
            let a = k as f32 * std::f32::consts::TAU / 16.0;
            p + Vec3::new(a.cos(), 0.0, a.sin()) * (r as f32 * 0.05)
        })
        .find(|q| !solid(*q))
        .unwrap_or(p)
}

/// One bot from `from` to `to` (feet, engine meters): the seconds it took,
/// or None if it didn't get there (on the ground, within 1 m and `dy` m
/// of its height) within `limit`.
fn walk(sim: &mut Sim, from: Vec3, to: Vec3, limit: f64, dy: f32, trace: bool) -> Option<f64> {
    let bot = mashup::bot::add_bot(sim.app.world_mut(), Team(1)).expect("bot");
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 0;
    sim.ticks(1);
    let from = floor_under(sim, from, bot);
    let from = clear_spot(sim, from);
    {
        let w = sim.app.world_mut();
        w.get_mut::<Transform>(bot).unwrap().translation = from + Vec3::Y * (HALF + 0.05);
        w.get_mut::<Velocity>(bot).unwrap().0 = Vec3::ZERO;
        w.get_mut::<Bot>(bot).unwrap().send_to(Some(to));
    }
    // Every tick when tracing every tick (MASHUP_BOT_TRACE=0).
    let every = trace_every();
    let step = if trace && every == 0 { TICK_INTERVAL } else { 0.1 };
    let mut t = 0.0;
    while t < limit {
        sim.seconds(step);
        t += step;
        let at = feet(sim, bot);
        if trace && (every == 0 || ((t * 10.0).round() as u64).is_multiple_of(every)) {
            let b = sim.app.world().get::<Bot>(bot).unwrap();
            let i = sim.app.world().get::<mashup::core::Intent>(bot).unwrap();
            let st = sim.state(bot);
            eprintln!(
                "{t:5.1} at {at:.2} v {:.2} next {:.2?} {:?} axis {:.1} yaw {:.0} pitch {:.0} jump {} crouch {} fire {} ladder {} ground {}",
                sim.velocity(bot),
                b.route().0.get(b.route().1),
                b.activity(),
                i.move_axis,
                i.yaw.to_degrees(),
                i.pitch.to_degrees(),
                i.jump,
                i.crouch,
                i.fire,
                st.on_ladder,
                st.on_ground,
            );
        }
        if (at - to).xz().length() < 1.0 && (at.y - to.y).abs() < dy && sim.state(bot).on_ground {
            sim.app.world_mut().despawn(bot);
            return Some(t);
        }
    }
    eprintln!("stopped at {:.2}", feet(sim, bot));
    sim.app.world_mut().despawn(bot);
    None
}

/// (what, start, goal, seconds), engine meters (feet): spots where bots
/// got stuck in `bot_rounds` runs.
type Case = (&'static str, [f32; 3], [f32; 3], f64);

const NUKE: &[Case] = &[
    // A vent grille (func_breakable) closes the duct's end over B.
    ("ducts down into B", [11.4, -15.24, 35.5], [11.43, -16.26, 32.6], 6.0),
    ("ladder room up", [29.5, -10.56, 12.0], [25.8, -2.64, 10.8], 8.0),
    ("ladder room down", [27.14, -2.64, 11.9], [29.5, -10.56, 12.0], 8.0),
    ("A hatch down into the ducts", [11.7, -10.57, 32.0], [12.9, -15.24, 36.5], 10.0),
    ("east duct down into B", [21.3, -15.24, 36.0], [21.27, -16.26, 32.8], 8.0),
    // A grille in a wall, then a crouch-jump into the duct.
    ("back way up into the ducts", [16.19, -16.24, 41.0], [16.19, -15.24, 36.6], 8.0),
];

const DUST2: &[Case] = &[
    // Up onto the boxes at B (a jump area).
    ("B boxes up", [-31.3, 2.4, -67.2], [-33.5, 3.24, -67.8], 5.0),
];

fn cases(map: &str, cases: &[Case]) {
    if !installed() {
        return;
    }
    let mut sim = sim(map);
    let trace = std::env::var("MASHUP_BOT_TRACE").is_ok();
    let only = std::env::var("MASHUP_BOT_CASE").ok();
    let mut failed = Vec::new();
    for (what, from, to, limit) in cases {
        if only.as_ref().is_some_and(|o| !what.contains(o.as_str())) {
            continue;
        }
        let took = walk(&mut sim, Vec3::from(*from), Vec3::from(*to), *limit, 0.3, trace);
        eprintln!("{map}, {what}: {took:?}");
        if took.is_none() {
            failed.push(*what);
        }
    }
    assert!(failed.is_empty(), "bots didn't make it on {map}: {failed:?}");
}

#[test]
fn nuke_stuck_spots_are_passable() {
    cases("de_nuke", NUKE);
}

/// The vents' ladders up to A: player clip flush with the ladder on both
/// sides wins the ladder probe from the duct's centre line, as in CS:S;
/// the ladders attach only from the duct's south half (box centre y <
/// about -1438 Source units; see
/// `map_de_nuke::vent_and_outside_ladders_climb`), so bots sidestep at
/// the foot. At the top a grille in the shaft's side (a breakable) closes
/// the way out and the duct's ceiling stops the climb a little short of
/// the mesh's ladder top: bots climb as far as it goes, hang there and
/// shoot the grille, then step off.
const NUKE_VENT_LADDERS: &[Case] = &[
    ("west vent up to A", [12.9, -15.24, 36.5], [11.7, -10.57, 32.0], 10.0),
    ("east vent up to A", [20.0, -15.24, 36.5], [21.0, -10.57, 30.0], 10.0),
];

#[test]
fn nuke_vent_ladders_are_climbable() {
    cases("de_nuke", NUKE_VENT_LADDERS);
}

/// Every ladder on a map's mesh, up and down: a bot from inside the area
/// at one end to inside the area at the other (`walk` puts it on the real
/// floor there, clear of walls). The ones that fail.
fn every_ladder(map: &str) -> Vec<String> {
    let mut sim = sim(map);
    let trace = std::env::var("MASHUP_BOT_TRACE").is_ok();
    let only = std::env::var("MASHUP_BOT_CASE").ok();
    let nav = sim.app.world().resource::<NavMesh>().clone();
    // A point inside `area` near `p` (off the edges, where walls are).
    let inside = |area: usize, p: Vec3| {
        let a = &nav.areas[area];
        let c = a.closest_point(p);
        let to = (a.center - c).with_y(0.0);
        c + to.normalize_or_zero() * to.length().min(1.0)
    };
    let mut failed = Vec::new();
    for (k, l) in nav.ladders.iter().enumerate() {
        let link = nav.areas.iter().enumerate().find_map(|(i, a)| {
            a.links
                .iter()
                .find(|(_, via)| matches!(via, Via::LadderUp(j) if *j == k))
                .map(|(n, _)| (i, *n))
        });
        let Some((below, above)) = link else { continue };
        // Clear of the ladder (a foot area can be narrower than a body).
        let out = l.bottom + l.normal.with_y(0.0).normalize_or_zero() * 0.8;
        let foot = nav.nearest_area(out).map_or_else(|| inside(below, out), |a| inside(a, out));
        let top = inside(above, l.top);
        for (what, from, to) in [("up", foot, top), ("down", top, foot)] {
            let name = format!("ladder {k} {what}");
            if only.as_ref().is_some_and(|o| name != *o) {
                continue;
            }
            // (Mesh heights at a ladder's ends can be half a metre off.)
            // MASHUP_BOT_TRACE_CASE: trace only that one, running the rest
            // (earlier cases change what a later one meets: stuck reports,
            // broken grilles).
            let trace = trace
                && std::env::var("MASHUP_BOT_TRACE_CASE").map_or(true, |c| c == name);
            let took = walk(&mut sim, from, to, 12.0, 0.6, trace);
            eprintln!("{map}, ladder {k} {what} ({from:.2} to {to:.2}): {took:?}");
            if took.is_none() {
                failed.push(name);
            }
        }
    }
    failed
}

/// Bots climb every ladder on de_nuke, up and down (vents, the ladder
/// room, ...): off the top only once the feet clear the lip, shooting a
/// grille in the way first.
#[test]
fn nuke_ladders_climb_both_ways() {
    if !installed() {
        return;
    }
    let failed = every_ladder("de_nuke");
    assert!(failed.is_empty(), "de_nuke: {failed:?}");
}

/// `every_ladder` on `MASHUP_NAV_MAP` (or a few stock maps).
#[test]
#[ignore]
fn ladders_climb_both_ways() {
    if !installed() {
        return;
    }
    let maps = std::env::var("MASHUP_NAV_MAP").unwrap_or_else(|_| "cs_office,de_train,de_aztec,de_port,cs_italy".into());
    let mut failed = Vec::new();
    for map in maps.split(',') {
        failed.extend(every_ladder(map).into_iter().map(|f| format!("{map} {f}")));
    }
    assert!(failed.is_empty(), "{failed:?}");
}

#[test]
fn dust2_stuck_spots_are_passable() {
    cases("de_dust2", DUST2);
}

/// Prints the nav areas within 4 m of `MASHUP_NAV_AT` and the ladders
/// within 8 m.
#[test]
#[ignore]
fn nav_near() {
    if !installed() {
        return;
    }
    let map = std::env::var("MASHUP_NAV_MAP").unwrap_or_else(|_| "de_nuke".into());
    let at: Vec<f32> = std::env::var("MASHUP_NAV_AT")
        .expect("MASHUP_NAV_AT=x,y,z")
        .split(',')
        .map(|v| v.trim().parse().expect("number"))
        .collect();
    let at = Vec3::new(at[0], at[1], at[2]);
    let sim = sim(&map);
    let nav = sim.app.world().resource::<NavMesh>();
    eprintln!("area at: {:?}", nav.area_at(at + Vec3::Y * 0.1));
    for (i, a) in nav.areas.iter().enumerate() {
        let p = a.closest_point(at);
        if p.distance(at) > 4.0 {
            continue;
        }
        eprintln!(
            "area {i} (id {}) flags {:#x} x {:.2}..{:.2} z {:.2}..{:.2} heights {:.2?} place {:?}",
            a.id,
            a.flags,
            a.min.x,
            a.max.x,
            a.min.y,
            a.max.y,
            a.heights,
            a.place.map(|p| &nav.places[p])
        );
        for (n, via) in &a.links {
            let b = &nav.areas[*n];
            let back = b.links.iter().any(|l| l.0 == i);
            eprintln!(
                "    -> {n} {via:?} centre {:.2} {}",
                b.center,
                if back { "two-way" } else { "one-way" }
            );
        }
    }
    if let Some(ents) = sim.app.world().get_resource::<mashup::map::MapEntities>() {
        use mashup::map::entities::{entity_rotation, entity_to_engine};
        for e in ents.entities.iter() {
            let pts: Vec<Vec3> = e.hulls.iter().flat_map(|h| h.points.iter().copied()).collect();
            let local = if pts.is_empty() {
                Vec3::ZERO
            } else {
                pts.iter().sum::<Vec3>() / pts.len() as f32
            };
            let p = entity_to_engine(e.origin() + entity_rotation(e.angles()) * local, ents.scale);
            if p.distance(at) < 4.0 {
                eprintln!(
                    "entity {} {:?} at {p:.2} ({} hulls){}",
                    e.classname(),
                    e.get("targetname"),
                    e.hulls.len(),
                    if e.classname().starts_with("func_breakable") {
                        format!(" {:?}", e.keyvalues)
                    } else {
                        String::new()
                    }
                );
            }
        }
    }
    let data = games::load_map(&format!("cs_source:{map}")).expect("load map");
    for b in data.collision_brushes.iter().filter(|b| b.ladder) {
        let c = (b.min + b.max) / 2.0;
        if (c - at).xz().length() < 4.0 {
            eprintln!("ladder brush {:.2}..{:.2}", b.min, b.max);
        }
    }
    // MASHUP_NAV_BRUSHES=r: every collision brush within r m of the point
    // (its box), with its index (the tie order between flush faces).
    if let Some(r) = std::env::var("MASHUP_NAV_BRUSHES").ok().and_then(|r| r.parse::<f32>().ok()) {
        for (i, b) in data.collision_brushes.iter().enumerate() {
            if (at.clamp(b.min, b.max) - at).length() < r {
                eprintln!("brush {i} {:.2}..{:.2}{}", b.min, b.max, if b.ladder { " ladder" } else { "" });
            }
        }
    }
    for (i, l) in nav.ladders.iter().enumerate() {
        if l.top.distance(at).min(l.bottom.distance(at)) < 8.0 {
            eprintln!(
                "ladder {i}: bottom {:.2} top {:.2} normal {:.2} length {:.2}",
                l.bottom, l.top, l.normal, l.length
            );
            for (j, a) in nav.areas.iter().enumerate() {
                for (n, via) in &a.links {
                    if matches!(via, Via::LadderUp(k) | Via::LadderDown(k) if *k == i) {
                        eprintln!("    {j} -> {n} {via:?}");
                    }
                }
            }
        }
    }
}

/// Walks a plain character from `P_AT` (feet, engine meters) with
/// `P_OPT` = yaw degrees, crouch, jump, seconds[, pitch degrees[,
/// forward[, side]]] and prints where it goes: for checking what a spot's
/// geometry lets through. `P_OPT2`, `P_OPT3`: more phases after it, the
/// same way (e.g. climb a ladder, then step off it at the top).
#[test]
#[ignore]
fn probe_walk() {
    if !installed() {
        return;
    }
    let v = |k: &str| -> Option<Vec<f32>> {
        std::env::var(k)
            .ok()
            .map(|s| s.split(',').map(|x| x.trim().parse().unwrap()).collect())
    };
    let at = v("P_AT").expect("P_AT=x,y,z");
    let phases: Vec<Vec<f32>> = ["P_OPT", "P_OPT2", "P_OPT3"].iter().filter_map(|k| v(k)).collect();
    let mut sim = sim(&std::env::var("MASHUP_NAV_MAP").unwrap_or_else(|_| "de_nuke".into()));
    let p = sim.spawn_character(Vec3::new(at[0], at[1] + HALF + 0.05, at[2]), movement::ID);
    sim.ticks(1);
    let mut t = 0.0;
    for (k, opts) in phases.iter().enumerate() {
        let mut tp = 0.0;
        while tp < opts[3] as f64 {
            {
                let mut i = sim.intent(p);
                i.yaw = opts[0].to_radians();
                i.pitch = opts.get(4).copied().unwrap_or(0.0).to_radians();
                i.move_axis = Vec2::new(opts.get(6).copied().unwrap_or(0.0), opts.get(5).copied().unwrap_or(1.0));
                i.crouch = opts[1] > 0.0;
                i.jump = opts[2] > 0.0 && (k > 0 || tp > 0.3);
            }
            sim.seconds(TICK_INTERVAL);
            tp += TICK_INTERVAL;
            t += TICK_INTERVAL;
            if ((t / TICK_INTERVAL).round() as u64).is_multiple_of(7) {
                eprintln!(
                    "{t:5.2} {:.2} v {:.2} ladder {} ground {}",
                    feet(&sim, p),
                    sim.velocity(p),
                    sim.state(p).on_ladder,
                    sim.state(p).on_ground
                );
            }
        }
    }
}
