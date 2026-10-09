//! Round objectives for any game with Counter-Strike-style scenarios
//! (specs/cs_source/objectives.md): the bomb (`bomb`: a carried weapon
//! armed inside a bomb target, a planted bomb that ticks, defusing,
//! the explosion) and hostages (`hostages`: characters led by +use to a
//! rescue zone). The zones and spawns come from the map's entities
//! (`MapObjectives`); the numbers, models and sounds from the game
//! (`bomb::BombRules`, `hostages::HostageRules`). The rules layer starts
//! each round's objectives (`round_start`), reads their state to end
//! rounds and pays for what happens (`ObjectiveEvent`).
//!
//! Units: meters, seconds, engine space (Y up) unless a name says
//! "units" (Source inches).

pub mod bomb;
pub mod hostages;
pub mod use_search;

use bevy::prelude::*;

use crate::{
    core::SimSet,
    map::{
        MapEntities,
        entities::{MapEntity, MapHull, engine_to_entity, entity_rotation, entity_to_engine},
    },
};

/// Source units (inches) to meters.
pub const UNIT: f32 = 0.0254;

pub struct ObjectivesPlugin;

impl Plugin for ObjectivesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapObjectives>()
            .init_resource::<RoundOpen>()
            .add_message::<ObjectiveEvent>()
            .add_message::<crate::map::entities::FireEntityOutput>()
            .add_message::<crate::map::GameSound>()
            .add_message::<crate::map::PlaySound>()
            .add_systems(
                FixedUpdate,
                load_objectives.before(SimSet::Rules).run_if(crate::core::authoritative),
            );
        bomb::plugin(app);
        hostages::plugin(app);
    }
}

/// Whether objectives may be worked on now (planting): the rules close it
/// during the freeze and after a round ends. Open without rounds.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoundOpen(pub bool);

impl Default for RoundOpen {
    fn default() -> Self {
        Self(true)
    }
}

/// Which scenario a map plays (spec 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    Bomb,
    Hostage,
    Neither,
}

/// A volume from the map: a brush entity's hulls (entity space, local to
/// its origin and angles), or a point entity's sphere.
#[derive(Clone, Debug)]
pub struct Zone {
    /// Index into `MapEntities` (its outputs), if from the map.
    pub map_index: Option<usize>,
    origin: Vec3,
    rotation: Quat,
    hulls: Vec<MapHull>,
    /// A point zone: this many units around the origin.
    radius: Option<f32>,
    /// Meters per unit.
    scale: f32,
}

impl Zone {
    /// A brush entity's volume, or (`radius`) a point entity's sphere.
    pub fn from_entity(index: usize, e: &MapEntity, scale: f32, radius: Option<f32>) -> Self {
        Self {
            map_index: Some(index),
            origin: e.origin(),
            rotation: entity_rotation(e.angles()),
            hulls: e.hulls.clone(),
            radius: if e.hulls.is_empty() { radius } else { None },
            scale,
        }
    }

    /// An axis-aligned box in engine space (tests, tools).
    pub fn engine_box(lo: Vec3, hi: Vec3) -> Self {
        let scale = UNIT;
        let (a, b) = (engine_to_entity(lo, scale), engine_to_entity(hi, scale));
        let (lo, hi) = (a.min(b), a.max(b));
        let planes = vec![
            (Vec3::X, hi.x),
            (Vec3::NEG_X, -lo.x),
            (Vec3::Y, hi.y),
            (Vec3::NEG_Y, -lo.y),
            (Vec3::Z, hi.z),
            (Vec3::NEG_Z, -lo.z),
        ];
        let points = vec![lo, hi];
        Self {
            map_index: None,
            origin: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            hulls: vec![MapHull { planes, points }],
            radius: None,
            scale,
        }
    }

    /// Whether an engine-space box (a character's hull) touches it.
    pub fn overlaps(&self, lo: Vec3, hi: Vec3) -> bool {
        let (a, b) = (engine_to_entity(lo, self.scale), engine_to_entity(hi, self.scale));
        let (c, h) = ((a + b) / 2.0, (a - b).abs() / 2.0);
        if let Some(r) = self.radius {
            let nearest = self.origin.clamp(c - h, c + h);
            return nearest.distance(self.origin) <= r;
        }
        let inv = self.rotation.inverse();
        let local = inv * (c - self.origin);
        // The box's half extents along the zone's axes.
        let m = Mat3::from_quat(inv);
        let half = Vec3::new(
            m.x_axis.x.abs() * h.x + m.y_axis.x.abs() * h.y + m.z_axis.x.abs() * h.z,
            m.x_axis.y.abs() * h.x + m.y_axis.y.abs() * h.y + m.z_axis.y.abs() * h.z,
            m.x_axis.z.abs() * h.x + m.y_axis.z.abs() * h.y + m.z_axis.z.abs() * h.z,
        );
        self.hulls.iter().any(|hull| {
            !hull.planes.is_empty()
                && hull
                    .planes
                    .iter()
                    .all(|(n, d)| n.dot(local) - (n.x.abs() * half.x + n.y.abs() * half.y + n.z.abs() * half.z) <= *d)
        })
    }

    /// The middle of its bottom (engine space): where to walk to.
    pub fn floor_centre(&self) -> Vec3 {
        let points: Vec<Vec3> = self.hulls.iter().flat_map(|h| h.points.iter().copied()).collect();
        let local = if points.is_empty() {
            Vec3::ZERO
        } else {
            let lo = points.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
            let hi = points.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
            Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, lo.z)
        };
        entity_to_engine(self.origin + self.rotation * local, self.scale)
    }
}

/// A hostage the map places (`hostage_entity`): feet (engine space), yaw
/// (radians, our convention: 0 looks down -Z), and its keyvalues' model
/// choice (`HostageType`, `model`).
#[derive(Clone, Debug, PartialEq)]
pub struct HostageSpawn {
    pub feet: Vec3,
    pub yaw: f32,
    pub kind: Option<i32>,
}

/// The objectives the loaded map has (spec 1, 9): bomb targets, rescue
/// zones, hostages and the bomb radius. Rebuilt when the map changes;
/// tests may set it directly.
#[derive(Resource, Clone, Debug, Default)]
pub struct MapObjectives {
    pub bomb_targets: Vec<Zone>,
    pub rescue_zones: Vec<Zone>,
    pub hostages: Vec<HostageSpawn>,
    /// `info_map_parameters` `bombradius`, units (None: the game's default).
    pub bomb_radius: Option<f32>,
    /// Which `MapEntities` this came from (0: set by hand).
    pub source: usize,
}

impl MapObjectives {
    pub fn kind(&self) -> MapKind {
        if !self.bomb_targets.is_empty() {
            MapKind::Bomb
        } else if !self.hostages.is_empty() {
            MapKind::Hostage
        } else {
            MapKind::Neither
        }
    }

    /// The bomb target an engine-space box touches (index).
    pub fn bomb_target_at(&self, lo: Vec3, hi: Vec3) -> Option<usize> {
        self.bomb_targets.iter().position(|z| z.overlaps(lo, hi))
    }

    /// From the map's entities (spec "Mapping": `func_bomb_target`,
    /// `func_hostage_rescue`, `hostage_entity`, `info_map_parameters`).
    pub fn from_entities(map: &MapEntities) -> Self {
        let mut out = Self {
            source: std::sync::Arc::as_ptr(&map.entities) as usize,
            ..default()
        };
        for (i, e) in map.entities.iter().enumerate() {
            let class = e.classname().to_ascii_lowercase();
            match class.as_str() {
                "func_bomb_target" | "info_bomb_target" => {
                    out.bomb_targets
                        .push(Zone::from_entity(i, e, map.scale, Some(POINT_TARGET_RADIUS)))
                }
                "func_hostage_rescue" | "info_hostage_rescue" => {
                    out.rescue_zones
                        .push(Zone::from_entity(i, e, map.scale, Some(POINT_RESCUE_RADIUS)))
                }
                "hostage_entity" => out.hostages.push(HostageSpawn {
                    feet: entity_to_engine(e.origin(), map.scale),
                    // Source yaw 0 looks down +X (engine +X); ours 0 is -Z.
                    yaw: (e.angles().y - 90.0).to_radians(),
                    kind: e.get("HostageType").and_then(|v| v.trim().parse().ok()),
                }),
                "info_map_parameters" => {
                    out.bomb_radius = e.get("bombradius").and_then(|v| v.trim().parse().ok());
                }
                _ => {}
            }
        }
        out
    }
}

/// `info_bomb_target` / `info_hostage_rescue` reach, units (*hyp.*, spec
/// Q13: no stock map uses them).
pub const POINT_TARGET_RADIUS: f32 = 128.0;
pub const POINT_RESCUE_RADIUS: f32 = 256.0;

fn load_objectives(map: Option<Res<MapEntities>>, mut objectives: ResMut<MapObjectives>) {
    let Some(map) = map else { return };
    let key = std::sync::Arc::as_ptr(&map.entities) as usize;
    if objectives.source != key {
        *objectives = MapObjectives::from_entities(&map);
    }
}

/// What happened with an objective this tick: for money (rules), messages
/// and sounds (client), bots.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ObjectiveEvent {
    /// Given the bomb at the round's start.
    GotBomb {
        who: Entity,
    },
    DroppedBomb {
        who: Entity,
    },
    PickedUpBomb {
        who: Entity,
    },
    /// Arming began / stopped short (`left_zone`: walked out of the
    /// target).
    BeginPlant {
        who: Entity,
    },
    AbortPlant {
        who: Entity,
        left_zone: bool,
    },
    /// Fire with the bomb did nothing: why (the game's words).
    PlantRefused {
        who: Entity,
        reason: &'static str,
    },
    Planted {
        who: Entity,
        at: Vec3,
        site: Option<usize>,
    },
    BeginDefuse {
        who: Entity,
        kit: bool,
    },
    AbortDefuse {
        who: Entity,
    },
    DefuseRefused {
        who: Entity,
        reason: &'static str,
    },
    Defused {
        who: Entity,
    },
    Exploded {
        at: Vec3,
    },
    /// A planted bomb beeped.
    Beep {
        at: Vec3,
    },
    PickedUpKit {
        who: Entity,
    },
    /// A hostage started following `leader` (`first`: the first time it
    /// was taken this round).
    HostageFollows {
        hostage: Entity,
        leader: Entity,
        first: bool,
    },
    HostageStops {
        hostage: Entity,
        leader: Entity,
    },
    /// Someone who may not lead hostages used one.
    HostageRefused {
        who: Entity,
    },
    HostageRescued {
        hostage: Entity,
        leader: Option<Entity>,
    },
    HostageHurt {
        hostage: Entity,
        attacker: Option<Entity>,
        amount: f32,
    },
    HostageKilled {
        hostage: Entity,
        attacker: Option<Entity>,
    },
}

/// Put the objectives back for a new round (called by the rules after
/// everyone is at their spawns): the bomb to a random terrorist, kits and
/// bombs from the last round gone, hostages back.
pub fn round_start(world: &mut World) {
    world.insert_resource(RoundOpen(false));
    bomb::round_start(world);
    hostages::round_start(world);
}
