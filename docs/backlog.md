# Backlog

Things to build, **in priority order** (top first; reprioritized
2026-10-05). Shortcuts already in the code live in
[tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a
plan when work starts; delete them when done.

## 1. In-game console and debug overlays

Unlocks fast iteration on everything else (movement cvars, toggles,
diagnostics without restarting).

- **In-game console** (toggle with `~`, Source-style), feature rich:
  - Commands and cvars from every system that registers them (movement
    cvars like `sv_enablebunnyhopping`, `sv_airaccelerate`, `sv_gravity`;
    `map`, `noclip`, `god`, `give`, `kill`, `setpos`/`getpos`, `exec`,
    `bind`, `alias`, `echo`, `clear`, `quit`), each with help text, type,
    default, min/max and "changed from default" marking.
  - Tab completion of names and values (enum values, map names from the
    install, file paths for `exec`), cycling with repeated Tab, and
    inline suggestions as you type, ranked fuzzy matches.
  - History (up/down, Ctrl+R reverse search), persisted across sessions;
    `;`-separated commands; quoted arguments; `+`/`-` actions for binds.
  - `find <text>` over names and help; `differences` lists cvars changed
    from defaults; `reset <cvar>`/`resetall`; `cvarlist <prefix>`.
  - Config files: autoexec.cfg at startup, `host_writeconfig` to save
    binds and changed cvars; `exec` reads CS:S-style .cfg (surf/bhop/kz
    configs work as-is, unknown cvars reported, not fatal).
  - Output: colored by severity, log lines (warnings/errors from `log`)
    mirrored in, scrollback with page up/down, selectable and copyable,
    a filter box; timestamps on demand.
  - Live values: `watch <cvar|expr>` overlays a value on the HUD (speed,
    position, velocity, water level, ladder state); `toggle`, `incrementvar`
    for binds; `wait` for scripted sequences.
  - Remote: the same commands over the dev remote protocol, so scripts
    and tests can drive it.
- **Debug overlays, CS:S-style**, as console cvars:
  - `cl_showpos 1`: map name, position, angles and velocity in the top
    corner, as CS:S draws it.
  - `cl_showfps 1` / `net_graph`-like FPS counter (frame time, min/max).
  - Sound emitter markers: where each sound was emitted, with the entry
    name (and wave) drawn at that spot for a few seconds after it plays,
    fading out; filters by channel/name. Source has `snd_show`/
    `snd_visualize` as a reference for the idea.
  - Other Source debug text worth copying as we go (`developer 1`
    notify lines, `cl_showpos 2`).

## 2. Weapons

The MVP's biggest gap. specs/cs_source/weapons.md has the generic rules
from the SDK and all 29 CS:S weapons' script values; CS:S's own rules
need measuring first (its M1–M17, extending tools/css_probe).

- **Weapon slot** from the MVP plan (trigger, cost, delivery, effect;
  hitscan, health, crosshair, ammo HUD).
- **Knife** first (the movement speed the movement already assumes), then
  one rifle (AK-47): fire timing, spread/inaccuracy, recoil/punch, damage
  by hitgroup and range, penetration, reload and deploy times, from
  measurements.
- View models and world models from the install; weapon sounds through
  the sound pipeline.
- Physics props: bullet and explosion impulses (spec physics_props 5).
- **Bots** (MVP): intent from a brain on a waypoint graph; deathmatch with
  respawns.

## 3. Sound, remaining

docs/plans/active/sound.md.

- Physics impacts and scrapes (avian contact speeds; surfaceprops'
  impact/scrape entries).
- Map ambience: `ambient_generic`, soundscapes (selection by nearest
  visible env_soundscape, 3 s crossfade, random sounds).
- Measure on the probe server: the distance curves (replace the H1/H2
  guesses), CS:S footstep silence rules, the jump sound, wave choice.

## 4. Physics props, remaining

- The model's `prop_data` physicsmode override (wins over the map's).
- The player physics shadow for `prop_physics` (dust2 has none).
- Impact damage, breakable props.

## 5. Visual fidelity

- **Reflective floors / env maps**: the lower tunnels' floor (and other
  `$envmap` materials, incl. map-patched ones pointing at `env_cubemap`):
  read the BSP's cubemap lump and baked cubemaps, pick the nearest per
  surface, apply `$envmaptint`, `$envmapmask` / base or normal-map alpha
  masks and fresnel as the shader spec describes; compare in refcmp.
- **Tunnel lamp glows**: iterate on the brightness of the billboard
  glows (env_sprite) in dust2's tunnels against CS:S (refcmp
  `glow_lamp`, `glow_lamp_down`; RenderDoc a lamp draw for the sprite
  shader's colour, alpha and scale).
- **Water surfaces**: swimming works (`MapWater`), but water faces draw as
  plain textured surfaces, without the Water shader's refraction,
  reflection or fog. Water currents (base velocity) aren't applied.
- **HDR parity**: CS:S defaults to mat_hdr_level 2 on dust2 (HDR lightmaps,
  tonemapping, bloom); the reference install runs LDR. Compare and match
  both if players use HDR. Tonemap (`env_tonemap_controller`).
- **More refcmp views** across dust2 (mid, long, B, spawns) and other maps.
- Fog on ropes; detail blend modes other than 0 and 1;
  `$basetexturetransform` (unused on dust2).

## 6. Long tail

Counts are from de_dust2's entity lump and static prop lump.

- **Remaining decals**: 6 of dust2's 135 sit on props or brush entities
  rather than world faces; decals on displacements (none on dust2).
- **Verify inferred Source rules** with the comparison tool, using a local
  copy of a map with test entities added where dust2 has no example: floor
  and ceiling decal orientation, decal reach, overall brightness/tonemapping.
- **Brush entities**: doors and visible `func_brush` if a map needs them
  (dust2's one `func_brush` is render mode 10, never drawn).
- **Fire** (`env_fire`, 16) and other effects, if they show in normal play.
- **Lightmap styles** (switchable lights).
- **Baked per-vertex prop lighting (`.vhv`)** for maps that ship it (dust2
  doesn't; its props use the per-prop light probe, as in the game).
