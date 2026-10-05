# Architecture

A map of the code. Design rationale lives in the [README](../README.md); this
file says where things are and which way dependencies may point.

## Layers

```
            client            (window, input, camera, debug, agent tools)
              |
   harness    |    games/<name>   (one module per game)
      \       |       /    \
       movement   greybox  map  mount  (map: neutral MapData + spawner; mount: VFS, local config)
            \     /
           character             (components every character has)
               |
             slots               (registries, loadout, swapping)
               |
             core                (shared vocabulary)
```

A module may use only the modules below it. Enforced by
`tests/architecture.rs` (`ALLOWED`); update both together.

- `core` never depends on anything in this crate.
- Simulation code (everything except `client`) never depends on `client`, so
  it runs headless in tests and, later, on a dedicated server.
- `games/<a>` never uses `games/<b>`. Shared needs go into `core` once two or
  three games need them (README, "rule of three"), or through a bridge.

## Modules

| Module | What it owns |
|---|---|
| `src/core.rs` | `Intent`, `Velocity`, `MovementState`, `Health`, `Team`, `SpawnPoint`, `LocalPlayer`, `SimTick`, the `SimSet` tick ordering; the collision world maps share with movement: `MapBrush`/`MapBrushes` (brush planes for exact swept-box movement, ladder flag), `MapBrushCollider`, `MapWater` volumes |
| `src/slots.rs` | `MovementRegistry`, `Loadout`, `set_movement` (swap an entity's movement) |
| `src/character.rs` | `character_bundle`, `spawn_character`, capsule size |
| `src/movement/` | `placeholder` (stand-in walking) and `noclip` Movement implementations |
| `src/greybox.rs` | Map slot built in code (ramps, crates, a ladder, a water tank); collision always, visuals only when rendering |
| `src/mount/` | `Mount` (ordered layers, first match wins), `FileSource`, `LooseDir`, path normalization, `mashup.local.toml` loading |
| `src/map/world_material.rs`, `world.wgsl` | World surfaces: texture x baked light with radiosity normal mapping (Source LightmappedGeneric at LDR) |
| `src/map/mod.rs` | `MapData` (meters, Y up, meshes per material, textures, lightmap atlas, collision, prop models and placements, spawns, ropes) and `MapPlugin` that spawns it (re-exports the `core` collision types) |
| `src/map/sound.rs` | Sounds for any game: entries and clips on `MapData`, `PlaySound` messages, playback (distance model, panning, pitch, channels) through Bevy audio, `SurfaceGrid` (surface under a point) |
| `src/map/shadows.rs`, `shadow.wgsl` | Dynamic prop shadows (Source render-to-texture shadows): silhouettes rasterized into a coverage atlas, clipped onto world surfaces, multiplied over them |
| `src/map/rope_material.rs`, `rope.wgsl` | Ropes as camera-facing strips (Source Cable shader, fake anti-aliasing back strip) |
| `src/games/mod.rs` | `load_map("game:name")` dispatcher |
| `src/games/cs_source/` | VPK reader, the CS:S search path, BSP to `MapData` (via `vbsp`), VMT/VTF materials (map pak first, then the mount), lightmap atlas from the lighting lump, brush collision hulls, static props and prop entities (`prop_physics*`, `prop_dynamic*`) via `vmdl`, ambient cubes and world lights for prop light probes, infodecals clipped onto faces, overlays (lump 45) clipped onto their listed faces, 2D sky and 3D skybox, ropes (simulated to rest), Source player movement (`cs_source:movement`: swept box, exact against brush planes, physics shape casts for props and displacements; ladders, water, walking), `.phy` collision models, surface properties, physics props (avian bodies) and the multiplayer push-away between players and props, WAV decoding (PCM, MS-ADPCM) and sound scripts, footstep/jump/landing/water sounds in movement |
| `src/games/combat_arms/` | `.rez` reader and the archive cipher payload decryption (from the spec), all archives as one mount |
| `src/bin/refcmp.rs` | Dev tool: compare views against real CS:S (RCON-driven capture, metrics, side-by-sides) |
| `src/bin/dump.rs` | Dev tool: summarize, list and extract a game install's files |
| `src/harness.rs` | `Sim`: headless app stepped by exact fixed ticks, for tests |
| `src/client/` | Local input, first-person camera, debug UI, `--screenshot`, remote protocol |
| `src/lib.rs` | `SimPlugins` (everything the simulation needs) |
| `src/main.rs` | The game binary: `SimPlugins` + map + `ClientPlugin` |

## Data flow per tick

```
Update:       keyboard/mouse ──> Intent (local player)
              bot brain ──────> Intent (later)
FixedUpdate:  SimSet::Movement  Intent ──> Transform, Velocity, MovementState
              SimSet::Weapons   Intent + MovementState ──> effects (later)
Update:       camera <── Intent (look) + MovementState (eye offset)
```

Intent buttons are levels, not edges. Movement runs at a fixed tick
(`DEFAULT_TICK_HZ` until a spec sets one); rendering interpolates translation.

## Slots

An implementation registers under a namespaced ID (`mashup:noclip`,
`combat_arms:movement`) with a marker component. Its systems act only on
entities with that marker, so swapping is removing one marker and inserting
another (`slots::set_movement`). Only Movement exists so far; the other slots
follow the same pattern when they arrive.
