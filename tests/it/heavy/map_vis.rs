//! Visibility culling on stock CS:S maps (map::vis): from places players
//! stand, every world surface a ray from the eye reaches belongs to a chunk
//! that is potentially visible, so culling never hides something the
//! camera could see. Skipped without an install.
//!
//! `MASHUP_VIS_FULL=1` checks every nav area and more rays (slow).

use bevy::prelude::*;
use mashup::{
    games::{self, cs_source},
    map::{
        MapAlpha, MapData,
        vis::{self, MapVisibility},
    },
    mount::config::LocalConfig,
};

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

/// Eye height of a standing player (64 units).
const EYE: f32 = 64.0 * 0.0254;

struct Tri {
    p: [Vec3; 3],
    chunk: usize,
    /// Rays stop here (no alpha test or blending).
    opaque: bool,
    /// Drawn from behind too; otherwise rays pass through its back.
    double_sided: bool,
}

/// World triangles in a uniform grid, for ray casts.
struct Grid {
    tris: Vec<Tri>,
    cells: std::collections::HashMap<IVec3, Vec<u32>>,
    size: f32,
    lo: Vec3,
    hi: Vec3,
}

impl Grid {
    fn new(tris: Vec<Tri>, size: f32) -> Self {
        let mut cells: std::collections::HashMap<IVec3, Vec<u32>> = Default::default();
        let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
        for (i, t) in tris.iter().enumerate() {
            let a = t.p[0].min(t.p[1]).min(t.p[2]);
            let b = t.p[0].max(t.p[1]).max(t.p[2]);
            lo = lo.min(a);
            hi = hi.max(b);
            let (ca, cb) = ((a / size).floor().as_ivec3(), (b / size).floor().as_ivec3());
            for x in ca.x..=cb.x {
                for y in ca.y..=cb.y {
                    for z in ca.z..=cb.z {
                        cells.entry(IVec3::new(x, y, z)).or_default().push(i as u32);
                    }
                }
            }
        }
        Self { tris, cells, size, lo, hi }
    }

    /// Chunks the ray reaches: every translucent surface before the first
    /// opaque one, and that one.
    fn hits(&self, from: Vec3, dir: Vec3, out: &mut Vec<(f32, usize)>) {
        let max_t = (self.hi - self.lo).length() + 1.0;
        let mut cell = (from / self.size).floor().as_ivec3();
        let step = dir.signum().as_ivec3();
        let next = |c: i32, s: i32, o: f32, d: f32| {
            if d == 0.0 {
                return f32::INFINITY;
            }
            let edge = (c + if s > 0 { 1 } else { 0 }) as f32 * self.size;
            (edge - o) / d
        };
        let mut t_max = Vec3::new(
            next(cell.x, step.x, from.x, dir.x),
            next(cell.y, step.y, from.y, dir.y),
            next(cell.z, step.z, from.z, dir.z),
        );
        let t_delta = (Vec3::splat(self.size) / dir.abs()).map(|v| if v.is_finite() { v } else { f32::INFINITY });
        let mut best = f32::INFINITY;
        let mut best_chunk = None;
        let mut seen: Vec<(f32, usize)> = Vec::new();
        let mut t_cell = 0.0;
        while t_cell < max_t && t_cell <= best {
            if let Some(list) = self.cells.get(&cell) {
                for &i in list {
                    let tri = &self.tris[i as usize];
                    let front = (tri.p[1] - tri.p[0]).cross(tri.p[2] - tri.p[0]).dot(dir) < 0.0;
                    if !front && !tri.double_sided {
                        continue;
                    }
                    if let Some(t) = ray_tri(from, dir, tri.p) {
                        if tri.opaque {
                            if t < best {
                                best = t;
                                best_chunk = Some(tri.chunk);
                            }
                        } else {
                            seen.push((t, tri.chunk));
                        }
                    }
                }
            }
            // Step to the next cell.
            if t_max.x < t_max.y && t_max.x < t_max.z {
                t_cell = t_max.x;
                t_max.x += t_delta.x;
                cell.x += step.x;
            } else if t_max.y < t_max.z {
                t_cell = t_max.y;
                t_max.y += t_delta.y;
                cell.y += step.y;
            } else {
                t_cell = t_max.z;
                t_max.z += t_delta.z;
                cell.z += step.z;
            }
        }
        out.extend(seen.into_iter().filter(|(t, _)| *t < best));
        out.extend(best_chunk.map(|c| (best, c)));
    }
}

/// Möller-Trumbore, both faces.
fn ray_tri(o: Vec3, d: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(-1e-4..=1.0 + 1e-4).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -1e-4 || u + v > 1.0 + 1e-4 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-3).then_some(t)
}

/// Whether the ray passes through solid (or out of the map) before `t`,
/// sampled every 5 cm.
fn through_solid(vis: &MapVisibility, from: Vec3, dir: Vec3, t: f32) -> bool {
    let steps = (t / 0.05) as usize;
    (1..steps).any(|i| vis.cluster_at(from + dir * (i as f32 * 0.05)).is_none())
}

/// Unit vectors spread evenly over the sphere.
fn directions(n: usize) -> Vec<Vec3> {
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    (0..n)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
            let r = (1.0 - y * y).sqrt();
            let a = golden * i as f32;
            Vec3::new(r * a.cos(), y, r * a.sin())
        })
        .collect()
}

/// Positions players stand at: spawns and nav area centres (on the area's
/// surface), at eye height.
fn eyes(map: &MapData, full: bool) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = map.spawns.iter().map(|(p, _)| *p + Vec3::Y * EYE).collect();
    if let Some(nav) = &map.nav {
        let step = if full { 1 } else { (nav.areas.len() / 250).max(1) };
        for a in nav.areas.iter().step_by(step) {
            let c = (a.min + a.max) / 2.0;
            out.push(Vec3::new(c.x, a.height_at(c.x, c.y) + EYE, c.y));
        }
    }
    out
}

fn check(name: &str) {
    let Some(map) = load(name) else { return };
    let full = std::env::var("MASHUP_VIS_FULL").is_ok_and(|v| v == "1");
    let vis: &MapVisibility = map.visibility.as_deref().expect("map has visibility data");
    assert!(vis.cluster_count > 10, "{} clusters", vis.cluster_count);

    // The chunks as spawned, and their triangles.
    let mut chunks: Vec<Vec<u32>> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut tris = Vec::new();
    for m in &map.meshes {
        for (chunk, clusters) in vis::split_mesh(m, Some(vis), vis::CHUNK_SIZE) {
            if chunk.skybox {
                continue;
            }
            let opaque = chunk.alpha == MapAlpha::Opaque;
            for t in chunk.indices.chunks_exact(3) {
                tris.push(Tri {
                    p: [0, 1, 2].map(|k| Vec3::from(chunk.positions[t[k] as usize])),
                    chunk: chunks.len(),
                    opaque,
                    double_sided: chunk.double_sided,
                });
            }
            chunks.push(clusters);
            names.push(chunk.material.clone());
        }
    }
    let grid = Grid::new(tris, 2.0);
    let dirs = directions(if full { 2048 } else { 400 });

    let eyes = eyes(&map, full);
    let (mut checked, mut outside, mut hidden_total, mut culled) = (0, 0, 0, 0usize);
    let mut failures = Vec::new();
    let mut hits = Vec::new();
    for eye in &eyes {
        let Some(cluster) = vis.cluster_at(*eye) else {
            outside += 1;
            continue;
        };
        checked += 1;
        culled += chunks.iter().filter(|c| !c.is_empty() && !vis.sees_any(cluster, c)).count();
        for d in &dirs {
            hits.clear();
            grid.hits(*eye, *d, &mut hits);
            for &(t, c) in &hits {
                // Untagged chunks are always drawn.
                // Rays that pass through solid on the way don't see it in
                // the game either: sky brushes aren't meshes here, but the
                // game draws them and they hide what's behind.
                if !chunks[c].is_empty()
                    && !vis.sees_any(cluster, &chunks[c])
                    && !through_solid(vis, *eye, *d, t)
                {
                    hidden_total += 1;
                    if failures.len() < 10 {
                        failures.push(format!(
                            "eye {eye} (cluster {cluster}) dir {d}: {} at {t:.2} m, hit point cluster {:?}, chunk {c} {:?}",
                            names[c],
                            vis.cluster_at(*eye + *d * t * 0.999),
                            chunks[c]
                        ));
                    }
                }
            }
        }
    }
    eprintln!(
        "{name}: {} chunks, {} clusters; {checked} eyes checked ({outside} in solid or outside); \
         {:.0}% of chunks culled on average",
        chunks.len(),
        vis.cluster_count,
        100.0 * culled as f32 / (checked.max(1) * chunks.len()) as f32
    );
    assert!(outside * 10 < eyes.len(), "{outside} of {} eyes found no cluster", eyes.len());
    assert!(hidden_total == 0, "{hidden_total} rays reached hidden chunks:\n{}", failures.join("\n"));
    // Culling must do something on these maps.
    assert!(culled > 0, "nothing culled");
}

#[test]
fn de_dust2_never_hides_what_the_eye_reaches() {
    check("de_dust2");
}

#[test]
fn de_nuke_never_hides_what_the_eye_reaches() {
    check("de_nuke");
}

#[test]
fn cs_office_never_hides_what_the_eye_reaches() {
    check("cs_office");
}

#[test]
fn outside_the_map_has_no_cluster() {
    let Some(map) = load("de_dust2") else { return };
    let vis = map.visibility.as_deref().unwrap();
    let (lo, hi) = map.bounds();
    // Far above everything, and far beyond the bounds.
    assert_eq!(vis.cluster_at(Vec3::new(0.0, hi.y + 500.0, 0.0)), None);
    assert_eq!(vis.cluster_at(lo - Vec3::splat(500.0)), None);
    // A CT spawn is inside one.
    assert!(vis.cluster_at(map.spawns[0].0 + Vec3::Y * EYE).is_some());
}
