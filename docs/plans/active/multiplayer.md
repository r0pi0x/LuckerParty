# Plan: multiplayer

Status: slice 0 done (its per-tick server hitbox poses came with slice
4); slice 1 done (2026-10-08): a listen server and a dedicated
server, direct-IP connect, characters replicated and drawn where the
server puts them; slice 2 done (2026-10-08): usercmds bound to server
ticks, clock sync, the client's own movement predicted and reconciled
(weapons wait for slice 4); slice 3 done (2026-10-08): others and
physics props drawn 0.1 s in the past from snapshots keyed by server
tick, movers replicated by motion state and stepped by the client to the
tick it predicts, so riding one is predicted; slice 4 done (2026-10-08):
weapons predicted on the client (fire, reload, switch, zoom, punch,
spread, grenade throws) and checked bit for bit, the server's hits lag
compensated against per-tick hitbox poses, others' shots drawn from the
server's seeds, grenades, drops and pickups the server's; slice 5 done
(2026-10-09): rounds, money, buying, team changes, the bomb, hostages,
the scoreboard with ping, chat and radio over the network, replicated
cvars and `sv_cheats`; slice 6 done (2026-10-09): bots on the server
with `bot_quota`; slice 7 done (2026-10-09): `changelevel` takes clients
along, late joiners get the whole state, maps downloaded (connection or
`sv_downloadurl`) and checked, the loading dialog for joining and map
changes; slice 8 done (2026-10-09): server queries on the game port
(A2S_INFO's layout), LAN discovery, the Find Servers dialog with
favourites, history and passwords. Recommendation:
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
| Others' shots: sound, muzzle flash, tracer, impacts | server event with the shot's seed; each client re-traces locally for impacts (slice 4: `FireBullets`, the same pellets as the server's) |
| Money, buying, rounds, timers, objectives (bomb, hostages) | server only; buy menu sends a request |
| Bots | server only (they are just replicated characters) |
| Logic entities (I/O, triggers, movers, breakables, fire) | server only; movers' transforms and visible state replicated |
| Physics props, dropped weapons, grenades in flight | server only, interpolated (the throw itself is predicted: pin, release, redraw) |
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
   Progress (2026-10-08):
   - [x] `core::NetRole { Standalone, Server, Client }` and the
     `core::authoritative` run condition on bots, rules (rounds included),
     `core::apply_damage`, the logic sets and objectives (bomb, hostages).
   - [x] `Intent::command`, stamped in a new `SimSet::Commands` (after the
     rules, before movement): the tick plus the character's `Seed`.
     **Changed from §1:** Source numbers commands per client, and seeds
     come from the command number while curtime comes from the tickbase;
     with "command number == tick" for everyone, all players firing on the
     same tick would share one spread pattern. So command numbers are
     offset per character, and time stays the tick's (`core::SimClock`).
     Slice 2 maps a client's command numbers to ticks with that offset.
   - [x] Weapon timers on `core::SimClock` (set from `Time<Fixed>` at the
     start of every tick, bit-identical to what `Time` gave; a replay sets
     it per command), also movement's `dt`, punch decay, recoil reset.
   - [x] Spread seed from the command number (spec 4.2: MD5, `& 255`,
     pellets at `seed + 1 + i`; T1/T1b unit tests in
     `weapon::random`); recoil rolls by the spec's CRC32 shared-random rule
     (label and extra ours). The generator stays ours (Q1).
   - [x] `core::Predict` schedules (`Select`, `Movement`, `Weapons`) run
     from their places in `FixedUpdate`; `core::predict` runs a command
     through all three. `core::FirstTimePredicted` gates sounds, damage,
     armour and pushes. `core::PredictedComponents` saves and restores the
     registered predicted components. Not yet: `WeaponEvent`s are still
     written on a replay (recoil reads them); slice 4 marks or drops them
     for the effects that read them. "One entity" holds because on a
     client only the local player will carry movement/weapon components.
   - [x] Cvar ownership: `console::CvarScope` (Local, Server, Replicated)
     from the name (`SERVER_PREFIXES`); not acted on yet.
   - [x] Pose history keyed by tick (`PoseFrame::tick`,
     `SkeletonPose::at_tick`), kept 1 s (`sv_maxunlag`).
   - [x] Hitboxes posed per tick from simulation values (not the drawn,
     client-rate animation): done in slice 4 (a network server's
     `map::SimAnimator`).
   - [x] Tests: `tests/it/prediction.rs` (100 commands of walking,
     jumping, ducking, an AK-47 spray and weapon switches replayed through
     `core::predict` from saved state: bit-identical, no sounds or damage;
     a client runs no damage or bots; pose history by tick).
1. **[x] Transport, connect/disconnect, replicated characters on the
   greybox** (M, medium risk: the library choice is proven here). `net`
   module and layering rule, renet + replicon, `connect`/`disconnect`/
   `hostport`/`maxplayers`, listen server in the game binary, the
   dedicated binary, map handshake by name and hash (no download),
   characters spawned per client and their transforms replicated
   (drawn without prediction yet), `NetSim` with simulated latency.
   Progress (2026-10-08):
   - [x] bevy_replicon 0.44.3 + bevy_replicon_renet 0.20.0 (renet 2.0,
     netcode UDP, unsecure) build on Bevy 0.19.1; `src/net/` with its
     `ALLOWED` row (above the simulation, below client and harness).
   - [x] Console: `connect <ip[:port]>` (27015 default), `disconnect`
     (leaves, or stops hosting), `listen` (hosts the loaded map),
     `status`, cvars `hostport`, `maxplayers`, `name`; `-port <n>` on
     the command line. Source's way to host works: `maxplayers 4; map
     <name>` (also `map greybox`) starts a listen server once the map has
     loaded (`client::net::listen_if_hosting`).
   - [x] Join handshake (replicon `AuthMethod::Custom`): the client sends
     `Join { version, protocol hash, name }`; another `NET_VERSION` or
     protocol hash, or a full server, gets `Refused { reason }` (the
     client hangs up with it; the server drops it after 1 s). The
     netcode protocol id is the same for every build so another build
     hears why instead of timing out. Then `Welcome { map, SHA-256 of the
     .bsp, tick interval, id }`: the client loads that map if it has
     another one (`NetEvent::LoadMap` -> `map <name>`), compares the
     hash (`MapData::file_hash`, kept as `map::MapFile`) and leaves on a
     mismatch; it runs its fixed tick at the server's.
   - [x] A character per client on the server (team with fewer players,
     respawned by the rules at a spawn with the starting weapons). The
     host's and bots' characters replicate too. Replicated:
     `NetCharacter { owner, name }`, `NetBody` (origin, velocity, look,
     eye offset, on ground/crouching/ladder/dead), `Team` (which picks
     the body model) and `Health`. On a client they get the character
     components without a movement slot; its own is `LocalPlayer` (camera
     attached by the client). Leaving removes the character and weapons.
   - [x] **Changed from the slice list:** so a client can move at all
     before slice 2, it sends its latest `Intent` once a frame
     (`NetIntent`, unreliable, made safe by the server: axis ≤ 1, finite
     angles, pitch range, slot range) and the server applies the latest
     one each tick. No command numbers, buffering or redundancy: slice 2
     replaces it with usercmds bound to ticks.
   - [x] Dedicated server `mashup_server` (`-port`, `+map greybox` or a
     CS:S map, `+maxplayers`, `+bot_add`; console on stdin).
   - [x] `harness::NetSim` (server + N clients in one process over
     `net::memory`, seeded latency/jitter/loss) and `tests/it/net.rs`:
     joining and seeing each other (and the host) move, looks and
     crouch, disconnect cleanup both ways, version refused with its
     reason, full server, map file mismatch, a lossy link converging,
     and the real netcode over loopback on a system-picked port.
   - [x] Two real games on the dev box (host `-port 27031`, client
     `connect 127.0.0.1:27031`): each sees the other's capsule move
     (screenshots), `status` shows the players and ping, `disconnect`
     returns the client to the menu; a client on the dedicated server
     sees its bot. How-to and Windows firewall notes:
     docs/OBSERVABILITY.md, "Network play".
   - Not yet (later slices): a `map` change while hosting doesn't take
     clients along (slice 7: `changelevel`); `map` typed on a client
     loads locally while still connected; clients see no weapons, shots,
     chat, rounds or kill feed (slices 4-5); others are drawn at the
     latest snapshot, stepping at the packet rate (slice 3); the
     scoreboard's ping column isn't filled (slice 5).
2. **[x] Usercmds, server-authoritative movement, prediction and
   reconciliation** (L, high risk: the core of the feel). Command
   buffering bound to ticks, clock sync, the prediction loop and replay,
   error smoothing, `net_graph` prediction errors. Tests: determinism
   suite above.
   Progress (2026-10-08):
   - [x] `UserCmds` (`net::NetCmd`: tick, move axis, exact `f32` angles,
     buttons, slot), unreliable, every frame with ticks: the new commands
     and the 3 before them (`CMD_BACKUP`). The client normalizes its own
     intent the way the server makes commands safe (`NetCmd::normalize`)
     before simulating, so both run the same values. Replaces slice 1's
     `NetIntent`.
   - [x] Server: commands queued by tick per player
     (`server::CommandBuffer`), exactly one applied per tick before the
     rules; a missing one repeats the last; late ones and ones more than
     64 ticks ahead dropped and counted. No client moves faster than the
     tick allows, whatever it sends (test).
   - [x] Clock sync: the server reports each player's command lead (the
     newest command's tick less its own tick on arrival) in `OwnState`;
     the client keeps it at 2 ticks plus twice the jitter
     (`predict::CommandClock`) by lengthening or shortening its fixed
     timestep by up to 8 % (the simulated tick length stays the server's
     via `SimClock`), or jumps when more than 6 ticks off. First guess:
     the server's tick + RTT + the target.
   - [x] Prediction: the local player gets the server's movement
     implementation and `Seed` and runs its commands in the normal fixed
     tick (the `Predict` schedules), each tick's command, clock and
     outcome kept (`PredictionHistory`). `core::number_commands` numbers
     from `SimClock::tick`, so a client's command numbers are the
     server's. The rules' hold (dead, freeze time) is mirrored from
     `OwnState::held`.
   - [x] Reconciliation: **changed from §1/§3:** the server's state of a
     client's own player isn't replicated components but an `OwnState`
     message each tick, its predicted components as one blob
     (`PredictedAppExt::predicted_net`, `PredictedComponents::encode`:
     postcard per component, entity fields skipped). The client compares
     it byte for byte with its prediction for that tick; on a mismatch it
     decodes it and re-runs the later commands (`core::predict`,
     `FirstTimePredicted` false). No tolerance: any bit differs, it
     corrects (cheap: one entity, ~10-20 commands).
   - [x] Error smoothing: the drawn eye keeps the correction and eases it
     out over `cl_smoothtime` (0.1 s); a correction larger than a tick's
     teleport distance (`map::interp::SNAP_SPEED`) snaps.
   - [x] Readout: `NetGraph` in the perf overlay and the F2 Perf tab
     (`net:` ping, loss, KB/s; `cmds:` lead, target, nudge, jumps, server
     buffer, missed, late; `prediction:` errors/s, worst, totals,
     replayed, easing), `status` (client: prediction; server: each
     player's buffered and missed commands), `cl_showerror 1` (each error:
     tick, metres, components; clock jumps).
   - [x] `net_fakelag`, `net_fakejitter`, `net_fakeloss` (Source names) in
     the client's own UDP transport (`net::udp`, renetcode's netcode
     client; lag and jitter on what it receives, loss both ways).
   - [x] Tests (`tests/it/net_prediction.rs`, `NetSim`): no loss at 50,
     100 and 150 ms: 0 prediction errors in ~260 compared states each
     (walking, strafing, turning, jumps, ducking, a duck jump, walk key),
     and on the greybox ladder; with jitter and loss (50±20 ms 5 %,
     100±40 10 %, 150±60 20 %, 100±120 35 %) errors are rare (0-1 per
     run, worst 0.03 m), nothing is left to correct once idle and the
     prediction is the server's bit for bit; the server's movement for a
     command stream equals single player's for the same commands, bit for
     bit; a flood of commands (80 a frame, far ahead, axis 40) moves no
     faster than 6.35 m/s; a 0.3 m server-side shove is one error, eased
     from the old position. `net::fake_lag_delays_what_the_client_receives`
     covers the UDP transport's lag.
   - [x] Two real games on the dev box (`-port 27031`, the client with
     `+net_fakelag 100 +net_fakejitter 20 +net_fakeloss 5 +cl_showerror
     1`): ping ~155 ms, walking, strafing, jumping and ducking with no
     prediction errors; a client `setpos` was corrected back (5.2 m, one
     error); at 150±60 ms and 20 % loss one 0.12 m error in a few seconds
     of strafing (view moved 0.04 m, eased).
   - Not yet: the weapon frame isn't predicted on a client because a
     client has no inventory or weapon state until slice 4 (the
     machinery is there: `Predict::Select`/`Weapons` run in the replay;
     slice 4 registers the weapon components with `predicted_net` and
     sends them); the server's movement cvars aren't sent (slice 5's
     replicated cvars); `OwnState` is the whole state each tick, no
     delta. (Movers: slice 3.)
3. **[x] Interpolation of others** (M, medium). Snapshot buffers keyed by
   server tick feeding `map::interp`'s `Interpolated`/`RenderedView`,
   `cl_interp`, brush entities, props and loose items replicated and
   eased, other players' bodies animating from replicated state.
   Progress (2026-10-08):
   - [x] Snapshots by server tick (`net::interp::Snapshots`): replicon's
     write functions for `NetBody`, `NetMover` and `NetProp` also keep
     each value by its message tick, which the server keeps equal to its
     `SimClock::tick` (`server::lock_replication_tick`). The server sends
     a mutate message every tick (replicon's `track_mutate_messages`), so
     a tick heard without a value for an entity is one it held still at
     (a held copy), and a character that starts walking after standing
     isn't smeared from where it stood.
   - [x] The render clock (`InterpClock`). **Changed from §1 and the
     task's "predicted time less the delay":** others are drawn at the
     server's clock as the arrivals show it (an average of tick time less
     arrival time, eased in at most 5 % of real time so it never steps
     back), less the delay, as Source's client clock does; the predicted
     tick runs ahead of every snapshot by the round trip. Delay
     `max(cl_interp, cl_interp_ratio × update interval)` with Source's
     defaults 0.1 s and 2 (VDC, "Source Multiplayer Networking").
   - [x] Drawing (`interp::draw_others`): between the two snapshots around
     the render tick, linear (positions, velocity, eye height; look the
     short way round; flags at the newer one's tick); a jump faster than
     `map::interp::SNAP_SPEED` or a death/respawn between them is drawn at
     once. **Changed from §1:** past the newest snapshot others carry on
     along their velocity for `cl_extrapolate_amount` (0.25 s, Source's
     default), then hold, instead of "at most one snapshot": with a 0.1 s
     delay that only happens after ~6 lost ticks in a row.
   - [x] Others' bodies animate from the drawn state: `Transform`,
     `Velocity` (speed), `MovementState` (crouch, ground, ladder, eye),
     look in `Intent` and `RenderedView`, `Dead`; the tick blend
     (`map::interp`) skips them (`NetDrawn`).
   - [x] Moving brushes. **Changed from §1/§3 ("interpolated"):** the
     server replicates each pusher's motion state (`NetMover`: pose,
     velocity, local time, move end and goal, flags) and the client steps
     it, with the logic's own arithmetic (`movers::Stepper`), to the tick
     it predicts, then pushes its own player with the logic's push code
     (`logic::push_players`, shared; `logic::carry_player`) before its
     movement, in live and replayed ticks; `OwnState::ground` (the mover's
     map index) keeps it riding after a correction. Movers are drawn at
     the predicted tick (eased between ticks). Interpolating them in the
     past (Source) would put the client's own player out of step with what
     it stands on, and every ride would mispredict.
   - [x] Physics props (`net::props`): `NetProp` (pose, velocity, shown,
     solid); a client's copies are kinematic and drawn from snapshots at
     the render tick. Loose items wait for slice 4 (a client has none).
   - [x] Found on the way: the physics' transform sync turns a -0.0 in a
     body's position into 0.0, but only when the body moved more than
     avian's tolerance since the physics last had it, which a replay
     can't know: a player at exactly 0 on an axis never matched again.
     A predicted player's position is written with +0.0 at the end of
     every tick and replay in a network game (`net::canonical_position`).
   - [x] Readout: `NetGraph`'s `interp:` line (delay, update interval,
     snapshots ahead, newest ahead, frames extrapolated/held, snaps,
     movers, ticks carried) in the perf overlay and F2 Perf tab; cvars
     `cl_interp`, `cl_interp_ratio`, `cl_extrapolate`,
     `cl_extrapolate_amount`.
   - [x] Tests (`tests/it/net_interp.rs`, `NetSim` at 240 frames a
     second): another player walking a curve at 50±10 ms, 100±40 ms 10 %
     loss and 150±60 ms 20 % loss is drawn exactly on the server's path
     without loss (0.000 mm off) and within 8 mm with loss (a lost
     snapshot bridged by a line), moving every frame (0 still frames,
     mean 26.7 mm, max 28.1 mm a frame), 0 frames extrapolated, 3-7
     snapshots buffered ahead; its speed, look and crouch are drawn; a
     3 m teleport is drawn at once; a crate the server lifts falls on the
     server's path; riding a lift up and back down at 50, 100±20 and
     150±40 ms mispredicts only when the lift starts (1-2 errors per
     ride, inside the first round trip; 0.04 m, eased) and ends on the
     server's state bit for bit (before: an error every tick of the ride).
   - [x] Two real games on the dev box (host `-port 27031` strafing left
     and right, the client with `+net_fakelag 100 +net_fakejitter 20
     +net_fakeloss 5`): ping ~150-175 ms, the host drawn 100 ms behind
     with 3-4 snapshots ahead (the listen server sends every ~28 ms at
     its frame rate), 15 of ~1900 frames extrapolated, no prediction
     errors on the client's own walking.
   - Not yet: parented brushes and breakables aren't stepped ahead (they
     hold their last snapshot until the next); a mover starting,
     reversing after its wait, blocked by someone else or a train at a
     path corner mispredicts for a round trip; model doors
     (`prop_door_rotating`), window panes, prop skins/sequences aren't
     replicated (slice 7's late join needs them as state); others riding
     a mover are drawn in the past on a mover drawn in the present.
4. **[x] Weapons** (L, high). Inventory and weapon state for the owner,
   predicted firing (timing, ammo, spread, punch, effects first time),
   hitboxes posed per tick from simulation values (from slice 0),
   server hit detection with lag compensation, shot events to others with
   seeds, impacts and tracers for others' shots, damage/death/kill feed
   from the server, grenades thrown server-side and interpolated,
   dropping and picking up. Needs the weapon-prediction spec session.
   Progress (2026-10-08; no spec session yet: what is predicted and what
   the server sends follows the plan's table, the Valve Developer Wiki's
   "Prediction", "Lag compensation" and "Source Multiplayer Networking"
   pages, and specs/cs_source/weapons.md 2, 4.2 and 4.6; where those are
   silent the choice is ours and marked here):
   - [x] Replay events: `WeaponEvent::replay` marks what a re-run command
     writes (recoil still reads it); effects, sounds, view model, HUD,
     body animation and ragdoll pushes skip it. Damage, armour and pushes
     only where `core::authoritative` (a client deals none: test).
   - [x] What a client's player carries goes with its `OwnState`
     (`weapon::sync`, registered with the new
     `PredictedAppExt::predicted_codec`): each weapon as its registry
     index and the changing state of its predicted parts (`NetPart`:
     `WeaponState`, clip and reserve, mode, CS:S's accuracy and recoil,
     grenade count and throw), the active, last and wanted weapons as
     indices, the timers and button edges; `ViewPunch` and `Zoomed` with
     `predicted_net`. **Changed from §3 ("inventory and weapon state for
     the owner" as replicated components):** the client's weapons are its
     own entities built from the registry (`weapon::spawn_weapon`), rebuilt
     when the server's list differs (a pickup, a buy, a drop, a respawn);
     no entity ids on the wire. Giving, equipping, picking up and dropping
     run only on the server.
   - [x] The weapon frame predicted: selection, firing (timing, clip,
     spread from the command's seed, punch and recoil, inaccuracy),
     reloads (both kinds), zoom and the sniper unzoom, bursts, and the
     grenade throw (`throw_frame` moved into `core::Predict::Weapons`, on
     `SimClock`; the projectile and the radio call are the server's). Own
     shots' tracers, impacts, sounds and view-model animation show the
     first time a command runs.
   - [x] Lag compensation (`weapon::lagcomp`). **Changed from §1 ("N's
     tick − the shooter's interp delay"):** each `NetCmd` carries the
     render tick the client drew others at when it made the command
     (`view_tick`, fractional; Source's command tick less its lerp: in our
     model the command tick runs ahead of the server by the round trip, so
     "tick − delay" would be wrong by that much); the server keeps it on
     the character (`ViewTick`, at most the command's own tick). The
     server keeps every living character's hit volume per tick for
     `sv_maxunlag` (`HitHistory`: origin, look yaw, box, hitboxes),
     recorded after the tick, so tick T's is what clients draw for T. A
     remote player's shot or swing traces others as they were at its view
     tick, between the two kept ticks around it, not further back than
     `sv_maxunlag`, not across a 64-unit jump between two ticks
     (`rewind`). **Changed from Source:** nothing is moved and restored:
     the trace tests the rewound volumes in place of the characters'
     colliders (hitboxes; a model-less character its box). Bots and a
     listen server's host aren't rewound (no latency). Cvars `sv_unlag`,
     `sv_maxunlag`, `sv_showlagcompensation` (logs each rewind), client
     `cl_lagcompensation`; `LagCompStats` and a `lagcomp` line in the
     server's `status`.
   - [x] Per-tick server hitbox poses (slice 0's last item): on a network
     server CS:S's player animation state runs again in the fixed tick
     from the tick's own values (the look as simulated, its shots and
     reloads) into a second animator (`map::SimAnimator`,
     `player_anim::SimAnimPlugin`), and the hitboxes are posed from it
     after every tick (`map::SimPose`), before the history records them;
     the drawn animation is untouched. Single player keeps posing
     hitboxes from the drawn animation (as before).
   - [x] Others' weapons: `NetHeld` (held weapon's registry index, mode,
     pin out) on every character; a client puts one weapon built from the
     registry in their hands (`weapon::remote::show_held`: the body's
     world model, silencer and animations), never running their weapon
     frame (the weapon systems leave out `NetDrawn` characters). Each
     round fired becomes `FireBullets` to every other client (shooter,
     weapon, origin, angles after punch, spread seed, inaccuracy and
     spread, mode; unreliable, as Source's temporary entities): the client
     traces the same pellets (`weapon::pellet_dirs`, shared with the
     server) through its own world with no damage, so tracers, impacts,
     decals and the fire sound show, and plays the body's fire gesture.
     Reloads, swings, pin pulls and throws come as `WeaponFx` for the
     body's animation.
   - [x] Outcomes from the server: `Killed` (kill feed and the client's
     own ragdoll, pushed by the killing hit), `HitConfirm` to the shooter
     (hit marker), health replicated as before.
   - [x] Grenades: thrown by the server, in flight and loose weapons as
     `NetItem` (model, pose, velocity) drawn from snapshots at the render
     tick as `map::loose::ShownItem`s; a detonation as `Detonation` (the
     explosion's look and sound from the grenade's rule), smoke clouds as
     `NetSmoke` (the client runs its own cloud from the rule), a flash's
     blindness and a blast's ringing as `Senses` to the player hit.
   - [x] Dropping: `drop` on a client asks the server (`DropRequest`);
     picking up (walking over, +use) is the server's; `give`, `impulse
     101` and `buy` are refused on a client (buying is slice 5).
   - [x] Tests (`tests/it/net_weapons.rs`, `NetSim`, greybox with CS:S's
     weapons): a client tapping single AK-47 shots at a player running
     across its view 10 m away, aimed where it draws it, hits 6 of 6 at
     100 and 150 ms with lag compensation (rewound 336 and 426 ms; the
     line of fire within 9 and 6 cm of the body's centre as seen) and 0
     of 6 with `sv_unlag 0`; `sv_maxunlag 0.05` holds every rewind at
     48 ms (0 of 6 hit); 1100 ticks of shotgun pumps, an AK-47 spray
     walking and jumping, reloads, switches, AWP zoom and unzoom and
     pistol taps at 100 ms: 0 prediction errors in 1160 states, clips,
     reserves and punch the server's bit for bit; another client draws
     25 of 25 traces (two 9-pellet shotgun blasts, an AK-47 spray) in the
     server's directions exactly (0 rad); a dead player holding fire
     fires nothing on either side; a client writes no damage, and its
     hits are the server's confirmations (6 of 6); an HE grenade: the
     throw predicted with 0 errors, no projectile on the client, drawn
     in flight on the other, one detonation heard; a drop asked for, seen
     lying by the other client, picked up again by walking over it; a bot
     on the server shoots a client: its health drops and the client draws
     the bot's shots (bots' rounds go out as `FireBullets` like anyone's).
     `tests/it/heavy/map_net_weapons.rs` (de_dust2, the install's player
     models): head shots at a player as seen standing, fired once it has
     ducked on the server, at 100 ms: 6 of 6 heads with lag
     compensation, 0 of 6 without.
   - [x] Two real games on the dev box (host `-port 27047 +map greybox
     +sv_showlagcompensation 1`, the client with `+net_fakelag 100
     +net_fakejitter 20 +net_fakeloss 5 +cl_showerror 1`), the client
     aimed at the host as it drew it over the remote protocol (`setang`,
     `+attack`/`-attack`): ping ~150 ms, shots rewound 280-306 ms; the
     host standing: 3 of 3 hit (killed: "You killed Host" in the client's
     feed, its hit marker); strafing left and right: 12 of 12 hit, 3 of
     12 with `sv_unlag 0`; the client's ammo counted down and reloaded as
     predicted; prediction errors: one at the join and a burst of five
     while a screenshot stalled the client (its late commands were
     repeated by the server, as designed), none while firing otherwise.
   - Not yet: the knife's box sweep (when its line misses) and grenades'
     blast traces aren't rewound; others' reload, draw and knife sounds
     aren't sent (only their fire sounds, from `FireBullets`); a client's
     own trace tests others' colliders where they were drawn a frame or
     two before (its impacts on others are cosmetic; the server decides);
     the planted bomb, defusing and buying wait for slice 5; `OwnState`
     carries the whole inventory every tick (no delta).
5. **[x] Rounds, money, buying, objectives, scoreboard, chat, radio**
   (M-L, medium). Server-owned round state replicated; buy, team, say,
   radio as requests; bomb and hostages replicated; scoreboard ping;
   cvar replication and `sv_cheats`.
   Progress (2026-10-09; no spec session for CS:S's join rules yet, so
   joining mid-round waits for the next round as §3 says):
   - [x] The round (`net::game`): one replicated entity (`NetRound`:
     phase, its times as server ticks, round number, wins, why buying is
     closed, the bomb's outcome, the hostage tally). **Changed from §3
     ("round state as components"):** a client writes it into single
     player's own resources (`RoundState`, `BuyWindow`, `FreezeTime`,
     `RoundOpen`, `BombState`, `HostageTally`) with each time moved onto
     its fixed clock (`ServerTime`: the server's tick as its updates
     arrive), so the HUD, scoreboard, spectator bars and buy menu read
     them unchanged. A round's end comes as `RoundOver` (the banner and
     announcer).
   - [x] Money to its owner only, as CS:S shows it: in `OwnState` with
     armour, the defusal kit and arming (the progress bar). Scores, ping
     (renet's round trip measured by the server, every 64 ticks), bots,
     the bomb carrier and kits on every character (`NetScore`); the
     scoreboard shows the bomb to teammates only, as before.
   - [x] Requests the server checks: `buy` (console, the buy menu,
     `buyammo1/2`) as `BuyRequest` (`economy::buy`: buy zone, buy time,
     money, team-only items; a refusal comes back as `Notice`, the game's
     hint), `jointeam` as `TeamRequest` (`rules::join_team`, with
     `mp_limitteams` in network games), `drop` (slice 4), `say`/
     `say_team` as `SayRequest`, radio commands as `RadioRequest`.
   - [x] The bomb and hostages: the planted bomb (`NetBomb`: where, its
     model, its clock and the defuse in server ticks), hostages as
     characters with `NetHostage` (index, model, leader), every
     `ObjectiveEvent` and `HostagePenalty` as `ObjectiveNews` (the HUD's
     words, the LED's beeps), the rules' sounds as `ServerSound` (new
     `map::GameSound`: the bomb, hostages, explosions from the rules,
     buying ammo; played locally and sent to clients). The freeze's end
     is sent as a tick (`OwnState::frozen_until`) so the client releases
     its hold on the same tick as the server; arming or defusing holds a
     client's movement like the server's (`OwnState::still`).
   - [x] Chat and radio (`net::chat`): the server picks who reads a line
     (`sees_say`: team chat within the team, the living don't read the
     dead) and sends `ChatMessage` (name, team, alive, place, text) to
     each, the host's copy locally; the client layer formats it
     (`SayFormats`). Every `core::Radio` (players', bots', grenade calls)
     goes as `RadioCall` to the remote players who hear it.
   - [x] Cvars (`net::cvars`): every `CvarScope::Replicated` value goes to
     a client when it joins and on change (`CvarValues`); the client sets
     them, refuses local changes while connected (Source's message), puts
     back anything that changes one (a slider, a map load) and restores
     its own on leaving. `mashup_rounds` counts as the server's.
     `sv_cheats` (replicated) gates `console::CHEATS` (noclip, god,
     setpos, setang, give, impulse, ent_fire, `mashup_set*`,
     `mashup_hurtme`) in network games; single player always has them;
     maps may not set it (point_servercommand).
   - [x] Spectating while dead works on a client as it was: it targets the
     interpolated others.
   - [x] Tests (`tests/it/net_rules.rs`, `NetSim`, greybox with CS:S's
     weapons and rounds): a full round with two clients at 50±5 ms (the
     freeze clock on both clients within 0.06 s of the server's, money,
     buying a Desert Eagle in the freeze, the round going live, a win by
     elimination, kill reward and win/loss bonuses on both clients, the
     other's money not shown, both scoreboards 1/0 and 0/1 with ping
     125 ms, the next round); buying refused outside a buy zone, without
     the money, the other team's rifle and after the buy time, in the
     game's words; the bomb planted by one client and defused by the
     other (the arming bar and the defuse bar shown, client timers within
     0.07 s of the server's, plant, beeps and the defuse heard on both,
     the bomb win and plant bonuses); chat, team chat and dead chat read
     by exactly who should, radio heard by living teammates only;
     `sv_airaccelerate 100` set on the server reaching the client, which
     can't change it, 0 prediction errors in 440 states of air
     strafing, `sv_cheats` gating `give`, the client's own value back on
     leaving; a late joiner dead and watching someone (drawn where the
     server has them) until the next round.
   - [x] Two real games on the dev box on de_dust2 (host `-port 27031
     +maxplayers 4 +sv_cheats 1 +mashup_rounds 1 +map de_dust2`, the
     client with `+net_fakelag 100 +net_fakejitter 20 +net_fakeloss 5`):
     ping ~165-176 ms; `jointeam 2` on the client moved it to the
     terrorists; `mp_restartgame 1`; both frozen with $800 and the same
     clock; the client's buy menu in the game's look; `buy deagle; buy
     vest; buy awp` on the client: the Desert Eagle bought ($150; the old
     pistol dropped), the rest refused; `say`/`say_team` from the client
     and `say` from the host: the host read the client's line and its
     own, not the terrorists' team line; the host shot the client: the
     round ended on both, money 4350 and 1550 on both sides, both
     scoreboards 1-0 with the client's ping; the bomb went to the
     client in the next round ("You have the bomb..."); 12 prediction
     errors in ~3800 states over the session (restarts, the buy, the
     death); `disconnect` put the client's `sv_cheats` and
     `mashup_rounds` back to its own.
   - Found on the way: `mp_restartgame` timed the restart on the frame
     clock while the rules compare it with the fixed clock, so a host
     whose ticks fell behind (a long map load) never restarted; it uses
     the fixed clock now. `+map cs_source:de_dust2` on the command line
     doesn't load (`+map de_dust2` does; not looked into). Once, after a
     client was killed and reconnected and ~30 minutes of rounds, neither
     the host nor the client moved (cs_source movement only; noclip did);
     not seen again after restarting both.
   - Not yet: arming the bomb isn't predicted (a client's hold and the
     bomb leaving its hands come a round trip late, corrected);
     objective news and the rules' sounds go to every client (each shows
     what concerns it); the kill feed still names the local player
     "Player" (the HUD's, unchanged); radio requests
     aren't rate-limited; a team change mid-round waits for the next
     round.
6. **[x] Bots on the server** (S, low). They already write `Intent` and
   use the same movement; make sure their perception never reads client
   state and a new `bot_quota` counts humans.
   Progress (2026-10-09):
   - [x] Bots are the server's characters like the host's (they were
     replicated since slice 1): names ("Bot N"), teams, "BOT" in the
     scoreboard's latency column (`score_flags::BOT`), their shots as
     `FireBullets`, kills as `Killed`, radio as `RadioCall` to the
     teammates who hear it. Their perception reads only the server's
     simulation (positions in the fixed tick, the server's sounds and
     sight traces): nothing of a client's.
   - [x] `bot_quota` and `bot_quota_mode` (normal, fill, match; CS:S's
     names): a network server keeps the quota (`bot::keep_quota`, one bot
     added or kicked a frame, added to the smaller team, kicked from the
     bigger); `bot_add` (`bot::add_bot`) raises it, `bot_kick` sets it to
     0, `bot_kick <name>` kicks one and lowers it; fill counts the host
     and remote players. Single player never enforces it (bots as
     before). A server starting with bots already in keeps them
     (`sync_quota`). A kicked bot drops the bomb it carries; its weapons
     go with it.
   - [x] A client can't run server commands (`console::SERVER_COMMANDS`:
     `bot_add`, `bot_kick`, `bot_give`, `bot_goto`, `changelevel`,
     `mp_restartgame`) or set `bot_*` cvars (server scope, refused as
     before).
   - [x] Deterministic: bot seeds come from their number and team (as
     before), the same with and without clients connected; two runs of
     the same network game put the bots in the same places bit for bit.
   - [x] Tests (`tests/it/net_bots.rs`, `NetSim`, greybox): two clients
     see both bots with the server's names, teams and positions (within
     5 cm) and "BOT"; their `bot_add`, `bot_kick` and `bot_quota` are
     refused; `bot_kick "Bot 1"` kicks one on every side; two bots 12 m
     apart fight with two clients connected: each client saw the kill,
     drew the bots' shots (27), heard their radio ("enemyspot",
     "enemydown") and shows 1-0 / 0-1 on its scoreboard; `bot_quota 3`,
     `bot_kick`, `bot_add` x2; fill 4: 4, 3, 2 bots as clients join, 3
     again when one leaves; match 2 with one human: 2. Cost: two clients
     on the greybox, the server's frame 1.40 ms mean (1.63 p95) without
     bots, 1.66 ms (1.99 p95) with ten bots fighting; each client receives
     ~37 KB/s with ten bots (debug build).
   - Not yet: bot names from CS:S's bot profiles; `bot_join_after_player`;
     `kick` for players.
7. **[x] Maps: change, mid-game join, download** (M, medium).
   `changelevel` (new; `map` today) with everyone reloading, late join
   with full state (logic state as components), map download with hash check and bz2
   (custom-maps step 6), the mount doctor check before joining.
   Progress (2026-10-09):
   - [x] The offer (`net::maps`): the server serves the map it has loaded
     (`ServedMap`: id, SHA-256 of the file, size, the file to send);
     `Welcome` carries it with `sv_downloadurl`, `sv_allowdownload`, the
     server's `hostname`, players and `maxplayers`.
   - [x] Map changes: `changelevel <map>` (and `map <map>` while hosting)
     in the game, `changelevel`/`map` on the dedicated server (loaded in
     the background, swapped in with `swap_map`). `watch_map` tells
     clients when the load starts (`ChangingLevel`) and, once the new map
     is in, starts a new game there (the rules' `new_game`, scores
     cleared, players' queued commands and intents dropped, uploads
     stopped) and sends `ChangeLevel`; a failed load calls the change off
     (clients go back to the map they're on). A client keeps its
     connection, its id and its character: prediction and drawing reset,
     the map fetched as on joining, then in the game again. The server's
     fixed clock keeps running through a map change
     (`core::set_tick_length`; single player still starts it over), so
     its time and tick never go back.
   - [x] Joining mid-game: everything that lasts is replicated state
     (the round, scores, a player's own money, characters, movers with
     opened doors and broken breakables, props, loose items, the bomb),
     so a late joiner sees it at once. **Changed from §3:** no separate
     "logic state as components" pass was needed for brush entities;
     model doors and window panes still aren't replicated (tech debt).
   - [x] The client's copy (`maps::Fetch`): the loaded map if it is the
     same file; else the copy the game would load (`MapFiles::read`: the
     install, its downloads, the content cache) or the cache's own,
     hashed in the background; else a download: `sv_downloadurl` first
     (`<url>/maps/<name>.bsp.bz2`, then `.bsp`; `net::http`, plain HTTP
     as the game's), then over the connection if `sv_allowdownload`
     (`MapRequest`, 16 KB `MapChunk`s, at most 256 KB beyond the
     client's `MapAck`, `net_maxfilesize`, refusals as `MapDenied`). What
     arrives must have the offer's size and hash, is written to the
     content cache (`maps/<name>.bsp`, noted in `index.toml` with its
     SHA-256 and source) and loaded from that file
     (`NetEvent::LoadMap`, `games::load_map_file`). A web copy with the
     wrong hash falls back to the server's when it sends maps.
     **Changed from custom-maps.md:** the cache keeps one copy per name
     (`maps/<name>.bsp`, replaced by a newer download), not one per hash.
   - [x] Refusals with reasons (`client::JoinProgress::failure`,
     `JoinFailure`): another map file and none to fetch ("Your map
     [maps/x.bsp] differs from the server's."), none and no downloads
     ("Missing map maps/x.bsp, disconnecting"), a download that failed
     (the HTTP error, the server's refusal) or arrived wrong, a full
     server, an older or newer server, a timeout (also a server that
     never welcomes us: 20 s).
   - [x] The loading dialog (backlog 2c): the main menu's GameUI loading
     dialog for `connect` (connecting, retrieving server info), a
     server's map change ("Server is changing level..."), downloads
     ("Verifying and downloading resources...", the bar following the
     bytes), the map's own load stages, a host's own map change, and
     "Starting local game server..." when `maxplayers` > 1, all in the
     game's words; Cancel disconnects. Failures in it ("Disconnected",
     the game's strings where it has one: server full, older/newer
     server, timeout, the download errors; else the server's or our
     words; Close). `mashup_loading_details 1` (off by default) adds a
     panel: each stage's time, bytes and percentage, the server's name,
     address, map, players and ping.
   - [x] Tests (`tests/it/net_maps.rs`, maps built in code from a folder
     of files: `harness::TestMaps`): a map change takes two clients
     along (told while it loads, the same connection and id, both on the
     new map's file, the round restarted at 0-0 and scores cleared, a
     client walking 9.3 m there with 2 of 537 states mispredicted); a
     late joiner sees the round score 3-2, its money, a bot's 4/1, a door
     opened and a breakable broken; a client with no copy downloads the
     220 KB map over the connection, caches and loads it, and joins from
     the cache next time without downloading; another downloads it from
     a local HTTP server as `.bsp.bz2` (sv_allowdownload 0); a web copy
     of another version is refused ("differs from the server's ...
     refused", .bz2 then .bsp asked for, nothing kept), no downloads is
     "Missing map", a map over `net_maxfilesize` the server's refusal,
     another build "newer server". Unit tests: the HTTP client (chunked,
     404), download checks, refusal kinds, the dialog's lines and the
     failure dialog.
   - [x] `tests/it/heavy/map_net_changelevel.rs`: the greybox to de_dust2
     from the install with a client and two bots (the client's copy
     checked by hash, followed on the same connection, the bots along).
   - [x] Two real games on the dev box (debug builds, a busy box): host
     `-port 27031 +maxplayers 4 +hostname "Lucker Party test" +map
     greybox`, the client with `+net_fakelag 100 +net_fakejitter 20
     +net_fakeloss 5 +mashup_loading_details 1`, `connect` through its
     console: the dialog showed "Retrieving server info..." with the
     detailed view, then the game (ping ~150 ms). `changelevel de_dust2`
     on the host: the client showed "Server is changing level..." for
     the host's ~4.5 s load, then its own stages (checking the map 0.09
     s, "Loading world...", "Initializing world...", "Loading
     resources..." with the bar), and played on de_dust2 at the
     terrorists' spawn on the same connection (1 of 4259 states
     mispredicted). `bot_add 1; bot_add 2; bot_add 2` on the host: the
     client's scoreboard listed Bot 1-3 with "BOT", its kill feed showed
     Bot 1 killing Bot 3, its chat the bots' radio ("Enemy spotted",
     "Enemy down"); its own `bot_add` was refused. `connect` to a port
     with no server: "Disconnected / Connection to server timed out." in
     the dialog (the game's string).
     Found on the way: maps from the install spawn with
     `cs_source:<name>` as their file's name, so a server never saw its
     new map in and a joining client couldn't match one: names are
     compared without the game now.
   - Not yet: the mount doctor check before joining; content besides the
     map file (custom materials, models, sounds); a changelevel to the
     same map file doesn't reload clients; decals for late joiners.
8. **[x] Find Servers, LAN discovery, direct connect UI** (M, low). LAN
   broadcast query (a small UDP info request on the server port, like
   Source's A2S_INFO in spirit), the Find Servers page in the GameUI
   look (`openserverbrowser`, greyed today), favourites and history,
   password prompt.
   Progress (2026-10-09):
   - [x] **Query protocol** (`net::query`), on the server's game port,
     beside the game connection: the server's socket is ours now
     (`net::udp::UdpServer`, renetcode's netcode server plus the query
     answers): a packet starting `FF FF FF FF` is a query (no netcode
     packet starts with `FF`), anything else netcode's. The packets are
     A2S_INFO's as the Valve Developer Wiki's public "Server queries" page
     documents them, little-endian, strings NUL-terminated:
     - request `FF FF FF FF 'T' "Source Engine Query\0"`, then the same
       with the 4-byte challenge appended;
     - challenge reply `FF FF FF FF 'A' <i32>` (never bigger than the
       request: a spoofed source address gains nothing); the challenge is
       a keyed hash of the asker's address and a 30 s window (the current
       and the previous window are good), so the server keeps nothing per
       request;
     - info reply `FF FF FF FF 'I'`, protocol 17, name (`hostname`), map
       (without the game prefix), folder `mashup`, game `Lucker Party`,
       app id 0, players (bots included), max players (`maxplayers`),
       bots, type `d` (dedicated: no player of its own) or `l` (listen),
       OS `l`/`w`/`m`, visibility 1 with `sv_password`, VAC 0, version
       (`NET_VERSION`), extra data flag `0x80` port, `0x10` an instance
       id (random per server process, in Source's SteamID field: one
       server heard at its LAN address and on loopback is listed once),
       `0x20` keywords (the map's game, `cs_source`).
     Rate limits (`query::Responder`): 8 answers a second per address
     (16 saved up), 200 a second in total (400 saved up); challenges count.
     The client (`ServerQueries`, one socket) measures the ping as the
     round trip of the challenged request; it asks twice, 1.2 s apart,
     then marks the server not responding. Replies whose folder isn't
     `mashup` (a Source server on a scanned port) are left out. Only
     A2S_INFO: no player list (`A2S_PLAYER`) or rules (`A2S_RULES`) yet.
     `serverinfo <ip[:port]>` and `lanscan` print what they hear; the
     server's `status` counts answers.
   - [x] **LAN discovery**: the request broadcast to every port of
     `net_lan_ports` (27015-27020, Source's LAN tab) at each address of
     `net_lan_broadcast` (255.255.255.255) and sent to 127.0.0.1 on the
     same ports; servers answer the broadcast with a challenge from their
     own address, asked from there on as any other; the scan listens
     1.5 s. Linux and Windows (`SO_BROADCAST` on the client's socket;
     servers bound to 0.0.0.0 hear broadcasts). Windows: a broadcast
     leaves by the default route's interface only, so with several
     adapters add the LAN's directed broadcast to `net_lan_broadcast`;
     the server's firewall rule for its game port covers queries
     (docs/OBSERVABILITY.md).
   - [x] **The Find Servers dialog** (`client::server_browser`), from the
     install's `servers/DialogServerBrowser.res`, `InternetGamesPage.res`
     (and `_Filters`), `DialogAddServer.res`, `DialogServerPassword.res`,
     `serverbrowser_english.txt` and the column icons (read at run time
     through `GameUi`); built-in sizes and words without the install.
     Tabs Internet (greyed: there is no master server to list internet
     servers; a master server of our own would fill it), Favorites, History, Lan. Columns password,
     bots, "Servers (n)", game, players, map, latency (History: last
     played); a header click sorts, again reverses (unsorted: by
     latency). Filters as the original's panel: map, latency, max
     players, has users playing, not full, no password (game, location
     and anti-cheat shown greyed). Refresh (Lan: a new scan; Favorites,
     History: each address asked), Quick refresh (the listed servers
     again), Add a Server (the address added, or its servers found and
     the one picked added), Connect, double-click, Enter; a server with a
     password asks for it first (`password <it>; connect <addr>`). The
     browser closes as it connects; the connect flow (slices 6-7, the
     loading dialog) is the console's. Favorites and History in the cfg
     folder's `serverbrowser.vdf` (KeyValues: name, address, last
     played); a joined server goes first in History (100 kept). The main
     menu's Find Servers opens it (`openserverbrowser [tab]`).
     Ours beyond the original: Delete takes the selected server off
     Favorites or History (CS:S: its right-click menu); double-clicking
     the server found in Add a Server joins it (a connect to an address
     from the browser).
   - [x] **Passwords**: `sv_password` on the server, `password` on the
     client sent in netcode's user data (no protocol change); a wrong
     one is refused with "Bad password.".
   - [x] Tests: `net::query` unit tests (the documented layout, round
     trips, challenges per address and window, rate limits, user data);
     `client::server_browser` unit tests (sorting, filters, connect,
     double-click, password dialog, Internet greyed, Add a Server,
     history order, the saved file's round trip);
     `tests/it/net_query.rs`: a server answers with its name, map,
     players, bots, max, dedicated or listen, password, version, port and
     a ping, and its game connection on the same port checks the
     password; a LAN scan of two ports finds the two servers there once
     each, and leaves out another game's; favourites and history written
     and read back.
   - [x] Live (2026-10-09): two dedicated servers on 27051 ("LAN one",
     greybox, a bot) and 27052 ("LAN two", de_dust2, `sv_password`), a
     client with `+net_lan_ports 27051-27052 +openserverbrowser lan`: both
     listed with players, bots, maps, the lock and bot icons, latency
     0-21 ms; sorting, the filters panel, History after joining LAN one
     (double-click), the password dialog, then de_dust2 joined with the
     password. An independent A2S_INFO client (Python, the wiki's layout)
     read both servers. On this box a broadcast doesn't come back to the
     local servers (the host firewall drops it; unicast to the LAN address
     answers), so the scan found them on 127.0.0.1: broadcast between two
     machines (Windows) is still to be seen.
   - Not yet: Internet (a master server), the server's
     player list and rules (`A2S_PLAYER`, `A2S_RULES`; Source's Game Info
     dialog and right-click menu), the browser's filters and column
     widths saved, a LAN scan refreshing on its own, the add server
     dialog's list for more than one server per address.
9. **[-] Lucker Party lobby and party flow** (dropped for now, by the
   user's decision: multiplayer stays standard CS:S — servers, Find
   Servers, `connect`, map rotation by the server's cvars). Revisit if
   Lucker Party later needs parties, minigame rotation or NAT relays.

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
