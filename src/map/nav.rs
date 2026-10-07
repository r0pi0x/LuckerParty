//! Navigation meshes for bots, game-neutral, in engine space (meters, Y
//! up). Areas are axis-aligned XZ rectangles with a height at each corner;
//! links between them are directed. Queries and the A* search follow
//! specs/cs_source/nav.md (the Source nav system, the only one loaded so
//! far), so its constants are Source's, converted to meters.

use bevy::prelude::*;

const UNIT: f32 = 0.0254;
/// Climbable without jumping.
pub const STEP_HEIGHT: f32 = 18.0 * UNIT;
/// How far below a query point an area may lie and still contain it.
const BENEATH_LIMIT: f32 = 120.0 * UNIT;
/// How far above a query point an area may lie (feet on a slope).
const ABOVE_LIMIT: f32 = 5.0 * UNIT;
/// Stock cost penalties for entering crouch and jump areas (× distance).
const CROUCH_PENALTY: f64 = 20.0;
const JUMP_PENALTY: f64 = 5.0;
/// Path crossing points stay this far inside an edge with no area beyond.
const EDGE_MARGIN: f32 = 25.0 * UNIT;

/// Area attribute bits (the Source file's values).
pub mod flags {
    pub const CROUCH: u32 = 0x0001;
    pub const JUMP: u32 = 0x0002;
    pub const PRECISE: u32 = 0x0004;
    pub const NO_JUMP: u32 = 0x0008;
    pub const AVOID: u32 = 0x0080;
    pub const STAND: u32 = 0x0400;
    pub const STAIRS: u32 = 0x1000;
}

/// Hiding spot flag bits (the Source file's values).
pub mod spot {
    pub const IN_COVER: u8 = 0x01;
    pub const GOOD_SNIPER: u8 = 0x02;
    pub const IDEAL_SNIPER: u8 = 0x04;
    pub const EXPOSED: u8 = 0x08;
}

/// A place a bot can hide or hold from, on the ground of its area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HidingSpot {
    pub pos: Vec3,
    /// `spot` bits.
    pub flags: u8,
}

/// Potential visibility bits of `NavArea::visible`.
pub mod vis {
    pub const PARTLY: u8 = 0x01;
    pub const FULLY: u8 = 0x02;
}

/// Hiding spots that come into view, in order, when walking through an
/// area from one neighbour to another (spec "Encounter record").
#[derive(Clone, Debug, PartialEq)]
pub struct Encounter {
    /// Area indices entered from and left toward.
    pub from: usize,
    pub to: usize,
    /// Each spot's position and the fraction (0..1) of the way through
    /// where it becomes visible.
    pub spots: Vec<(Vec3, f32)>,
}

/// Which edge of an area a link leaves by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    MinX,
    MaxX,
    MinZ,
    MaxZ,
}

impl Side {
    pub fn opposite(self) -> Self {
        match self {
            Side::MinX => Side::MaxX,
            Side::MaxX => Side::MinX,
            Side::MinZ => Side::MaxZ,
            Side::MaxZ => Side::MinZ,
        }
    }
}

/// How a link is travelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Walk(Side),
    /// Climb ladder (index into `ladders`) up or down.
    LadderUp(usize),
    LadderDown(usize),
}

#[derive(Clone, Debug)]
pub struct NavArea {
    /// The file's id.
    pub id: u32,
    pub flags: u32,
    pub min: Vec2,
    pub max: Vec2,
    /// Heights at (min.x, min.y), (max.x, min.y), (max.x, max.y),
    /// (min.x, max.y), where `.y` of the Vec2s is engine Z.
    pub heights: [f32; 4],
    /// Centre for path costs. Its height is the mean of two diagonal
    /// corners, as the source format defines it (spec quirk), not the
    /// surface height there.
    pub center: Vec3,
    /// Outgoing links in the file's order: (area index, how).
    pub links: Vec<(usize, Via)>,
    /// Index into `NavMesh::places`, if named.
    pub place: Option<usize>,
    pub hiding: Vec<HidingSpot>,
    /// Potentially visible areas (index, `vis` bits), sorted by index;
    /// empty when the file has none (versions before 16).
    pub visible: Vec<(usize, u8)>,
    /// Corner checks along paths through this area (version 9 files).
    pub encounters: Vec<Encounter>,
}

#[derive(Clone, Debug)]
pub struct NavLadder {
    pub top: Vec3,
    pub bottom: Vec3,
    /// Path cost length, m.
    pub length: f32,
    /// Outward normal of the climbable face.
    pub normal: Vec3,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct NavMesh {
    pub areas: Vec<NavArea>,
    pub ladders: Vec<NavLadder>,
    pub places: Vec<String>,
}

impl NavArea {
    /// Surface height at (x, z), clamped onto the rectangle (bilinear).
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let size = self.max - self.min;
        if size.x <= 0.0 || size.y <= 0.0 {
            return self.heights[2];
        }
        let u = ((x - self.min.x) / size.x).clamp(0.0, 1.0);
        let v = ((z - self.min.y) / size.y).clamp(0.0, 1.0);
        let [a, b, c, d] = self.heights;
        let near = a + u * (b - a);
        let far = d + u * (c - d);
        near + v * (far - near)
    }

    /// Whether (x, z) lies on the rectangle, edges included.
    pub fn overlaps(&self, x: f32, z: f32) -> bool {
        self.min.x <= x && x <= self.max.x && self.min.y <= z && z <= self.max.y
    }

    /// The point of the area nearest to `p` (clamped, on the surface).
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        let x = p.x.clamp(self.min.x, self.max.x);
        let z = p.z.clamp(self.min.y, self.max.y);
        Vec3::new(x, self.height_at(x, z), z)
    }

    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }
}

impl NavMesh {
    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.areas.iter().position(|a| a.id == id)
    }

    /// The area under `p`: the highest one whose surface lies between
    /// 120 units below and 5 units above it (ties: first in file order).
    pub fn area_at(&self, p: Vec3) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, a) in self.areas.iter().enumerate() {
            if !a.overlaps(p.x, p.z) {
                continue;
            }
            let y = a.height_at(p.x, p.z);
            if y > p.y + ABOVE_LIMIT || y < p.y - BENEATH_LIMIT {
                continue;
            }
            if best.is_none_or(|(_, by)| y > by) {
                best = Some((i, y));
            }
        }
        best.map(|(i, _)| i)
    }

    /// `area_at`, else the area whose closest point is nearest to `p`.
    pub fn nearest_area(&self, p: Vec3) -> Option<usize> {
        self.area_at(p).or_else(|| {
            self.areas
                .iter()
                .enumerate()
                .map(|(i, a)| (i, a.closest_point(p).distance_squared(p)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        })
    }

    /// Stock path cost of entering `to` from `from` (spec "shortest path").
    pub fn step_cost(&self, from: usize, to: usize, via: Via) -> f64 {
        let (a, b) = (&self.areas[from], &self.areas[to]);
        let dist = match via {
            Via::LadderUp(l) | Via::LadderDown(l) => self.ladders[l].length as f64,
            Via::Walk(_) => (b.center.as_dvec3() - a.center.as_dvec3()).length(),
        };
        let mut cost = dist;
        if b.has(flags::CROUCH) {
            cost += CROUCH_PENALTY * dist;
        }
        if b.has(flags::JUMP) {
            cost += JUMP_PENALTY * dist;
        }
        cost
    }

    /// A* from area `start` to area `goal` with the stock cost. Returns the
    /// areas with how each was entered (the first has none) and the total
    /// cost, or None when unreachable.
    pub fn find_path(&self, start: usize, goal: usize) -> Option<(Vec<(usize, Option<Via>)>, f64)> {
        self.find_path_with(start, goal, |from, to, via| self.step_cost(from, to, via))
    }

    /// The step cost's distance part: ladder length or centre distance.
    pub fn step_length(&self, from: usize, to: usize, via: Via) -> f64 {
        match via {
            Via::LadderUp(l) | Via::LadderDown(l) => self.ladders[l].length as f64,
            Via::Walk(_) => (self.areas[to].center.as_dvec3() - self.areas[from].center.as_dvec3()).length(),
        }
    }

    /// `find_path` with another step cost (`cost(from, to, via)`, the cost
    /// of entering `to`; infinite: impassable). The heuristic stays the
    /// straight distance, so costs should not fall below the distance.
    pub fn find_path_with(
        &self,
        start: usize,
        goal: usize,
        cost_of: impl Fn(usize, usize, Via) -> f64,
    ) -> Option<(Vec<(usize, Option<Via>)>, f64)> {
        let n = self.areas.len();
        let goal_pos = self.areas[goal].center.as_dvec3();
        let h = |i: usize| (self.areas[i].center.as_dvec3() - goal_pos).length();
        let mut g = vec![f64::INFINITY; n];
        let mut parent: Vec<Option<(usize, Via)>> = vec![None; n];
        // Open list sorted by f; equal f keeps insertion order (FIFO).
        let mut open: Vec<(f64, usize)> = vec![(h(start), start)];
        let mut is_open = vec![false; n];
        g[start] = 0.0;
        is_open[start] = true;
        while !open.is_empty() {
            let (_, cur) = open.remove(0);
            is_open[cur] = false;
            if cur == goal {
                let mut out = vec![(cur, parent[cur].map(|p| p.1))];
                let mut at = cur;
                while let Some((p, _)) = parent[at] {
                    out.push((p, parent[p].map(|q| q.1)));
                    at = p;
                }
                out.reverse();
                return Some((out, g[goal]));
            }
            for &(next, via) in &self.areas[cur].links {
                if next == cur || parent[cur].is_some_and(|(p, _)| p == next) {
                    continue;
                }
                let step = cost_of(cur, next, via);
                if !step.is_finite() {
                    continue;
                }
                let mut cost = g[cur] + step;
                cost = cost.max(1.00001 * g[cur] + 0.00001);
                if g[next] <= cost {
                    continue;
                }
                g[next] = cost;
                parent[next] = Some((cur, via));
                if is_open[next] {
                    open.retain(|e| e.1 != next);
                }
                let f = cost + h(next);
                let at = open.partition_point(|e| e.0 <= f);
                open.insert(at, (f, next));
                is_open[next] = true;
            }
        }
        None
    }

    /// Where to cross from area `from` into `to` across `side`, nearest to
    /// `p`: on `from`'s edge, within both areas' shared extent, kept off
    /// edges with no area beyond (spec "closest crossing point").
    pub fn crossing(&self, from: usize, to: usize, side: Side, p: Vec3) -> Vec3 {
        let (a, b) = (&self.areas[from], &self.areas[to]);
        let along_x = matches!(side, Side::MinZ | Side::MaxZ);
        let (lo_a, hi_a, lo_b, hi_b, fixed) = if along_x {
            let z = if side == Side::MinZ { a.min.y } else { a.max.y };
            (a.min.x, a.max.x, b.min.x, b.max.x, z)
        } else {
            let x = if side == Side::MinX { a.min.x } else { a.max.x };
            (a.min.y, a.max.y, b.min.y, b.max.y, x)
        };
        let mut lo = lo_a.max(lo_b);
        let mut hi = hi_a.min(hi_b);
        let (lo_side, hi_side) = if along_x {
            (Side::MinX, Side::MaxX)
        } else {
            (Side::MinZ, Side::MaxZ)
        };
        if !self.two_way(to, lo_side) {
            lo += EDGE_MARGIN;
        }
        if !self.two_way(to, hi_side) {
            hi -= EDGE_MARGIN;
        }
        if lo > hi {
            lo = (lo + hi) / 2.0;
            hi = lo;
        }
        let t = if along_x { p.x } else { p.z }.clamp(lo, hi);
        let (x, z) = if along_x { (t, fixed) } else { (fixed, t) };
        Vec3::new(x, a.height_at(x, z), z)
    }

    /// Whether some neighbour across `side` of `area` links back to it.
    fn two_way(&self, area: usize, side: Side) -> bool {
        self.areas[area].links.iter().any(|&(n, via)| {
            via == Via::Walk(side)
                && self.areas[n]
                    .links
                    .iter()
                    .any(|&(m, v)| m == area && v == Via::Walk(side.opposite()))
        })
    }

    /// World points to walk through from `from` to `to` (engine space):
    /// crossing points between consecutive areas, ladder ends, then `to`.
    pub fn route(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let start = self.nearest_area(from)?;
        let goal = self.nearest_area(to)?;
        let (areas, _) = self.find_path(start, goal)?;
        Some(
            self.points_along(&areas, from, to, |_, _, _| 0.0)
                .into_iter()
                .map(|p| p.0)
                .collect(),
        )
    }

    /// World points (with the area each one leads into) to walk along an
    /// area path from `from` to `to`, as `route` makes them. `lean(from,
    /// to)` (-1..1) shifts each floor crossing from the nearest point
    /// toward one end of the shared edge (0: nearest; used to spread
    /// walkers over wide openings); it also gets the edge's length.
    pub fn points_along(
        &self,
        areas: &[(usize, Option<Via>)],
        from: Vec3,
        to: Vec3,
        lean: impl Fn(usize, usize, f32) -> f32,
    ) -> Vec<(Vec3, usize)> {
        let mut points = Vec::new();
        let mut at = from;
        for w in areas.windows(2) {
            let (prev, (next, via)) = (w[0].0, w[1]);
            let p = match via {
                Some(Via::Walk(side)) => {
                    let near = self.crossing(prev, next, side, at);
                    let (lo, hi) = self.portal_ends(prev, next, side);
                    let l = lean(prev, next, lo.distance(hi)).clamp(-1.0, 1.0);
                    let end = if l < 0.0 { lo } else { hi };
                    near.lerp(end, l.abs())
                }
                Some(Via::LadderUp(l)) => {
                    let ladder = &self.ladders[l];
                    points.push((ladder.bottom + ladder.normal * 32.0 * UNIT, prev));
                    ladder.top
                }
                Some(Via::LadderDown(l)) => {
                    let ladder = &self.ladders[l];
                    points.push((ladder.top - ladder.normal * 32.0 * UNIT, prev));
                    ladder.bottom
                }
                None => continue,
            };
            points.push((p, next));
            at = p;
        }
        points.push((to, areas.last().map_or(0, |a| a.0)));
        points
    }

    /// The two ends of the crossing range from `from` into `to` across
    /// `side` (as `crossing` clamps to), on `from`'s edge.
    pub fn portal_ends(&self, from: usize, to: usize, side: Side) -> (Vec3, Vec3) {
        let far = Vec3::splat(1e6);
        (self.crossing(from, to, side, -far), self.crossing(from, to, side, far))
    }

    /// Areas reachable from `start` within `max` of path length (centre
    /// distances, ladders by length), with their distance, nearest first.
    pub fn within(&self, start: usize, max: f32) -> Vec<(usize, f32)> {
        let mut dist = vec![f32::INFINITY; self.areas.len()];
        let mut out = Vec::new();
        let mut open = vec![(0.0f32, start)];
        dist[start] = 0.0;
        while let Some(i) = open
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
            .map(|(i, _)| i)
        {
            let (d, cur) = open.swap_remove(i);
            if d > dist[cur] {
                continue;
            }
            out.push((cur, d));
            for &(next, via) in &self.areas[cur].links {
                let nd = d + self.step_length(cur, next, via) as f32;
                if nd <= max && nd < dist[next] {
                    dist[next] = nd;
                    open.push((nd, next));
                }
            }
        }
        out
    }

    /// Whether area `b` is in `a`'s potentially visible set (any bits),
    /// or None when the mesh has no visibility data for `a`.
    pub fn sees(&self, a: usize, b: usize) -> Option<bool> {
        let list = &self.areas[a].visible;
        if list.is_empty() {
            return None;
        }
        Some(a == b || list.binary_search_by_key(&b, |v| v.0).is_ok())
    }
}
