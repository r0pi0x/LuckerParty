//! Community surf maps from the user's content cache (never in the repo),
//! headless: a CS:S-movement player put on a map's ramps keeps its speed
//! (no stop with open space ahead), and surf_sedona, compiled with HDR
//! lighting only, is fullbright as in CS:S. Each test skips maps that
//! aren't in the cache.

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use mashup::{
    core::{MapBrush, MapBrushes, MapTerrain, Velocity},
    games::{
        self, cs_source,
        cs_source::movement::{self, SourceMovementConfig, SourceMovementPlugin, to_engine, to_source},
    },
    harness::Sim,
    map::{MapData, MapPlugin},
    mount::config::{LocalConfig, content_dir},
};

fn cached(name: &str) -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    let present = content_dir(cs_source::GAME).is_some_and(|d| d.join("maps").join(format!("{name}.bsp")).is_file());
    if !installed || !present {
        eprintln!("skipping: {name} not in the content cache");
        return None;
    }
    Some(games::load_map(&format!("cs_source:{name}")).expect(name))
}

/// The player's box half size, Source units.
const HALF: Vec3 = Vec3::new(16.0, 16.0, 36.0);

/// A surfable face: a point on it (Source units), its normal (Source), the
/// horizontal direction along it and the brush's extent that way.
#[derive(Clone, Copy, Debug)]
struct Ramp {
    brush: usize,
    at: Vec3,
    normal: Vec3,
    along: Vec3,
    span: (f32, f32),
}

/// Faces too steep to stand on (normal z 0.2-0.68) of brushes over 600
/// units long, a point on each from the brush's centre.
fn ramps(brushes: &[MapBrush]) -> Vec<Ramp> {
    let mut out = Vec::new();
    for (i, b) in brushes.iter().enumerate() {
        if ((b.max - b.min) / 0.0254).max_element() < 600.0 {
            continue;
        }
        for (k, (n, d)) in b.planes.iter().enumerate() {
            // Engine: y up.
            if !(0.2..0.68).contains(&n.y) {
                continue;
            }
            let c = (b.min + b.max) / 2.0;
            let p = c + *n * (d - n.dot(c));
            if !b
                .planes
                .iter()
                .enumerate()
                .all(|(j, (m, e))| j == k || m.dot(p) <= e + 1e-3)
            {
                continue;
            }
            let normal = to_source(*n).normalize();
            let along = normal.cross(Vec3::Z).normalize();
            let (lo, hi) = (to_source(b.min), to_source(b.max));
            let span = (0..8)
                .map(|k| {
                    Vec3::new(
                        if k & 1 == 0 { lo.x } else { hi.x },
                        if k & 2 == 0 { lo.y } else { hi.y },
                        if k & 4 == 0 { lo.z } else { hi.z },
                    )
                    .dot(along)
                })
                .fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)));
            out.push(Ramp {
                brush: i,
                at: to_source(p),
                normal,
                along,
                span,
            });
        }
    }
    out
}

/// Whether a box (Source centre and half size) overlaps any brush.
fn blocked(brushes: &[MapBrush], centre: Vec3, half: Vec3) -> bool {
    let c = to_engine(centre);
    let h = Vec3::new(half.x, half.z, half.y) * 0.0254;
    brushes.iter().any(|b| {
        b.max.cmpgt(c - h).all()
            && b.min.cmplt(c + h).all()
            && b.planes.iter().all(|(n, d)| n.dot(c) - (d + n.abs().dot(h)) < -1e-3)
    })
}

/// Ramps with open space above them for `len` units along them, one face
/// per brush, each set to run the way with more room.
fn open_ramps(brushes: &[MapBrush], len: f32) -> Vec<Ramp> {
    let mut out: Vec<Ramp> = Vec::new();
    for mut r in ramps(brushes) {
        if out.iter().any(|o| o.brush == r.brush) {
            continue;
        }
        let a = r.at.dot(r.along);
        if r.span.1 - a < a - r.span.0 {
            r.along = -r.along;
            r.span = (-r.span.1, -r.span.0);
        }
        if r.span.1 - r.at.dot(r.along) < len {
            continue;
        }
        let support = r.normal.abs().dot(HALF);
        let clear = (0..=(len as i32 / 32)).all(|k| {
            let at = r.at + r.along * (k as f32 * 32.0);
            [2.0, 24.0, 64.0]
                .iter()
                .all(|lift| !blocked(brushes, at + r.normal * (support + lift), HALF))
        });
        if clear {
            out.push(r);
        }
    }
    out
}

/// Put a player on the ramp moving along it at 1200 units/s, with the
/// strafe key into the ramp held (`strafe`, view along the velocity) or no
/// keys, for 40 ticks. Returns the ghost stops: speed lost by more than
/// 20% in a tick with open space ahead (no brush where the box was
/// heading). Stops against real geometry and teleports end the ride.
fn ride(sim: &mut Sim, brushes: &[MapBrush], r: &Ramp, strafe: bool) -> Vec<String> {
    let support = r.normal.abs().dot(HALF);
    let p = sim.spawn_character(to_engine(r.at + r.normal * (support + 2.0)), movement::ID);
    sim.app.world_mut().get_mut::<Velocity>(p).unwrap().0 = to_engine(r.along * 1200.0);
    let into = Vec3::new(-r.normal.x, -r.normal.y, 0.0).normalize_or_zero();
    let mut ghosts = Vec::new();
    let mut prev = (to_source(sim.position(p)), r.along * 1200.0);
    for tick in 0..40 {
        let v = prev.1;
        let yaw = v.y.atan2(v.x);
        let right = Vec3::new(yaw.sin(), -yaw.cos(), 0.0);
        let side = if right.dot(into) > 0.0 { 1.0 } else { -1.0 };
        {
            let mut i = sim.intent(p);
            i.yaw = yaw - std::f32::consts::FRAC_PI_2;
            i.move_axis = Vec2::new(if strafe { side } else { 0.0 }, 0.0);
        }
        sim.ticks(1);
        let (pos, vel) = (to_source(sim.position(p)), to_source(sim.velocity(p)));
        if (pos - prev.0).length() > prev.1.length() / 64.0 + 100.0 {
            break; // teleported
        }
        if vel.length() < prev.1.length() * 0.8 {
            if tick == 0 {
                // Put down overlapping something `blocked` doesn't see (a
                // prop's mesh collider): no ride.
                break;
            }
            // Where the box was heading: a step on from where it stopped,
            // lifted off the ramp a little.
            let ahead = pos + prev.1.normalize_or_zero() * 8.0 + r.normal * 2.0;
            // Props (static ones are mesh colliders) count as real
            // geometry too.
            let centre = to_engine(ahead);
            let prop = sim
                .app
                .world_mut()
                .run_system_once(
                    move |q: avian3d::prelude::SpatialQuery,
                          chars: Query<Entity, With<mashup::core::Intent>>,
                          bodies: Query<(), With<mashup::core::MapBrushCollider>>| {
                        let shape = avian3d::prelude::Collider::cuboid(
                            HALF.x * 2.0 * 0.0254,
                            HALF.z * 2.0 * 0.0254,
                            HALF.y * 2.0 * 0.0254,
                        );
                        let filter = avian3d::prelude::SpatialQueryFilter::from_excluded_entities(chars.iter())
                            .with_mask(mashup::core::SOLID_LAYERS);
                        q.shape_intersections(&shape, centre, Quat::IDENTITY, &filter)
                            .into_iter()
                            .any(|e| !bodies.contains(e))
                    },
                )
                .unwrap_or(false);
            // And brush entities (func_brush, doors, movelinears).
            let movers: Vec<MapBrush> = {
                let world = sim.app.world_mut();
                let mut q = world.query::<&mashup::core::MovingSolid>();
                q.iter(world)
                    .filter(|m| m.solid)
                    .flat_map(|m| m.brushes.clone())
                    .collect()
            };
            if std::env::var("MASHUP_SURF_DEBUG").is_ok() {
                // The real hull (62 high, its centre 5 below the transform).
                let c = to_engine(pos - Vec3::Z * 5.0);
                let h = Vec3::new(16.0, 31.0, 16.0) * 0.0254;
                let hits = sim
                    .app
                    .world_mut()
                    .run_system_once(
                        move |q: avian3d::prelude::SpatialQuery,
                              chars: Query<Entity, With<mashup::core::Intent>>,
                              names: Query<(Option<&Name>, Has<mashup::core::MapBrushCollider>)>| {
                            let shape = avian3d::prelude::Collider::cuboid(
                                h.x * 2.0 - 0.0005,
                                h.y * 2.0 - 0.0005,
                                h.z * 2.0 - 0.0005,
                            );
                            let filter = avian3d::prelude::SpatialQueryFilter::from_excluded_entities(chars.iter())
                                .with_mask(mashup::core::SOLID_LAYERS);
                            q.shape_intersections(&shape, c, Quat::IDENTITY, &filter)
                                .into_iter()
                                .map(|e| format!("{e} {:?}", names.get(e).ok().map(|(n, b)| (n.map(|n| n.to_string()), b))))
                                .collect::<Vec<_>>()
                        },
                    )
                    .unwrap_or_default();
                eprintln!("  ramp {} stop: colliders {hits:?}", r.brush);
                for (i, b) in brushes.iter().enumerate() {
                    if b.max.cmpgt(c - h * 2.0).all() && b.min.cmplt(c + h * 2.0).all() {
                        let seps: Vec<f32> = b
                            .planes
                            .iter()
                            .map(|(n, d)| (n.dot(c) - (d + n.abs().dot(h))) / 0.0254)
                            .collect();
                        let worst = seps.iter().copied().fold(f32::MIN, f32::max);
                        eprintln!(
                            "  ramp {} stop: brush {i} ({} planes) deepest separation {worst:.4}: {seps:.3?}",
                            r.brush,
                            b.planes.len()
                        );
                    }
                }
            }
            if !prop && !blocked(brushes, ahead, HALF) && !blocked(&movers, ahead, HALF) {
                ghosts.push(format!(
                    "ramp {} ({:?}), strafe {strafe}, tick {tick}: speed {:.0} -> {:.0} at {pos:.1}",
                    r.brush,
                    r.at,
                    prev.1.length(),
                    vel.length()
                ));
            }
            break;
        }
        prev = (pos, vel);
    }
    sim.app.world_mut().despawn(p);
    ghosts
}

/// Players surf every open ramp of several surf maps without a ghost stop
/// (a stop with open space ahead). Before, most surf_sedona ramps (high
/// and far from the origin) stopped players dead: a velocity just clipped
/// along a ramp read as going into it again (`Tracer::sweep_brushes`).
#[test]
fn surfing_keeps_speed_on_surf_maps() {
    rides(&["surf_sedona", "surf_boreas", "surf_apollo", "surf_jive"], 15);
}

/// The other surf maps of the cache, the same rides (course flows,
/// community-maps.md); some have few long open ramps (short or boxed-in
/// ones, curved displacement ramps), so no minimum. surf_nebula's 13 stops
/// of 84 rides were its Propper ramp models, which ship no collision
/// model: we collided with their meshes, a hair off the player-clip ramps
/// under them, and the box ended inside a mesh (static props without a
/// collision model block nothing now). To see why a ride stops:
/// `MASHUP_SURF_MAPS=<map> MASHUP_SURF_DEBUG=1` lists what the hull is
/// near at each stop, and `MASHUP_SLIDE_DEBUG=1` prints every slide move's
/// sweeps (docs/OBSERVABILITY.md).
#[test]
fn surfing_keeps_speed_on_the_other_surf_maps() {
    rides(
        &[
            "surf_botanica",
            "surf_demise",
            "surf_halloween_tf2",
            "surf_happyhands",
            "surf_hellenic",
            "surf_holiday",
            "surf_inferno",
            "surf_kismet",
            "surf_nebula",
            "surf_nsz_fix",
            "surf_sacrifice",
            "surf_slob",
            "surf_stickybutt_alpha",
            "surf_surreal",
            "surf_threnody",
        ],
        0,
    );
}

/// Every open ramp of each map ridden with and without strafing: no ghost
/// stop, and at least `min_ramps` ramps found on each.
fn rides(maps: &[&str], min_ramps: usize) {
    let mut failed = Vec::new();
    // `MASHUP_SURF_MAPS=surf_a,surf_b`: those instead.
    let chosen: Vec<String> = match std::env::var("MASHUP_SURF_MAPS") {
        Ok(only) => only.split(',').map(String::from).collect(),
        Err(_) => maps.iter().map(|m| m.to_string()).collect(),
    };
    for name in &chosen {
        let name = name.as_str();
        let Some(map) = cached(name) else { continue };
        let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin));
        sim.app.insert_resource(SourceMovementConfig::default());
        sim.ticks(2);
        // A surf server's air acceleration.
        sim.app
            .world_mut()
            .resource_mut::<SourceMovementConfig>()
            .set_cvar("sv_airaccelerate", "150")
            .unwrap();
        let brushes = sim.app.world().resource::<MapBrushes>().0.clone();
        let ramps = open_ramps(&brushes, 700.0);
        // What blocks: brushes and terrain (displacements).
        let mut solids = brushes;
        if let Some(t) = sim.app.world().get_resource::<MapTerrain>() {
            solids.extend(t.brushes.iter().cloned());
        }
        let ghosts: Vec<String> = ramps
            .iter()
            .flat_map(|r| [false, true].map(|strafe| (r, strafe)))
            .flat_map(|(r, strafe)| ride(&mut sim, &solids, r, strafe))
            .collect();
        eprintln!("{name}: {} open ramps, {} ghost stops", ramps.len(), ghosts.len());
        if ramps.len() < min_ramps {
            failed.push(format!("{name}: only {} open ramps found", ramps.len()));
        }
        failed.extend(ghosts.into_iter().map(|g| format!("{name}: {g}")));
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

/// surf_sedona has HDR lighting only (no LDR lightmap lump, its LDR face
/// lump's lighting offsets and styles meaningless) and CS:S draws it
/// fullbright: the world at full light, props too (not black from its
/// all-zero LDR ambient samples), instead of LDR faces read through the
/// HDR lump (garbage).
#[test]
fn surf_sedona_is_fullbright() {
    let Some(map) = cached("surf_sedona") else { return };
    let lm = map.lightmap.as_ref().expect("lightmap");
    // Only the full-light block faces without lighting share (the rest of
    // the atlas unused).
    let lit = lm
        .rgb
        .iter()
        .filter(|c| c.iter().any(|v| *v != 0.0 && *v != 1.0))
        .count();
    assert_eq!(lit, 0, "{lit} of {} luxels from lighting data", lm.rgb.len());
    assert!(
        lm.rgb.len() <= 1024 * 8,
        "an atlas of {}x{}: lightmaps read",
        lm.width,
        lm.height
    );
    let statics: Vec<_> = map.props.iter().filter(|p| p.entity.is_none()).collect();
    let dark = statics
        .iter()
        .filter(|p| {
            let probe = p.lighting.as_ref().unwrap();
            [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                .iter()
                .any(|d| probe.eval(*d).min_element() < 0.99)
        })
        .count();
    assert_eq!(dark, 0, "{dark} of {} static props not at full light", statics.len());
    // At mat_hdr_level 2 it has its HDR lighting, through its HDR faces.
    let hdr = games::load_map_level("cs_source:surf_sedona", 2).unwrap();
    let lm = hdr.lightmap.as_ref().expect("HDR lightmap");
    let lit = lm
        .rgb
        .iter()
        .filter(|c| c.iter().any(|v| (v - 1.0).abs() > 1e-3))
        .count();
    let wild = lm
        .rgb
        .iter()
        .filter(|c| c.iter().any(|v| !v.is_finite() || *v > 64.0))
        .count();
    assert!(lit * 2 > lm.rgb.len(), "HDR lighting read");
    assert!(wild * 1000 < lm.rgb.len(), "{wild} wild luxels");
}

