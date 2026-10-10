//! An in-memory transport for renet, in place of netcode's UDP: a server
//! and clients in one process (`harness::NetSim`) exchange packets
//! through a shared `Link` that delays, jitters and drops them with a
//! seeded generator, so network tests are fast and repeat exactly.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use bevy::prelude::*;
use bevy_replicon_renet::{RenetClient, RenetClientPlugin, RenetReceive, RenetSend, RenetServer, RenetServerPlugin};

/// How the link treats packets (each way).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinkConditions {
    /// One-way delay.
    pub latency: Duration,
    /// Up to this much more delay, at random per packet (packets may
    /// arrive out of order).
    pub jitter: Duration,
    /// Share of data packets lost, 0-1.
    pub loss: f64,
}

#[derive(Debug)]
enum Kind {
    Connect,
    Disconnect,
    Data(Vec<u8>),
}

#[derive(Debug)]
struct Packet {
    at: Duration,
    to_server: bool,
    client: u64,
    kind: Kind,
}

/// Packets in flight between a server and its clients, and the clock
/// they travel by (advanced by whoever steps the apps: `Link::advance`).
#[derive(Debug)]
pub struct Link {
    now: Duration,
    pub conditions: LinkConditions,
    rng: u64,
    in_flight: Vec<Packet>,
    /// Data packets dropped so far.
    pub lost: u64,
    pub delivered: u64,
    /// Conditions for one client's packets (both ways) in place of
    /// `conditions`: a mix of good and bad links on one server.
    pub per_client: std::collections::HashMap<u64, LinkConditions>,
    /// Data bytes sent to each client and from each client (lost ones
    /// included: what each end put on the wire).
    pub bytes_to: std::collections::HashMap<u64, u64>,
    pub bytes_from: std::collections::HashMap<u64, u64>,
}

impl Link {
    pub fn new(conditions: LinkConditions, seed: u64) -> Arc<Mutex<Link>> {
        Arc::new(Mutex::new(Link {
            now: Duration::ZERO,
            conditions,
            rng: seed ^ 0x6C69_6E6B_5345_4544,
            in_flight: Vec::new(),
            lost: 0,
            delivered: 0,
            per_client: Default::default(),
            bytes_to: Default::default(),
            bytes_from: Default::default(),
        }))
    }

    pub fn advance(&mut self, by: Duration) {
        self.now += by;
    }

    fn random(&mut self) -> f64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn send(&mut self, to_server: bool, client: u64, kind: Kind) {
        // Connection control (netcode's handshake and disconnect, which it
        // retries) always arrives, the handshake at once, a disconnect after
        // the latency; data may be lost or jittered.
        let mut at = self.now;
        let conditions = self.per_client.get(&client).copied().unwrap_or(self.conditions);
        match &kind {
            Kind::Connect => {}
            Kind::Disconnect => at += conditions.latency,
            Kind::Data(bytes) => {
                let counter = if to_server { &mut self.bytes_from } else { &mut self.bytes_to };
                *counter.entry(client).or_default() += bytes.len() as u64;
                if self.random() < conditions.loss {
                    self.lost += 1;
                    return;
                }
                let jitter = conditions.jitter.mul_f64(self.random());
                at += conditions.latency + jitter;
            }
        }
        self.in_flight.push(Packet {
            at,
            to_server,
            client,
            kind,
        });
    }

    /// Packets due now for the server (`None`) or one client, in order of
    /// arrival.
    fn take(&mut self, client: Option<u64>) -> Vec<(u64, Kind)> {
        let now = self.now;
        let mut due = Vec::new();
        let mut keep = Vec::new();
        for p in self.in_flight.drain(..) {
            let ours = match client {
                None => p.to_server,
                Some(c) => !p.to_server && p.client == c,
            };
            if ours && p.at <= now {
                due.push(p);
            } else {
                keep.push(p);
            }
        }
        self.in_flight = keep;
        // Stable: equal times keep their send order.
        due.sort_by_key(|p| p.at);
        self.delivered += due.len() as u64;
        due.into_iter().map(|p| (p.client, p.kind)).collect()
    }
}

/// A server's end of a `Link` (instead of `NetcodeServerTransport`).
#[derive(Resource, Clone)]
pub struct MemoryServer(pub Arc<Mutex<Link>>);

/// A client's end of a `Link` (instead of `NetcodeClientTransport`).
#[derive(Resource, Clone)]
pub struct MemoryClient {
    pub link: Arc<Mutex<Link>>,
    pub id: u64,
    connected: bool,
    told_disconnect: bool,
}

impl MemoryClient {
    pub fn new(link: Arc<Mutex<Link>>, id: u64) -> Self {
        Self {
            link,
            id,
            connected: false,
            told_disconnect: false,
        }
    }
}

impl MemoryClient {
    /// Leaving: the server hears it (netcode's disconnect packet).
    pub(super) fn hang_up(&self) {
        if !self.told_disconnect {
            self.link.lock().unwrap().send(true, self.id, Kind::Disconnect);
        }
    }
}

impl MemoryServer {
    /// Stopping: every client hears it.
    pub(super) fn hang_up(&self, server: &RenetServer) {
        let mut link = self.0.lock().unwrap();
        for id in server.clients_id() {
            link.send(false, id, Kind::Disconnect);
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        PreUpdate,
        (
            server_receive
                .in_set(RenetReceive)
                .after(RenetServerPlugin::update_system)
                .before(RenetServerPlugin::emit_server_events_system)
                .run_if(resource_exists::<MemoryServer>.and_then(resource_exists::<RenetServer>)),
            client_receive
                .in_set(RenetReceive)
                .after(RenetClientPlugin::update_system)
                .run_if(resource_exists::<MemoryClient>.and_then(resource_exists::<RenetClient>)),
        ),
    )
    .add_systems(
        PostUpdate,
        (
            server_send.run_if(resource_exists::<MemoryServer>.and_then(resource_exists::<RenetServer>)),
            client_send.run_if(resource_exists::<MemoryClient>.and_then(resource_exists::<RenetClient>)),
        )
            .in_set(RenetSend),
    );
}

fn server_receive(transport: Res<MemoryServer>, mut server: ResMut<RenetServer>) {
    let mut link = transport.0.lock().unwrap();
    // Connections the server dropped: tell their clients (netcode's
    // disconnect packet).
    for id in server.disconnections_id() {
        link.send(false, id, Kind::Disconnect);
        server.remove_connection(id);
    }
    for (client, kind) in link.take(None) {
        match kind {
            Kind::Connect => server.add_connection(client),
            Kind::Disconnect => server.remove_connection(client),
            Kind::Data(bytes) => {
                let _ = server.process_packet_from(&bytes, client);
            }
        }
    }
}

fn server_send(transport: Res<MemoryServer>, mut server: ResMut<RenetServer>) {
    let mut link = transport.0.lock().unwrap();
    for id in server.clients_id() {
        if let Ok(packets) = server.get_packets_to_send(id) {
            for p in packets {
                link.send(false, id, Kind::Data(p));
            }
        }
    }
}

fn client_receive(mut transport: ResMut<MemoryClient>, mut client: ResMut<RenetClient>) {
    let id = transport.id;
    if !transport.connected {
        // The handshake netcode would do: the server takes the connection
        // at once, the client counts as connected.
        transport.connected = true;
        transport.link.lock().unwrap().send(true, id, Kind::Connect);
        client.set_connected();
    }
    let due = transport.link.lock().unwrap().take(Some(id));
    for (_, kind) in due {
        match kind {
            Kind::Data(bytes) => client.process_packet(&bytes),
            Kind::Disconnect => client.disconnect_due_to_transport(),
            Kind::Connect => {}
        }
    }
}

fn client_send(mut transport: ResMut<MemoryClient>, mut client: ResMut<RenetClient>) {
    let id = transport.id;
    if client.is_disconnected() {
        if !transport.told_disconnect {
            transport.told_disconnect = true;
            transport.link.lock().unwrap().send(true, id, Kind::Disconnect);
        }
        return;
    }
    let packets = client.get_packets_to_send();
    let mut link = transport.link.lock().unwrap();
    for p in packets {
        link.send(true, id, Kind::Data(p));
    }
}

/// Become a server on `link` (tests: `harness::NetSim`).
pub fn serve(world: &mut World, link: Arc<Mutex<Link>>) -> Result<(), String> {
    super::server::start(world)?;
    world.insert_resource(MemoryServer(link));
    Ok(())
}

/// Become client `id` of the server on `link` (tests).
pub fn join(world: &mut World, link: Arc<Mutex<Link>>, id: u64) -> Result<(), String> {
    super::client::start(world, id)?;
    world.insert_resource(MemoryClient::new(link, id));
    Ok(())
}
