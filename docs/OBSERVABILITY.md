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
cargo run --bin dump -- combat_arms --list             # per-archive key title and entropy
```

Extraction goes to `~/.local/share/mashup/dump/<game>/` (or `--out`) and
refuses any folder inside the repository.

## Planned

- Per-tick JSONL traces of chosen entities, for comparing movement against
  the original game's measurements.
- Scripted intent playback in the windowed game (record a demo, replay it,
  screenshot at given ticks).
