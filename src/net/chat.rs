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
//! - **Flood protection**: each player may say a few lines (and radio
//!   calls) in a burst, then one a second (`Flood`; a call every 1.5 s);
//!   more are dropped, with a word to the sender. Bots aren't limited.
//! - **Names**: the `name` cvar changed while connected goes to the server
//!   (`NameRequest`, Source's setinfo); the server takes it (made safe),
//!   renames the character everyone sees and tells everyone
//!   (`NameChanged`: "* X changed name to Y"). The host's own change too.

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use super::{
    ChatMessage, NameChanged, NameRequest, NetCharacter, NetSettings, RadioCall, RadioRequest, SayRequest,
    UserInfoRequest, server::Player,
};
use crate::{
    core::{Health, LocalPlayer, NetRole, Radio, Team},
    map::radio::{hears, sees_say},
    rules::Dead,
};

/// Most bytes a chat line keeps (Source's say limit).
pub const MAX_SAY: usize = 127;

pub(super) fn plugin(app: &mut App) {
    app.add_message::<Radio>()
        .init_resource::<Flood>()
        .add_systems(
            PreUpdate,
            (receive_says, receive_radio, receive_names, receive_userinfo)
                .after(ServerSystems::Receive)
                .run_if(in_state(ServerState::Running)),
        )
        .add_systems(
            PostUpdate,
            (forward_radio, rename_host, forward_hud)
                .before(ServerSystems::Send)
                .run_if(resource_equals(NetRole::Server)),
        )
        .add_systems(
            PreUpdate,
            (receive_calls, name_characters, receive_hud)
                .after(ClientSystems::Receive)
                .run_if(resource_equals(NetRole::Client)),
        )
        .add_systems(
            PostUpdate,
            (send_name, send_userinfo)
                .before(ClientSystems::Send)
                .run_if(resource_equals(NetRole::Client)),
        );
}

/// A player's allowance of lines (or calls): `tokens` left, refilled up
/// to the burst at the rate (`Flood`).
#[derive(Clone, Copy, Debug)]
struct Allowance {
    tokens: f64,
    at: f64,
}

impl Allowance {
    /// Spend one if there is one (refilling first at `per_second` up to
    /// `burst`); false: flooding.
    fn take(&mut self, now: f64, burst: f64, per_second: f64) -> bool {
        self.tokens = (self.tokens + (now - self.at).max(0.0) * per_second).min(burst);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Lines a player may say in a burst, then one a second; radio calls:
/// three, then one every 1.5 s. (Ours: CS:S drops chat and radio spam
/// too, its exact limits unmeasured.) Typing never reaches them; a bound
/// key held down does.
pub const SAY_BURST: f64 = 4.0;
pub const SAY_PER_SECOND: f64 = 1.0;
pub const RADIO_BURST: f64 = 3.0;
pub const RADIO_PER_SECOND: f64 = 1.0 / 1.5;

/// Each speaking character's chat and radio allowances (a resource: a
/// component would reorder the characters' archetypes).
#[derive(Resource, Default)]
pub struct Flood {
    chat: std::collections::HashMap<Entity, Allowance>,
    radio: std::collections::HashMap<Entity, Allowance>,
    /// Lines and calls dropped so far.
    pub dropped: u32,
}

/// Whether `who` may say another line (`radio`: make another call) now;
/// counts it if so. Real time: a stalled tick doesn't open the gate.
fn allowed(world: &mut World, who: Entity, radio: bool) -> bool {
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    // Forget speakers that are gone (players who left), as a new one
    // comes: the maps stay as long as the players.
    let known = {
        let flood = world.resource::<Flood>();
        if radio { flood.radio.contains_key(&who) } else { flood.chat.contains_key(&who) }
    };
    if !known {
        let gone: Vec<Entity> = {
            let flood = world.resource::<Flood>();
            flood.chat.keys().chain(flood.radio.keys()).copied().filter(|e| world.get_entity(*e).is_err()).collect()
        };
        let mut flood = world.resource_mut::<Flood>();
        for e in gone {
            flood.chat.remove(&e);
            flood.radio.remove(&e);
        }
    }
    let mut flood = world.resource_mut::<Flood>();
    let (map, burst, rate) = if radio {
        (&mut flood.radio, RADIO_BURST, RADIO_PER_SECOND)
    } else {
        (&mut flood.chat, SAY_BURST, SAY_PER_SECOND)
    };
    let a = map.entry(who).or_insert(Allowance { tokens: burst, at: now });
    let ok = a.take(now, burst, rate);
    if !ok {
        flood.dropped += 1;
    }
    ok
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
        // Spectators have no team: their lines read "*SPEC*" or
        // "(Spectator)", and only the dead and spectators read them.
        team: world
            .get::<Team>(e)
            .filter(|_| world.get::<crate::core::Spectating>(e).is_none())
            .map(|t| t.0),
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
            if !allowed(w, from, false) {
                debug!("chat flood: dropped a line from {}", sender_of(w, from).name);
                if let Some(c) = client.entity() {
                    super::game::notify(w, c, "You are flooding the server: wait a moment.".into());
                }
                return;
            }
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
            if !known || !sender_of(w, sender).alive || !allowed(w, sender, true) {
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

/// A client's new name: taken (made safe) for its character, and told to
/// everyone.
fn receive_names(mut requests: MessageReader<FromClient<NameRequest>>, mut commands: Commands) {
    for r in requests.read() {
        let (client, name) = (r.client_id, r.message.name.clone());
        let Some(c) = client.entity() else { continue };
        commands.queue(move |w: &mut World| {
            let Some(character) = w.get::<Player>(c).map(|p| p.character) else { return };
            let id = w
                .get::<bevy_replicon::shared::backend::connected_client::NetworkId>(c)
                .map_or(0, |n| n.get());
            let name = super::server::clean_name(&name, id);
            if rename(w, character, &name) {
                w.get_mut::<Player>(c).expect("checked").name = name;
            }
        });
    }
}

/// Rename `character` (the name everyone sees) and tell everyone. False
/// when it already had that name.
pub fn rename(world: &mut World, character: Entity, name: &str) -> bool {
    let Some(old) = world.get::<NetCharacter>(character).map(|c| c.name.clone()) else {
        return false;
    };
    if old == name {
        return false;
    }
    info!("{old} changed name to {name}");
    world.get_mut::<NetCharacter>(character).expect("checked").name = name.to_string();
    world.entity_mut(character).insert(Name::new(name.to_string()));
    world.write_message(ToClients {
        targets: SendTargets::All,
        message: NameChanged {
            old,
            new: name.to_string(),
        },
    });
    true
}

/// A listen server's host changed its `name`: its character too.
fn rename_host(world: &mut World, mut last: Local<Option<String>>) {
    let name = world.resource::<NetSettings>().name.clone();
    if last.as_ref() == Some(&name) {
        return;
    }
    let first = last.is_none();
    *last = Some(name.clone());
    if first {
        return;
    }
    let host = world
        .query_filtered::<Entity, (With<LocalPlayer>, With<NetCharacter>)>()
        .iter(world)
        .next();
    if let Some(host) = host {
        let name = super::server::clean_name(&name, super::HOST_ID);
        rename(world, host, &name);
    }
}

/// Our `name` changed while connected: ask the server for it.
fn send_name(settings: Res<NetSettings>, mut last: Local<Option<String>>, mut out: MessageWriter<NameRequest>) {
    if last.as_ref() == Some(&settings.name) {
        return;
    }
    let first = last.is_none();
    *last = Some(settings.name.clone());
    if !first {
        out.write(NameRequest {
            name: settings.name.clone(),
        });
    }
}

/// Our userinfo cvars changed while connected: tell the server (the
/// first values went with `Join`).
fn send_userinfo(world: &mut World, mut last: Local<Option<Vec<(String, String)>>>) {
    let values: Vec<(String, String)> = crate::console::userinfo(world).into_iter().collect();
    if last.as_ref() == Some(&values) {
        return;
    }
    let first = last.is_none();
    *last = Some(values.clone());
    if !first {
        world.write_message(UserInfoRequest { values });
    }
}

/// A client's userinfo: its character's `core::UserInfo` (at most 64
/// keys of 64 bytes, values of 256).
fn receive_userinfo(mut requests: MessageReader<FromClient<UserInfoRequest>>, mut commands: Commands) {
    for r in requests.read() {
        let Some(c) = r.client_id.entity() else { continue };
        let values = r.message.values.clone();
        commands.queue(move |w: &mut World| {
            let Some(character) = w.get::<Player>(c).map(|p| p.character) else { return };
            super::server::set_userinfo(w, character, &values);
        });
    }
}

/// Characters renamed by the server: their `Name` here too.
fn name_characters(q: Query<(Entity, &NetCharacter, Option<&Name>), Changed<NetCharacter>>, mut commands: Commands) {
    for (e, c, name) in &q {
        if name.is_none_or(|n| n.as_str() != c.name) {
            commands.entity(e).insert(Name::new(c.name.clone()));
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

    /// The flood allowances of players who left are forgotten (the soak's
    /// joins and leaves grew them for ever).
    #[test]
    fn flood_forgets_who_left() {
        let mut world = World::new();
        world.init_resource::<Time<Real>>();
        world.init_resource::<Flood>();
        let ids: Vec<Entity> = (0..50).map(|_| world.spawn_empty().id()).collect();
        for &e in &ids {
            assert!(allowed(&mut world, e, false));
            assert!(allowed(&mut world, e, true));
            world.despawn(e);
        }
        let flood = world.resource::<Flood>();
        assert!(flood.chat.len() <= 1 && flood.radio.len() <= 1, "{} {}", flood.chat.len(), flood.radio.len());
    }
}
