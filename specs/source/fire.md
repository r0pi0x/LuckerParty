# Source engine: fire (env_fire, entity flames, burning props) as CS:S uses it

Source basis: Valve's public Source SDK 2013 only (the current multiplayer
branch). I read the server fire entity and its heat system (the placed fire,
the fire source and fire sensor helpers), the networked fire-effect entity
and its client side (which particle system it starts), the entity flame
(server and client), the base animating entity's ignite inputs, the
breakable prop's fire rules and break-piece ignition, the gib shooter's
flaming flag, the multiplayer game rules' damage-type tables, the bubble
helper and the particle attribute numbering. CS:S's own game DLL (game
rules, round restart, player, hostage) is not in the SDK; where it could
change a result it is an Open question with an in-game check.

From the user's CS:S install (read at runtime by throwaway scripts outside
the repo) I read the entity lumps of the 18 stock maps, their world brushes
(to work out where each fire lands) and static prop lumps, the particle
manifest and the particle files `particles/fire_01.pcf` and
`particles/burning_fx.pcf`, the fire and smoke materials and their sprite
sheets, the sound scripts (`scripts/game_sounds*.txt`) and the fire wave
files' headers, `scripts/propdata.txt` and the `prop_data` /
interaction blocks of every model placed as a `prop_*` entity (252 models).
Status: draft

## Summary

Fire in CS:S is almost entirely **bomb-site decoration**: 54 `env_fire`
entities on 8 of the 18 stock maps, all unlit until the bomb target's
`BombExplode` output sends them StartFire, all infinite, all with the same
settings except their size (128, 256 or 512). A started fire drops to the
floor, then grows linearly from nothing to full "heat" over about 4 s, and
once a second burns (damage type burn) everything that can take damage
inside a box that grows with it: at full size a 256 fire hurts 7 per
second, a 512 fire 26 and a 128 fire 2. Fires never spread over the
ground; heat passes only between fire entities and from burning objects.
On screen every stock fire looks the same: one fixed particle system
(`env_fire_large_smoke`: flame sprites rising roughly 3 m, embers and an
orange-to-grey smoke column), with no light of its own; the maps add
looping fire sounds (`fire_medium`, `General.BurningObject`) and, on two maps, switchable
baked lights.

The second kind of fire is the **entity flame**: a flame attached to one
entity (a flammable prop, or anything sent the Ignite input) that hurts it
5 hp/s and its surroundings, plays `General.BurningObject`, shows the
`burning_character` particles and ends after its lifetime. The only stock
props that can catch fire are the two gas cans (cs_militia, de_inferno).

## Units and conventions

- Distances in Source units (inches), z up. Times in seconds; server tick
  dt = 0.015 s (CS:S default 66.67 tick; specs/source/entity_io.md). Thinks
  round to the nearest tick (entity_io.md): a 0.1 s think is **7 ticks =
  0.105 s**, 0.2 s is 13 ticks = 0.195 s, 0.25 s is 17 ticks = 0.255 s,
  0.5 s is 33 ticks = 0.495 s. Fire code advances its own clock by the
  nominal 0.1 s per think, so real-time growth is 0.1/0.105 of the nominal
  rate. (At 64 tick a 0.1 s think is 6 ticks = 0.09375 s.)
- "Heat" is a unitless fire-strength number; a fire burns while heat > 0.
- Damage-type bits as in specs/source/prop_damage.md: burn 8, direct 2^28,
  blast 64, bullet 2; plasma 2^24 (only for the unused plasma fire type).
- "Size" = the `firesize` keyvalue S. "Full heat" H = 64 × S / 256 = S/4.
- Keyvalue, input, output, classname, cvar, sound-entry, particle-system and
  material names are the game's data names.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| heat_per_256 | 64 | heat | full heat of a size-256 fire (H = S/4) |
| strength_ref | 64 | heat | heat that counts as "strength 1" for damage and effect scale (absolute, not per size) |
| fire_think | 0.1 | s nominal | env_fire update interval (7 ticks) |
| drop_distance | 1024 | in | StartFire looks this far down for a floor |
| damage_box | half-width max(16, (h/H) × S/2), height (h/H) × S | in | from the floor point (2.3) |
| min_damage_radius | 16 | in | smallest half-width of the damage box |
| object_box_factor | 0.5 | × | non-player victims must touch the box shrunk by this |
| go_out_penalty | 20 | heat | taken off when a fire goes out (then clamped ≤ 0) |
| fuel_go_out_fade | 1 | s | fade time when fuel runs out |
| out_box | (−8,−8,0)–(8,8,8) | in | an unlit fire's bounds |
| `fire_maxabsorb` | 50 | heat | cvar: cap on stored absorb after extinguishing |
| `fire_absorbrate` | 3 | × | cvar: absorb cost multiplier |
| `fire_extscale` | 12 | heat | cvar: extinguish strength (unused in CS:S) |
| `fire_extabsorb` | 5 | × | cvar: absorb gained per extinguished heat |
| `fire_heatscale` | 1.0 | × | cvar: heat passed to neighbouring fires |
| `fire_incomingheatscale` | 0.1 | × | cvar: heat a burning fire accepts from others |
| `fire_dmgscale` | 0.1 | × | cvar: damage per unit of output heat |
| `fire_dmgbase` | 1 | hp | cvar: base damage per damage tick |
| `fire_growthrate` | 1.0 | × | cvar: self-heating rate |
| `fire_dmginterval` | 1.0 | s | cvar: time between damage ticks (and damage multiplier) |
| absorb_from_ignition | 0.05 | × | absorb = `ignitionpoint` × this at spawn |
| flame_think | 0.2 | s | entity flame interval (13 ticks); first think 0.1 s (7 ticks) after creation |
| flame_direct | 1 | hp/think | burn + direct to the burning entity (5 hp/s nominal) |
| flame_radius_dmg | 4 | hp/think | burn radius damage to others, radius = flame size / 2 |
| flame_heat | 2 | heat/think | added to every env_fire whose origin is within flame size / 2 |
| flame_min_size | 16 | in | flame size = max(16, (box x + box y) / 2) of the target |
| flame_default_life | 2 | s | lifetime set at creation before the caller's value |
| ignite_input_life | 30 | s | Ignite input, IgniteLifetime/... inputs when not yet burning |
| flame_remove_delay | 0.5 | s | after "stop burning" until the flame entity is removed (33 ticks) |
| firesource_think | 0.25 | s | env_firesource interval (17 ticks) |
| sensor_min_think | 0.1 | s | env_firesensor smallest interval |

## Behavior

### 1. env_fire keyvalues, flags, inputs, outputs

Keyvalues:

| Keyvalue | Default | Meaning |
|---|---|---|
| `firesize` | 0 | size S (height of a full fire in units; also sets full heat S/4 and the box) |
| `fireattack` | 0 | nominal seconds to grow from 0 to full heat (0 → H per nominal second, 10 thinks) |
| `health` | 0 | fuel in seconds (ignored with flag 1) |
| `ignitionpoint` | 0 | heat that must be absorbed before an unlit fire lights (stored as absorb = 0.05 × value) |
| `damagescale` | 0 | multiplies the heat part of the damage |
| `firetype` | 0 | 0 natural (smoke-and-flame particles, burn damage), 1 plasma (unused, below) |
| `StartDisabled` | 0 | start disabled |

Spawnflags: 1 infinite (no fuel), 2 smokeless, 4 start on, 8 start full,
16 don't drop, 32 no glow, 128 delete when out, 256 visible from above.

Inputs: **StartFire** (light it, if enabled and not already lit),
**Extinguish** *t* (clear flag 1, fade out over *t* s, then go out),
**ExtinguishTemporary** *t* (same but keeps flag 1), **Enable**,
**Disable** (disable; if burning, go out at once).

Outputs: **OnIgnited** (activator = caller = the fire) every time it lights
(also by heat), **OnExtinguished** (same) every time it goes out.

Not damageable: a fire ignores all damage (blasts do not blow it out).

Flags 32 and 256 only change networked bits the CS:S client ignores (2.6).
Plasma fires make a different effect entity and loop `Fire.Plasma`
(`ambient/nature/fire/fire_small1.wav`, volume 1, voice channel) and deal
plasma damage; no stock map uses them and they are not specified further.

### 2. env_fire state and life cycle

State per fire: heat h (starts 0), absorb a, full heat H, attack time A,
fuel f, damage due time, enabled, an effect (present while lit).

**2.1 Spawn.** a := 0.05 × `ignitionpoint`; h := 0; H := S/4; fuel 0 (the
`health` keyvalue is only read at StartFire); if flag 8, h := H. Bounds =
out box at the map origin; not drawn; not solid. Enabled unless `StartDisabled` (a start-disabled fire goes through Disable).
After all entities spawned: if flag 4 (and it was not done before), h := H
and StartFire (no enabled check here).

**2.2 StartFire** (input; also when heat first rises above 0, 2.4):
1. Ignored if the fire is disabled (input only) or already has an effect.
2. Position: with flag 16, its origin. Otherwise trace a ray straight down
   1024 units from its origin against the world, static props and solid
   entities such as props and brush entities (solid, moveable and window
   contents; players, NPCs and grates do not stop it) and use the end point; if nothing is hit the end
   point is 1024 units below. The fire entity **moves** there.
3. Re-initialise: fuel f := `health` unless flag 1 (then 0); if f ≠ 0 the
   fire is marked "delete when out". H := S/4. Flag 8 → h := H. (h is not
   reset otherwise: a fire relit after going out starts from its negative
   or zero heat.)
4. Bounds := (−S/4, −S/4, 0)–(S/4, S/4, S). So its centre, the start of
   its damage line-of-sight traces, is S/2 above the floor point.
5. Create the effect (2.6), started on; fire **OnIgnited**; set the damage
   due time to 0; run the first update (2.3) **immediately**, then every
   0.1 s.

**2.3 Update** (every think, nominal step σ = 0.1 s):
1. Fuel: if f ≠ 0: f −= σ; if f ≤ 0 → fade out over 1 s then go out
   (2.5); stop this update.
2. Effect size: if h changed since the last update, send the effect scale
   h_old/64 over 0.5 s (h_old = heat before this update's growth; cosmetic,
   the CS:S client ignores it).
3. Grow: g = (H / A if A > 0 else H) × σ × `fire_growthrate`; add g to
   itself as self-heat (2.4).
4. Output heat Q = (h_old / 64) × h_new, where h_new is the heat after step 3.
5. Damage box from h_new (if h_new ≤ 0, an empty box at the origin):
   scale c = h_new / H; half-width r = max(16, c × S/2); height z_top = c × S.
   Box B = origin + (−r, −r, 0)–(r, r, z_top). Object box B' = origin +
   (−r/2, −r/2, 0)–(r/2, r/2, z_top/2).
6. Damage tick: if the due time ≤ now: due := now + `fire_dmginterval`;
   D = trunc((`fire_dmgbase` + Q × `fire_dmgscale` × `damagescale`) ×
   `fire_dmginterval`) (an integer); a tick happens if D ≠ 0.
7. For every entity whose bounds touch B (not the fire itself):
   - another `env_fire` (lit or not, up to 16) → remember it as a
     neighbour;
   - cannot take damage → skip;
   - on a damage tick: players qualify anywhere in B; other entities only
     if their world bounds touch B'. A qualifying entity is hit if a ray
     from the fire's centre (2.2 step 4) to the entity's centre, against
     the same contents as the drop trace and ignoring the victim, is clear
     and did not start inside a solid. Hit = D burn damage, inflictor and
     attacker = the fire.
8. Spread: Q' = Q × `fire_heatscale` × σ; if there are n > 0 neighbours,
   each receives Q'/n as heat from others (2.4).

**2.4 Adding heat** (self-heat or heat from others/helpers), ignored while
disabled:
1. From others only: if this fire is burning (h > 0), the heat × 0.1
   (`fire_incomingheatscale`).
2. Absorb: if a > 0: cost = heat × 3 (`fire_absorbrate`). If cost > a: heat
   −= a/3, a := 0. Else a −= cost, heat := 0.
3. h += heat. If h was ≤ 0 before and is now > 0 and the fire has no
   effect → StartFire (2.2, without the enabled check, which already
   passed). Then h := min(h, H).

**2.5 Going out.** "Fade out over t" sends the effect scale 0 over t and
replaces the burn think with a single go-out think t later (rounded to a
tick): **no growth and no damage while fading**. Going out: fire
**OnExtinguished**; remove the effect; h := min(h − 20, 0); stop thinking;
if marked "delete when out" (flag 128, or it had fuel) remove the fire
entity; otherwise shrink to the out box (it stays at its dropped position)
and can be lit again. The Extinguish inputs go through the fade even when
the fire is not lit (OnExtinguished still fires, h drops by 20).

**2.6 What the client shows** (the effect entity is created at the fire's
floor point and parented to it; it is networked like any entity, so a
client sees it while it is in its PVS):
- On creation the client starts one particle system at the effect's origin,
  chosen by n = floor(S / 36): n = 0 `env_fire_tiny`, 1 `env_fire_small`,
  2 `env_fire_medium`, ≥ 3 `env_fire_large`, each with the suffix `_smoke`
  unless flag 2. So **every S ≥ 108 looks identical**; all stock fires use
  `env_fire_large_smoke`.
- Scale changes, the glow flag and "visible from above" are ignored; there
  is **no glow sprite and no dynamic light**.
- When the effect is removed (going out, round restart, PVS exit) the
  system stops emitting and its live particles finish their lives.
- The systems are in `particles/fire_01.pcf` (preloaded by
  `particles/particles_manifest.txt`); contents in section 7.

### 3. Heat helpers

**env_firesource** (keyvalues `fireradius` R, `firedamage` P; spawnflag 1
start on; inputs Enable, Disable): while on, every 0.25 s it adds P × 0.25
heat (as "from others", 2.4) to every env_fire, lit or not, whose origin is
within R of its origin (up to 128). It cannot be put out.

**env_firesensor** (keyvalues `fireradius` R, `heatlevel` L, `heattime` T;
spawnflag 1 start on; inputs Enable, Disable; outputs OnHeatLevelStart,
OnHeatLevelEnd, activator = caller = the sensor): thinks every
max(0.1, T/4) s = τ. Each think: sum the heat of burning fires whose origin
is within R. If the sum ≥ L: accumulated += τ; when accumulated ≥ T and not
yet "at level" → at level, OnHeatLevelStart. Else accumulated := 0 and, if
at level, OnHeatLevelEnd. Enable resets the accumulator and the "at level"
state; Disable while at level fires OnHeatLevelEnd.

**env_fire_trail** is a code-made smoke-and-fire trail (Half-Life 2's
helicopter); maps do not place it and CS:S does not create it. Not
specified.

None of the three appears in a stock CS:S map.

### 4. Entity flames

**4.1 Creating one** (on target T): flame size = max(16, (x size + y size)/2)
of T's collision box; placed at T's origin and parented to T (it follows T
and is deleted with it); first think 0.1 s later; lifetime 2 s (callers
then set their own); it plays a loop on itself: `General.BurningFlesh` if T
is an NPC, else `General.BurningObject`; clients are told which entity it is
attached to.

**4.2 Who ignites**:
- **Ignite** input (any model entity: props, players, hostages...): if T is
  not already burning, a flame with lifetime 30 s, T is marked burning, T
  fires **OnIgnite** (activator = caller = T). If T is already burning,
  nothing happens (no OnIgnite). Props: only flammable ones (prop data
  `fire_interactions` → `flammable`); a non-flammable prop gets no flame
  and no OnIgnite. The input skips the NPC-only rule, so players do get a
  flame (Q3).
- **IgniteLifetime** *t*: ignite for 30 s if not burning, then set the
  remaining lifetime to *t* (from now). **IgniteNumHitboxFires** /
  **IgniteHitboxFireScale**: ignite for 30 s if not burning; the numbers are
  stored but nothing in CS:S reads them.
- **Prop fire rules** (specs/source/prop_damage.md section 4 step 8):
  non-lethal blast or burn damage → ignite U(10, 15) s; bullet damage that
  takes a half-health-interaction prop to ≤ half → full health and ignite
  U(10, 15) s; explosive-resist props hit by a lethal blast burn for a
  computed short time. Only flammable props burn.
- **Exploding props with `onbreak` `explode_fire`** (the gas can): after
  the explosion, every "combat character" (players, NPCs) within the
  explosion radius that passes the damage filter is asked to ignite for
  30 s **with the NPC-only rule**: a non-NPC refuses. In the SDK a
  **player is therefore not ignited** (this corrects the assumption in
  prop_damage.md section 7 step 7 / Open question 5; CS:S's player could
  still override it, Q3).
- **Break pieces of a burning prop** created on the server are each
  ignited for the parent flame's remaining lifetime. In CS:S break pieces
  are client-side (prop_damage.md 2.3/7) and client pieces never burn
  (`cl_burninggibs` 0, and its particle system is not in CS:S).
- **env_shooter** with spawnflag 2 (flaming gibs) gives each gib a flame
  for the gib's life. Stock: de_inferno's two shooters have spawnflags 0.
- **env_entity_igniter** (keyvalues `target`, `lifetime`; input Ignite):
  for each entity matching `target` (name or class, `!activator` allowed):
  a combat character is asked to ignite for `lifetime` with the NPC-only
  rule (players refuse, as above); anything else gets a new flame with that
  lifetime **directly**, without the flammable check and without marking it
  burning (so it can stack several flames). Not in stock maps.

**4.3 Each flame think** (every 0.2 s):
1. Target gone → remove the flame (the remove plays `General.StopBurning`
   if the loop was playing).
2. Target is turning into a ragdoll → hide the flame and skip this think.
   Target is a dead NPC → remove the flame and tell it it stopped burning.
3. Target in water (water level > 0): spawn bubbles (the bubbles temp
   entity, 12 bubbles of `sprites/bubble.vmt` in the box from its centre
   ± 32 in x/y and 32 below to its centre, rising to the water surface,
   speed 8). **Water does not put the flame out.**
4. Lifetime over (lifetime < now): play `General.StopBurning` on the flame
   (null wave on the same channel, which stops the loop), remove the flame
   0.5 s later, tell a combat-character target it stopped burning. Props
   are **not** told: they stay marked burning forever (Quirks). Stop.
5. Radius damage: 4 burn from the flame's origin with radius
   flame size / 2, ignoring the target, through the game's radius damage
   (CS:S's: specs/cs_source/grenades.md 5.3, D = 4, R = size/2; Q6).
6. Direct damage: 1 burn + direct to the target, inflictor = attacker = the
   flame (props: prop_damage.md 4, "already burning and burn without
   direct → ignore" does not apply because of the direct bit).
7. Heat: 2 heat (as from a helper, 2.4) to every env_fire, lit or not,
   whose origin is within size/2 of the flame. With the stock ignition
   point (absorb 1.6) one such think lights an unlit fire (2 − 1.6/3 =
   1.47 > 0).

**4.4 Looks** (client): the flame starts the particle system
`burning_character` (`particles/burning_fx.pcf`) attached to itself and
following the target: control point 0 at the flame (= target origin),
control point 1 on the target; flame particles spawn on the target model's
hitboxes. When the flame entity goes away the system stops emitting. No
dynamic light: the burning-prop light needs game rules that ask for it
(base rules say no; CS:S Q7), and the client's flame light exists only in
the episodic build. A flame on a target that becomes a client-side ragdoll
is copied to the ragdoll for 7 s with `General.BurningFlesh` (only matters
if players or hostages can burn, Q3).

### 5. Flammable props in CS:S

`scripts/propdata.txt` templates carry no fire interactions. Of the 252
models placed as `prop_*` in stock maps, only `models/props_junk/gascan001a.mdl`
has any: `fire_interactions` → `flammable` `yes`, and `physgun_interactions`
→ `onbreak` `explode_fire`, `onfirstimpact` `break` (the latter needs the
gravity gun). Its prop data: template Metal.Small, health 20, bullets ×1,
club ×1, blast ×1, explosive damage 25, radius 80; box 8.56 × 20.17 ×
30.23 (hull ±4.28, −10.11..10.05, ±15.11) → flame size max(16, 14.37) = 16,
radius 8. Placements: cs_militia (438, 1959, −109), de_inferno (126, 189,
198); neither is near an env_fire.

A burning gas can loses 1 hp per flame think and breaks when its health
reaches 0 (explosion and `PropaneTank.Burst`, prop_damage.md 7.5), long
before the 10–15 s lifetime ends. The flame and its loop are deleted with
it (`General.StopBurning` plays as it is removed). Burn damage from an
env_fire ignites it (if not lethal) and further env_fire hits are then
ignored; a lethal env_fire hit (D ≥ health) breaks it at once.

`fire_extinguisher` props (cs_office, cs_assault, de_nuke) only make steam
(prop_damage.md); nothing in CS:S puts fires out (the extinguish-in-radius
path has no caller here).

### 6. Sounds

| Entry | Wave | Channel | Volume | Pitch | Level | Used by |
|---|---|---|---|---|---|---|
| `General.BurningObject` | `ambient/fire/fire_small_loop2.wav` (44.1 kHz 8-bit mono, 4.13 s, loop) | weapon | 1 | 100 | normal (75 dB) | entity flame on a non-NPC; de_chateau's 9 ambient_generic |
| `General.BurningFlesh` | `npc/headcrab/headcrab_burning_loop2.wav` (44.1 kHz 8-bit, 3.20 s, loop) | weapon | 1 | 100 | normal | entity flame on an NPC; burning client ragdolls |
| `General.StopBurning` | `common/null.wav` (0.1 s silence) | weapon | 1 | 100 | normal | flame end: cuts the loop on the same channel |
| `fire_medium` | `ambient/fire/fire_med_loop1.wav` (44.1 kHz 16-bit mono, 4.55 s, cue loop) | static | 0.5 | 100 | normal | de_dust, de_dust2 ambient_generic |
| `Fire.Plasma` | `ambient/nature/fire/fire_small1.wav` (4.55 s, loop) | voice | 1 | – | compatibility attenuation 1 | plasma env_fire (unused) |
| `PropaneTank.Burst` | `ambient/fire/gascan_ignite1.wav` (1.1 s) | weapon | 1 | 95–105 | 90 dB | gas can explosion (prop_damage.md) |

env_fire itself makes **no sound**. The flame loops are emitted from the
flame entity (so they follow the target). Map sounds are ambient_generic
(rules in specs/cs_source/sounds.md section 6): all "start silent" (16),
looped, volume 10, radius 1250 (level 70 by the radius rule; the script
entry's level may win, sounds.md Q11), started by `BombExplode` PlaySound.

### 7. Particle systems (data in the install)

Operator meanings are as in specs/cs_source/impact_effects.md section 14,
plus the interpretations listed at the end of this section (to be checked
like impact_effects Q7). Every system has a maximum step of 0.1 s. "Up" is
world +z (the fire's control point has no rotation).

**7.1 `env_fire_large_smoke`** = smoke emitter (root) + children
`env_fire_large` (flames, which has children `env_fire_large_b` and
`env_embers_large`) and `env_fire_large_smoke_b`; all start together and
emit forever.

| Part | Rate /s (max alive) | Material | Life (s) | Radius (in) | Placement and motion | Colour, alpha |
|---|---|---|---|---|---|---|
| smoke (root) | 2 (10) | `particle/vistasmokev1/vistasmokev1` (sprite card, dual sequence: first seq 0–3 at rate 0.8, second seq 4 at 0.075; fades as screen size grows 1→1.4) | U(3, 4) | U(18, 24), grows ×1→×5 (bias 0.4) | sphere shell 24–48 in, upper half only; then +(U(−4,4), U(−4,4), U(48,64)); start velocity up ≈ 22–24 + noise (±12, ±12, 25..42); gravity 75 **up**; drag 0.01; twist 15 about z; roll U(0,360), roll oscillation ±0.4 | U between (255,131,9) and (194,108,32) → fades to (59,58,56) from 10 % to 80 % of life; alpha U(90,100), fade in 0→1 over first 15 %, out 1→0 from 50 % to 100 % |
| smoke b | 4 (20) | same material (rates 1.25 / 0.125) | U(3, 4) | U(18, 24), ×1→×5 (0.4) | 24 in from the point, upper half; +(U(−4,4), U(−4,4), 48); up 22–28 + noise (±12, ±12, 15..32); gravity 45 up; drag 0.01; twist 15 | between (255,131,9) and (255,120,0) → (52,50,50) from 20 % to 60 %; alpha U(120,140); same fades |
| flames | 17 (32) | `particle/fire_burning_character/fire_env_fire` (sprite card, self-additive, overbright 2.15, blended frames; fades as screen size grows 0.425→0.525) | from sheet: seq U{3,4,5} at 20 fps = 1.2 / 1.75 / 1.4 | U(24, 32), radius oscillation 0.1 at 0.25 Hz | disc 0–20 in (z flattened); raised 0.3 × radius; up 0–5 + noise (±24, ±24, 12..24); gravity 100 up; drag 0.0055; random force ±16 per axis; roll U(−8, 8); 50 % mirrored | between (222,222,222) and (161,161,161) → black from 70 % of life (end time 1.2: about 60 % of the way at death); alpha U(80,180), fades out over its last U(0.1, 0.125) s |
| flames b | 12 (10) | same as flames | seq U{0,1,2} at 14 fps = 0.857 / 0.786 / 0.857 | U(32, 48) | disc 0–18 in; raised 0.45 × radius; up U(48, 96); gravity 90 up; random force ±16; roll U(−24, 24); 50 % mirrored | between (156,156,156) and (194,194,194); alpha U(120,180), fades out over its last 25 % |
| embers | 8 (15) | `particle/particle_glow_05_additive` (additive glow 256²) | U(0.5, 2) | U(1, 3) | shell 32–48 in (upper half, x/y halved); +U(32, 64) up; random speed 0–15 + local up 12–38; gravity 40 up; drag 0.01; random force ±150 | between (169,16,16) and (238,204,18); alpha fades in over first 25 %, out over last 25 %, flickers (oscillation rate 5–10 at 3–8 Hz) |

Flame-sheet sequences (`particle/fire_burning_character/fire_burning_character.vtf`,
2048², clamped sequences): 0: 12 frames, 1: 11, 2: 12, 3: 24, 4: 35, 5: 28.
Smoke sheet (`particle/vistasmokev1/vistasmokev1.vtf`, 1024², looping):
0–3: 16 frames, 4–9: 6 frames.

Overall (estimate): a flame bed ~40 in across whose sprites rise roughly
100–150 in before dying, sparks scattering 1–2 m, and a slow
orange-turning-grey smoke column whose puffs grow to 90–120 in radius over
3–4 s while rising several metres. The other
sizes (`env_fire_tiny/small/medium` and the smokeless variants) are in the
same file and unused by stock maps.

**7.2 `burning_character`** (entity flames) = smoke (root) → child
`burning_character_b` (flames) → children `burning_character_d` and
`burning_character_e` (`burning_character_c` is in the file but no system
uses it).

| Part | Rate /s (max) | Material | Life (s) | Radius (in) | Placement and motion | Colour, alpha |
|---|---|---|---|---|---|---|
| smoke (root) | 13.5 (32) | `particle/vistasmokev1/vistasmokev1_nearcull` (fades as screen size grows 0.4→0.55); second sequence U{4..9} | 1.5 | U(18, 24), ×1→×4 (0.4) | random point on the target's hitboxes (CP 1), +16 up; velocity noise (±15, ±15, 80..100); gravity 47 up; drag 0.025; follows CP 0 for its first 2.5–10 % of life; roll U(0,360) | between (50,50,50) and (39,39,39), 40 % toward the local light; → black over life; alpha U(170,190), fade in over first 25 %, out over the last 1.17 s; × a ramp on the system's age at birth: 0 for ≤ 1 s, 1 at ≥ 4 s (smoke builds up) |
| flames b | 7 (15) | `particle/fire_burning_character/fire_burning_character` (self-additive, overbright 2.75, depth blend 7; fades at screen size 0.45→0.575) | seq U{3,4,5} at 22 fps = 1.09 / 1.59 / 1.27 | noise 9–30, × age ramp 0 (system age 0) → 1 (0.75 s); then ×0.75→×1.25 (0.6) | on hitboxes scaled ×2, biased up; +0.35–0.5 × radius along the target's up; noise (±25, ±25, 10..20); gravity 75 up; drag 0.015; random force ±15; follows the target bone for its first 12.5–25 %; roll noise ±30 | between (207,174,174) and (207,205,205) → black from 65 % to 95 %; alpha U(100,200) × (1 at the target's origin → 0 at 92 in from it) |
| flames d | 30 (24) | same | seq U{0,1,2} at 20 fps = 0.6 / 0.55 / 0.6 | noise 11–17 × age ramp (0.75 s); grows ×0→×1 (bias 0.75) | on hitboxes, biased up 0.5; +0.125–0.25 × radius up; pre-aged 0–0.2 s; positions pulled toward CP 0 by a factor going 0.1→1 over the system's first second; noise (±55, ±55, 0..25); gravity 170 up; drag 0.025; follows the bone 12.5–52.5 % | between (201,147,147) and (216,178,141) → black from 85 %; alpha U(64,128), fade in over first 10–12.5 %, out over last 12.5–25 % |
| flames e | 150 (150) | same | seq U{0,1,2} at 14 fps | noise 18–24 × age ramp 0→0.6 (0.75 s); ×0→×1 over the first half (0.8), ×1→×0 over the second (0.5) | on hitboxes; +0.25–0.5 × radius up; pre-aged 0–0.2 s; noise (±25, ±25, 0..25); gravity 200 up; drag 0.015; random force ±15; follows the bone 12.5–40 % | between (153,150,146) and (165,146,127) → black from 75 %; alpha noise 0.35–0.5, fade in first 10 %, out last 12.5–25 % |

**7.3 Extra operator interpretations** (hypotheses, same status as
impact_effects.md Q7):
- *Oscillate scalar*: adds rate × sin(2π × frequency × t + phase) style
  wobble to the named field (3 radius, 4 roll, 7 alpha).
- *Velocity noise*: initial velocity from smooth 3D noise, mapped per axis
  into [output minimum, output maximum].
- *Random force*: a random acceleration per axis in [min, max] each step.
  *Twist around axis*: a tangential acceleration of the given amount around
  the axis through the control point.
- *Position modify offset random*: adds U(min, max) per axis (× radius
  when "proportional to radius", in the control point's frame when "local").
- *Color fade*: blend toward `color_fade` between `fade_start_time` and
  `fade_end_time` as fractions of life.
- *Remap noise to scalar*: initial value of the field from noise in
  [output minimum, output maximum]. *Remap initial scalar* with input 8
  (creation time = system age at birth): multiplies the field by the
  linear map of that age from [input min, input max] to [output min,
  output max], clamped. *Remap distance to control point to scalar*: per
  step, multiplies alpha by the linear map of the distance to the point.
- *Position on model random*: a random point inside a random hitbox of the
  control point's entity (box scaled by "hitbox scale"), nudged by the
  direction bias. *Movement lock to bone / control point*: the particle
  moves rigidly with its hitbox / point, the lock fading out between the
  two life fractions. *Lifetime pre-age noise*: starts the particle already
  aged by a noise value. *Position modify warp random*: scales positions
  about the control point by a factor moving from "warp min" to "warp max"
  over the transition time.
- Sequence ranges are inclusive. "Lifetime from sequence" = frames / fps.

### 8. Stock maps

`env_fire` placements (counted from the entity lumps; no other map has
env_fire, env_firesource, env_firesensor, env_fire_trail,
env_entity_igniter, or an Ignite-family input):

| Map | env_fire | Names (count) | Sizes | Started by |
|---|---|---|---|---|
| de_dust2 | 16 | `firea` 9, `fireb` 7 | 512 ×4, 256 ×1, 128 ×11 | site A target `*5`: `firea` StartFire 0.2 s; site B `*2`: `fireb` StartFire 0.2 s |
| de_chateau | 14 | `bomb_a_flames` 7, `bomb_b_flames` 7 | 256 | `*35`: A, `*34`: B, delay 0 |
| de_dust | 9 | `fire_a` 5, `fire_b` 4 | 256 ×8, 128 ×1 | `*2`: `fire_a`; `*1`: `fire_b`; delay 0 |
| de_inferno | 5 | `bomba_fire` 3, `bombb_fire` 2 | 256 | `*6`: A, `*4`: B |
| de_prodigy | 3 | `fire_a` 2, `fire_b` 1 | 256 ×2, 128 | `*40`: A, `*39`: B |
| de_tides | 3 | `fire_a` 1, `fire_b` 2 | 256 | `bombsite_A` / `bombsite_B` |
| de_nuke | 2 | `bomba_fire` 2 | 256 | site `A` only (site B has no fire) |
| de_train | 2 | `fire_a` 1, `fire_b` 1 | 256 | `Bombsite A` / `Bombsite B` |
| total | 54 | | | |

Every one has spawnflags 1 (infinite), `ignitionpoint` 32, `health` 30
(fuel, ignored), `fireattack` 4, `damagescale` 1.0, `firetype` 0 or absent,
no `StartDisabled`, no outputs. So all are: enabled, unlit, absorb 1.6, full
heat S/4, grow over 4 nominal seconds, burn until the round restarts (Q4).

What goes with them (all on the same `BombExplode`, same delay):
- de_dust2: `ag_fire_a` (832, 2624, 123.678) and `ag_fire_b` (−1732, 2700,
  91.678) PlaySound, `fire_medium`.
- de_dust: `ag_fire_a` (1836, 412, 99.678) with the `fire_b` group and
  `ag_fire_b` (188, −1652, 123.678) with `fire_a` (names crossed, positions
  match), `fire_medium`.
- de_chateau: `bomb_a_flamesound` ×5 and `bomb_b_flamesound` ×4
  ambient_generic `General.BurningObject` (positions at the fires).
- de_prodigy, de_tides: `light` entities `bombafirelight` /
  `bombbfirelight` TurnOn: colour (248, 146, 54), brightness 100, quadratic
  falloff, `_distance` 256, spawnflag 1 (start dark), switchable styles
  (prodigy 34/35, tides 32/33); placed 16 in below the fire origins. These
  are baked lightmap styles switched on, not dynamic lights.
- de_train: `func_dustcloud` `dustclouda`/`dustcloudb` TurnOn.

Where the fires land (drop trace against **world brushes only**; static
props and displacements not included, so marked rows may land higher):

| Map, fire (origin) | S | Floor z | Centre z (= floor + S/2) | Note |
|---|---|---|---|---|
| de_dust2 `firea` (864, 2744, 104), (560, 2792, 104) | 512 | 96 | 352 | |
| de_dust2 `firea` (1148, 2456, 104) | 256 | 96 | 224 | |
| de_dust2 `firea` (756, 2428, 104), (540, 2584, 104), (332, 2740, 104) | 128 | 96 | 160 | grain-basket static props at 96 nearby |
| de_dust2 `firea` (916, 2856, 304) / (644, 2944, 340) | 128 | 296 / 328 | 360 / 392 | |
| de_dust2 `firea` (−1988, 1704, 232) | 128 | 224 | 288 | **at site B** (Quirks) |
| de_dust2 `fireb` (−1828, 2644, 40) | 128 | 32 | 96 | |
| de_dust2 `fireb` (−1412, 2660, 24), (−1636, 2548, 24), (−1572, 1812, 16) | 128 | 0 | 64 | |
| de_dust2 `fireb` (−1824, 1800, 32) | 512 | 0 | 256 | crate static prop 24 in away |
| de_dust2 `fireb` (−1680, 1600, 32) | 512 | 0 | 256 | origin inside a solid block (z 0–112), centre inside a brush: its damage traces start in solid → it never damages (Q9) |
| de_dust2 `fireb` (−2020, 3092, 80) | 128 | 32 | 96 | |
| de_chateau A (16,160,0), (−16,48,0), (272,48,0), (240,152,0) | 256 | −8 | 120 | |
| de_chateau A (500,16,320) / (478,538,318) / (304,552,456) | 256 | 312 / 268 / 450 | 440 / 396 / 578 | |
| de_chateau B (1872,1560,360) / (2216,1010,158) / (1874,992,376) | 256 | 352 / 154 / 372 | 480 / 282 / 500 | |
| de_chateau B (2360,1560,320), (1960,984,320), (2366,1462,320), (2432,1312,158) | 256 | 312, 312, 312, 154 | 440, 440, 440, 282 | centre inside a brush → never damages (Q9) |
| de_inferno A (573,2489,233), (203,3132,264) | 256 | 160 | 288 | origin inside a detail brush (z 162.3–280/274); drops through it |
| de_inferno A (179,3029,240) | 256 | 238 | 366 | |
| de_inferno B (2119,187,237), (2072,688,188) | 256 | 160 | 288 | hay-bale static props at 160 |
| de_nuke (744,−816,−384), (555,−1062,−392) | 256 | −416 | −288 | crate static props at −416 |
| de_prodigy A (1936,−368,−348), (2020,24,−344) | 256 | −386 | −258 | |
| de_prodigy B (1504,−1240,−452) | 128 | −480 | −416 | |
| de_tides A (573,−341,39) | 256 | 0 | 128 | |
| de_tides B (−1051,−1632,−74), (−1040,−1184,−73) | 256 | −104 | 24 | |
| de_dust (all) | | (−32 brush floor) | | displacement ground likely higher: not computed |
| de_train A (1233,−160,−104), B (−17,−1282,−250) | 256 | −216 / −360 (brushes) | | a FlatCar static prop below each may stop the drop: not computed |

## Per-tick order

Server tick (dt 0.015 s):
1. Player commands and movement.
2. Entity thinks, in entity order: env_fire updates (2.3: fuel, effect
   scale, self-heat, box, damage, heat to neighbours), entity flames (4.3:
   radius damage, direct damage, heat), fire sources and sensors.
   A fire lit by heat during another fire's update starts its own first
   update immediately (2.2 step 5).
3. Physics (props broken by burn damage break in their damage call;
   prop_damage.md).
4. Event queue (entity_io.md): BombExplode → StartFire (and its first
   update and first damage tick happen **here**, inside the input),
   OnIgnited / OnExtinguished deliveries, PlaySound, light TurnOn.
5. Entities removed this tick are deleted (an entity flame removed with its
   prop plays `General.StopBurning`).

Client, per frame: particle systems step (max 0.1 s per step) and draw;
sound loops continue.

## Edge cases

- StartFire twice: the second does nothing. StartFire on a disabled fire:
  nothing. Enable later does not light it.
- Disable while burning: goes out at once (OnExtinguished, no fade),
  heat := min(h − 20, 0). Disabled fires ignore heat.
- Extinguish *t* on a lit infinite fire: no damage or growth for t, then
  out, the fire stays (not marked delete); a later StartFire now gives it
  fuel = `health` (flag 1 was cleared), so it burns that long, goes out
  with a 1 s fade and is **deleted**. ExtinguishTemporary keeps it infinite.
- Relighting: absorb was used up the first time, so a relit fire skips the
  ignition delay; its heat restarts from min(h − 20, 0) (0 for a fire that
  was full).
- Extinguish on an unlit fire still waits t and fires OnExtinguished;
  heat becomes −20, so heat from others must first make up 20.
- The damage box is measured from the floor point; anyone standing on a
  roof or ledge within the box height is in it. Only the LOS ray limits it.
- The LOS ray starts at S/2 above the floor: a fire under a lower ceiling
  or inside geometry has its rays start in solid and **damages nothing**,
  though it still burns and heats neighbours.
- Players are hit anywhere in the full box; everything else (hostages,
  props, breakables, weapons that can take damage) only if it touches the
  half-size box.
- Damage is never fractional: a size-128 fire does 1, 1, 1, 1, 2, 2, ...
- Neighbour heat is counted only from fires inside the box; at most 16.
  An unlit fire inside a lit fire's box lights after one update if the lit
  one has enough output (2.4).
- A fire whose drop trace hits nothing within 1024 units lands 1024 below
  its origin (in the void or inside geometry).
- Entity-flame radius damage uses CS:S's radius damage: for a player the
  target point is at 70–100 % of eye height (grenades.md 5.3), far beyond
  the 8–20 in radius of any stock-sized flame, so in practice **a burning
  object never hurts a player standing next to it** (Q6).

## Quirks

- **All stock fires look the same** regardless of `firesize` (keep): the
  size only changes damage, box and heat.
- **de_dust2: bombing site A also lights one fire at site B**
  (`firea` at (−1988, 1704, 232), size 128), and bombing B leaves that one
  dark (keep).
- **de_nuke: only site A burns** (keep).
- **Fires inside geometry never hurt anyone**: de_dust2's B fire at
  (−1680, 1600) and four of de_chateau's B fires (keep; Q9 to confirm).
- **A prop stays "burning" after its flame ends** (props are never told):
  it can never be re-ignited, ignores burn-only damage, and a half-health
  bullet prop would break on the next bullet (keep; no stock prop outlives
  its flame).
- **Water does not extinguish** entity flames, it only makes bubbles
  (keep).
- **Damage tick is the first update**: a just-started fire does 1 damage
  at once, in a 16-unit-radius box only a few units high (keep).
- **Growth and damage timing run on 0.105 s thinks** with a nominal 0.1 s
  step: full heat at 4.2 s, damage every 1.05 s (keep tick rounding).
- **Negative heat after going out** (keep).

## Test cases

Fire think k = 0 is the StartFire tick t0; think k is at t0 + 0.105k s
(66.67 tick). "ign 32" = stock `ignitionpoint` (absorb 1.6).

| Setup | Input | After | Expected |
|---|---|---|---|
| env_fire S 256, ign 32, sf 1 | spawn | – | unlit, h 0, absorb 1.6, H 64, out box (−8,−8,0)–(8,8,8), no effect, no damage |
| same | StartFire | think 0 (same tick) | dropped to the floor; OnIgnited; g = 64/4 × 0.1 = 1.6; absorb: 4.8 > 1.6 → g = 1.6 − 0.5333 = 1.0667 → h 1.0667; damage tick D = trunc(1 + 0 × 0.1) = 1 in box r 16, height 4.267 |
| same | – | think k (1 ≤ k ≤ 39) | h = 1.0667 + 1.6k |
| same | – | think 40 (t0 + 4.2 s) | h = 64 (clamped); box ±128 × 0..256; object box ±64 × 0..128 |
| same | – | damage ticks at thinks 0, 10, 20, 30, 40, 50 | D = 1, 1, 2, 4, 7, 7 (e.g. think 30: Q = (47.467/64) × 49.067 = 36.39 → trunc(4.64) = 4) |
| S 512 (de_dust2 `firea` (864, 2744, 104)) | StartFire | thinks 0, 10, 20, 30, 40 | h 2.667, 34.667, 66.667, 98.667, 128; D = 1, 2, 7, 15, 26; full box ±256 × 0..512 from floor z 96 |
| S 128 (de_dust2 `fireb` (−1828, 2644, 40)) | StartFire | thinks 0, 10, ..., 40, 50 | h 0.2667, 8.267, 16.267, 24.267, 32 (think 40); D = 1, 1, 1, 1, 2, 2; full box ±64 × 0..128 from floor z 32 |
| full 512 fire at (864, 2744) floor 96 | player standing at (1064, 2744, 96), clear line from (864, 2744, 352) | each damage tick | 26 burn, attacker = the fire |
| same | player at (1130, 2744, 96) | – | box x 608..1120; hull x 1114..1146 overlaps → 26 |
| same | player at (1140, 2744, 96) | – | hull x 1124..1156, no overlap → nothing |
| full 256 fire, a 30 kg crate (box 40.5 × 40.3 × 40.2) whose centre is 100 in away horizontally on the floor | damage tick | – | crate box spans 79.75..120.25 from the fire: touches B (±128) but not B' (±64) → not hit |
| same crate at 80 in | – | – | spans 59.75..100.25 → touches B' → 7 burn if LOS clear (wooden crate: burn has no multiplier, prop_damage.md) |
| de_dust2, bomb explodes at site A at tick N | BombExplode | tick N + 14 (ceil(0.2/0.015)) | 9 `firea` fires light (8 at A, 1 at (−1988, 1704) on B) and `ag_fire_a` starts; site B fires stay dark |
| de_dust2 site B bombed | BombExplode | +14 ticks | 7 `fireb` fires light; the B-side `firea` stays dark; `ag_fire_b` plays |
| de_chateau B bombed | – | full heat | fires centred inside brushes (2360,1560), (1960,984), (2366,1462), (2432,1312) damage nobody (LOS starts in solid) |
| lit full 256 fire, unlit enabled fire (ign 32) 50 in away inside its box | one update of the lit fire | same tick | Q = 64 → Q' = 6.4 (one neighbour) → absorb: 19.2 > 1.6 → 6.4 − 0.533 = 5.867 → the unlit fire StartFires (OnIgnited), drops, its first update at once |
| two lit full fires in each other's box | updates | – | each receives ≤ 0.1 × share; both stay at H (no change) |
| lit full 256 fire | Extinguish 2 | t + 1.995 s (133 ticks) | no damage meanwhile; then OnExtinguished, effect removed, h = min(64 − 20, 0) = 0, flag 1 cleared, fire stays |
| that fire | StartFire | think 0 | fuel = 30; absorb 0 → h 1.6; burns; when fuel ≤ 0 (think 299, ≈ t + 31.4 s; float rounding may give 300) → 1 s fade → OnExtinguished → fire deleted |
| unlit fire | Extinguish 0 | next tick | OnExtinguished; h = −20 |
| sf 1 fire, StartDisabled 1 | StartFire | – | nothing; Enable then StartFire → lights |
| burning fire | Disable | same tick | OnExtinguished at once, h ≤ 0, no fade |
| S 256 sf 0 / S 30 / S 50 / S 100 / S 128, sf 2 | StartFire | client | `env_fire_large_smoke` / `env_fire_tiny_smoke` / `env_fire_small_smoke` / `env_fire_medium_smoke` / `env_fire_large` |
| gas can (health 20) | Ignite input | – | flame size 16, life 30 s, `General.BurningObject` loops on it; OnIgnite; direct 1 per think at +0.105, +0.300, +0.495 ... (0.195 s apart) → 20th think at +0.105 + 19 × 0.195 = 3.81 s → health 0 → breaks, explodes (25/80) + `PropaneTank.Burst`; flame deleted, `General.StopBurning` |
| gas can burning | radius part each think | – | 4 burn at its origin, radius 8, to others (not the can) |
| gas can burning, an unlit stock env_fire whose origin is 5 in from the can's origin | one flame think | – | that fire gets 2 heat → 1.467 → lights |
| crate (not flammable) | Ignite input | – | no flame, no OnIgnite |
| player | Ignite input | – | SDK: a flame for 30 s on the player (Ignite input bypasses the NPC-only rule): 1 burn+direct per 0.195 s; Q3 |
| gas can explodes, player 40 in away | explode_fire rule | – | player not ignited (NPC-only rule), only the blast damage (prop_damage.md) |
| prop flame lifetime 2 s (e.g. IgniteLifetime 2) on a flammable prop with health 100 | – | thinks at +0.105 ... | 10 direct hits (+0.105 + 9 × 0.195 = 1.86 s; next at 2.055 > 2) → `General.StopBurning`, flame removed 0.495 s later; prop keeps its burning mark: a later Ignite does nothing |
| burning prop in water | each flame think | – | 12 bubbles; still burning, still 1 hp per think |
| env_firesource R 100, P 10, sf 1; unlit fire (ign 32) 50 away | first think (spawn tick) | – | adds 10 × 0.25 = 2.5 heat; cost 7.5 > 1.6 → 2.5 − 0.533 = 1.967 → lights |
| env_firesensor R 300, L 100, T 2, sf 1; two full 256 fires inside (sum 128) | thinks every 0.5 s | – | accumulated 0.5, 1.0, 1.5, 2.0 → OnHeatLevelStart at the 4th think where the sum ≥ 100; one fire Disabled → next think OnHeatLevelEnd |

## Open questions

1. **CS:S client fire visuals.** Does CS:S's client run the SDK 2013 logic
   (one particle system chosen by size, no glow, no scale animation)? The
   install's manifest and `fire_01.pcf` contain exactly those systems,
   which supports it. Check: bomb de_dust2 site A; the 512 and 128 fires
   should look identical and appear at full size at once (no 4 s growth).
2. **Burn damage to CS:S players.** Armour (does burn bypass it?), pain
   sound, HUD damage icon (base multiplayer rules flag burn as "show on
   HUD"), the death notice (attacker world / `env_fire`, kill icon) and
   score (suicide −1?). Check: stand in a de_dust2 fire after the A bomb
   with and without kevlar; watch health per second (expect 26 at full
   size, 1 then 2 then 7 then 15 while growing).
3. **Player ignition.** Does CS:S's player accept the Ignite input / entity
   flames (SDK: yes for the input, no for the gas can's explode-fire rule)?
   Does a burning player see/hear anything? Check on a test map with an
   `Ignite` output to `!activator`, and break de_inferno's gas can next to
   a bot.
4. **Round restart.** Are env_fire, their effects and entity flames
   removed and recreated unlit each round (not in CS:S's preserve list)?
   Check: bomb de_dust2 A, watch the next round start.
5. **Floors on de_dust and de_train.** Where do those fires land (the drop
   trace hits displacements and static props, which this spec did not
   test)? Check in game, or trace against displacements and the static
   prop collision models.
6. **CS:S radius damage for entity flames** (grenades.md Q10): can a
   burning gas can hurt anyone (radius 8)? Check with a bot crouched against
   a burning can.
7. **Burning-prop light.** Do CS:S's game rules ask burning props to emit a
   light (base: no)? Check: ignite the de_inferno gas can (Ignite input via
   `ent_fire`, or an HE that leaves it alive) in a dark corner and look for
   a dynamic light on nearby walls.
8. **Tick rate.** The stock server runs 66.67 tick (think 0.105 s); a
   64-tick server gives 0.09375 s thinks (full heat at 3.75 s, damage every
   11 thinks = 1.03 s). Which does our server emulate? (Follows the
   engine-wide choice in entity_io.md.)
9. **Fires buried in geometry.** Confirm that de_dust2's B fire at
   (−1680, 1600, 32) and de_chateau's four B fires never damage (LOS starts
   in solid). Check by standing in each after the bomb.
10. **Particle operator meanings** (7.3) as impact_effects.md Q7; the
    flames' colour-fade end time 1.2 > 1 (fraction of life or seconds?).
    Compare a screenshot of a lit de_dust2 fire with ours.
11. **Fire sound level**: `fire_medium` through ambient_generic, radius
    1250 → level 70 or the entry's 75 (sounds.md Q11).
