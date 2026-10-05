//! Ropes and cables (`move_rope` / `keyframe_rope`), from
//! specs/cs_source/ropes.md. Each rope entity with a `NextKey` hangs a rope
//! to that entity: a short chain of nodes simulated until it settles, then
//! smoothed with a spline and drawn as a camera-facing strip (the map's
//! `MapRope`; the strip is built per view by the rope shader).

use std::collections::HashMap;

use bevy::prelude::*;
use vbsp::Bsp;

use super::{
    ambient::{MapLighting, Occluders},
    bsp::{METERS_PER_UNIT, to_engine},
    material::MaterialLoader,
    props::probe,
};
use crate::map::{MapData, MapRope};

/// Fixed simulation step, seconds.
pub const STEP: f32 = 1.0 / 50.0;
/// Rope gravity in the SDK 2013 rope code, units/s² (the spec's value).
pub const SPEC_GRAVITY: f32 = 1500.0;
/// Rope gravity in CS:S, measured with refcmp on de_dust2's overhead
/// cables: 800 (= sv_gravity) puts the 5-node cable within 2 px of the game
/// across the view; 1500 hangs it 15 px low. See specs/cs_source/ropes.md.
pub const CSS_GRAVITY: f32 = 800.0;
const DAMPING: f32 = 0.98;
const SWEEPS: usize = 3;
/// Added to every rope's length: Slack 100 is a rope exactly its span.
const SLACK_OFFSET: f32 = -100.0;
/// Steps simulated before a rope is first drawn (5 s, rounded up).
pub const INITIAL_STEPS: usize = 251;
/// A node is still moving while its squared step displacement exceeds this.
const REST_THRESHOLD: f32 = 0.03;
/// Safety cap when running to rest.
const MAX_STEPS: usize = 20_000;
const MAX_SUBDIV: usize = 7;

/// Simulated node count for the `Type` key (absent: 5).
pub fn node_count(rope_type: Option<i32>) -> usize {
    match rope_type {
        None => 5,
        Some(0) => 10,
        Some(1) => 4,
        Some(_) => 2,
    }
}

/// A hanging rope between two pinned ends, in Source units (Z up).
#[derive(Clone, Debug)]
pub struct RopeSim {
    pub nodes: Vec<Vec3>,
    previous: Vec<Vec3>,
    link: f32,
    /// Gravity's displacement per step: acceleration times step²/2, not
    /// step² (a quirk the sag depends on).
    gravity_step: f32,
}

impl RopeSim {
    /// A straight rope at rest. The rope's length is the span truncated to
    /// whole units, plus `slack` - 100, split evenly over the links.
    /// `gravity` in units/s² ([`SPEC_GRAVITY`] or [`CSS_GRAVITY`]).
    pub fn new(start: Vec3, end: Vec3, slack: i32, nodes: usize, gravity: f32) -> Self {
        let n = nodes.max(2);
        let span = start.distance(end).trunc();
        let link = ((span + slack as f32 + SLACK_OFFSET) / (n - 1) as f32).max(0.0);
        let nodes: Vec<Vec3> = (0..n).map(|i| start.lerp(end, i as f32 / (n - 1) as f32)).collect();
        Self {
            previous: nodes.clone(),
            nodes,
            link,
            gravity_step: gravity * STEP * STEP / 2.0,
        }
    }

    /// Maximum length of each link.
    pub fn link_length(&self) -> f32 {
        self.link
    }

    /// One fixed step; returns the largest squared node displacement.
    pub fn step(&mut self) -> f32 {
        let (start, end) = (self.nodes[0], *self.nodes.last().unwrap());
        for (p, q) in self.nodes.iter_mut().zip(self.previous.iter_mut()) {
            let next = *p + (*p - *q) * DAMPING - Vec3::Z * self.gravity_step;
            *q = *p;
            *p = next;
        }
        let last = self.nodes.len() - 1;
        for _ in 0..SWEEPS {
            for k in 0..last {
                let d = self.nodes[k] - self.nodes[k + 1];
                let len = d.length();
                if len > self.link {
                    let fix = d * (0.5 * (1.0 - self.link / len));
                    self.nodes[k] -= fix;
                    self.nodes[k + 1] += fix;
                }
            }
            self.nodes[0] = start;
            self.nodes[last] = end;
        }
        self.nodes
            .iter()
            .zip(&self.previous)
            .map(|(p, q)| p.distance_squared(*q))
            .fold(0.0, f32::max)
    }

    /// The shape the player sees once the rope stops moving: the initial
    /// hang, then steps until no node moves more than the rest threshold.
    pub fn settle(&mut self) {
        for _ in 0..INITIAL_STEPS {
            self.step();
        }
        for _ in INITIAL_STEPS..MAX_STEPS {
            if self.step() <= REST_THRESHOLD {
                break;
            }
        }
    }
}

/// The drawn points: every node, with `subdiv` (clamped to 0-7) uniform
/// Catmull-Rom points between neighbours. Also returns, per point, the
/// index of the node before it and how far it is toward the next one.
pub fn spline(nodes: &[Vec3], subdiv: usize) -> Vec<(Vec3, usize, f32)> {
    let s = subdiv.min(MAX_SUBDIV);
    let n = nodes.len();
    let mut out = vec![(nodes[0], 0, 0.0)];
    for i in 0..n - 1 {
        let p0 = nodes[i.saturating_sub(1)];
        let (p1, p2) = (nodes[i], nodes[i + 1]);
        let p3 = nodes[(i + 2).min(n - 1)];
        for k in 1..=s {
            let t = k as f32 / (s + 1) as f32;
            let c = p1
                + (p2 - p0) * (t / 2.0)
                + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * (t * t / 2.0)
                + (-p0 + p1 * 3.0 - p2 * 3.0 + p3) * (t * t * t / 2.0);
            out.push((c, i, t));
        }
        out.push((p2, i + 1, 0.0));
    }
    out
}

/// Texture V step per drawn point. The divisor undercounts the points, so
/// textures stretch more than `TextureScale` suggests (kept: it's how the
/// game looks).
pub fn v_step(span: f32, slack: i32, nodes: usize, subdiv: usize, texture_scale: f32, texture_height: f32) -> f32 {
    let length = (span.trunc() + slack as f32 + SLACK_OFFSET).max(0.0);
    let points = ((nodes - 1) * subdiv.min(MAX_SUBDIV) + 1) as f32;
    4.0 / texture_scale.max(0.01) * length / points / texture_height.max(1.0)
}

fn parse(v: &str) -> Option<Vec3> {
    let mut it = v.split_whitespace().filter_map(|p| p.parse::<f32>().ok());
    Some(Vec3::new(it.next()?, it.next()?, it.next()?))
}

/// Entity keys are case-insensitive in practice (Hammer writes "Slack").
fn key<'a>(e: &vbsp::RawEntity<'a>, name: &str) -> Option<&'a str> {
    e.properties()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

pub fn add_ropes(
    bsp: &Bsp,
    materials: &mut MaterialLoader,
    lighting: &MapLighting,
    occluders: &Occluders,
    data: &mut MapData,
) {
    let ropes: Vec<_> = bsp
        .entities
        .iter()
        .filter(|e| matches!(e.prop("classname"), Some("move_rope") | Some("keyframe_rope")))
        .collect();
    // The first entity with a name wins, as in the game.
    let mut by_name: HashMap<&str, Vec3> = HashMap::new();
    for e in &ropes {
        if let (Some(name), Some(origin)) = (key(e, "targetname"), key(e, "origin").and_then(parse)) {
            by_name.entry(name).or_insert(origin);
        }
    }
    let mut material_cache: HashMap<String, (Option<usize>, Option<usize>, Option<(Option<usize>, Option<usize>)>)> =
        HashMap::new();

    for e in &ropes {
        let (Some(start), Some(end)) = (
            key(e, "origin").and_then(parse),
            key(e, "nextkey").and_then(|n| by_name.get(n).copied()),
        ) else {
            continue;
        };
        let int = |k: &str| key(e, k).and_then(|s| s.trim().parse::<f32>().ok());
        let slack = int("slack").unwrap_or(0.0) as i32;
        let nodes = node_count(int("type").map(|t| t as i32));
        let subdiv = int("subdiv").map(|s| s.max(0.0) as usize).unwrap_or(2);
        let width = int("width").unwrap_or(2.0).max(0.0);
        let texture_scale = int("texturescale").unwrap_or(4.0);
        let material = key(e, "ropematerial")
            .unwrap_or("cable/cable")
            .trim_end_matches(".vmt")
            .to_lowercase();
        let (texture, normal_map, back) = *material_cache.entry(material.clone()).or_insert_with(|| {
            let r = materials.resolve(&material);
            // A "_back" material turns on the game's fake anti-aliasing.
            let back = materials
                .read(&format!("materials/{material}_back.vmt"))
                .is_some()
                .then(|| {
                    let b = materials.resolve(&format!("{material}_back"));
                    (b.texture, b.normal_map)
                });
            (r.texture, r.normal_map, back)
        });
        let texture_height = texture.map(|t| materials.textures[t].height as f32).unwrap_or(1.0);

        let mut sim = RopeSim::new(start, end, slack, nodes, CSS_GRAVITY);
        sim.settle();
        // One light sample per node: the probe averaged over its six axis
        // directions, clamped to 0-1 (the game's point-lighting query).
        let node_light: Vec<Vec3> = sim
            .nodes
            .iter()
            .map(|p| {
                let at = to_engine(vbsp::Vector { x: p.x, y: p.y, z: p.z });
                let lp = probe(bsp, lighting, occluders, at + Vec3::Y * 4.0 * METERS_PER_UNIT);
                let sum: Vec3 = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                    .into_iter()
                    .map(|d| lp.eval(d))
                    .sum();
                (sum / 6.0).clamp(Vec3::ZERO, Vec3::ONE)
            })
            .collect();

        let dv = v_step(start.distance(end), slack, nodes, subdiv, texture_scale, texture_height);
        let points = spline(&sim.nodes, subdiv);
        let mut rope = MapRope {
            width: width * METERS_PER_UNIT,
            texture,
            normal_map,
            back,
            ..default()
        };
        for (i, (p, node, t)) in points.iter().enumerate() {
            rope.points.push(to_engine(vbsp::Vector { x: p.x, y: p.y, z: p.z }));
            rope.v.push(dv * i as f32);
            let next = node_light[(*node + 1).min(nodes - 1)];
            rope.light.push(node_light[*node].lerp(next, *t));
        }
        data.ropes.push(rope);
    }
}
