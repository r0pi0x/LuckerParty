//! Decals for players joining mid-game (docs/plans/active/multiplayer.md,
//! slice 7's leftover): the server keeps the impact decals put on the
//! world since its map loaded (`DecalLog`, the newest
//! `map::decal::MAX_DECALS`) and sends them to a client when it comes into
//! the game (`Loaded`); the client places them as its own
//! (`map::decal::PlaceDecal`). Decals on props and brush entities (which
//! move, break and come back) aren't kept, nor is which variant a decal
//! took (each client picks its own, as for every shot it draws).

use std::collections::VecDeque;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use super::server::Player;
use crate::{
    core::NetRole,
    map::{
        MapBrushEntity, PropIndex,
        decal::{DecalGroup, MAX_DECALS, PlaceDecal},
    },
};

/// One world decal: its group (a named one or a surface material's), where,
/// the surface's normal, the direction of what made it, spun or upright.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetDecal {
    pub named: Option<String>,
    pub material: Option<char>,
    pub point: [f32; 3],
    pub normal: [f32; 3],
    pub dir: [f32; 3],
    pub spin: bool,
}

impl NetDecal {
    fn of(p: &PlaceDecal) -> Self {
        let (named, material) = match &p.group {
            DecalGroup::Named(n) => (Some(n.clone()), None),
            DecalGroup::Material(m) => (None, Some(*m)),
        };
        Self {
            named,
            material,
            point: p.point.to_array(),
            normal: p.normal.to_array(),
            dir: p.dir.to_array(),
            spin: p.spin,
        }
    }

    fn place(&self) -> Option<PlaceDecal> {
        let group = match (&self.named, self.material) {
            (Some(n), _) => DecalGroup::Named(n.chars().take(64).collect()),
            (None, Some(m)) => DecalGroup::Material(m),
            (None, None) => return None,
        };
        let finite = |v: [f32; 3]| v.iter().all(|x| x.is_finite());
        (finite(self.point) && finite(self.normal) && finite(self.dir)).then(|| PlaceDecal {
            target: None,
            group,
            point: Vec3::from_array(self.point),
            normal: Vec3::from_array(self.normal),
            dir: Vec3::from_array(self.dir),
            spin: self.spin,
        })
    }
}

/// Server -> a client coming into the game: the world's decals so far,
/// oldest first.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Decals {
    pub list: Vec<NetDecal>,
}

/// On a server: the world's decals since the map loaded, oldest first.
#[derive(Resource, Default, Debug)]
pub struct DecalLog {
    pub decals: VecDeque<NetDecal>,
    /// The map loads counted when it was last cleared (`rules::MapLoads`).
    loads: u32,
}

pub(super) fn plugin(app: &mut App) {
    app.add_server_message::<Decals>(Channel::Ordered)
        .make_message_independent::<Decals>()
        .init_resource::<DecalLog>()
        .add_message::<PlaceDecal>()
        .add_systems(PostUpdate, record.run_if(resource_equals(NetRole::Server)))
        .add_systems(
            PreUpdate,
            receive
                .after(ClientSystems::Receive)
                .run_if(resource_equals(NetRole::Client)),
        );
}

/// Keep the world's new decals (a new map starts the log over).
fn record(
    mut asks: MessageReader<PlaceDecal>,
    loads: Option<Res<crate::rules::MapLoads>>,
    movable: Query<(), Or<(With<PropIndex>, With<MapBrushEntity>)>>,
    parents: Query<&ChildOf>,
    mut log: ResMut<DecalLog>,
) {
    let loads = loads.map_or(0, |l| l.0);
    if log.loads != loads {
        log.loads = loads;
        log.decals.clear();
    }
    for ask in asks.read() {
        let on_movable = ask.target.is_some_and(|t| {
            std::iter::once(t)
                .chain(parents.get(t).ok().map(|c| c.parent()))
                .any(|e| movable.contains(e))
        });
        if on_movable {
            continue;
        }
        log.decals.push_back(NetDecal::of(ask));
        while log.decals.len() > MAX_DECALS {
            log.decals.pop_front();
        }
    }
}

/// Send `client` the world's decals so far (it just came into the game).
pub(super) fn send(world: &mut World, client: Entity) {
    let list: Vec<NetDecal> = world.resource::<DecalLog>().decals.iter().cloned().collect();
    if list.is_empty() || world.get::<Player>(client).is_none() {
        return;
    }
    world.write_message(ToClients {
        targets: SendTargets::Single(ClientId::Client(client)),
        message: Decals { list },
    });
}

/// The server's decals, placed here.
fn receive(mut got: MessageReader<Decals>, mut place: MessageWriter<PlaceDecal>) {
    for d in got.read() {
        place.write_batch(d.list.iter().take(MAX_DECALS).filter_map(NetDecal::place));
    }
}
