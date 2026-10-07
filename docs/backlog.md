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
  src/logic/breakables.rs), model doors (`prop_door_rotating`) and
  prop damage/outputs, skins, body groups and sequences
  (src/logic/props.rs), sprites, dust and switchable lights
  (src/logic/visuals.rs, lightmap styles relit at run time), env_global,
  trigger_soundscape through the touch code. Prop damage follows
  specs/source/prop_damage.md (src/logic/prop_damage.rs: prop data,
  pieces, impact damage, explosive props, client props, Wake/Sleep/motion
  inputs, OnAwakened/OnMotionEnabled, pressure). Left: breakable follow-ups
  (section 7), prop `OnAnimationBegun`/`OnAnimationDone`, train
  facing/banking and player train control. (Round restarts re-create
  entities: rounds plan.) Target: two
  real minigame maps from the user's downloads.

## 2b. HUD and debug views

- CS:S HUD: health/armour/ammo/money/round-timer panels, death notices
  and the Tab scoreboard are in (`client/game_hud.rs`,
  `client/scoreboard.rs`), and the weapon selection
  (`client/weapon_select.rs`, kill icons standing in for the scripts'
  selection icons). The team menu is in (M; you start as CT). The radio
  is in (`client/radio.rs`: Z/X/C menus, the calls as console commands,
  "Fire in the hole!" on throws, bots' enemy spotted/down and need
  backup), with a chat area and hint text (`client/chat.rs`) and a
  scoreboard latency column (0 until networking) and its BOMB / DEFUSER
  markers, the radio icon over a teammate's head (`sprites/radio`), and
  text chat (Y / U, `say`, `say_team`). The buy and team menus draw in the game's VGUI look from its `.res` files
  (`client/vgui.rs`). Left: the class menu (`classmenu_*.res`, needs player
  models per class), spectating from the team menu (no spectator team),
  autobuy / rebuy / favourites, checking the widescreen placement against
  the game, `ignorerad`, bots answering radio commands ("Roger that", following "Follow me"),
  other game messages in the chat (team joins; bomb pickups and drops
  are in). The radar is in (`client/radar.rs`: the map overview turning
  with you, team dots, your place name); its range (2200 units) is a guess.
- The game menu (Esc; `client/game_menu.rs`: new game with map, mode,
  bots per team and difficulty; bots; team; options; bug report; quit)
  is in, in its own look. Left: the game's own GameUI look
  (`SourceScheme.res` frames, its fonts on Linux, where Tahoma is
  missing), more options (binds, crosshair size, video), a scrollbar for
  long map lists (pages of 42 now), map thumbnails.
- Debug overlays: `mashup_drawhitboxes`, `mashup_healthbars`,
  `mashup_drawnav`, `mashup_drawbots` exist; add more as features need
  them (sound radii, triggers).

## 3. Weapons, remaining

In progress: [plans/active/weapons.md](plans/active/weapons.md). The
framework, the knife, every CS:S gun (zoom, silencers, bursts, pellets,
shell-by-shell reloads, the dual Elites), grenades, HUD, deathmatch and
a first bot are in.

- Probe the new guns' unmeasured rules (listed in the weapons plan):
  recoil of most guns, the shotgun reload and pellets, the other ammo
  types' penetration. Ammo prices and box sizes are the well-known
  values, not measured (docs/tech-debt.md); measure them, `primammo`'s
  fill-up and the spawn pistols' reserves on the probe.
- The AWP's view model is in (MDL v48 reads like v44,
  specs/cs_source/mdl_v48.md); compare its fire and reload against the
  game. (MDL 45–48, sections, `.ani` blocks and the zero-frame cache
  are read: `anim.rs`.)
- Zoom: measure `zoom_sensitivity_ratio` and compare the scope overlay
  (the game's textures, laid out by eye) with CS:S.
- Rounds, money and buying: slice 1 done (`mashup_rounds 1`,
  [plans/active/rounds.md](plans/active/rounds.md)); ammo buying is in.
- Objectives, remaining (bomb and hostages are in: `src/objectives/`,
  specs/cs_source/objectives.md): measure the spec's open questions on the
  probe (Q1 movement while arming, Q3 blast shape/walls/armour, Q4 beep
  schedule and loudness, Q5 money, Q6 carrier choice, Q7 the bomb's
  solidity, Q9 hostage models, Q10 follow speeds, Q11 rescue rules, Q14
  tick rounding); the C4's screen text (7355608) and LED glow sprite, the
  `sprites/c4` marker through walls, screen shake, the explosion's own
  effect (it uses the HE's); the scoreboard's own bomb/kit icons (ours
  are words); hostage
  animation beyond idle/walk/run and a nod (`hostage_anim.rs`: compare
  with CS:S's hostages, which aren't measured; head/aim pose parameters
  toward the leader, flinch and cower), hostages avoiding
  "no hostages" nav areas, crouching and jumping; drop/pickup game events
  and server log lines; bots leading hostages, buying kits, guarding;
  which `Player.UseDeny` CS:S plays (doors_buttons.md Q2).
- Grenades, remaining (HE, flashbang and smoke are in: `weapon/grenade.rs`,
  `games/cs_source/grenades.rs`): measure the spec's open questions on the
  probe (fuse ticks, release timing, flash amounts and overlay curve, HE vs
  armour, smoke vs bots, HE shake and hearing) and replace the fits
  (tech-debt); bots throwing grenades, the `_thrown` models.
- Death animations on bodies (the grenade gesture is in; the reload gesture is in:
  `<Move>_Reload_<weapon>` by activity, an assumption for the spec's
  `reload_<suffix>`, with the shotguns' `_start/_loop/_end`).
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
  glass) against the game; tracers are in (every 4th bullet, spec
  tracers.md; check N and the look in play); explosion impulses.
- Dropping weapons, remaining (`drop`/G, pickup and death drops are in,
  `weapon/drop.rs`): throw speed, the re-pick delay, pickup reach and
  mass, friction (0.5) and the box collider are guesses (measure on
  the probe server: how far a shot weapon skids); bullets hit loose
  weapons; the use key doesn't swap a weapon for the one you look at.
- Ragdolls, remaining (`map/ragdoll.rs` has the spec's bodies, joints,
  self-collision pairs, death poses, impulse, bone velocities, bullet
  pushes, separation repair, settling and removal): blast pushes (send
  `RagdollShot { blast: true }` from grenades once they exist); check the
  spec's open questions (force scale, bone_dt, death-pose facing and
  crouch choice, joint limit convention) in the game.
- Held weapons don't stay in other players' hands (they float around the
  hands). Findings 2026-10-06: the world model's mesh follows the player's
  animated `weapon_bone` (child of the spine), which is right; the arms
  only follow their own animation. At idle the hands already sit on the
  AK (checked with `+thirdperson +cam_idealyaw 140`). A two-bone IK onto
  `weapon_bone_RHand`/`_LHand` was tried and dropped: `_LHand` is a child
  of `_RHand` and pulled the left hand onto the grip, so those bones
  aren't plain hand targets. Needs a spec of Source's IK chains, the
  sequences' IK rules and locks (the animation spec's open question "IK"),
  and a reproduction of the floating. With `mashup_freecam 2` the AK
  sits in both hands standing, crouched and firing (2026-10-06); still to
  check: jumping and other weapons; a running bot (`mashup_watch 1`) holds
  its AK with both hands too.

## 4. Bots

- Ladders on other maps (`tests/bot_nav.rs::ladders_climb_both_ways`,
  ignored; de_nuke's all pass, and 81 of 128 climbs on cs_office,
  de_train, de_port, de_cbble, cs_militia, cs_assault, de_piranesi and
  de_prodigy, up from 62): de_train's ladders on the train cars start
  about 1.5 m above the floor and bots circle or pace under them without
  getting on (33 of 70 climbs fail there); the rest (cs_militia 0 down,
  2-3 up; de_cbble 0/4/5 down; cs_assault 0/5 down; de_prodigy 0/5 down;
  de_port 0 down; cs_office 1 up; de_piranesi 0) mostly get stuck on
  boxes or ledges beside the ladder's top, or the test's start spot is
  awkward. Look at each with `MASHUP_BOT_CASE="ladder 3 up"` and the
  trace.
- A CS:S bot behaviour spec (nav spec open questions 2-4) to check our
  team play against: path costs, how bots pick sites, hold and rotate,
  what they say. Ours (`bot::tactics`) plants and defuses only through
  `bot::objectives`' simple hooks (carrier to the nearest target,
  defenders straight to a planted bomb: no guarding, covering a defuse,
  or retaking), leads no hostages, buys no kits, has no buy strategy beyond autobuy, no sniper spots
  (the spot flags are loaded), no crouching at hold spots, no lurkers or
  split attacks, and only uses approach data it computes (v9 files'
  approach records are skipped). Bots don't step around each other
  beyond pushing apart (a bot short of a taken hold spot holds where it
  is). Balance (15 rounds 5v5, after the staging/rotation/retake pass):
  terrorists won 10/15 on de_dust2 and 11/15 on de_nuke, counter-
  terrorists (the attackers there) 9/15 on cs_office: now leaning to
  the attackers on bomb maps; next: defenders falling back to retake
  instead of dying one by one on a lost site, and a measured reaction
  time.
- Grenades, beyond the first pass: lineups from the nav mesh's hiding and
  approach spots (smokes cutting sight lines rather than landing on the
  objective point), flashes thrown around corners so they pop out of the
  thrower's view without turning, not flashing teammates, running and
  jump throws (carried velocity in the plan), "Fire in the hole" radio.
- Movement: gap jumps, avoiding teammates in doorways, the door into A
  from Inside on de_nuke (bots now and then stall at it).

## 5. Console, remaining

The console and overlays are in (src/console.rs, src/client/console.rs).

- `net_graph` shows local numbers only (fps, frame time, tick rate,
  entities); ping and traffic once there is networking. `cl_showpos 2`.
- Mouse selection copies whole lines (drag over the output); selecting
  part of a line isn't possible.

## 6. Sound, remaining

docs/plans/active/sound.md.

- Scrapes (looping friction sounds): needs a stand-in for Source's
  friction energy (spec open question 8); breakables' spec pitch/volume
  rules (we play the entries as scripted) and gib bounce sounds.
- Soundscape DSP presets (room reverb), env_soundscape visibility
  checks; soundscape loops onto `map::live_sound` (intro, live panning).
- Stock ambient_generics the bomb starts (de_nuke's alarm, dust2's
  fires) now play when it explodes (`BombExplode`); check them by ear. (Prop outputs are in: de_nuke's steam, cs_office's
  projector.) de_nuke's env_steam jets themselves aren't drawn.
- Measure ambient_generic's level for script entries vs raw waves (spec
  open question 11).
- Measure on the probe server: the distance curves (replace the H1/H2
  guesses), CS:S footstep silence rules, the jump sound, wave choice.

## 7. Physics props, remaining

- The player physics shadow for `prop_physics` (dust2 has none).
- Prop damage follow-ups (docs/tech-debt.md "Prop damage"): stress crush, the
  velocity restore after an impact breaks a prop, pieces as real avian
  bodies with their skin, the spec's open questions on the probe server
  (Q1-Q14: damage types, gas-can ignition, player impact rules, client
  break sounds, round restarts of client props).
- Model doors: the hardware's latch/lock sounds and the spec's open
  question 8 (which entries the hardware and surface pick; we use the
  model's `door_options` move/open/close), swing-side checks against the
  world (only players are checked), forceclosed pushing physics props.
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

- **Water surfaces** (specs/cs_source/water.md, `map::water`): refraction,
  planar reflection, the cheap cubemap pass, under-water fog (world,
  props, ropes, decals, particles), bottom materials, the
  `$underwateroverlay` screen warp, the intersection view's height fog
  and the water cvars are in. Left: a Refract shader spec (the warp's
  strength and blur are guesses: de_port's `water_warp01` looks subtle),
  `$blurrefract`/`$refracttint` and the `$basetexture` variant, and a
  refcmp comparison of de_port/de_chateau water (no reference captures
  yet; it would settle de_port's dark speckles: refraction lookups at
  `$refractamount` 5 landing on barely submerged shore, which the spec's
  shore fade leaves unreflected). Water currents: no stock map has
  current contents (spec movement.md open question 15), so not applied.
- **HDR parity**: CS:S defaults to mat_hdr_level 2 on dust2 (HDR lightmaps,
  tonemapping, bloom); the reference install runs LDR. Compare and match
  both if players use HDR. Tonemap (`env_tonemap_controller`).
- **More refcmp views** across dust2 (mid, long, B, spawns) and other maps.
- Detail blend modes other than 0 and 1;
  `$basetexturetransform` (unused on dust2).

## 9. Performance

Measured and culled (docs/performance.md): `mashup_perf`, `refcmp bench`,
PVS culling by world chunk, prop fade distances. Left:
- Areaportals (closed doors and windows hide what's behind them) and
  `func_occluder`.
- Prop fade bands (alpha between `fademindist` and `fademaxdist`) and
  LOD models.
- Measure on the Windows PC (`refcmp bench` there) and set a budget.
- Frame-time follow-ups (performance.md, "Cheap wins found"): take
  before/after numbers on a quiet machine; props as hierarchies of their
  own (cheaper collider propagation) without changing how physics props
  settle; skip posing bodies nobody sees (hidden local body, culled bots)
  if hitboxes and muzzles don't read the joints.

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
- **Fire** (specs/source/fire.md) is in: env_fire (heat, growth, burn damage box with line of sight, fuel, Extinguish, outputs, re-created unlit each round, Q4), env_firesource/env_firesensor, entity flames (Ignite inputs, burning props, the gas can), the `env_fire_large_smoke` and `burning_character` looks (docs/tech-debt.md "Fire"). Next: compare a lit de_dust2 fire with CS:S's (Q1, Q10 operator meanings), and the probe-server questions: burn vs armour, the kill icon and score (Q2), player ignition (Q3), the radius damage of a burning can (Q6), burning-prop light (Q7), buried fires (Q9), floors on de_dust/de_train (Q5).
- **Lightmap styles**: animated styles (1-31) at their pattern (they're
  baked at normal brightness); switchable ones (32+) switch.
- **Baked per-vertex prop lighting (`.vhv`)** for maps that ship it (dust2
  doesn't; its props use the per-prop light probe, as in the game).
