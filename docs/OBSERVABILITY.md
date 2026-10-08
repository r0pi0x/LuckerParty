# Observability: how to check your work

Agents can't see the screen. These tools make the game's state readable.
Prefer them, in this order, over asking a human to look.

## 1. Headless scenario tests (fastest, most precise)

`cargo test` runs `tests/it/movement.rs` and friends on `harness::Sim`: no
window, time advanced by exact fixed ticks, `Intent` driven directly.

```rust
let mut sim = Sim::new(GreyboxMapPlugin);
let p = sim.spawn_character(Vec3::new(0.0, 1.0, 12.0), placeholder::ID);
sim.intent(p).move_axis = Vec2::Y;
sim.seconds(1.0);
assert!(sim.velocity(p).xz().length() > 4.9);
```

Every spec test case becomes one of these. New behavior gets a scenario test
before it is called done.

Integration tests are one test crate, so the game links once for all of
them (docs/performance.md, "Test cycle"): `tests/it/main.rs` lists each
file as a module. A new test file goes in `tests/it/` with a `mod name;`
line there, or in `tests/it/heavy/` (inside `mod heavy { }`) when it loads
real maps from the install or simulates long (bot rounds). A file directly
in `tests/` would be another test binary: `architecture::tests_are_one_crate`
fails and says how to move it. Test names read `weapons::name` and
`heavy::map_de_dust2::name`.

- Fast tier, before every commit (`.githooks/pre-commit`): `cargo test
  --features dev -- --skip heavy::` (unit tests and everything outside
  `heavy`; under a minute when warm).
- Full suite, before a push or a merge to main (`.githooks/pre-push`, which
  `MASHUP_PUSH_TESTS=0` skips): `cargo nextest run --features dev`, or
  `cargo test --features dev` without nextest (twice as slow: in one
  process the heavy tests contend). nextest runs each test in its own
  process; install it once with `cargo install cargo-nextest --locked` (or
  its prebuilt binary, https://nexte.st). It accepts the same `-- --skip
  heavy::` filter. It doesn't run doctests (there are none);
  `cargo test --doc` does.
- One file: `cargo test --features dev --test it weapons::`; one test:
  `cargo test --features dev --test it weapons::name`; `-- --ignored` runs
  the ignored debug aids (filters work the same).

## 2. Screenshots

```
cargo run --features dev -- --screenshot shot.png --frames 90 \
    --spawn 0,1,12 --look 30,-10 --movement mashup:noclip
```

With no `--map` the game starts at CS:S's main menu (the greybox loaded
unseen behind it), unless `--spawn`, `--look`, `--movement` or `--views`
is given: those start playing on the greybox, as above. `--map greybox`
starts on the greybox too. So `--window 1280x720 --screenshot menu.png`
photographs the main menu.
Add `--map cs_source:de_dust2` to load a real map (noclip plus
`--spawn x,75,z --look 0,-89` gives a top-down view). `--lightmap-only`
renders surfaces white so only baked lighting shows: misoriented lightmaps
appear as shadows that break at face edges.

Runs 90 frames, saves the primary window, exits. Read the PNG to check
rendering. Write screenshots to a scratch directory, never into the repo.
Needs a display: on the Linux dev box set `WAYLAND_DISPLAY=wayland-1` and
`XDG_RUNTIME_DIR=/run/user/1000` if the shell lacks them. `--frames N`
without `--screenshot` just runs N frames and exits (smoke test).
`--window 1920x1080` fixes the window's size in pixels (a fixed-size window
floats under Hyprland instead of being tiled), for UI checked at a known
resolution. `--help` lists all options.

To photograph a live run step by step (e.g. before and after a round
restart, where frame counts are too uncertain), run the game without
`--screenshot` and send `screenshot <file.png>` over the remote console
(section 3b) between other commands: `setpos ...; setang ...`, `+attack`,
`-attack`, then `screenshot /scratch/after.png`.

## 3. Live ECS over HTTP (Bevy Remote Protocol)

With `--features dev`, the game serves JSON-RPC on `localhost:15702`.

```
curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":1,"method":"world.query","params":{
  "data":{"components":["mashup::core::MovementState","mashup::core::Velocity"]},
  "filter":{"with":["mashup::core::LocalPlayer"]}}}'
```

```
curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":2,"method":"world.get_resources",
  "params":{"resource":"mashup::core::SimTick"}}'
```

Other methods: `world.list_components`, `world.get_components`,
`world.mutate_components`, `world.list_resources`. Don't try to drive the
local player by mutating its `Intent`: local input rewrites it every frame
(see docs/tech-debt.md). Use a scenario test instead.
Only `Reflect`-registered types are visible; register new core components in
`CorePlugin`.

## 3b. The console from outside

- Command line, Source style: `cargo run --features dev -- +sv_airaccelerate
  150 +cl_showpos 1 +bind f noclip` runs those console commands at startup;
  `--console` starts with the console open (screenshots of it).
  `++attack` (one `+` for the command line, one for the action) holds an
  action from the start, e.g. to fire in a `--screenshot` run; held
  console actions work without mouse capture. Automated runs never write
  config.cfg, and only archived cvars are saved there.
- `menu [main|newgame|maps|bots|team|options]` opens the game menu (Esc)
  on a page, for screenshots of it: `--window 1280x720 --screenshot
  menu.png +menu options`; `menu keyboard` (or `mouse`, `audio`, `video`,
  `multiplayer`) opens the options on that tab. Cvars it shows are read
  when it opens: put `+cl_crosshaircolor 3` before `+menu`. The log
  line `game menu: GameUI look (...)` says the install's look loaded,
  with how many main menu backgrounds and the title it found (else the
  built-in one is drawn). Its logic is unit-tested in
  `client::game_menu` (keys and clicks in, console lines out; rebinding
  with real key presses headless; which entries show in and out of a
  game; `disconnect` and `map greybox` on a world). The main menu over
  a map: `--map cs_source:de_dust2 --screenshot m.png +wait 30 +menu`;
  back to the main menu: `+disconnect`; `map greybox` plays the greybox
  from anywhere; `toggleconsole` opens the console as the menu's
  Console entry does.
- Fonts: the startup log line `fonts: tahoma -> tahoma.ttf, ...` says
  which file each scheme family resolved to (`(stand-in)` when the
  system lacks the real face, e.g. Liberation Sans for Tahoma on Linux;
  `client::fonts`). Text sizes and faces per element are unit-tested
  there; screenshots of the console, chat, scoreboard, buy menu, main
  menu and `debugui` at 1280x720 show them.
- Binds: `bindlist` lists them; every game key is one (`client::binds`),
  `binddefaults` puts the defaults back.
- `buymenu [n]` and `chooseteam` open the buy menu (on category n) and
  the team menu, e.g. `--map cs_source:de_dust2 --screenshot buy.png
  +wait 30 +buymenu 4` (the `wait` lets the map's HUD and menu layouts
  load first, so the game-look pages are used). Their key and button
  logic is tested headless in `client::buy_menu` and `client::team_menu`;
  `tests/it/heavy/map_de_dust2.rs` checks the install's layouts load.
- `ent_fire <target> <input> [value]` sends a map entity an input through
  the logic layer (names, `*` wildcards, classnames; the local player is
  the activator), e.g. `+wait 30 +ent_fire logic_timer Disable +ent_fire
  computer0* Skin 2` in a `--screenshot` run on de_nuke (the `wait` lets
  the map's logic load first). Logged at info level.
- `mashup_drawnav 1` outlines the nav areas near you (2: all), coloured by
  place, with half-links toward their neighbours, and the mesh's ladders
  (yellow line, green cross where bots get on at the foot, orange where
  they start down from behind the top); `mashup_drawbots 1`
  shows the map's sites (yellow rings) with where routes from the
  attackers' (orange balls) and defenders' (blue balls) spawns come onto
  them, and per bot: its role (ring at its feet: orange attacker, blue
  defender, grey roaming), look direction (white) and look target
  (yellow line; a yellow ball on the hiding spot it is checking),
  target (red), last known enemy position (orange), the teammate it
  answers (green), route (cyan), goal (purple), hold spot (a box in its
  role's colour, lines to the approaches it watches), its planned
  grenade arc (red HE, pale yellow flash, grey smoke) with the target
  (green cross) and where it should go off (sphere), and a flash it is
  looking away from (white), and a teammate's radio command it carries
  out (magenta: a line to whom it follows or where it regroups or falls
  back to, a ring where it holds, and the order as text over its head,
  "following Player"). `bot_debug 1` lists each team's plan and
  every bot's team, role, site, activity (ToSite, Following, Waiting,
  Holding, Chasing, Assisting, Obeying, ...; `*` marks the group
  leader), health and radio order on screen. To watch a round on dust2 from above: `--map
  cs_source:de_dust2 +mashup_rounds 1 +bot_add 1 +bot_add 1 +bot_add 1
  +bot_add 2 +bot_add 2 +bot_add 2 +mashup_drawbots 1 +bot_debug 1
  +noclip +setpos -400 3000 1200 +setang 89 90 0`. Headless round
  statistics (winners, kills, time to first contact, and every spot where
  a bot walked a route without getting anywhere for 4 s, summed up):
  `MASHUP_BOT_MAP=de_dust2 MASHUP_BOT_ROUNDS=8 cargo test --features dev
  --test it bot_rounds:: -- --ignored --nocapture`. To look at such a spot:
  `MASHUP_NAV_MAP=de_nuke MASHUP_NAV_AT=11.4,-15.2,34.4 cargo test
  --features dev --test it bot_nav::nav_near -- --ignored --nocapture` prints
  the nav areas, links, ladders, ladder brushes and entities (breakables
  with their keyvalues) around it (engine meters, as the report prints
  them; `MASHUP_NAV_BRUSHES=1` also lists every collision brush within
  1 m, e.g. player clip flush with a ladder's face); add a case to `tests/it/heavy/bot_nav.rs` (a lone bot sent from a start
  to a goal, `MASHUP_BOT_CASE=<name>` to run one, `MASHUP_BOT_TRACE=0`
  for every tick of its intent, ladder and ground state), and try raw
  inputs with `probe_walk` (`P_AT=x,y,z P_OPT=yaw,crouch,jump,seconds,
  pitch,forward,side`; `P_OPT2`, `P_OPT3`: further phases, e.g. climb a
  ladder, then step off its top). `nuke_ladders_climb_both_ways` sends a
  bot up and down every ladder on de_nuke's mesh; the ignored
  `ladders_climb_both_ways` does it on `MASHUP_NAV_MAP` (comma-separated
  maps) and lists the ladders bots fail, one `MASHUP_BOT_CASE="ladder 3
  up"` at a time (`MASHUP_BOT_TRACE_CASE="ladder 3 up"` with
  `MASHUP_BOT_TRACE` traces just that one while running the whole map:
  earlier cases leave stuck reports and broken grilles behind). Bot dice come from bot numbers, never entity ids:
  `MASHUP_TEST_PAD=<n>` spawns n entities and a resource in every `Sim`
  first, and a test's outcome must not change with it (run the bot tests
  with a few values after touching bot randomness). In game, `bot_goto <x> <y> <z>` (CS:S units) sends
  every bot somewhere; `bot_goto` alone lets them go back. `bot_grenades 2` makes bots throw whenever
  an arc works (`mashup_watch 1` to follow one). On the greybox map:
  `--spawn 0,1,-5 +bot_dont_shoot 1 +bot_stop 1 +bot_grenades 2 +bot_add 1
  +bot_give weapon_hegrenade +mashup_drawbots 1 +mashup_watch 1
  +cam_idealdist 600 +cam_idealyaw 70 ++attack --frames 130` shows the bot
  hearing you behind the long crate and lobbing an HE over it.
- Spectating: `mashup_hurtme chest 500` (in a rounds game, `+mashup_rounds 1`) starts the
  death cam; 2 s later the camera watches a living teammate. Drive it
  with `spec_mode 4|5|6` (first person, chase, free look), `spec_next`
  and `spec_prev`, `mp_forcecamera 0` to watch enemies too; the state is
  the `Spectator` resource (`tests/it/spectate.rs` drives it headless).
  The spectator bars on a real map: `--map cs_source:de_dust2 --window
  1280x720 --screenshot spec.png --frames 400 +mashup_rounds 1
  +mp_freezetime 0 +bot_add 2 +bot_add 2 +wait 60 +mashup_hurtme chest
  500` (the death cam ends 2 s after the death, then a teammate is
  watched in first person, with their crosshair).
- Scoreboard: `++showscores` holds it from the start (`--map
  cs_source:de_dust2 --window 1280x720 --screenshot sb.png +bot_add 1
  +bot_add 2 ++showscores`); a dead player shows the skull icon,
  the bomb carrier the bomb to Ts, a kit owner the defuser to CTs.
- Loading dialog: from the main menu, `+menu newgame` and Start Game, or
  over the remote console while at the main menu, `map de_dust2` then
  screenshots while it loads (the bar follows `map::loading`).
- `mashup_objectives 1` prints the objectives' state on screen: who
  carries the bomb, a planted bomb's place, site and time left, a defuse's
  progress (who, kit, seconds), the outcome, and each hostage's health,
  leader and position. To plant in a screenshot: `--map
  cs_source:de_dust2 --spawn 29.46,3.5,-62.99 +give cs_source:weapon_c4`
  (de_dust2's A site), then `+attack` over the remote console; hostages
  appear with `mp_freezetime 0; mashup_rounds 1` on a cs_ map.
- `mashup_drawhitboxes 1` outlines other characters' hitboxes, coloured
  by hitgroup, where shots test them (e.g. `+bot_stop 1 +bot_add 2
  +mashup_drawhitboxes 1` with `--screenshot` to check they follow the
  animated body).
- `mashup_drawphys 1` outlines physics props by state (green moving,
  blue asleep, grey still or frozen; multiplayer props darker) and
  players' physics shadows (white, a yellow line to each prop the shadow
  touches), and lists the props nearest the local player (distance, mass,
  push mode, state) on the debug HUD.
- `mashup_ragdoll_debug 1` draws ragdoll bodies (bounds, axes) and
  joints (yellow to the anchor on the parent; a red dot when the child
  drifted from it). `cl_ragdoll_physics_enable 0` turns ragdolls off,
  `ragdoll_sleepaftertime` sets when still ones are frozen. To see one:
  `--map cs_source:de_dust2 --frames 400 +mp_respawn_delay 1000
  +thirdperson +cam_idealyaw 90 +wait 150 +mashup_hurtme head` (or
  `chest 100 90`: a body shot from the side) with `--screenshot`. The
  log line `ragdoll ... (death pose Some((Front, 1)))` names the death
  pose. To shoot a body after death, run live with `+bot_stop 1
  +bot_dont_shoot 1 +bot_add 1`, find the bot with a `world.query` of
  `Transform` on `mashup::core::Intent` entities, `setpos` near it, and
  tap `+attack`/`-attack` over the remote console between
  `screenshot`s.
- Flashbang look (white plus the frozen after-image): run live on
  de_dust2 with `+mp_freezetime 0 +god`, then over the remote console
  `give cs_source:weapon_flashbang`, `setang 35 255 0`, `+attack`,
  `-attack` a second later, and `screenshot`s from 1.7 s after the
  release; a `setang` to another direction after the flash shows the
  frozen frame over the new view. Hearing (muffle, ringing) can't be
  photographed: `map::hearing::Hearing` and its tests
  (`cargo test --lib hearing`) show the curves; `tests/it/cs_grenades.rs`
  checks which effect each blast or flash gives.
- `mashup_healthbars 1` draws a health bar over every other living
  character (green full, red nearly dead), e.g. to watch damage land in a
  `--screenshot` run with `+bot_add 2`.
- `mashup_particles` prints the live particle groups and counts (and how
  many count against the 2048 cap), e.g. over the remote console after
  shooting. Impact effects obey `r_drawflecks`, `cl_show_splashes` and
  `violence_hblood` (0/1). To see a shot's effects in a `--screenshot`
  run, fire with `++attack` and keep `--frames` low enough that they are
  still alive (dust lives about a second, sparks a tenth of one).
- **Bug reports while playtesting:** press F9 (or `bugreport [note]` in the
  console). It saves `screenshot.png` and `report.txt` (build commit,
  map, a `setpos`/`setang` line to paste back, weapon, health, the last
  40 console lines) to `<data dir>/mashup/bugreports/report-<time>/`
  (`~/.local/share` on Linux, `%LOCALAPPDATA%` on Windows) and prints the
  path.
- `thirdperson` (back with `firstperson`; distance `cam_idealdist`, CS:S
  units) puts the camera behind the local player and draws its own
  animated body: `+thirdperson` in a `--screenshot` run shows the local
  player's model and animation. `cam_idealyaw` orbits the camera
  (degrees; `+cam_idealyaw 140` looks at the player's front, e.g. to see
  how the hands hold the weapon). Holding Left Alt (`+freelook`) turns
  only the camera; the aim stays put.
- `mashup_freecam 1` detaches the camera and flies it (WASD, mouse,
  space/ctrl up and down, shift faster) while your body stands still and
  is drawn; `mashup_freecam 2` leaves the camera where it is and gives
  the controls back, so you can walk, crouch and shoot in front of it
  (watching your own animation and held weapon); 0 returns to the eye. In
  a `--screenshot` run: `+thirdperson +cam_idealyaw 180 +wait 20
  +mashup_freecam 2 +wait 3 +firstperson +wait 20 ++forward` films you
  walking toward the camera.
- `mashup_watch <n>` chases bot n from behind (`cam_idealdist`,
  `cam_idealyaw` apply; 0 returns): `+bot_add 2 +mashup_watch 1
  +cam_idealyaw 150` shows a bot running with its weapon.
- View model (CS:S cvars): `viewmodel_fov` (80 here; CS:S 54), `cl_righthand` (1;
  0 puts weapons in the left hand), `r_drawviewmodel 0` hides it,
  `cl_bobcycle`/`cl_bobup` (bob), `cl_wpn_sway_interp`/`cl_wpn_sway_scale`
  (sway: e.g. 20 to exaggerate it), `muzzleflash_light 0` turns the
  flash's light off, `cl_ejectbrass 0` stops shells. The first two and
  `muzzleflash_light` are archived. To check the flash lighting a wall,
  stand at one and fire: `+setpos 470 2330 -78 +setang 0 15 0 ++attack`
  on de_dust2 with a few `--frames` values (a flash lasts 0.05 s, every
  0.1 s), against the same without `++attack`. `dump cs_source
  --sequences <model>` shows a model's animation events and attachments.
- Water (CS:S cvars): `r_WaterDrawReflection 0` / `r_WaterDrawRefraction
  0` turn the planar reflection / refraction off (to tell which one an
  artefact comes from), `mat_drawwater 0` hides water surfaces,
  `r_waterforcereflectentities 1` reflects models too. Under water the
  material's screen warp shows (de_port: `+setpos 700 2700 235 +setang
  -5 140 0` with `+noclip +god`); with the eye a few units above a surface
  (`+setpos 700 2700 258 +setang 20 140 0`) the strip under the waterline
  shows the water's height fog.
- Remote: `curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":1,
  "method":"mashup/console","params":{"line":"getpos; cvarlist sv_"}}'`
  runs a line now and returns the lines it printed.
- Configs live in `~/.local/share/mashup/cfg` (Windows `%APPDATA%\mashup\cfg`):
  config.cfg (written on quit when binds/cvars changed; every bind, as
  `bind` lines after `unbindall`), autoexec.cfg,
  history.txt, bookmarks.txt (the debug UI's places); `exec name` runs
  name.cfg from there.
- Typing in the console: suggestions show under it as you type, with a
  help line for the command being typed (its arguments from the help
  text, the one being typed in green; a cvar's value, default, range and
  values). Up/Down pick a suggestion (with an empty line they browse
  history), Enter or Tab takes it and adds a space, Esc hides the list
  (a second Esc closes the console). Completion knows each command's
  arguments (keys and what they are bound to, maps, `ent_fire` targets
  and inputs, teams, hitgroups, debug UI tabs, bot numbers, weapons) and
  works on the last command of a `;` line. `help <name>` prints the
  same; Ctrl+Backspace / Ctrl+Delete delete a word, Ctrl+V pastes,
  Ctrl+L clears, Ctrl+Home / Ctrl+End jump through the output. Its
  logic is tested headless in `client::console` (keys typed into the
  editing system). For a screenshot of it, `con_input "<text>" [n]`
  opens the console with that input and suggestion n picked:
  `--window 1280x720 --screenshot con.png +con_input "sv_a" 2`.

## 3b2. Network play

Plan and progress: [plans/active/multiplayer.md](plans/active/multiplayer.md).
The default port is UDP 27015 (Source's). **On the Linux dev box a CS:S
server holds 27015: never bind it there.** Use another port for every run
(`-port 27016`, `hostport 27016`); tests use ports the system picks.

Headless first: `harness::NetSim` runs a server and N clients in one
process over an in-memory link (`net::memory`) with seeded latency, jitter
and loss, stepped tick by tick (`tests/it/net.rs`):

```rust
let mut sim = NetSim::new(LinkConditions { latency: Duration::from_millis(30), ..default() }, 1, 2, |app| {
    app.add_plugins((GreyboxMapPlugin, SourceMovementPlugin)).insert_resource(Loadout { movement: source::ID });
});
sim.until_joined(200);
let me = sim.local_player(0).unwrap();          // client 1's own character
sim.clients[0].app.world_mut().get_mut::<Intent>(me).unwrap().move_axis = Vec2::Y;
sim.ticks(64);
let a = sim.character_of(0).unwrap();           // the server's copy
```

`net::status(world)` is the `status` text; `net::LastDisconnect` says why
a game ended. `netcode_over_loopback` covers the real UDP transport.

Prediction (`tests/it/net_prediction.rs`): a client's `net::predict::NetGraph`
counts the server states it compared (`checked`), the ones that differed
from its prediction (`errors`, `last_error`/`worst_error` in m) and
restarts (`resyncs`); `CommandClock` has the command tick and the lead
the server reports; the server's `net::server::CommandBuffer` on a
player's character has its queued commands, `missed`, `late` and `early`
counts. `PredictionHistory` holds the client's predicted ticks not yet
confirmed: compare them with the server's state at the same tick
(`PredictedComponents::encode`), never the two worlds' present (the
client runs ahead). `cargo test --features dev --test it net_prediction
-- --nocapture` prints the numbers per latency, jitter and loss.

Interpolation and movers (`tests/it/net_interp.rs`, at 240 frames a
second with `NetSim::set_frame(1.0 / 240.0)`): `net::interp::InterpClock`
has the render tick others are drawn at (fractional, server ticks) and
the delay; `net::interp::Snapshots<NetBody>` on another player's character
(and `<NetMover>`, `<NetProp>` on the mover and prop proxies) holds what
arrived, by server tick. To check a drawn path, record the server's
position per `SimTick` and compare the client's drawn `Transform` with
the server's between the two ticks around the render tick; `NetGraph`
counts frames extrapolated or held past the newest snapshot and
teleports snapped, and `carried` (ticks the client's mover step pushed
its own player). `cargo test --features dev --test it net_interp --
--nocapture` prints the off-path, per-frame step and error numbers.

In a game: the perf overlay (`mashup_perf 1`) and the F2 Perf tab show,
while connected, `net:` (ping, loss, KB/s), `cmds:` (lead in ticks and
its target, the clock's speed nudge and jumps, the server's buffer of
our commands, missed and late), `prediction:` (errors per second and
their worst, totals, commands replayed, the view's correction still
being eased out) and `interp:` (how far in the past others are drawn,
the update interval, characters drawn, snapshots buffered ahead of the
render time and how far ahead the newest is, frames extrapolated or
held, snaps, movers stepped and ticks the player was carried);
`cl_interp` (0.1 s), `cl_interp_ratio` (2), `cl_extrapolate` (1) and
`cl_extrapolate_amount` (0.25 s) set the drawing; `status` on a client prints the prediction and interp lines too,
on a server each player's buffered and missed commands. `cl_showerror 1`
logs every prediction error (tick, metres, which components differed)
and clock jump. `cl_smoothtime` (0.1 s) eases corrections out of the
view; 0 snaps. Fake network conditions on a client (Source's names; its
UDP transport only): `net_fakelag <ms>` delays what it receives (ping
grows by that), `net_fakejitter <ms>` adds up to that much at random,
`net_fakeloss <percent>` drops packets both ways.

Two real games on this box (`--features dev`; each needs its own remote
port, `MASHUP_REMOTE_PORT`, so both answer `curl`):

```
MASHUP_REMOTE_PORT=15791 cargo run --features dev -- --window 1280x720 -port 27031 +name Host +maxplayers 4 +map greybox
MASHUP_REMOTE_PORT=15792 cargo run --features dev -- --window 1280x720 +name Client +connect 127.0.0.1:27031
```

Then drive each through its console (section 3b): `status` (players,
ping), `getpos`, `+moveleft`/`-moveleft`, `setang`, `screenshot
<file.png>`, `disconnect`. The host's `setpos` moves the host; a client's
position is predicted and the server's state corrects it (a client's `setpos` snaps back). Add
`+net_fakelag 100 +net_fakeloss 5 +cl_showerror 1` to the client's line to feel
a bad link. The dedicated server:
`cargo run --features dev --bin mashup_server -- -port 27032 +map greybox
+bot_add` (console on stdin: `status`, `bot_add`, `quit`). Logs show
`listening on UDP ...`, `<name> joined`, `<name> left`, `disconnected:
<reason>`.

In game: `maxplayers 4; map <name>` hosts (Source's way), `listen` hosts
the loaded map, `connect <ip[:port]>` joins, `disconnect` leaves (or stops
hosting), `status`, `name <you>`.

**LAN test with Windows.** The host's firewall must let the game take UDP
on its port: the first time `mashup.exe` (or `mashup_server.exe`) listens,
Windows asks; allow it on private networks. Without the prompt (or on a
public network), add a rule in an administrator PowerShell:
`New-NetFirewallRule -DisplayName "mashup" -Direction Inbound -Protocol UDP -LocalPort 27015 -Action Allow`
(use the port you host on). Clients need no rule. Find the host's address
with `ipconfig` (IPv4 Address) and join with `connect 192.168.x.y:27015`.
Over the internet the host forwards that UDP port on its router. Both
games must be the same build (`status` shows the version); another build
is refused with a message.

## 3c. Performance

Details and baseline numbers: [performance.md](performance.md).

- `mashup_perf 1` (2: every render pass) shows frame times (avg, p95,
  max), the main world's CPU time, GPU time per render pass, entity,
  mesh and triangle counts and visibility culling (camera cluster,
  clusters and map parts potentially visible), and what changes each
  frame (transforms written, under the map's root, mesh and material
  assets modified). `mashup_perf 3` also logs each second which entities
  (by name) write transforms: anything writing every frame for nothing
  costs transform propagation and GPU re-preparation.
- `mashup_perf_log 1` logs the same readout as one text line a second
  (`mashup_perf: 60 fps  frame 16.67 ms ... | vis: cluster ...`), so runs
  can be compared without reading screenshots (`--frames N ... 2>&1 |
  grep mashup_perf:`); `bugreport` saves the latest readout in
  report.txt whether or not the overlay is on.
- Drawing between ticks (`map::interp`): the readout's `interpolation:`
  line gives the tick rate, the frame's blend between the last two ticks
  (`Time<Fixed>` overstep fraction), ticks run that frame and teleport
  snaps so far; the F2 Perf tab shows it live every frame.
  `cl_interpolate 0` draws the latest tick instead (stepping at the tick
  rate), for A/B. `cargo test --features dev --test it interpolation::
  -- --nocapture` prints a walking, ducking camera eye per frame at 240 fps on the CS:S
  tick with it on and off.
- `refcmp bench --views tools/refcmp/<map>.toml` times 200 frames at each
  view (vsync off) and prints a table (frame, main-world CPU, process CPU
  and GPU ms; with pipelined rendering a frame takes the longer of the
  main world and the render world); `-- <args>` passes options to
  mashup (e.g. `-- +r_novis 1`). Build first; it runs the mashup next to
  it (`--profile playtest` for optimized numbers).
- `r_portalsopenall 1` ignores areaportals (closed doors no longer hide
  what's behind them, no clipping through openings): PVS culling only.
  `mashup_perf 1` shows the camera's area, the areas it reaches and how
  many areaportals logic closed.
- `r_occlusion 0` ignores the map's occluders (func_occluder);
  `mashup_perf 1` shows how many are active and how many map parts they
  hide.
- `r_novis 1` draws every map part (no visibility culling);
  `MASHUP_MERGED_WORLD=1` spawns the world as one mesh per material with no
  culling, as before chunking (A/B comparisons); `MASHUP_MERGE_BRUSHES=0`
  draws every brush entity from its own meshes (no merged combined
  meshes, `map::merge`).
- `refcmp vischeck --views tools/refcmp/<map>.toml` renders the views
  plus views from spawns and nav areas with culling off (`r_novis 1`,
  `r_occlusion 0`) and on (game time
  frozen with `host_timescale 0`), lists the views that differ and fails
  if any differs by more than 0.5% of its pixels (small differences:
  geometry seen through sky brushes, which culling hides as the game
  does; see performance.md). `cargo test --features dev --test it map_vis::` checks the same with rays from
  ~300 player positions per map (`MASHUP_VIS_FULL=1`: every nav area).
- `REFCMP_OUT=<dir> refcmp ...` writes mashup's captures, reports, bench
  and vischeck output under `<dir>/<map>` (references are still read from
  the shared dump folder): use a folder in your own `target/` when other
  sessions may be using the shared captures.
- Traces: a `--features profile` build (Bevy's `trace_chrome`) writes a
  Chrome trace with every system, schedule and render pass as a span;
  `TRACE_CHROME=<file>` names it (else `trace-*.json` in the working
  directory). Keep runs short (`--frames 400`: about 1.5 GB) and the
  file in your `target/`. `tracesum <file> [--skip N] [--top N]
  [--filter text] [--by-total]` (`src/bin/tracesum.rs`) prints the main
  world's (`main app`) and render world's (`sub app: name=RenderApp`)
  time per frame and the spans with the most self time per frame, by
  thread (systems run on the `workers`); open the file in
  https://ui.perfetto.dev for timelines. Our exclusive systems carry
  their own spans where Bevy's per-system ones say too little (the
  logic bridge: `logic: phase`, `logic: sync to the ECS`, `logic:
  sync_movers`). Example (performance.md has real output):
  `cargo build --profile playtest --features profile`, then
  `TRACE_CHROME=target/t.json target/playtest/mashup --map cs_source:de_dust2 --frames 400`,
  then `cargo build --profile playtest --bin tracesum` and
  `target/playtest/tracesum target/t.json --skip 200`.
  The profile build replaces `target/playtest/mashup`: copy it aside
  or rebuild without the feature before benchmarking. Tracy instead:
  `--features bevy/trace_tracy` and the Tracy profiler (not set up on the
  dev box). CPU sampling (`perf record -g`, `cargo flamegraph`) works on
  any optimized build on Linux; `perf` isn't installed on the dev box:
  sample the dev build under gdb instead (performance.md, "Community
  maps' slow first views"). A main-thread schedule with much self time
  in `tracesum` is waiting for workers (or descheduled under load).

## 3d. The debug UI

F2 (a bind: `debugui`) opens a tabbed window; `debugui <tab>` opens it
on a tab (`player`, `world`, `movement`, `bots`, `rendering`, `perf`,
`audio`, `logic`, `rounds`, `cvars`), `debugui close` closes it, e.g.
`--window 1280x720 --screenshot ui.png +debugui perf` photographs one.
Every control runs a console line (hover a button for it), so anything
done there can be typed, bound or scripted too; the window only reads
state. The mouse stays free while it is open (the game ignores input),
and keys typed into its fields don't reach binds.

- Player: position, angles, velocity, movement state, team; god, noclip,
  respawn, `impulse 101`; health, armour and money sliders
  (`mashup_sethealth`, `mashup_setarmor <n> [helmet]`,
  `mashup_setmoney`); weapons held (clip/reserve), give any registered
  weapon, drop; teleport to each spawn point; bookmarks (saved places,
  kept in bookmarks.txt).
- World: the map and a map list to load, tick and game time, entity
  counts, map logic, time scale presets, `host_timescale`, `sv_gravity`.
- Movement: every CS:S movement cvar with its range and a reset, all
  back to defaults, surf preset, the mouse cvars.
- Bots: add T / CT, kick, back to orders, skill presets (the new game
  page's), the bot cvars (`bot_stop` freezes them), a table of each
  bot's team, health, role and activity with watch (`mashup_watch`) and
  go-to buttons.
- Rendering: the overlays (`cl_showpos`, `net_graph`,
  `mashup_drawhitboxes`, `mashup_drawcollision` (= F3),
  `mashup_drawphys`, `mashup_drawnav`, `mashup_drawbots`, ...), culling
  (`r_novis`, `r_portalsopenall`, `r_occlusion`, with the vis readout),
  HDR, water, view model and effect cvars, third person and free camera.
- Perf: a frame-time graph of the last 300 frames (white frame, green
  main-world CPU, 60 and 30 fps lines), the live interpolation line, the
  `mashup_perf` readout, `mashup_perf`, `mashup_perf_log`,
  `cl_interpolate`, how to take a trace.
- Audio: volume, DSP, `snd_show`, the soundscape and room readout.
- Logic: the map's entities (filter by name or class), an entity's
  keyvalues and output connections, fire an input at it (`ent_fire`);
  `mashup_logic_record 1` lists the outputs fired (latest 40 shown) and
  the logic's messages.
- Rounds: the round, its phase and score; restart (`mp_restartgame 1`),
  rounds on or off, the round cvars.
- Cvars: every cvar, searchable, "changed only", edited in place (Enter
  sets it), reset to default; matching commands with their help.

Headless: `tests/it/debug_ui.rs` opens it, walks the tabs and runs the
lines its tabs run in a `Sim` (`DebugUiStatePlugin`).

## 4. Logs

`RUST_LOG=mashup=debug cargo run --features dev`. Log state changes that matter
(movement swaps, mounts, match events) at `info`; per-tick detail at `trace`.

## 5. Exploring game files

`dump` lists, summarizes and extracts files from a game install, using the
same readers as the runtime mount. Install paths come from
`mashup.local.toml`.

```
cargo run --bin dump -- cs_source                      # counts and sizes by type
cargo run --bin dump -- cs_source --list --filter de_dust2
cargo run --bin dump -- cs_source --filter materials/de_dust --extract
cargo run --bin dump -- cs_source --sequences models/weapons/v_rif_ak47.mdl  # bones, sequences, activities
cargo run --bin dump -- combat_arms --archives         # per-archive title, entropy, file count
cargo run --bin dump -- combat_arms --list --filter worlds2
cargo run --bin dump -- combat_arms --filter worlds2/warehouse.dat --extract
```

Encrypted Combat Arms archives list but skip extraction until their scheme's
key is in `mashup.local.toml`. Extraction goes to `~/.local/share/mashup/dump/<game>/` (or `--out`) and
refuses any folder inside the repository.

### Sweeping every cached map (mapsweep)

`mapsweep` loads every map in mashup's content cache
(`~/.local/share/mashup/content/cs_source/maps/`) headless, each under
`catch_unwind`, runs its map logic for a few seconds, and writes
`report.md` (problems ranked by how many maps they affect, then one row
per map), `report.csv` and `warnings.txt` (every warning and logic line)
to `target/mapsweep/` (or `--out`). Load warnings are grouped by kind
(missing or unreadable materials, textures, models, sounds; decals and
overlays without a surface), plus entity classes nothing handles, classes
handled only as static brushes, and logic complaints (refused server
commands, unhandled inputs). With `--shots <mashup build>` it also runs
that build per map for a 1280x720 screenshot from the first spawn and the
last `mashup_perf_log` line (needs a display).

```
cargo run --features dev --bin mapsweep                      # every cached map
cargo run --features dev --bin mapsweep -- --filter surf_     # some of them
cargo run --features dev --bin mapsweep -- --shots target/playtest/mashup --filter mg_
```

The committed summary is in docs/plans/active/community-maps.md.

## 6. Comparing with the real game (refcmp)

`refcmp` captures the same camera views in real CS:S and in mashup and
reports, per view, brightness (luma), saturation, sharpness (mean absolute
Laplacian) and mean absolute difference, plus side-by-side images
(reference | ours | difference x4).

```
cargo build --features dev && cargo run --features dev --bin refcmp
cargo run --features dev --bin refcmp -- report            # re-compare existing captures
cargo run --features dev --bin refcmp -- capture-ours --only a_sign
```

- Views: `tools/refcmp/<map>.toml` (Source eye positions and angles; add
  more as needed; pass `--views tools/refcmp/<map>.toml` for maps other
  than dust2). de_aztec, cs_office and de_nuke have views generated from
  their own entities (intermission cameras, spawns, bomb/rescue zones);
  cs_assault, de_port and cs_compound (maps with occluders, for `bench`)
  have spawn views only, with no reference captures yet.
  `camera` names the map's `point_viewcontrol`; maps whose cameras have
  no name (de_aztec) use the class name, which moves all of them.
  Output: `~/.local/share/mashup/dump/refcmp/<map>/` (game imagery; never
  in the repo). `--out <dir>` writes our captures, reports, bench and
  vischeck output under `<dir>/<map>/` instead (reference captures are
  still read from the shared folder), so parallel worktrees don't
  overwrite each other's captures. Our captures currently show the view model.
- Load warnings of every stock map: `cargo test --features dev --test
  it all_stock_maps_warnings -- --ignored --nocapture`.
- CS:S runs through the Steam client (logged in once on this machine) and
  is driven over RCON on 127.0.0.1:27015, so it works with the desktop
  locked: the tool waits for the map to load, repositions the map's
  `point_viewcontrol` for each view and takes `jpeg` screenshots. Keys and
  `setpos` don't work for this (the lock screen owns the keyboard; RCON has
  no player).
- `refcmp fit` also captures mashup's `--debug-view albedo` and
  `--debug-view lighting` renders and fits, per pixel, how the real game
  combines texture and light. That is where `cs_source::bsp::source_look`
  (no tonemapping, bilinear mip snapping) comes from. Its 1.22 exposure
  turned out to be the bump page encoding; `report`'s luma column checks it.
- `refcmp` builds nothing: run `cargo build --features dev` after code
  changes, or it captures the old binary.
- The report's `detail` column (correlation of high-pass detail) is noisy
  on bump-mapped walls: it preferred a green-flipped normal map that crops
  show is wrong. Check close-up crops (`magick ... -crop`) side by side
  before trusting it.
- Measuring thin features (cables): sample columns of the reference and
  ours for dark pixels (`magick <img> -crop 1x260+X+0 txt:-`) and compare
  their rows; that's how rope gravity was found.
- The reference CS:S install runs at mat_hdr_level 0, mat_trilinear 0,
  mat_forceaniso 1, no AA (queried over RCON). HDR (level 2, the game's
  default) is an option (`+mat_hdr_level 2`, client/hdr.rs) but not
  matched yet: captures stay LDR unless a run sets it.
- `refcmp skyconv` measures how the engine samples sky cubemaps (an encoded
  debug sky, `MASHUP_SKY_DEBUG=1`) and fits each layer's texture and
  orientation to the reference: Bevy's skybox flips z, so looking toward -Z
  shows the +Z layer.
- mashup renders views off-screen at 1280x720 (`--views`), matching CS:S's
  framing (90 degrees horizontal at 4:3 = 74 vertical).

## Measuring CS:S behaviour live

The reference CS:S (see refcmp) can also be measured directly over RCON:
- Client commands run too: `getpos` returns the local player's position,
  `+jump`/`+forward`/`setang` move it. A `point_viewcontrol` holds the
  view (and `getpos`) until removed (`ent_remove cam1`; a map reload
  restores it for refcmp).
- Console output that RCON doesn't return (e.g. `ent_dump`) goes to a file
  with `con_logfile <name>` (under `cstrike/`).
- `ent_fire player addoutput "targetname <x>"` names the player for
  `ent_dump`/`ent_fire`.
- Sampling `getpos` over RCON is ~20 Hz with timing noise; fine for
  heights (jump apex), not for per-tick speeds. `host_timescale` slows the
  game but doesn't remove the noise.
- Weapon scripts are ICE-encrypted (`.ctx`); decrypt copies in a scratch
  folder only, never in the repo.

## Tick-exact CS:S comparisons (movecmp)

```
cargo run --features dev --bin movecmp            # all scenarios
cargo run --features dev --bin movecmp -- --only bhop --keep-running
cargo run --features dev --bin movecmp -- fuzz --runs 16 --ticks 150   # ladders and water
cargo run --features dev --bin movecmp -- --offline   # against the last CS:S logs, no server
```

- Each scenario (in `src/bin/movecmp.rs`) is a start position and per-tick
  inputs. A CS:S dedicated server with the `tools/css_probe` SourceMod
  plugin plays them on a bot and logs every tick; mashup plays the same
  inputs from the game's tick-0 state on the same map at the same 0.015 s
  tick. Setup: tools/css_probe/README.md.
- Report: per scenario, the largest position/velocity difference and the
  first tick that differs by more than 0.1 unit (or in ground/duck
  state), with both states. Per-tick CSVs go to
  `~/.local/share/mashup/dump/movecmp/`.
- The probe can also be driven directly over RCON (127.0.0.1:27030,
  `mashup_run <in> <out>`, `mashup_weapon <weapon>`); its log has the
  player's box top, eye height and the buttons the movement ran with.
  The plugin resets the bot's move type, remembered ladder and jump
  stamina when it places it, and logs its water level and ladder state.
  CS:S bots duck-jump on their own (inside the game, not through their
  buttons): movecmp holds duck on our side while the bot is ducked in the
  air, so jumps differ only by the bot's instant 8.5-unit duck on the jump
  tick.
- `--offline` replays the scenarios against the CS:S logs the last online
  run left in the server's `cstrike/mashup/<name>.out` (same inputs), for
  checking movement changes without starting the server.
- Damage measurements: `tools/css_probe/fallmeas.py` (fall damage; see
  tools/css_probe/README.md). `player_hurt` rows of `mashup_wrun` logs
  carry each hit's damage.
- `movecmp fuzz`: seeded random inputs (keys, jump, duck, turning and
  pitch, in bursts) from ladder spots on de_nuke and water spots on
  de_aztec (found and checked in our sim), plus a straight climb per
  ladder. Each scenario names its map; the probe server changes level as
  needed. Runs fail past `--tolerance` units or when water level or ladder
  state disagree; the exit code says whether all passed. Same `--seed`,
  same inputs.
- It has found: CS:S's fixed 0.015 s tick, the alternating diagonals of
  displacement triangles (walking and landing on dust2 terrain now match
  to 0.002 units), the 0.34 ducked speed and the 8.5-unit air-duck lift.
  Fuzzing water and ladders found: the 260 swim lift, CS:S's duck scale
  rule (duck held, or ducked/mid-duck at the tick's start; on ladders
  too), the per-tick contents cache at the water surface, and that a
  coincident player-clip face can win over a ladder brush (which one wins
  follows the BSP tree's order, `MapBrushTree`).

## Planned

- Per-tick JSONL traces of chosen entities, for comparing movement against
  the original game's measurements.
- Scripted intent playback in the windowed game (record a demo, replay it,
  screenshot at given ticks).
