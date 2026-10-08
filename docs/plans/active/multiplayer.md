# Plan: multiplayer

Status: research and design (2026-10-08). Nothing built. Recommendation:
**bevy_replicon + renet (netcode over UDP)** for transport and
replication; **our own Source-style prediction, interpolation and lag
compensation** on top. Slices below; slice 0 is refactoring that pays off
even without networking.

Engine-split order puts networking at step 4 (after the crate split and
minigame definitions). The `net` module described here is new code that
moves into `engine/` as a whole, so slices 0-2 can start before the split
if the user wants (open question 1).

## Sources

Public documentation only (allowed for implementation sessions):

- Valve Developer Wiki, "Source Multiplayer Networking": tick, usercmds,
  snapshots and deltas, `cl_interp`, `cl_updaterate`/`cl_cmdrate`,
  `net_graph`.
- VDC, "Prediction": predicted entities, `IsFirstTimePredicted`, prediction
  errors (`cl_showerror`) and smoothing.
- VDC, "Lag compensation": rewinding players to the shooter's view time,
  `sv_maxunlag`, `sv_showlagcompensation`.
- VDC, "Latency Compensating Methods in Client/Server In-game Protocol
  Design and Optimization" (Bernier's paper): the whole model.
- Our specs: `specs/cs_source/weapons.md` (4.2 the shared random seed from
  the command number, 4.6 lag compensation, "Per-tick order", punch),
  `specs/cs_source/movement.md` (one movement step per user command).

Facts that need a **spec session** (public SDK, separate session) before
we copy their numbers: CS:S's default `cl_interp`/`cl_interp_ratio`/
`cl_updaterate`/`cl_cmdrate`/`rate`; what CS:S predicts for weapons
(which effects, sounds and animations are predicted vs sent); how the
server tells other clients about a shot (the "fire bullets" temp entity:
fields, and whether clients re-trace impacts with the shared seed);
which events are reliable vs unreliable; how a usercmd is encoded (fields,
deltas, `sv_maxusrcmdprocessticks`); lag compensation details beyond
spec 4.6 (what is rewound: origin, angles, animation layers, pose
parameters; the teleport rule); how a late joiner and a dead player are
handled (`mp_*` join rules); voice is out of scope.

## 1. Model (recommendation: Source's, with one simplification)

**Authoritative server at the map's tick** (0.015 s for CS:S maps,
`games::cs_source::TICK_INTERVAL`; 64 Hz greybox). The server sends the
tick interval to the client, which runs its fixed loop at it.

**Clients send usercmds.** A usercmd is our `core::Intent` for one tick
plus a command number (`u32`, the client's tick for that command) and the
view angles as the exact `f32` bits the client simulated with. Each packet
carries the newest command and the last 2-3 again (redundancy against
loss, as Source's `cl_cmdbackup`), unreliable.

**Simplification vs Source:** Source runs each player's commands when
they arrive, on that player's own clock (tickbase). We instead **bind
command number to server tick**: the client's clock is kept ahead of the
server by RTT/2 plus a small jitter margin, so command N arrives before
server tick N; the server keeps a short buffer per player and, at tick N,
writes command N into that player's `Intent`, then runs the normal
`FixedUpdate` once for everyone. A missing command repeats the last one
(the client then gets corrected). Why: our simulation is one schedule for
all characters, bots, logic and physics in a fixed order (ARCHITECTURE.md,
"Data flow per tick"); per-player command execution would mean running
movement and weapons outside that order. The cost is a jitter buffer of
1-2 ticks of latency, and the server must reject or drop commands that
arrive too early or late (clock sync messages tell the client to speed up
or slow down by a fraction of a tick, as lightyear and Overwatch do).
Weapon `curtime` stays the spec's `tickbase × tick` because tickbase ==
command number == tick.

**Snapshots.** The server replicates changed components every tick (or
every `sv_maxupdaterate`-th tick); replicon sends changes per component
with acks, so unchanged state costs nothing.

**Prediction (local player only, as Source).** The client keeps its own
character simulated: each tick it writes the local `Intent`, runs the
**prediction schedule** (movement, weapon frame, punch) for that one
entity, and stores (command number → predicted state). When a snapshot for
the local player arrives (acknowledging command K), it compares the
server's state at K with the stored prediction; on a mismatch beyond a
tolerance it restores the server state and re-runs commands K+1..now
through the prediction schedule. The drawn view eases out the error over
~0.1 s (Source's `cl_smoothtime` idea) instead of snapping, unless it's a
teleport (`map::interp::SNAP_SPEED`).

Predicted state per player (all must be in components so it can be
saved and restored): `Transform`, `Velocity`, `MovementState`,
`SourceMovement` (already `Clone`, private fields included), `ViewPunch`,
the inventory's timers, clip, burst/shot counters, zoom, inaccuracy and
recoil state, `MaxSpeed`. Side effects (footsteps, landing sounds, weapon
sounds, muzzle flash, view-model animation, tracers and impacts of your
own shots) fire only the **first time** a command is predicted (VDC's
`IsFirstTimePredicted`), never on replays.

**Interpolation of everything else.** Other characters, brush entities,
props, loose items, grenades are drawn from a snapshot buffer at
`now - interp delay` (Source's `cl_interp`, default to be specced; start at
100 ms or 2 update intervals, whichever is larger). `map::interp` already
blends between two samples with a fraction; the network version keeps
more than two samples keyed by server tick and picks the pair around the
render time. Same `Interpolated`/`RenderedView` components, a different
sample source, so the HUD, bodies and cameras don't change. Extrapolation
only for a short gap (≤ 1 snapshot), then hold.

**Lag compensation for hitscan and melee.** The server keeps per-tick
history of each character's hitbox pose for `sv_maxunlag` (1 s; spec 4.6).
`map::ragdoll::SkeletonPose` already keeps 0.25 s per tick and hitboxes
use the newest; extend it to a tick-keyed ring of 1 s (origin, angles,
pose). When it runs a shooter's command N, the server moves the other
characters' hitboxes to the time that shooter was drawing them
(N's tick − the shooter's interp delay, sent in the usercmd), traces, and
restores. Not across a teleport (spec 4.6: > 64 units). Bots shoot
without rewinding (no latency). This also removes the tech-debt row
"hitboxes follow the client-rate animation" for the server: the server
must pose hitboxes per tick from simulation values, not from the drawn
(eased) look.

**Predicted vs server-only.**

| Thing | Where |
|---|---|
| Own movement (walk, jump, duck, ladders, water, punch roll) | predicted |
| Own weapon frame: switching, firing timing, ammo in clip, reload timing, spread/recoil (shared seed), zoom | predicted |
| Own fire effects: muzzle flash, view model anim, fire sound, tracers, bullet impacts and decals from your own trace | predicted, first time only |
| Damage, health, armour, death, kills, score | server only (hit marker on server confirmation) |
| Others' shots: sound, muzzle flash, tracer, impacts | server event with the shot's seed; each client re-traces locally for impacts (to confirm by spec) |
| Money, buying, rounds, timers, objectives (bomb, hostages) | server only; buy menu sends a request |
| Bots | server only (they are just replicated characters) |
| Logic entities (I/O, triggers, movers, breakables, fire) | server only; movers' transforms and visible state replicated |
| Physics props, dropped weapons, grenades in flight | server only, interpolated (grenade throw not predicted in v1) |
| Gibs, shells, particles, ragdolls, decals | client only, from replicated events or deaths (CS:S ragdolls are client-side) |
| Sounds | own predicted ones locally; the rest from server events or replicated state (ambient loops from logic state) |

**Determinism we need.** Prediction needs the client and server to compute
the same thing for the same command on the **same build**; it does not
need cross-platform bitwise determinism (errors are corrected, as in
Source). Concretely:

- Movement and the weapon frame are a function of (component state,
  usercmd, world collision, tick interval) only. No `Time::elapsed`, no
  frame time, no global RNG, no iteration-order dependence, no entity ids
  (`Sim::pad` already checks the latter for tests).
- Weapon timers today compare against `Time::elapsed_secs_f64()` (global
  sim time). Change to the command's time (`tick × interval`), as the
  spec's tickbase.
- Spread and recoil randomness: seed from the command number with the
  spec's `MD5(n)` rule (weapons.md 4.2, test T1) instead of today's
  `Seed ^ inv.command × const` (tech-debt "own RNG"). `inv.command` already
  counts per owner; it becomes the usercmd number.
- Bots keep their own `core::Seed` (server-only, independent of entity ids,
  already true).
- Angles: the client sends the `f32` it used; the server must not
  re-quantize them before simulating (or both quantize first).
- Movement colliding with other players: remote players are drawn in the
  past on the client, so pushing against them mispredicts. Accepted (Source
  has the same), corrected by reconciliation.
- Windows vs Linux: `sin`/`cos` and other libm calls may differ in the last
  bit. Expect rare tiny corrections in mixed games; count them in
  `net_graph`. If they show, use the `libm` crate in the movement step.

## 2. Transport and library

Checked on crates.io on 2026-10-08. Bevy 0.20 is at rc.2 (2026-09-28), so
another bump is weeks away; how fast each crate followed 0.19.0
(2026-06-19) matters.

| Crate | Latest | Bevy | Followed 0.19 | What it gives | Fit |
|---|---|---|---|---|---|
| bevy_replicon | 0.44.3 (2026-10-06) | 0.19 | 0.41.0 on 06-24 (5 days) | server-authoritative replication, component-level changes with acks, entity mapping (`ServerEntityMap`), pre-spawned entity matching (`Signature`), per-client visibility, priority (`PriorityMap`), protocol hash check, client/server messages, listen and dedicated modes, in-memory test helper (`test_app`), receive markers with history (for prediction); replication tick schedule is ours (`ServerPlugin::new(FixedPostUpdate)`) | best: does replication only, leaves prediction to us |
| bevy_replicon_renet / renet | 0.20.0 / renet 2.0.0 | 0.19 | yes | UDP, netcode.io (connect tokens, optional encryption, timeouts), reliable/unreliable channels, RTT and loss stats; `renet_steam` backend for Steam later | recommended backend |
| bevy_replicon_renet2 / renet2 | 0.19.1 / 0.17.1 | 0.19 | yes | renet fork: more transports (WebTransport, WebSocket, in-memory) | backend pins replicon 0.41; fallback |
| bevy_replicon_quinnet / quinnet | 0.20.0 / 0.21.0 | 0.19 | yes | QUIC (TLS certs, tokio) | heavier, cert handling for direct IP; no gain for us |
| aeronet (+ aeronet_replicon) | 0.22.0-rc.1 | 0.20-rc | fast | transports (WebTransport, WebSocket, Steam) | fallback backend; already on 0.20-rc |
| lightyear | 0.30.1 (2026-09-16) | 0.19 (0.28-0.30) | 0.27/0.28 on 06-22/26 | everything: transports, replication, input buffering, prediction with rollback, interpolation, lag compensation, avian integration, bandwidth priority | strong, but see below |
| naia | 0.25.0 | 0.18 | no | replication, prediction helpers | not on 0.19 |
| matchbox | 0.14.0 | 0.18 | no | WebRTC P2P | not on 0.19; P2P not our model |
| hand-rolled UDP | - | - | - | exactly Source's model | months of work replicon already did (acks, deltas, entity mapping, fragmentation) |

**Why replicon + renet over lightyear.**

- Lightyear's rollback restores predicted components and **re-runs the
  fixed schedule** for the replayed ticks. Our `FixedUpdate` holds bots,
  rules, logic (event queue), weapons, damage and avian physics with side
  effects; replaying all of it (or gating every system for replay) is a
  large, fragile change. Source predicts only the local player's movement
  and weapon, which we can run as a small dedicated schedule.
- Lightyear owns time sync, input buffering and its input crates
  (`lightyear_inputs_*`); we already have `Intent` as the single input
  path, plus `harness::Sim` stepping exact ticks. Replicon leaves those to
  us and lets replication tick in our fixed schedule.
- Lightyear has broken its API at every minor (0.26 → 0.30 in eight
  months, mostly one maintainer). Replicon also breaks often (0.42 → 0.44
  in a month) but its surface is smaller and we'd touch it in few files.
- What lightyear would save us (interpolation buffer, lag compensation,
  clock sync) is a few hundred lines each and well documented by Valve.

**Spike (2026-10-08):** bevy 0.19.1 (no default features) + bevy_replicon
0.44.3 + bevy_replicon_renet 0.20.0 (netcode, UDP on localhost) built and
ran: server and client `App`s in one process at 0.015 s fixed ticks,
replication in `FixedPostUpdate`, the client sending an unreliable
"usercmd" message each tick, the server moving a replicated component
from it, the client seeing the result. Not kept (no test value without
the real module). Lightyear was not built (its 0.30 declares Bevy 0.19 and
avian3d 0.7).

**Network basics with renet.**

- Listen server on the host (host plays) and a headless dedicated server.
  Direct IP: `connect <ip>[:port]`, default UDP port **27015** like Source
  (and the earlier prototype's direct-IP lobby); `hostport` cvar. On this
  dev box the CS:S probe server already uses 27015 (RCON on 127.0.0.1), so
  local tests use ephemeral ports and manual runs `+hostport 27016`.
- NAT: the host forwards the port, as in Source. Later options: Steam
  networking (`renet_steam`, relays and NAT punching, needs Steam on both
  ends; fits Lucker Party friends), or our own relay. Open question 3.
- Auth: netcode "unsecure" mode for direct IP and LAN (no encryption, as
  Source's game traffic), a random 64-bit client id, an optional server
  password and the player name in the 256-byte user data, a protocol id
  plus replicon's protocol hash plus our build version (mismatch → refused
  with a message). Secure connect tokens (encrypted) once a matchmaker
  issues them (Lucker Party lobby).
- Never trust the client: clamp `Intent` (axis length ≤ 1, finite angles,
  pitch range, slot range), rate-limit commands (no more than one per tick
  on average: speedhack), buy/team/say requests checked by the rules.
- Bandwidth: ~10 characters × (position, velocity, angles, state flags:
  ~40 bytes quantized) × 66 Hz ≈ 26 KB/s per client before compression;
  replicon sends only changed components. Quantize positions (1/32 unit)
  and angles (16 bits) in custom serializers. Fine for LAN and broadband;
  `rate`-style caps via replicon priorities later.

## 3. Architecture in this codebase

**Roles.** A resource `core::NetRole { Standalone, Server, Client }`.
Standalone is today's game (singleplayer = a listen server with no remote
clients, so it stays the default path). Simulation modules gate
server-only systems with a run condition on it (bots, rules, damage,
logic, prop physics, objectives); they read only `core`, so no new layer
edges. On a client, only the prediction schedule runs on the local player.

**A new `net` module** (later in `engine/`): transport setup, replication
registration, usercmd send/receive, clock sync, snapshot buffers, the
prediction schedule driver, lag-compensation rewind, connect/disconnect,
map handshake, server cvar replication. Layer: above `rules`, `weapon`,
`objectives`, `logic`, `bot`, `map`, `character`, `console`, `core`;
below `client` and `harness`; `games/<name>` may use it to register their
own replicated components (`SourceMovement`, CS:S weapon state). Add it to
`ALLOWED` in `tests/it/architecture.rs` and the ARCHITECTURE.md diagram in
slice 1.

```
client ──> net ──> rules, weapon, logic, bot, objectives, map, character, console, core
harness ──> net      games/<name> ──> net (register components)
```

**Binaries.** `main.rs` stays the game (standalone, listen server,
client). A dedicated server binary (`src/bin/mashup_server.rs`, later
`apps/`): `SimPlugins` + map + `net` server, no window or rendering
(`harness::Sim` already runs the same headless stack), console on stdin,
`+map`, `+maxplayers`, `+hostport`, `exec server.cfg`.

**Entity identity.** Replicon maps server entities to client entities.
Map-spawned entities (brush entities, props, `prop_door_rotating`, loose
items present at load) are spawned by both sides from the same map; give
them a `Signature` from the map's own id (entity lump index, static prop
index) so replicated state lands on the client's copy instead of a
duplicate. Characters and runtime entities (dropped weapons, grenades,
bomb) are spawned on the server and replicated; the client adds the
render side (meshes, `Interpolated`) through required components or
observers, as the client does for grenades now. Nothing may depend on
entity ids (already a rule).

**What replicates (first list).**

- Characters: `Transform` (quantized), `Velocity`, `MovementState`,
  `Health`, `Team`, `Intent` look angles and the buttons others' bodies
  animate from (fire, crouch; not the whole intent), held weapon id,
  `Zoomed`, `Dead`, `Score`, armour, money (own team only? CS:S shows
  teammates' money in some modes; spec question).
- Local player only (owner visibility): `SourceMovement`, inventory and
  weapon state, `ViewPunch`, `MaxSpeed`, ammo, the last acknowledged
  command number.
- World: brush entity transforms and visible/broken state, props and
  loose items (transform, skin, body group, sequence), light styles,
  areaportal states, `MapFires`, the bomb and hostages, round state
  (phase, timer, scores, `FreezeTime`, `RoundRestarts`).
- Events (server → clients messages): shots (shooter, weapon, origin,
  angles, seed), `Died` (killer, weapon, headshot), `PlaySound` that
  aren't from predicted actions, `ObjectiveEvent`, chat and radio lines,
  HUD messages (`game_text`), explosions and impacts the client can't
  derive, round sounds.
- Rule for late joiners: anything visible for longer than a moment is
  **state** (a component), not an event, so a client that joins
  mid-round sees it (broken windows, open doors, burning fires,
  planted bomb).

**Console and cvars.** Add flags to `console::Cvar`: `replicated`
(Source's FCVAR_REPLICATED) and `server` (server-only, not sent). Server
and game-rule cvars (`sv_*`, `mp_*`, `phys_*`, `bot_*`, `ammo_*`, the
movement cvars: the same prefixes `point_servercommand` accepts) are owned
by the server; replicated ones are sent on connect and on change; a client
setting one while connected is refused ("only the server can change
it"). `cl_*`, `m_*`, `sensitivity`, video and audio stay local. `rcon`
later. `sv_cheats` gates `noclip`, `god`, `setpos`, `mashup_*` debug
commands that change the simulation.

**Maps.** On connect and on `changelevel`, the server sends
`(game, map name, SHA-256 of the .bsp, tick interval, minigame
definition)`. The client loads from its install, the install's
`download/` or mashup's cache (custom-maps plan) and checks the hash;
missing or different → disconnect with a message, until slice 7 adds
download (`sv_downloadurl`-style HTTP `.bsp.bz2`, or chunks over the
connection, verified and cached per custom-maps.md). Every client must
have each game the loadout uses mounted (README "Multiplayer"); the mount
doctor reports it before joining.

**Joining mid-round, disconnects.** A joiner gets the full replicated
state, picks a team and spawns at the next round (CS:S; check the join
rules in the spec session); during deathmatch at once. A disconnect
removes the player's character (dropping the bomb and weapons as a death
would); renet timeouts (a few seconds) do the same; the client returns to
the main menu with the reason. Bots fill or leave slots per a new `bot_quota`
cvar (server-only; `bot_add`/`bot_kick` today).

**Chat and radio.** `say`/`say_team` and the radio commands become
client → server requests; the server decides who receives each line
(`map::radio::sees_say`, `hears`) and sends `ChatLine`/radio messages to
those clients. No voice.

**Scoreboard and net_graph.** Ping from renet's RTT replicated per player
(the scoreboard's latency column, 0 today); `net_graph` adds ping, loss,
in/out bytes per second, snapshot rate, prediction errors.

## 4. Testing

- **In-process, headless (the main tool).** `harness::NetSim`: one server
  `App` and N client `App`s built like `Sim`, stepped tick by tick,
  connected through an in-memory link (replicon's `test_app` exchange, or
  our own link) with seeded latency, jitter, loss and reordering. Fast,
  deterministic, runs in `tests/it/net.rs` (heavy cases with real maps in
  `tests/it/heavy/`).
- **Prediction determinism.** Script a client's intents (walk, strafe
  jumps, duck jumps, ladders, water, firing bursts with recoil) on the
  greybox and on de_dust2 (skip without the install); with no loss, every
  predicted state must equal the server's for the same command, bit for
  bit (same process). With loss and jitter, corrections must converge
  within one RTT and the drawn view must not jump more than a set
  distance.
- **Spread seeds.** The spec's T1/T1b values from command numbers; client
  and server pick the same pellet directions for the same command.
- **Lag compensation.** A shooter with 100 ms simulated latency aiming at
  where it draws a moving target hits; without rewind it misses; never
  rewinds past `sv_maxunlag` or across a teleport.
- **Late join and map change.** A client joining after a window broke and
  a door opened sees both; a map hash mismatch refuses cleanly.
- **Two real instances on this Linux box.** `cargo run -- +hostport 27016
  +map de_dust2` and `cargo run -- +connect 127.0.0.1:27016`, with
  `net_fakelag`/`net_fakeloss`/`net_fakejitter` cvars (Source names) for
  manual feel tests; screenshots of both windows (docs/OBSERVABILITY.md).
- **Windows ↔ Linux on the LAN** after slice 3: prediction error counts in
  `net_graph` show whether libm differences matter.

## 5. Slices

Sizes are relative (S ≈ a session, M ≈ 2-3, L ≈ 4+). Each slice ends
with tests passing and something to see.

0. **[ ] Prerequisites, no networking** (M, low risk). `core::NetRole`
   and server-only run conditions; weapon timers on command time instead
   of `Time::elapsed`; spread seed from the command number (spec 4.2,
   tests T1/T1b; closes a tech-debt row); the movement and weapon frame
   for one entity runnable as a schedule (`PredictSchedule`) without
   global side effects, sounds behind a "first time" flag; server-side
   per-tick hitbox poses from simulation values. Tests: existing ones
   unchanged; a replay test (save state, run 20 commands, restore, rerun:
   identical).
1. **[ ] Transport, connect/disconnect, replicated characters on the
   greybox** (M, medium risk: the library choice is proven here). `net`
   module and layering rule, renet + replicon, `connect`/`disconnect`/
   `hostport`/`maxplayers`, listen server in the game binary, the
   dedicated binary, map handshake by name and hash (no download),
   characters spawned per client and their transforms replicated
   (drawn without prediction yet), `NetSim` with simulated latency.
2. **[ ] Usercmds, server-authoritative movement, prediction and
   reconciliation** (L, high risk: the core of the feel). Command
   buffering bound to ticks, clock sync, the prediction loop and replay,
   error smoothing, `net_graph` prediction errors. Tests: determinism
   suite above.
3. **[ ] Interpolation of others** (M, medium). Snapshot buffers keyed by
   server tick feeding `map::interp`'s `Interpolated`/`RenderedView`,
   `cl_interp`, brush entities, props and loose items replicated and
   eased, other players' bodies animating from replicated state.
4. **[ ] Weapons** (L, high). Inventory and weapon state for the owner,
   predicted firing (timing, ammo, spread, punch, effects first time),
   server hit detection with lag compensation, shot events to others with
   seeds, impacts and tracers for others' shots, damage/death/kill feed
   from the server, grenades thrown server-side and interpolated,
   dropping and picking up. Needs the weapon-prediction spec session.
5. **[ ] Rounds, money, buying, objectives, scoreboard, chat, radio**
   (M-L, medium). Server-owned round state replicated; buy, team, say,
   radio as requests; bomb and hostages replicated; scoreboard ping;
   cvar replication and `sv_cheats`.
6. **[ ] Bots on the server** (S, low). They already write `Intent` and
   use the same movement; make sure their perception never reads client
   state and a new `bot_quota` counts humans.
7. **[ ] Maps: change, mid-game join, download** (M, medium).
   `changelevel` (new; `map` today) with everyone reloading, late join
   with full state (logic state as components), map download with hash check and bz2
   (custom-maps step 6), the mount doctor check before joining.
8. **[ ] Find Servers, LAN discovery, direct connect UI** (M, low). LAN
   broadcast query (a small UDP info request on the server port, like
   Source's A2S_INFO in spirit), the Find Servers page in the GameUI
   look (`openserverbrowser`, greyed today), favourites and history,
   password prompt.
9. **[ ] Lucker Party lobby and party flow** (L, design-dependent). Party
   host = listen server or a hosted server; lobby, loadout/minigame
   rotation chosen by the server (README: the server owns the loadout),
   possibly Steam networking or a relay for NAT, secure connect tokens.

## 6. Open questions for the user

1. Start slices 0-2 before the engine-split crate work, or after it (the
   engine-split order puts networking last)?
2. Listen server only at first (host plays), or is a dedicated server on a
   rented box wanted early?
3. NAT: is "host forwards UDP 27015" acceptable for now, or should Steam
   networking (needs Steam running on every player's PC; fine for
   CS:S owners) or a relay come early for Lucker Party friends?
4. How many players should a server hold (sizes buffers, bandwidth
   targets)? CS:S allows 32; Lucker Party parties are smaller.
