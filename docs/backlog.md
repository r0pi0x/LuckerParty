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
- Entity I/O, triggers and moving brushes are in (`src/logic`, slices 3
  and 4 of the plan), and breakables (func_breakable, func_breakable_surf;
  src/logic/breakables.rs). Left: breakable follow-ups (section 7),
  `prop_door_rotating` (model doors: cs_assault, de_port), train facing/banking and
  player train control, `trigger_soundscape` through the general touch
  code, round restarts re-creating entities, env_global. Target: two
  real minigame maps from the user's downloads.

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
framework, knife, AK-47, M4A1, AWP, USP, Glock, Deagle (zoom, silencers,
burst), HUD, deathmatch and a first bot are in.

- The other CS:S guns from the spec's script tables, as `Gun` rows in
  `games/cs_source/weapons.rs`: rifles (aug, famas with its burst, galil,
  sg552 and aug scopes), snipers (scout, sg550, g3sg1), SMGs, m249,
  p228/fiveseven/elite; their recoil isn't measured (probe M3 first), nor
  the 556MM/9MM/57MM/357SIG penetration. Shotguns (m3, xm1014) need
  pellets plus the shell-by-shell reload (M9).
- The AWP's view model: `v_snip_awp.mdl` is MDL version 48 and `anim.rs`
  reads 44 (spec the v48 header differences first); then its reload
  sounds from the model events.
- Zoom sensitivity (`zoom_sensitivity_ratio`), CS:S's own scope overlay
  texture, the silenced world models (`w_*_silencer.mdl`).
- Money and a buy menu (`buy` gives for free).
- Reload, grenade and death animations (world models are held by bodies).
- Impact effects, remaining (specs/cs_source/impact_effects.md; the
  surface effects, blood, bullet splashes and pane glass shards are in):
  section 9's exact shard burst at the hit point (ours spreads shards
  over each shattered pane), slime splashes
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
- Dropping weapons, remaining (`drop`/G, pickup and death drops are in,
  `weapon/drop.rs`): throw speed, the re-pick delay, pickup reach and
  mass are guesses (measure on the probe server); bullets hit loose
  weapons; the use key doesn't swap a weapon for the one you look at.
- Ragdolls on death: the player model's ragdoll from its `.phy` (bones as
  rigid bodies with joint limits), seeded with the death pose and the
  killing hit's impulse.
- Held weapons don't stay in other players' hands (they float around the
  hands). Findings 2026-10-06: the world model's mesh follows the player's
  animated `weapon_bone` (child of the spine), which is right; the arms
  only follow their own animation. At idle the hands already sit on the
  AK (checked with `+thirdperson +cam_idealyaw 140`). A two-bone IK onto
  `weapon_bone_RHand`/`_LHand` was tried and dropped: `_LHand` is a child
  of `_RHand` and pulled the left hand onto the grip, so those bones
  aren't plain hand targets. Needs a spec of Source's IK chains, the
  sequences' IK rules and locks (the animation spec's open question "IK"),
  and a reproduction of the floating (which pose: running, crouching,
  firing, bots' pitch).

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
  friction energy (spec open question 8); breakables' spec pitch/volume
  rules (we play the entries as scripted) and gib bounce sounds.
- `ambient_generic` (with entity inputs once maps need them), soundscape
  DSP presets (room reverb), env_soundscape visibility checks.
- Measure on the probe server: the distance curves (replace the H1/H2
  guesses), CS:S footstep silence rules, the jump sound, wave choice.

## 7. Physics props, remaining

- The player physics shadow for `prop_physics` (dust2 has none).
- Impact damage, breakable props.
- `prop_door_rotating` (model doors) with the use key; brush doors
  (`func_door`, `func_door_rotating`) and `+use` are done (src/logic).
- Breakables, remaining (vents and windows break: src/logic/breakables.rs,
  tests/map_breakables.rs): the cracked look of a broken window's panes
  (`$crackmaterial`, jagged edge pieces; spec open question 8), the
  falling pane pieces (`models/brokenglass_piece.mdl`; collapsing panes
  just shatter now), the GlassBreak/BulletProof decals, break-on-pressure
  (flag 4), physics impact damage to breakables, explosions on break, the
  window's flip to the attacked side, propdata templates, the spec's open
  questions on the probe server (bullet/knife damage types, broken brush
  visibility, shots after a window breaks). Needs a spec (public SDK) and
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
- **Brush entities**: movers (doors, buttons, func_rotating, trains,
  func_brush) move through the logic layer, breakables break; render modes other than normal and 10 (translucent func_brush) aren't
  applied.
- **Fire** (`env_fire`, 16) and other effects, if they show in normal play.
- **Lightmap styles**: switching lights and animated styles (lights lit at
  map start are baked in).
- **Baked per-vertex prop lighting (`.vhv`)** for maps that ship it (dust2
  doesn't; its props use the per-prop light probe, as in the game).
