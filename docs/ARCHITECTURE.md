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
| `src/client/game_hud.rs` | The game's own HUD when the map provides one (`map::hud::ActiveHud`): health, armour and ammo panels in the game's fonts and icon glyphs, death notices with team colours and weapon/headshot icons; scaled by window height like Source's proportional HUD |
| `src/client/radar.rs`, `radar.wgsl` | The radar: the map's overview picture (`map::hud::ActiveOverview`) in the HUD's `HudRadar` box, turned so you face up (in a UI material, so it clips), team-coloured dots (enemies while you see them), your nav place name below |
| `src/client/hud_sprites.rs` | The game HUD's sprite icons (`map::hud::HudSprite`, rectangles of the game's HUD sprite sheets): the ammo-type icon in the ammo panel and the damage direction indicators (`pain_*`) |
| `src/client/scoreboard.rs` | The scoreboard (Tab or `+showscores`): Terrorists and Counter-Terrorists columns with kills, deaths and dead status, the local player highlighted |
| `src/client/console.rs` | The in-game console UI (`~`): completion, fuzzy suggestions, history with Ctrl+R, scrollback/filter/timestamps, binds with `+`/`-` actions, log mirroring, overlays (cl_showpos, cl_showfps, snd_show, watch), client commands (noclip, god, getpos/setpos/setang, kill, map (in place, `map::change_map`), quit), `developer 1` notify lines, the `mashup/console` remote method |
| `src/core.rs` | `Intent`, `Velocity`, `MovementState`, `Health`, `Damage`/`Died` messages (damage applied after weapons), `Hitgroup`, `Hitbox`/`Hitboxes`, `MaxSpeed` (equipment speed cap), `God`, `Team`, `SpawnPoint`, `LocalPlayer`, `SimTick`, the `SimSet` tick ordering; the collision world maps share with movement: `MapBrush`/`MapBrushes` (brush planes for exact swept-box movement, ladder flag), `MapBrushCollider`, `MapTerrain` (terrain triangles as thin brushes, with a grid) and `MapTerrainCollider`, `MapWater` volumes |
| `src/weapon/` | Weapons as parts (README, "Weapons and items"): `Trigger`, `Magazine` (cost), `Hitscan`/`Melee` (delivery), `Penetration` with `PassMaterials` (bullets through walls and characters), `DamageEffect`, `WeaponSounds`; `Inventory`, `WeaponState` timers, `WeaponRegistry`/`StartingWeapons`, `give`; selection and deploy before movement, the Source-style weapon frame in `SimSet::Weapons` (spec weapons.md 3–4), `WeaponEvent`s; console `give`, `slot1`–`slot5`, `lastinv` |
| `src/rules.rs` | Deathmatch: the dead (`Dead`) stop acting and respawn at their team's spawn points with fresh weapons after `mp_respawn_delay`; `Score` (kills, deaths); `mp_friendlyfire` (applied in `core::apply_damage` through `core::FriendlyFire`); `jointeam 2|3` |
| `src/bot.rs` | Bots: `Bot` brain writing `Intent` (nearest visible enemy, limited turn rate, reaction time, strafing; otherwise walks the `NavMesh` to where it last saw or heard an enemy, or roams); `bot_add`, `bot_kick`, `bot_stop`, `bot_dont_shoot`, `bot_reaction`, `bot_turn_rate` |
| `src/slots.rs` | `MovementRegistry`, `Loadout`, `set_movement` (swap an entity's movement) |
| `src/character.rs` | `character_bundle`, `spawn_character`, capsule size |
| `src/movement/` | `placeholder` (stand-in walking) and `noclip` Movement implementations |
| `src/greybox.rs` | Map slot built in code (ramps, crates, a ladder, a water tank); collision always, visuals only when rendering |
| `src/mount/` | `Mount` (ordered layers, first match wins), `FileSource`, `LooseDir`, path normalization, `mashup.local.toml` loading |
| `src/map/prop_material.rs`, `prop.wgsl` | Props lit by a light probe (vertex colours, or per pixel from uniforms for moving models such as view models), plus the scene's point lights (`DynamicLight`, read from Bevy's light clusters), Source range fog, and `env_cubemap` reflections of the nearest baked cubemap |
| `src/map/world_material.rs`, `world.wgsl` | World surfaces: texture x baked light with radiosity normal mapping, plus the scene's point lights (muzzle flashes), plus baked-cubemap reflections (Source LightmappedGeneric at LDR) |
| `src/map/hurt.rs` | Hurt volumes (`trigger_hurt`): `MapHurt`, damage to characters inside every half second |
| `src/map/mod.rs` | `MapData` (meters, Y up, meshes per material, textures, lightmap atlas, collision, prop models and placements, spawns, ropes, the `MapLightField` light-at-a-point query, muzzle flash and shell data) and `MapPlugin` that spawns it (re-exports the `core` collision types); character bodies (the local player's hidden unless `ShowLocalBody`, which also removes the view model in third person) |
| `src/map/anim.rs` | Skeletal animation for any game: `AnimSet` (sampled animations, sequences on pose-parameter grids, autolayers), the blend/layer math, and the `Animator` component that `pose_bodies` turns into body joint transforms; games drive it in `DriveAnimation` |
| `src/map/view_model.rs` | First-person view models for any game: `MapViewModel` (skinned meshes in eye space, own skeleton, `AnimSet`, handedness, attachments, lighting origin), `ViewModels`, the `ViewAnimator` and `ViewModelOffset` (bob/sway) games drive on each character, `ViewModelSettings` (FOV, hand, draw), and the drawing at a `ViewModelAnchor` camera (a child camera on `VIEW_MODEL_LAYER` with its own depth and FOV, so the model never clips into walls; mirrored when its hand differs from the player's; lit per pixel by the `LightField` at its lighting origin); muzzle flashes from `ViewModelEvent`s (sprites re-projected into the world view, or at a held weapon's muzzle, and a pooled `DynamicLight`) |
| `src/map/shells.rs` | Spent shells for any game: `MapShell`/`MapShellPhysics` from the game, thrown from the view model's ejection attachment on brass events, flying, bouncing (sounds), resting and fading; drawn in the view-model pass until the first bounce |
| `src/map/decal.rs`, `decal.wgsl` | Runtime decals for any game: `MapDecals` (decal images in atlas rectangles, weighted groups, surface material → group), `PlaceDecal` projects one onto the world triangles near a hit, or onto a hit prop's own model (as its child), clipped to its rectangle, drawn modulate 2× (`DecalMaterial`) above the map's decals and overlays, oldest removed past 512 |
| `src/map/hud.rs` | A game's own HUD look for any game (`GameHud`: fonts, panels in a virtual 640×480 screen with `c`/`r` anchors, icon glyphs, colours), drawn by `client::game_hud` |
| `src/map/particles.rs` | Particles for any game: `Particles` (a CPU pool of groups that share a motion rule: gravity, decay, damping, drag, bounce planes probed once per group; a cap, a frame-step clamp, near fade, merging), `WorldTracer` (traces against the world only), `MapParticles` materials (texture, blend, vertex colour/alpha, sprite sheet); drawn each frame as one rebuilt mesh per material, camera-facing sprites, trails and flat quads, back to front |
| `src/map/surface_color.rs` | `SurfaceColors`: a surface's look at a hit point (texture average colour and baked light: lightmap for the world, light probe for props), for effects tinted by what they hit |
| `src/map/nav.rs` | `NavMesh` (any game): areas with corner heights, directed links and ladders in engine space; area queries, A* with the stock cost, crossing points, `route` |
| `src/map/soundscape.rs` | Soundscape playback: zone/emitter selection, looping ambience with crossfades, random one-shots |
| `src/map/sound.rs` | Sounds for any game: entries and clips on `MapData`, `PlaySound` messages, playback (distance model, panning, pitch, channels) through Bevy audio, `SurfaceGrid` (surface under a point) |
| `src/map/shadows.rs`, `shadow.wgsl` | Dynamic prop shadows (Source render-to-texture shadows): silhouettes rasterized into a coverage atlas, clipped onto world surfaces, multiplied over them |
| `src/map/rope_material.rs`, `rope.wgsl` | Ropes as camera-facing strips (Source Cable shader, fake anti-aliasing back strip) |
| `src/games/mod.rs` | `load_map("game:name")` dispatcher |
| `src/games/cs_source/` | VPK reader, the CS:S search path (plus the install's `download/` folder and mashup's content cache, last; `mount::import_map` copies `.bsp`/`.bsp.bz2` maps into the cache), BSP to `MapData` (via `vbsp`), VMT/VTF materials (map pak first, then the mount), lightmap atlas from the lighting lump, brush entities (drawn and solid where they spawn), brush collision hulls, static props and prop entities (`prop_physics*`, `prop_dynamic*`) via `vmdl`, ambient cubes and world lights for prop light probes, infodecals clipped onto faces, overlays (lump 45) clipped onto their listed faces, 2D sky and 3D skybox, ropes (simulated to rest), Source player movement (`cs_source:movement`: swept box, exact against brush planes and displacement triangles, physics shape casts for props; ladders, water, walking, fall damage), `trigger_hurt` volumes (`bsp.rs::hurt_volumes`), `.phy` collision models, surface properties, physics props (avian bodies) and the multiplayer push-away between players and props, WAV decoding (PCM, MS-ADPCM) and sound scripts, footstep/jump/landing/water sounds in movement, the knife and AK-47 (`weapons.rs`, script values and measured rules: recoil sets, inaccuracy, penetration materials; unmeasured rules marked), bullet and physics impact sounds (`impacts.rs`), impact particles by surface material, blood and water splashes (`impact_effects.rs`: the spec's numbers on `map::particles`; materials loaded with the map), `.nav` meshes (`nav.rs`, versions 9 and 16), soundscapes (`soundscape.rs`), player model animations (`anim.rs`: `.mdl` v44 animations, sequences and include models decoded from raw bytes into a `map::anim::AnimSet`) and the player animation state driving them (`player_anim.rs`), view models (`props.rs::load_view_model`, paths and handedness in `weapons.rs::VIEW_MODELS`; `anim.rs` also reads animation events and attachments), what they play from weapon events and the effects their animation events ask for (`view_anim.rs`: draw, fire, knife slash/stab, reload, idle by activity; muzzle flash and shell data; the view-model cvars), bob and sway (`view_motion.rs`), and the light field for moving models (`props::probe_with` at run time), the HUD look from the install (`hud.rs`: `hudlayout.res`, `clientscheme.res` and its TTFs, `mod_textures.txt` glyphs) |
| `src/games/combat_arms/` | `.rez` reader and the archive cipher payload decryption (from the spec), all archives as one mount |
| `src/bin/refcmp.rs` | Dev tool: compare views against real CS:S (RCON-driven capture, metrics, side-by-sides) |
| `src/bin/dump.rs` | Dev tool: summarize, list and extract a game install's files; `--sequences` lists a model's bones and sequences |
| `src/harness.rs` | `Sim`: headless app stepped by exact fixed ticks, for tests |
| `src/client/view.rs` | Third-person camera (`thirdperson`/`firstperson`, `cam_idealdist`, swept back from the eye against the world), master `volume` (Bevy `GlobalVolume`), `mashup_healthbars` |
| `src/client/` | Local input (mouse look as CS:S: `sensitivity` x `m_yaw`/`m_pitch` degrees per count), first-person camera, debug UI, `--screenshot`, remote protocol; `hud.rs`: crosshair (gap from spread), health, ammo, hit marker, killfeed, capsule bodies for other characters |
| `src/lib.rs` | `SimPlugins` (everything the simulation needs) |
| `src/main.rs` | The game binary: `SimPlugins` + map + `ClientPlugin` |

## Data flow per tick

```
Update:       keyboard/mouse ──> Intent (local player)
FixedUpdate:  respawn, bot brains ──> Intent; weapon selection/deploy ──> MaxSpeed
              SimSet::Movement  Intent (+ MaxSpeed) ──> Transform, Velocity, MovementState
              SimSet::Weapons   Intent + MovementState ──> traces, Damage, WeaponEvent, PlaySound
              then              Damage ──> Health, Died ──> Score, Dead
                                WeaponEvent, Damage ──> impact sounds, decals, Particles (game effects)
Update:       camera <── Intent (look) + MovementState (eye offset)
              Particles stepped (frame time, at most 0.1 s) and drawn
              DriveAnimation  WeaponEvent ──> Animator (bodies), ViewAnimator (view models) ──> joints
                              view-model animation events ──> ViewModelEvent ──> flash, light, shells
                              Intent + Velocity ──> ViewModelOffset (bob, sway)
```

Intent buttons are levels, not edges. Movement runs at a fixed tick
(`DEFAULT_TICK_HZ` until a spec sets one); rendering interpolates translation.

## Slots

An implementation registers under a namespaced ID (`mashup:noclip`,
`combat_arms:movement`) with a marker component. Its systems act only on
entities with that marker, so swapping is removing one marker and inserting
another (`slots::set_movement`). Only Movement exists so far; the other slots
follow the same pattern when they arrive.
