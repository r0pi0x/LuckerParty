# Backlog

Things to build, **in priority order** (top first; reprioritized
2026-10-06). Shortcuts already in the code live in
[tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a
plan when work starts; delete them when done.

## 1. Playtest essentials

A (settings, health bars, third person) is done. C is in progress in an
agent worktree. B, D and E were paused on 2026-10-06 to free the machine;
their partial work is in uncommitted agent worktrees under
`.claude/worktrees/` (B: agent-abc7f16e740c6aafe, D: agent-a052e118eeb8d4c8d).

- **B. Weapons: penetration and collaterals.** Implement M13 from
  specs/cs_source/weapons.md (walls by material, through players x0.5,
  object counts, compounding falloff); the measured recoil sets (moving and
  airborne AK-47).
- **C. First-person view models**: done (docs/plans/active/view-models.md);
  remaining: bob/sway, muzzle flash, `cl_righthand`.
- **D. Movement: terrain and falls.** Fuzz walking over displacements to
  find the rare "stubbed toe" stop and fix it; fall damage (measure on the
  probe server); `trigger_hurt` volumes.
- **E. Other maps.** Load de_aztec, de_nuke (then the rest of the stock
  maps), screenshot their main areas, catalog visual bugs and console
  errors (de_aztec logs missing textures/materials), and fix them; add
  refcmp views where the reference game shows a difference.

## 2. Custom maps and minigames

Plan: [plans/active/custom-maps.md](plans/active/custom-maps.md).

- A test map with every supported entity (`mashup_logic_test`, generated
  `.vmf`, compiled with Valve's tools; plan section "Test map").
- Find custom maps: mount the install's `download/` and `custom/` maps and
  a mashup cache (never write the Steam folder); `map <name>`, `maps`,
  `import <file.bsp|.bsp.bz2>` with a content hash.
- Entity I/O and triggers (spec first): outputs/inputs with delays,
  `logic_*`, `math_counter`, `trigger_teleport`/`push`/`hurt`/`multiple`/
  `once`; then moving brush entities (doors, buttons, platforms,
  breakables) with movement on moving solids. Target: two real minigame
  maps from the user's downloads.

## 3. Weapons, remaining

In progress: [plans/active/weapons.md](plans/active/weapons.md). The
framework, knife, AK-47, HUD, deathmatch and a first bot are in.

- The other CS:S weapons from the script tables (M4A1, USP, Glock,
  Deagle, AWP first: their recoil and inaccuracy are measured; zoom M15,
  burst/silencer M16 are measured too).
- Money and a buy menu (`buy` gives for free).
- Reload, grenade and death animations (world models are held by bodies).
- Impact decals, tracers, muzzle flash; explosion impulses.

## 4. Bots

- CS:S bot path costs and route variety (nav spec open questions 2–4),
  checking corners, teamwork.

## 5. Console, remaining

The console and overlays are in (src/console.rs, src/client/console.rs).

- `net_graph` beyond cl_showfps 2; `cl_showpos 2`.
- Select-and-copy in the output (clipboard); `con_dump` writes it to a
  file meanwhile.

## 6. Sound, remaining

docs/plans/active/sound.md.

- Scrapes (looping friction sounds): needs a stand-in for Source's
  friction energy (spec open question 8); break sounds with breakables.
- `ambient_generic` (with entity inputs once maps need them), soundscape
  DSP presets (room reverb), env_soundscape visibility checks.
- Measure on the probe server: the distance curves (replace the H1/H2
  guesses), CS:S footstep silence rules, the jump sound, wave choice.

## 7. Physics props, remaining

- The player physics shadow for `prop_physics` (dust2 has none).
- Impact damage, breakable props.

## 8. Visual fidelity

- **Water surfaces**: swimming works (`MapWater`), but water faces draw as
  plain textured surfaces, without the Water shader's refraction,
  reflection or fog. Water currents (base velocity) aren't applied.
- **HDR parity**: CS:S defaults to mat_hdr_level 2 on dust2 (HDR lightmaps,
  tonemapping, bloom); the reference install runs LDR. Compare and match
  both if players use HDR. Tonemap (`env_tonemap_controller`).
- **More refcmp views** across dust2 (mid, long, B, spawns) and other maps.
- Fog on ropes; detail blend modes other than 0 and 1;
  `$basetexturetransform` (unused on dust2).

## 9. Performance

- Visibility culling for maps: use the BSP's own visibility data (PVS from
  the vis lump, leaves and clusters) to skip world faces, props and
  entities the camera's leaf can't see; areaportals; frustum culling per
  leaf group instead of per material mesh (today world meshes are merged
  per material, so Bevy's frustum culling rarely skips anything).
- Prop fade distances (`fademindist`/`fademaxdist`) and LOD models.
- Profile frame time on dust2 (CPU systems, draw calls) and set a budget;
  measure on the Windows PC too.

## 10. Long tail

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
