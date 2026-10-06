# Backlog

Things to build, **in priority order** (top first; reprioritized
2026-10-06). Shortcuts already in the code live in
[tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a
plan when work starts; delete them when done.

## 1. Playtest essentials (in progress: parallel agents, 2026-10-06)

Each group is one agent's worktree; merged into main as they finish.

- **A. Client: settings and debug views.** `volume` and `sensitivity`
  cvars (archived; CS:S's m_yaw/m_pitch 0.022 scale); debug health bars
  above characters (`mashup_healthbars 1`); third-person camera
  (`thirdperson` / `firstperson`) showing the local player's animated
  body.
- **B. Weapons: penetration and collaterals.** Implement M13 from
  specs/cs_source/weapons.md (walls by material, through players x0.5,
  object counts, compounding falloff); the measured recoil sets (moving and
  airborne AK-47).
- **C. First-person view models** (`v_knife_*.mdl`, `v_rif_ak47.mdl`) with
  draw/idle/fire/reload sequences, through the animation decoder.

## 2. Weapons, remaining

In progress: [plans/active/weapons.md](plans/active/weapons.md). The
framework, knife, AK-47, HUD, deathmatch and a first bot are in.

- The other CS:S weapons from the script tables (M4A1, USP, Glock,
  Deagle, AWP first: their recoil and inaccuracy are measured; zoom M15,
  burst/silencer M16 are measured too).
- Money and a buy menu (`buy` gives for free).
- Reload, grenade and death animations (world models are held by bodies).
- Impact decals, tracers, muzzle flash; explosion impulses.

## 3. Bots

- CS:S bot path costs and route variety (nav spec open questions 2–4),
  checking corners, teamwork.

## 4. Console, remaining

The console and overlays are in (src/console.rs, src/client/console.rs).

- `net_graph` beyond cl_showfps 2; `cl_showpos 2`.
- Select-and-copy in the output (clipboard); `con_dump` writes it to a
  file meanwhile.

## 5. Sound, remaining

docs/plans/active/sound.md.

- Scrapes (looping friction sounds): needs a stand-in for Source's
  friction energy (spec open question 8); break sounds with breakables.
- `ambient_generic` (with entity inputs once maps need them), soundscape
  DSP presets (room reverb), env_soundscape visibility checks.
- Measure on the probe server: the distance curves (replace the H1/H2
  guesses), CS:S footstep silence rules, the jump sound, wave choice.

## 6. Physics props, remaining

- The player physics shadow for `prop_physics` (dust2 has none).
- Impact damage, breakable props.

## 7. Visual fidelity

- **Water surfaces**: swimming works (`MapWater`), but water faces draw as
  plain textured surfaces, without the Water shader's refraction,
  reflection or fog. Water currents (base velocity) aren't applied.
- **HDR parity**: CS:S defaults to mat_hdr_level 2 on dust2 (HDR lightmaps,
  tonemapping, bloom); the reference install runs LDR. Compare and match
  both if players use HDR. Tonemap (`env_tonemap_controller`).
- **More refcmp views** across dust2 (mid, long, B, spawns) and other maps.
- Fog on ropes; detail blend modes other than 0 and 1;
  `$basetexturetransform` (unused on dust2).

## 8. Long tail

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
