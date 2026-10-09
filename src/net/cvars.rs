//! Server cvars in a network game (docs/plans/active/multiplayer.md,
//! "Console and cvars"; Source's FCVAR_REPLICATED): the server owns its
//! `sv_*`, `mp_*`, ... settings (`console::CvarScope`). The replicated
//! ones go to a client when it joins and whenever they change, so it
//! predicts its movement with the server's numbers (`sv_airaccelerate`,
//! `sv_gravity`) and shows the server's rules. While connected a client
//! can't set them (`console::execute` refuses), and anything else that
//! changes one (a map load, a debug slider) is put back to the server's
//! value; on leaving, its own values come back.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use super::{CvarValues, server::Player};
use crate::{
    console::{Console, CvarScope},
    core::NetRole,
};

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<SentCvars>()
        .init_resource::<ServerCvars>()
        .add_systems(
            PostUpdate,
            send_changes
                .before(ServerSystems::Send)
                .run_if(resource_equals(NetRole::Server)),
        )
        .add_systems(
            PreUpdate,
            receive_values
                .after(ClientSystems::Receive)
                .run_if(resource_equals(NetRole::Client)),
        );
}

/// On a server: the replicated values clients have been sent.
#[derive(Resource, Default)]
struct SentCvars(HashMap<String, String>);

/// On a client: the server's values, and ours from before we joined (put
/// back when we leave).
#[derive(Resource, Default, Debug, Clone)]
pub struct ServerCvars {
    pub values: HashMap<String, String>,
    saved: HashMap<String, String>,
}

/// Every replicated cvar's current value.
pub fn replicated_values(world: &mut World) -> Vec<(String, String)> {
    let cvars: Vec<_> = world
        .resource::<Console>()
        .cvars()
        .filter(|c| c.scope == CvarScope::Replicated)
        .cloned()
        .collect();
    let mut out: Vec<(String, String)> = cvars
        .into_iter()
        .filter_map(|c| Some((c.name.clone(), (c.get)(world)?)))
        .collect();
    out.sort();
    out
}

/// All replicated values to one client (it just joined).
pub(super) fn send_all(world: &mut World, client: Entity) {
    let values = replicated_values(world);
    world.resource_mut::<SentCvars>().0.extend(values.iter().cloned());
    world.write_message(ToClients {
        targets: SendTargets::Single(ClientId::Client(client)),
        message: CvarValues { values },
    });
}

/// Replicated values that changed, to every client.
fn send_changes(world: &mut World) {
    if world.query_filtered::<(), With<Player>>().iter(world).next().is_none() {
        return;
    }
    let now = replicated_values(world);
    let sent = &mut world.resource_mut::<SentCvars>().0;
    let changed: Vec<(String, String)> = now
        .into_iter()
        .filter(|(k, v)| sent.get(k) != Some(v))
        .collect();
    if changed.is_empty() {
        return;
    }
    sent.extend(changed.iter().cloned());
    info!(
        "server cvars to clients: {}",
        changed.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")
    );
    world.write_message(ToClients {
        targets: SendTargets::CLIENTS_ONLY,
        message: CvarValues { values: changed },
    });
}

/// The server's values: set here (ours kept to put back), and kept so.
fn receive_values(world: &mut World) {
    let incoming: Vec<(String, String)> = world
        .resource_mut::<Messages<CvarValues>>()
        .drain()
        .flat_map(|m| m.values)
        .collect();
    {
        let mut s = world.resource_mut::<ServerCvars>();
        for (k, v) in incoming {
            s.values.insert(k.to_lowercase(), v);
        }
    }
    enforce(world);
}

/// Put any server cvar that differs back to the server's value.
fn enforce(world: &mut World) {
    let values = world.resource::<ServerCvars>().values.clone();
    for (name, value) in values {
        let Some(cvar) = world.resource::<Console>().cvar(&name).cloned() else {
            continue;
        };
        if cvar.scope != CvarScope::Replicated {
            continue;
        }
        let current = (cvar.get)(world);
        if current.as_deref() == Some(value.as_str()) {
            continue;
        }
        if let Some(old) = current {
            world.resource_mut::<ServerCvars>().saved.entry(name.clone()).or_insert(old);
        }
        if let Err(e) = (cvar.set)(world, &value) {
            warn!("server cvar {name} {value}: {e}");
        }
    }
}

/// Left the server: our own values back.
pub(super) fn restore(world: &mut World) {
    let saved = std::mem::take(&mut *world.resource_mut::<ServerCvars>());
    for (name, value) in saved.saved {
        if let Some(cvar) = world.resource::<Console>().cvar(&name).cloned() {
            let _ = (cvar.set)(world, &value);
        }
    }
}
