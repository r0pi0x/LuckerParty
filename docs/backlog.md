# Backlog

Things to build, **in priority order** (top first; reprioritized
2026-10-05). Shortcuts already in the code live in
[tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a
plan when work starts; delete them when done.

## 1. Console, remaining

The console and overlays are in (src/console.rs, src/client/console.rs).
Left for later:

- `god`, `impulse`, `buy`; `net_graph`
  beyond cl_showfps 2; `cl_showpos 2`; `developer 1` notify lines on the
  HUD.
- Select-and-copy in the output (clipboard); `con_dump` writes it to a
  file meanwhile.
- `map` without restarting (maps load at startup; `map` relaunches).

## 2. Weapons

In progress: [plans/active/weapons.md](plans/active/weapons.md). The
framework, knife, AK-47, HUD, deathmatch and a first bot are in.

- Merge the CS:S measurements (M1–M17): inaccuracy, recoil, hitgroups,
  falloff, armour, penetration, knife.
- Bot behaviour: awareness (sound, memory), CS:S bot path costs and
  route variety (nav spec open questions 2–4).
- Hitboxes from player models; character, view and world models.
- Impact decals, tracers, muzzle flash; explosion impulses.

## 3. Sound, remaining

docs/plans/active/sound.md.

- Scrapes (looping friction sounds): needs a stand-in for Source's
  friction energy (spec open question 8); break sounds with breakables.
- `ambient_generic` (with entity inputs once maps need them), soundscape
  DSP presets (room reverb), env_soundscape visibility checks.
- Measure on the probe server: the distance curves (replace the H1/H2
  guesses), CS:S footstep silence rules, the jump sound, wave choice.

## 4. Physics props, remaining

- The player physics shadow for `prop_physics` (dust2 has none).
- Impact damage, breakable props.

## 5. Visual fidelity

- **Env maps on props**: unpatched `env_cubemap` materials (models) take
  the nearest cubemap at run time (BSP cubemap lump); the model shader's
  envmap (gamma-converted tint, no fresnel). World reflections are in
  (refcmp `tunnel_floor_*`).
- **Tunnel lamp glows**: now close to CS:S (refcmp `glow_lamp`: 99/83/65
  vs 108/94/79 below the centre lamp; the rest is the dark-area gap
  below). Open: why mode-5 halos don't show in CS:S (sprites spec open
  question 3); the glow just above the centre lamp is a little strong.
- **Dark areas are 15-20% darker than CS:S** (dust2 tunnels; refcmp `fit`:
  CS:S = k·albedo·light with k 1.15-1.40 in tunnels, 1.00 outdoors). Not
  light styles (none on dust2), colour correction (no entities),
  mat_monitorgamma (2.2) or the sRGB/2.2 lightmap round trip (modelled).
  Next: RenderDoc a tunnel wall draw and compare lightmap texels.
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
