# Backlog

Things to build, **in priority order** (top first; reprioritized
2026-10-06). Shortcuts already in the code live in
[tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a
plan when work starts; delete them when done.

## 1. Playtest essentials

A (settings, health bars, third person) and C (view models) are done. B, D and E were paused on 2026-10-06 to free the machine;
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
- **E. Other maps.** In progress:
  [plans/active/other-maps.md](plans/active/other-maps.md) (catalog).
  Fixed: aztec's walls, props' ambient light on older maps, brush
  entities, additive glows, HDR skies, start-on switchable lights, murky
  water, decals on terrain. Left: reference views for aztec, office and
  nuke (needs the shared game), unplaced decals, a real Water shader.

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
- Impact effects: the dust/smoke puff, debris and sparks where bullets
  hit, by surface material (spec in progress: specs/cs_source/
  impact_effects.md). The muzzle flash's light on nearby walls is part of
  the view-model work (1C follow-up).
- Decals, remaining: on characters (blood);
  check the knife's mark (`ManhackCut` is a guess) and lit decals (wood,
  glass) against the game; tracers; explosion impulses.
- dust2's woven basket physics props (`props_junk`/`wicker` style pots):
  parts of the lid and rim don't draw (seen from above, faces missing).
  Suspect the model converter's winding fix-up (it winds triangles
  against the vertex normals) or back-face culling of a two-sided part.
- Dropping weapons (`drop`, CS:S's G key): the world model falls as a
  physics object, can be picked up by walking over it; dead players drop
  theirs.
- Ragdolls on death: the player model's ragdoll from its `.phy` (bones as
  rigid bodies with joint limits), seeded with the death pose and the
  killing hit's impulse.
- Held weapons don't stay in other players' hands (they float around the
  hands): check the bone merge onto `weapon_bone` and the hand bones, and
  the IK hand locks the animation spec leaves out (open question "IK").

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
- Breakable glass as in cs_office (`func_breakable_surf`): windows that
  take a hole per bullet, crack around it, shatter in pieces when hit
  hard or damaged enough, and let bullets and players through once
  broken; the glass-break decal, shard and grit effects (impact effects
  spec covers the shards) and break sounds. Needs a spec (public SDK) and
  brush entities, which the world loader skips today.

## 8. Visual fidelity

- **Water surfaces**: swimming works (`MapWater`), but water faces draw as
  plain textured surfaces (or, without a base texture, their opaque fog
  colour with cubemap reflections), without the Water shader's
  refraction, reflection or fog: clear water looks murky. Water currents
  (base velocity) aren't applied.
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
  rather than world faces. Other maps: assault 45, nuke and train 21
  each (not on brush entities; see plans/active/other-maps.md).
- **Verify inferred Source rules** with the comparison tool, using a local
  copy of a map with test entities added where dust2 has no example: floor
  and ceiling decal orientation, decal reach, overall brightness/tonemapping.
- **Brush entities**: they draw and collide where they spawn; doors
  don't open, breakables don't break, func_rotating doesn't turn, render
  modes other than normal and 10 (translucent func_brush) aren't applied.
- **Fire** (`env_fire`, 16) and other effects, if they show in normal play.
- **Lightmap styles**: switching lights and animated styles (lights lit at
  map start are baked in).
- **Baked per-vertex prop lighting (`.vhv`)** for maps that ship it (dust2
  doesn't; its props use the per-prop light probe, as in the game).
