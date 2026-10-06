# Architecture

A map of the code. Design rationale lives in the [README](../README.md); this
file says where things are and which way dependencies may point.

## Layers

```
            client            (window, input, camera, HUD, debug, agent tools)
              |
   harness    |    games/<name>   (one module per game)
      \       |       /    \
       bot   rules   movement   greybox   mount   (mount: VFS, local config)
        |      |        |          |
        |    weapon     |          |      (weapon: parts, inventory, weapon frame)
        |      |        |          |
        |     map       |          |      (map: neutral MapData + spawner, sound)
         \     |       /          /
           character             (components every character has)
               |
             slots               (registries, loadout, swapping)
               |
            console              (cvars, commands, queue; games and client register)
               |
             core                (shared vocabulary)
```

Exact edges: bot uses core, console, slots, character, map; rules uses core,
console, weapon; weapon uses core, console, map; map uses only core.

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
| `src/console.rs` | The console's core: cvar/command registry (`ConsoleAppExt::console_cvar`/`console_command`, `resource_cvar`), Source-style parsing (`;`, quotes, `//`), aliases, binds, `wait`, exec/config files, built-ins (help, find, cvarlist, differences, reset, toggle, incrementvar, watch, host_writeconfig, ...) |
| `src/client/console.rs` | The in-game console UI (`~`): completion, fuzzy suggestions, history with Ctrl+R, scrollback/filter/timestamps, binds with `+`/`-` actions, log mirroring, overlays (cl_showpos, cl_showfps, snd_show, watch), client commands (noclip, god, getpos/setpos/setang, kill, map (in place, `map::change_map`), quit), `developer 1` notify lines, the `mashup/console` remote method |
| `src/core.rs` | `Intent`, `Velocity`, `MovementState`, `Health`, `Damage`/`Died` messages (damage applied after weapons), `Hitgroup`, `Hitbox`/`Hitboxes`, `MaxSpeed` (equipment speed cap), `God`, `Team`, `SpawnPoint`, `LocalPlayer`, `SimTick`, the `SimSet` tick ordering; the collision world maps share with movement: `MapBrush`/`MapBrushes` (brush planes for exact swept-box movement, ladder flag), `MapBrushCollider`, `MapWater` volumes |
| `src/weapon/` | Weapons as parts (README, "Weapons and items"): `Trigger`, `Magazine` (cost), `Hitscan`/`Melee` (delivery), `Penetration` with `PassMaterials` (bullets through walls and characters), `DamageEffect`, `WeaponSounds`; `Inventory`, `WeaponState` timers, `WeaponRegistry`/`StartingWeapons`, `give`; selection and deploy before movement, the Source-style weapon frame in `SimSet::Weapons` (spec weapons.md 3–4), `WeaponEvent`s; console `give`, `slot1`–`slot5`, `lastinv` |
| `src/rules.rs` | Deathmatch: the dead (`Dead`) stop acting and respawn at spawn points with fresh weapons after `mp_respawn_delay`; `Score` (kills, deaths) |
| `src/bot.rs` | Bots: `Bot` brain writing `Intent` (nearest visible enemy, limited turn rate, reaction time, strafing; otherwise walks the `NavMesh` to where it last saw or heard an enemy, or roams); `bot_add`, `bot_kick`, `bot_stop`, `bot_dont_shoot`, `bot_reaction`, `bot_turn_rate` |
| `src/slots.rs` | `MovementRegistry`, `Loadout`, `set_movement` (swap an entity's movement) |
| `src/character.rs` | `character_bundle`, `spawn_character`, capsule size |
| `src/movement/` | `placeholder` (stand-in walking) and `noclip` Movement implementations |
| `src/greybox.rs` | Map slot built in code (ramps, crates, a ladder, a water tank); collision always, visuals only when rendering |
| `src/mount/` | `Mount` (ordered layers, first match wins), `FileSource`, `LooseDir`, path normalization, `mashup.local.toml` loading |
| `src/map/prop_material.rs`, `prop.wgsl` | Props lit by a light probe (vertex colours), Source range fog, and `env_cubemap` reflections of the nearest baked cubemap |
| `src/map/world_material.rs`, `world.wgsl` | World surfaces: texture x baked light with radiosity normal mapping, plus baked-cubemap reflections (Source LightmappedGeneric at LDR) |
| `src/map/mod.rs` | `MapData` (meters, Y up, meshes per material, textures, lightmap atlas, collision, prop models and placements, spawns, ropes) and `MapPlugin` that spawns it (re-exports the `core` collision types) |
| `src/map/anim.rs` | Skeletal animation for any game: `AnimSet` (sampled animations, sequences on pose-parameter grids, autolayers), the blend/layer math, and the `Animator` component that `pose_bodies` turns into body joint transforms; games drive it in `DriveAnimation` |
| `src/map/nav.rs` | `NavMesh` (any game): areas with corner heights, directed links and ladders in engine space; area queries, A* with the stock cost, crossing points, `route` |
| `src/map/soundscape.rs` | Soundscape playback: zone/emitter selection, looping ambience with crossfades, random one-shots |
| `src/map/sound.rs` | Sounds for any game: entries and clips on `MapData`, `PlaySound` messages, playback (distance model, panning, pitch, channels) through Bevy audio, `SurfaceGrid` (surface under a point) |
| `src/map/shadows.rs`, `shadow.wgsl` | Dynamic prop shadows (Source render-to-texture shadows): silhouettes rasterized into a coverage atlas, clipped onto world surfaces, multiplied over them |
| `src/map/rope_material.rs`, `rope.wgsl` | Ropes as camera-facing strips (Source Cable shader, fake anti-aliasing back strip) |
| `src/games/mod.rs` | `load_map("game:name")` dispatcher |
| `src/games/cs_source/` | VPK reader, the CS:S search path, BSP to `MapData` (via `vbsp`), VMT/VTF materials (map pak first, then the mount), lightmap atlas from the lighting lump, brush collision hulls, static props and prop entities (`prop_physics*`, `prop_dynamic*`) via `vmdl`, ambient cubes and world lights for prop light probes, infodecals clipped onto faces, overlays (lump 45) clipped onto their listed faces, 2D sky and 3D skybox, ropes (simulated to rest), Source player movement (`cs_source:movement`: swept box, exact against brush planes, physics shape casts for props and displacements; ladders, water, walking), `.phy` collision models, surface properties, physics props (avian bodies) and the multiplayer push-away between players and props, WAV decoding (PCM, MS-ADPCM) and sound scripts, footstep/jump/landing/water sounds in movement, the knife and AK-47 (`weapons.rs`, script values and measured rules: recoil sets, inaccuracy, penetration materials; unmeasured rules marked), bullet and physics impact sounds (`impacts.rs`), `.nav` meshes (`nav.rs`, versions 9 and 16), soundscapes (`soundscape.rs`), player model animations (`anim.rs`: `.mdl` v44 animations, sequences and include models decoded from raw bytes into a `map::anim::AnimSet`) and the player animation state driving them (`player_anim.rs`) |
| `src/games/combat_arms/` | `.rez` reader and the archive cipher payload decryption (from the spec), all archives as one mount |
| `src/bin/refcmp.rs` | Dev tool: compare views against real CS:S (RCON-driven capture, metrics, side-by-sides) |
| `src/bin/dump.rs` | Dev tool: summarize, list and extract a game install's files |
| `src/harness.rs` | `Sim`: headless app stepped by exact fixed ticks, for tests |
| `src/client/` | Local input, first-person camera, debug UI, `--screenshot`, remote protocol; `hud.rs`: crosshair (gap from spread), health, ammo, hit marker, killfeed, capsule bodies for other characters |
| `src/lib.rs` | `SimPlugins` (everything the simulation needs) |
| `src/main.rs` | The game binary: `SimPlugins` + map + `ClientPlugin` |

## Data flow per tick

```
Update:       keyboard/mouse ──> Intent (local player)
FixedUpdate:  respawn, bot brains ──> Intent; weapon selection/deploy ──> MaxSpeed
              SimSet::Movement  Intent (+ MaxSpeed) ──> Transform, Velocity, MovementState
              SimSet::Weapons   Intent + MovementState ──> traces, Damage, WeaponEvent, PlaySound
              then              Damage ──> Health, Died ──> Score, Dead
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
