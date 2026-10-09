//! The client's UDP transport: netcode (renet's protocol, the same as
//! `NetcodeClientTransport`) over a socket, with Source's fake network
//! conditions for testing by hand: `net_fakelag` and `net_fakejitter`
//! delay what this process receives, `net_fakeloss` drops a share of
//! what it receives and sends (so lost commands and lost snapshots both
//! happen). Off (0) by default.

use std::{net::UdpSocket, time::Duration};

use bevy::prelude::*;
use bevy_replicon_renet::{RenetClient, RenetClientPlugin, RenetReceive, RenetSend, netcode::ClientAuthentication};
use renetcode::{DisconnectReason, NETCODE_MAX_PACKET_BYTES, NetcodeClient, NetcodeError};

use crate::console::resource_cvar;

/// Fake network conditions (Source's cvars): what this process receives
/// is delayed by `lag` ms plus up to `jitter` ms more (so it may come out
/// of order), and `loss` percent of the packets it receives and sends are
/// dropped. Only a client's transport applies them (`connect`); the
/// round trip grows by `lag`.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct FakeLag {
    pub lag: f32,
    pub jitter: f32,
    pub loss: f32,
}

/// A client's netcode connection over UDP (instead of bevy_renet's
/// `NetcodeClientTransport`, which has nowhere to hold packets back).
#[derive(Resource)]
pub struct UdpClient {
    socket: UdpSocket,
    netcode: NetcodeClient,
    buffer: Box<[u8; NETCODE_MAX_PACKET_BYTES]>,
    /// Packets received and held back (`FakeLag`): when they're due.
    held: Vec<(Duration, Vec<u8>)>,
    clock: Duration,
    rng: u64,
    /// Packets `net_fakeloss` dropped, in and out.
    pub dropped: u64,
}

impl UdpClient {
    pub fn new(now: Duration, auth: ClientAuthentication, socket: UdpSocket) -> Result<Self, NetcodeError> {
        socket.set_nonblocking(true)?;
        let netcode = NetcodeClient::new(now, auth)?;
        Ok(Self {
            socket,
            netcode,
            buffer: Box::new([0; NETCODE_MAX_PACKET_BYTES]),
            held: Vec::new(),
            clock: Duration::ZERO,
            rng: now.as_nanos() as u64 | 1,
            dropped: 0,
        })
    }

    /// Hang up now (netcode's disconnect packets).
    pub fn disconnect(&mut self) {
        if self.netcode.is_disconnected() {
            return;
        }
        if let Ok((addr, packet)) = self.netcode.disconnect() {
            let _ = self.socket.send_to(packet, addr);
        }
    }

    pub fn disconnect_reason(&self) -> Option<DisconnectReason> {
        self.netcode.disconnect_reason()
    }

    /// 0-1, xorshift64*.
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn lost(&mut self, fake: &FakeLag) -> bool {
        let lost = fake.loss > 0.0 && self.random() * 100.0 < fake.loss;
        if lost {
            self.dropped += 1;
        }
        lost
    }

    fn send(&mut self, packet: &[u8], addr: std::net::SocketAddr, fake: &FakeLag) {
        if self.lost(fake) {
            return;
        }
        if let Err(e) = self.socket.send_to(packet, addr) {
            debug!("UDP send: {e}");
        }
    }

    /// Advance by `dt`, receive (holding packets back as `fake` says) and
    /// hand what's due to renet.
    fn update(&mut self, dt: Duration, client: &mut RenetClient, fake: &FakeLag) -> Result<(), String> {
        self.clock += dt;
        if let Some(reason) = self.netcode.disconnect_reason() {
            client.disconnect_due_to_transport();
            return Err(format!("{reason:?}"));
        }
        if client.disconnect_reason().is_some() {
            self.disconnect();
            return Ok(());
        }
        if self.netcode.is_connected() {
            client.set_connected();
        } else if self.netcode.is_connecting() {
            client.set_connecting();
        }
        let server = self.netcode.server_addr();
        loop {
            match self.socket.recv_from(&mut self.buffer[..]) {
                Ok((len, from)) => {
                    if from != server || self.lost(fake) {
                        continue;
                    }
                    let delay = (fake.lag.max(0.0) + fake.jitter.max(0.0) * self.random()) / 1000.0;
                    let due = self.clock + Duration::from_secs_f32(delay);
                    self.held.push((due, self.buffer[..len].to_vec()));
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    break;
                }
                // Windows reports an earlier send's ICMP "port unreachable"
                // here (the server not up yet); netcode's timeouts decide.
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        // Due packets in the order they're due (stable: same-time ones in
        // arrival order).
        self.held.sort_by_key(|(due, _)| *due);
        let n = self.held.partition_point(|(due, _)| *due <= self.clock);
        for (_, mut packet) in self.held.drain(..n).collect::<Vec<_>>() {
            if let Some(payload) = self.netcode.process_packet(&mut packet) {
                client.process_packet(payload);
            }
        }
        if let Some((packet, addr)) = self.netcode.update(dt) {
            let packet = packet.to_vec();
            self.send(&packet, addr, fake);
        }
        Ok(())
    }

    fn send_packets(&mut self, client: &mut RenetClient, fake: &FakeLag) -> Result<(), String> {
        if let Some(reason) = self.netcode.disconnect_reason() {
            return Err(format!("{reason:?}"));
        }
        for packet in client.get_packets_to_send() {
            let (addr, payload) = self
                .netcode
                .generate_payload_packet(&packet)
                .map_err(|e| e.to_string())?;
            let payload = payload.to_vec();
            self.send(&payload, addr, fake);
        }
        Ok(())
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<FakeLag>()
        .add_systems(
            PreUpdate,
            receive
                .in_set(RenetReceive)
                .after(RenetClientPlugin::update_system)
                .run_if(resource_exists::<UdpClient>.and_then(resource_exists::<RenetClient>)),
        )
        .add_systems(
            PostUpdate,
            send.in_set(RenetSend)
                .run_if(resource_exists::<UdpClient>.and_then(resource_exists::<RenetClient>)),
        )
        .add_systems(Last, hang_up_on_exit.run_if(resource_exists::<UdpClient>));
    resource_cvar::<FakeLag, f32>(
        app,
        "net_fakelag",
        "Delay what this client receives by this many ms (the ping grows by it). Testing only.",
        |f| &mut f.lag,
    );
    resource_cvar::<FakeLag, f32>(
        app,
        "net_fakejitter",
        "Delay each packet this client receives by up to this many ms more, at random (out of order). Testing only.",
        |f| &mut f.jitter,
    );
    resource_cvar::<FakeLag, f32>(
        app,
        "net_fakeloss",
        "Drop this percent of the packets this client receives and sends. Testing only.",
        |f| &mut f.loss,
    );
}

fn receive(
    mut transport: ResMut<UdpClient>,
    mut client: ResMut<RenetClient>,
    fake: Res<FakeLag>,
    time: Res<Time<Real>>,
) {
    if let Err(e) = transport.update(time.delta(), &mut client, &fake) {
        debug!("UDP transport: {e}");
    }
}

fn send(mut transport: ResMut<UdpClient>, mut client: ResMut<RenetClient>, fake: Res<FakeLag>) {
    if let Err(e) = transport.send_packets(&mut client, &fake) {
        debug!("UDP transport: {e}");
    }
}

fn hang_up_on_exit(exit: MessageReader<AppExit>, mut transport: ResMut<UdpClient>) {
    if !exit.is_empty() {
        transport.disconnect();
    }
}
