# Observability: how to check your work

Agents can't see the screen. These tools make the game's state readable.
Prefer them, in this order, over asking a human to look.

## 1. Headless scenario tests (fastest, most precise)

`cargo test` runs `tests/movement.rs` and friends on `harness::Sim`: no
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

## 2. Screenshots

```
cargo run --features dev -- --screenshot shot.png --frames 90 \
    --spawn 0,1,12 --look 30,-10 --movement mashup:noclip
```

Add `--map cs_source:de_dust2` to load a real map (noclip plus
`--spawn x,75,z --look 0,-89` gives a top-down view). `--lightmap-only`
renders surfaces white so only baked lighting shows: misoriented lightmaps
appear as shadows that break at face edges.

Runs 90 frames, saves the primary window, exits. Read the PNG to check
rendering. Write screenshots to a scratch directory, never into the repo.
Needs a display: on the Linux dev box set `WAYLAND_DISPLAY=wayland-1` and
`XDG_RUNTIME_DIR=/run/user/1000` if the shell lacks them. `--frames N`
without `--screenshot` just runs N frames and exits (smoke test).
`--help` lists all options.

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
- `ent_fire <target> <input> [value]` sends a map entity an input through
  the logic layer (names, `*` wildcards, classnames; the local player is
  the activator), e.g. `+wait 30 +ent_fire logic_timer Disable +ent_fire
  computer0* Skin 2` in a `--screenshot` run on de_nuke (the `wait` lets
  the map's logic load first). Logged at info level.
- `mashup_drawnav 1` outlines the nav areas near you (2: all), coloured by
  place, with half-links toward their neighbours; `mashup_drawbots 1`
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
  looking away from (white). `bot_debug 1` lists each team's plan and
  every bot's team, role, site, activity (ToSite, Following, Waiting,
  Holding, Chasing, Assisting, ...; `*` marks the group leader) and
  health on screen. To watch a round on dust2 from above: `--map
  cs_source:de_dust2 +mashup_rounds 1 +bot_add 1 +bot_add 1 +bot_add 1
  +bot_add 2 +bot_add 2 +bot_add 2 +mashup_drawbots 1 +bot_debug 1
  +noclip +setpos -400 3000 1200 +setang 89 90 0`. Headless round
  statistics (winners, kills, time to first contact):
  `MASHUP_BOT_MAP=de_dust2 MASHUP_BOT_ROUNDS=8 cargo test --features dev
  --test bot_rounds -- --ignored --nocapture`. `bot_grenades 2` makes bots throw whenever
  an arc works (`mashup_watch 1` to follow one). On the greybox map:
  `--spawn 0,1,-5 +bot_dont_shoot 1 +bot_stop 1 +bot_grenades 2 +bot_add 1
  +bot_give weapon_hegrenade +mashup_drawbots 1 +mashup_watch 1
  +cam_idealdist 600 +cam_idealyaw 70 ++attack --frames 130` shows the bot
  hearing you behind the long crate and lobbing an HE over it.
- `mashup_drawhitboxes 1` outlines other characters' hitboxes, coloured
  by hitgroup, where shots test them (e.g. `+bot_stop 1 +bot_add 2
  +mashup_drawhitboxes 1` with `--screenshot` to check they follow the
  animated body).
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
- `mashup_healthbars 1` draws a health bar over every other living
  character (green full, red nearly dead), e.g. to watch damage land in a
  `--screenshot` run with `+bot_add 2`.
- `mashup_particles` prints the live particle groups and counts (and how
  many count against the 2048 cap), e.g. over the remote console after
  shooting. Impact effects obey `r_drawflecks`, `cl_show_splashes` and
  `violence_hblood` (0/1). To see a shot's effects in a `--screenshot`
  run, fire with `++attack` and keep `--frames` low enough that they are
  still alive (dust lives about a second, sparks a tenth of one).
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
- View model (CS:S cvars): `viewmodel_fov` (54), `cl_righthand` (1;
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
  config.cfg (written on quit when binds/cvars changed), autoexec.cfg,
  history.txt; `exec name` runs name.cfg from there.

## 3c. Performance

Details and baseline numbers: [performance.md](performance.md).

- `mashup_perf 1` (2: every render pass) shows frame times (avg, p95,
  max), the main world's CPU time, GPU time per render pass, entity,
  mesh and triangle counts and visibility culling (camera cluster,
  clusters and map parts potentially visible).
- `refcmp bench --views tools/refcmp/<map>.toml` times 200 frames at each
  view (vsync off) and prints a table; `-- <args>` passes options to
  mashup (e.g. `-- +r_novis 1`). Build first; it runs the mashup next to
  it (`--profile playtest` for optimized numbers).
- `r_novis 1` draws every map part (no visibility culling);
  `MASHUP_MERGED_WORLD=1` spawns the world as one mesh per material with no
  culling, as before chunking (A/B comparisons).
- `refcmp vischeck --views tools/refcmp/<map>.toml` renders the views
  plus views from spawns and nav areas with culling off and on (game time
  frozen with `host_timescale 0`), lists the views that differ and fails
  if any differs by more than 0.5% of its pixels (small differences:
  geometry seen through sky brushes, which culling hides as the game
  does; see performance.md). `cargo test --test map_vis` checks the same with rays from
  ~300 player positions per map (`MASHUP_VIS_FULL=1`: every nav area).
- `cargo run --profile playtest --features profile` writes a Chrome trace
  (`trace-*.json`) with every system's CPU time; open it in
  https://ui.perfetto.dev.

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
  their own entities (intermission cameras, spawns, bomb/rescue zones).
  `camera` names the map's `point_viewcontrol`; maps whose cameras have
  no name (de_aztec) use the class name, which moves all of them.
  Output: `~/.local/share/mashup/dump/refcmp/<map>/` (game imagery; never
  in the repo). Our captures currently show the view model.
- Load warnings of every stock map: `cargo test --features dev --test
  map_stock -- --ignored --nocapture all_stock_maps_warnings`.
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
  default) is not matched yet.
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
  coincident player-clip face wins over a ladder brush.

## Planned

- Per-tick JSONL traces of chosen entities, for comparing movement against
  the original game's measurements.
- Scripted intent playback in the windowed game (record a demo, replay it,
  screenshot at given ticks).
