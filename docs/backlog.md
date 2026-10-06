# Backlog

Things to build, **in priority order** (top first; reprioritized
2026-10-06). Shortcuts already in the code live in
[tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a
plan when work starts; delete them when done.

## 1. Playtest essentials

A (settings, health bars, third person), B (penetration and collaterals,
docs/plans/active/weapons.md slice 5) and C (view models) are done. D and
E were paused on 2026-10-06 to free the machine; their partial work is in
agent worktrees under `.claude/worktrees/` (D: agent-a052e118eeb8d4c8d).

- **C. First-person view models**: done (docs/plans/active/view-models.md);
  remaining: check its open questions against CS:S (flash look, shell
  direction, near plane), view-model sound events, brass for other
  players.
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
- Entity I/O and triggers (spec first): outputs/inputs with delays,
  `logic_*`, `math_counter`, `trigger_teleport`/`push`/`multiple`/
  `once`; then moving brush entities (doors, buttons, platforms,
  breakables) with movement on moving solids. Target: two real minigame
  maps from the user's downloads.

## 2b. HUD and debug views

- CS:S HUD: health/armour/ammo panels, death notices and the Tab
  scoreboard are in (`client/game_hud.rs`, `client/scoreboard.rs`). Left:
  the team menu (`jointeam 2|3` exists; you start as CT), ping on the
  scoreboard, round timer and money panels once
  rounds and money exist, weapon selection,
  hint text. The radar is in (`client/radar.rs`: the map overview turning
  with you, team dots, your place name); its range (2200 units) is a guess.
- Debug overlays: `mashup_drawhitboxes`, `mashup_healthbars`,
  `mashup_drawnav`, `mashup_drawbots` exist; add more as features need
  them (sound radii, triggers).

## 3. Weapons, remaining

In progress: [plans/active/weapons.md](plans/active/weapons.md). The
framework, knife, AK-47, HUD, deathmatch and a first bot are in.

- The other CS:S weapons from the script tables (M4A1, USP, Glock,
  Deagle, AWP first: their recoil and inaccuracy are measured; zoom M15,
  burst/silencer M16 are measured too).
- Money and a buy menu (`buy` gives for free).
- Reload, grenade and death animations (world models are held by bodies).
- Impact effects, remaining (specs/cs_source/impact_effects.md; the
  surface effects, blood and bullet splashes are in): glass shards
  (need breakable glass, `func_breakable_surf`), slime splashes
  (`.pcf` systems), the knife's water splash, the 30 % ricochet sound,
  ragdoll pushes; check the spec's open questions in the game. The muzzle
  flash's light on nearby walls is part of the view-model work (1C
  follow-up).
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
- Select-and-copy with the mouse in the output; `con_copy [lines]` copies
  to the clipboard and `con_dump` writes a file meanwhile.

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
- The "use" key (CS:S `+use`, E): trace from the eye to usable
  entities; doors on de_nuke (and elsewhere) open and close with it
  (`func_door`, `func_door_rotating`, `prop_door_rotating`: movement,
  speed, wait/return, blocking, sounds). Shares the moving-brush work
  with the minigame plan.
- Breakable vents on de_nuke (`func_breakable` with health and material
  gibs): take damage, break into gibs, open the vent.
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
