//! Chat and radio in a network game (docs/plans/active/multiplayer.md,
//! slice 5).
//!
//! - **Chat**: `say`/`say_team` from a client (or a listen server's host)
//!   is a `SayRequest`; the server decides who reads it
//!   (`map::radio::sees_say`: team chat within the team, the living don't
//!   read the dead) and sends each of them a `ChatMessage` with the
//!   sender's name, team, whether alive and where; the client layer puts
//!   it in the game's format (`map::radio::SayFormats`).
//! - **Radio**: a client's radio command is a `RadioRequest`; the server
//!   makes it the sender's `core::Radio` (as its bots' and grenade
//!   throws' calls are) and sends each call to the players who hear it
//!   (`map::radio::hears`: living teammates) as `RadioCall`, which their
//!   client plays as its own `core::Radio`.

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use super::{ChatMessage, NetCharacter, RadioCall, RadioRequest, SayRequest, server::Player};
use crate::{
    core::{Health, LocalPlayer, NetRole, Radio, Team},
    map::radio::{hears, sees_say},
    rules::Dead,
};

/// Most bytes a chat line keeps (Source's say limit).
pub const MAX_SAY: usize = 127;

pub(super) fn plugin(app: &mut App) {
    app.add_message::<Radio>()
        .add_systems(
            PreUpdate,
            (receive_says, receive_radio)
                .after(ServerSystems::Receive)
                .run_if(in_state(ServerState::Running)),
        )
        .add_systems(
            PostUpdate,
            (forward_radio, forward_hud)
                .before(ServerSystems::Send)
                .run_if(resource_equals(NetRole::Server)),
        )
        .add_systems(
            PreUpdate,
            (receive_calls, receive_hud)
                .after(ClientSystems::Receive)
                .run_if(resource_equals(NetRole::Client)),
        );
}

/// What map logic shows on HUDs (`logic::HudEvent`) to the client of the
/// player it is for, or to every client.
fn forward_hud(
    mut events: MessageReader<crate::logic::HudEvent>,
    players: Query<(Entity, &Player)>,
    mut out: MessageWriter<ToClients<super::MapHud>>,
) {
    for ev in events.read() {
        let targets = match ev.to {
            None => SendTargets::CLIENTS_ONLY,
            Some(c) => match players.iter().find(|(_, p)| p.character == c) {
                Some((client, _)) => SendTargets::Single(ClientId::Client(client)),
                None => continue,
            },
        };
        out.write(ToClients {
            targets,
            message: super::MapHud(ev.what.clone()),
        });
    }
}

/// The server's HUD messages for this client's player.
fn receive_hud(mut shows: MessageReader<super::MapHud>, time: Res<Time>, mut commands: Commands) {
    let now = time.elapsed_secs_f64();
    for s in shows.read() {
        let what = s.0.clone();
        commands.queue(move |w: &mut World| crate::logic::hud::show_on_hud(w, what, now));
    }
}

/// A chat line made safe: no control characters, trimmed, at most
/// `MAX_SAY` bytes.
pub fn clean_say(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > MAX_SAY {
            break;
        }
        out.push(c);
    }
    out
}

/// The character a request came from: a client's, or the listen server
/// host's own (`ClientId::Server`: replicon hands a host's messages to
/// the server as its own).
fn speaker(world: &mut World, client: ClientId) -> Option<Entity> {
    match client.entity() {
        Some(c) => world.get::<Player>(c).map(|p| p.character),
        None => world
            .query_filtered::<Entity, With<LocalPlayer>>()
            .iter(world)
            .next(),
    }
}

/// What the readers of a line need of its sender.
struct Sender {
    name: String,
    team: Option<u8>,
    alive: bool,
    place: Option<String>,
}

fn sender_of(world: &World, e: Entity) -> Sender {
    let alive = world.get::<Dead>(e).is_none() && world.get::<Health>(e).is_none_or(|h| h.current > 0.0);
    let at = world.get::<Transform>(e).map(|t| t.translation);
    let place = world
        .get_resource::<crate::map::nav::NavMesh>()
        .zip(at)
        .and_then(|(nav, at)| nav.places.get(nav.areas[nav.area_at(at)?].place?).cloned());
    Sender {
        name: world
            .get::<NetCharacter>(e)
            .map(|c| c.name.clone())
            .or_else(|| world.get::<Name>(e).map(|n| n.to_string()))
            .unwrap_or_else(|| "Player".into()),
        team: world.get::<Team>(e).map(|t| t.0),
        alive,
        place,
    }
}

/// Chat lines: to everyone who reads them (the host's copy locally).
fn receive_says(mut requests: MessageReader<FromClient<SayRequest>>, mut commands: Commands) {
    for r in requests.read() {
        let (client, text, team_only) = (r.client_id, clean_say(&r.message.text), r.message.team_only);
        if text.is_empty() {
            continue;
        }
        commands.queue(move |w: &mut World| {
            let Some(from) = speaker(w, client) else { return };
            deliver_say(w, from, &text, team_only);
        });
    }
}

/// Send `from`'s line to each player who reads it.
pub fn deliver_say(world: &mut World, from: Entity, text: &str, team_only: bool) {
    let s = sender_of(world, from);
    info!(
        "{}{}: {text}",
        s.name,
        if team_only { " (team)" } else { "" }
    );
    let sender = world.get::<Replicated>(from).map(|_| from);
    // Remote players, then the host.
    let mut readers: Vec<(ClientId, Entity)> = world
        .query::<(Entity, &Player)>()
        .iter(world)
        .map(|(c, p)| (ClientId::Client(c), p.character))
        .collect();
    if let Some(host) = world
        .query_filtered::<Entity, With<LocalPlayer>>()
        .iter(world)
        .next()
    {
        readers.push((ClientId::Server, host));
    }
    for (client, viewer) in readers {
        let v = sender_of(world, viewer);
        if !sees_say(v.alive, v.team, s.alive, s.team, team_only) {
            continue;
        }
        world.write_message(ToClients {
            targets: SendTargets::Single(client),
            message: ChatMessage {
                sender: if client == ClientId::Server { Some(from) } else { sender },
                name: s.name.clone(),
                team: s.team,
                alive: s.alive,
                team_only,
                place: s.place.clone(),
                text: text.to_string(),
            },
        });
    }
}

/// Most bytes a radio command name has.
const MAX_RADIO: usize = 32;

/// A client's radio call: the sender's `Radio`, if it is alive and the
/// call is one the map's radio knows (any short name without it).
fn receive_radio(mut requests: MessageReader<FromClient<RadioRequest>>, mut commands: Commands) {
    for r in requests.read() {
        let (client, command) = (r.client_id, r.message.command.clone());
        if command.is_empty() || command.len() > MAX_RADIO || !command.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        commands.queue(move |w: &mut World| {
            let Some(sender) = speaker(w, client) else { return };
            let known = w
                .get_resource::<crate::map::radio::RadioCommands>()
                .is_none_or(|r| r.get(&command).is_some());
            if !known || !sender_of(w, sender).alive {
                return;
            }
            w.write_message(Radio { sender, command });
        });
    }
}

/// Radio calls (players', bots', grenade throws') to the remote players
/// who hear them; the host hears its own through `core::Radio`.
#[allow(clippy::type_complexity)]
fn forward_radio(
    mut calls: MessageReader<Radio>,
    players: Query<(Entity, &Player)>,
    who: Query<(Option<&Team>, Option<&Health>, Has<Dead>), With<Replicated>>,
    mut out: MessageWriter<ToClients<RadioCall>>,
) {
    for call in calls.read() {
        let Ok((team, health, dead)) = who.get(call.sender) else {
            continue;
        };
        let alive = !dead && health.is_none_or(|h| h.current > 0.0);
        for (client, p) in &players {
            let my_team = who.get(p.character).ok().and_then(|(t, ..)| t.map(|t| t.0));
            if !hears(p.character, my_team, call.sender, team.map(|t| t.0), alive) {
                continue;
            }
            out.write(ToClients {
                targets: SendTargets::Single(ClientId::Client(client)),
                message: RadioCall {
                    sender: call.sender,
                    command: call.command.clone(),
                },
            });
        }
    }
}

/// Radio calls we hear, played as our own `core::Radio`.
fn receive_calls(mut calls: MessageReader<RadioCall>, mut radio: MessageWriter<Radio>) {
    for c in calls.read() {
        if c.sender == Entity::PLACEHOLDER {
            continue;
        }
        radio.write(Radio {
            sender: c.sender,
            command: c.command.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_lines_are_made_safe() {
        assert_eq!(clean_say("  hi\u{7}there \n"), "hithere");
        assert_eq!(clean_say(&"x".repeat(300)).len(), MAX_SAY);
        assert_eq!(clean_say(&"é".repeat(100)).len(), 126);
    }
}
