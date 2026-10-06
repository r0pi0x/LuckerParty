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
- `mashup_drawhitboxes 1` outlines other characters' hitboxes, coloured
  by hitgroup, where shots test them (e.g. `+bot_stop 1 +bot_add 2
  +mashup_drawhitboxes 1` with `--screenshot` to check they follow the
  animated body).
- `mashup_healthbars 1` draws a health bar over every other living
  character (green full, red nearly dead), e.g. to watch damage land in a
  `--screenshot` run with `+bot_add 2`.
- `thirdperson` (back with `firstperson`; distance `cam_idealdist`, CS:S
  units) puts the camera behind the local player and draws its own
  animated body: `+thirdperson` in a `--screenshot` run shows the local
  player's model and animation.
- Remote: `curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":1,
  "method":"mashup/console","params":{"line":"getpos; cvarlist sv_"}}'`
  runs a line now and returns the lines it printed.
- Configs live in `~/.local/share/mashup/cfg` (Windows `%APPDATA%\mashup\cfg`):
  config.cfg (written on quit when binds/cvars changed), autoexec.cfg,
  history.txt; `exec name` runs name.cfg from there.

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
