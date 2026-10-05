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
| `src/core.rs` | `Intent`, `Velocity`, `MovementState`, `Health`, `Team`, `SpawnPoint`, `LocalPlayer`, `SimTick`, the `SimSet` tick ordering |
| `src/slots.rs` | `MovementRegistry`, `Loadout`, `set_movement` (swap an entity's movement) |
| `src/character.rs` | `character_bundle`, `spawn_character`, capsule size |
| `src/movement/` | `placeholder` (stand-in walking) and `noclip` Movement implementations |
| `src/greybox.rs` | Map slot built in code; collision always, visuals only when rendering |
| `src/mount/` | `Mount` (ordered layers, first match wins), `FileSource`, `LooseDir`, path normalization, `mashup.local.toml` loading |
| `src/map.rs` | `MapData` (meters, Y up, meshes per material, textures, lightmap atlas, collision, prop models and placements, spawns) and `MapPlugin` that spawns it |
| `src/games/mod.rs` | `load_map("game:name")` dispatcher |
| `src/games/cs_source/` | VPK reader, the CS:S search path, BSP to `MapData` (via `vbsp`), VMT/VTF materials (map pak first, then the mount), lightmap atlas from the lighting lump, brush collision hulls, static props via `vmdl`, ambient cubes and world lights for prop light probes, infodecals clipped onto faces |
| `src/games/combat_arms/` | `.rez` reader and the archive cipher payload decryption (from the spec), all archives as one mount |
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
