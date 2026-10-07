# Counter-Strike: Source: grenades (HE, flashbang, smoke)

Source basis: Valve's public Source SDK 2013. I read the SDK template mod's
grenade weapon base, its grenade projectile and its frag grenade (the template
is a stripped-down CS-derived example: its projectile header calls its
constants "constants for all CS grenades"), the template's radius-damage rule
and player animation state (grenade gesture layer), the shared base grenade
(detonation, explosion, scorch, sounds), the shared toss/fly-gravity movement
and velocity clipping, the generic radius damage and explosive damage force,
the base player's explosion ear-ringing hook, the client explosion temp entity
and base explosion effect, and the client smoke-grenade particle entity and
its screen fog overlay (this client file carries CS:S-only branches, so it is
CS:S's own smoke code). CS:S's game DLL (its grenade weapons and projectiles,
flashbang blindness, HE armour, buy rules, bots) is **not** public; those
parts are marked *inferred* or listed as hypotheses with an in-game check.
Data from the user's install, values only: the view models' sequences and
events (`v_eq_fraggrenade`, `v_eq_flashbang`, `v_eq_smokegrenade`), the sound
scripts, `scripts/dsp_presets.txt`, the flashbang/smoke materials and texture
headers, `resource/modevents.res`, the English localization (tutor hints), and
the weapon-script and ammo-cvar values already recorded in weapons.md.
Status: draft

## Summary

- A grenade is a weapon with one round of ammo per grenade (max carry: 2
  flashbangs, 1 HE, 1 smoke). Primary pulls the pin (0.9512 s view-model
  animation); releasing the button throws: 0.1 s later a projectile is
  spawned 16 in in front of the eye with speed `min(750, 6·(90 − p′))` along
  a view pitch `p′` that is bent 10° upward, plus the thrower's velocity.
  Secondary does nothing in CS:S (no lob).
- The projectile is a 4 × 4 × 4 in box under 0.4 × gravity (320 in/s²). On
  every contact it mirrors its velocity about the surface and keeps 45 % of
  the speed (13.5 % against players); it stops dead when a contact leaves it
  slower than 30 in/s. There is no rolling friction, so grenades bounce a few
  times and stop instead of rolling. This is most of the "CS feel".
- HE: after the fuse (1.5 s nominal, 1.56 s with the 0.2 s check), 100 damage
  falling linearly to 0 at 350 in, blocked only by world brushes (not by
  players or props), plus a fireball/smoke/spark effect, a scorch decal and
  `BaseGrenade.Explode`. Flashbang: blinds everyone (teammates and the
  thrower too) who can see it, less when facing away; white overlay plus a
  frozen after-image, and a muffled ringing (DSP). Smoke: a 64-sprite cloud
  of radius 240 that grows in 1 s, holds until 17 s and fades out by 22 s,
  and greys the screen when the camera is inside it.

## Units and conventions

- Distance: inches ("units"); Source axes X forward, Y left, Z up.
- Angles: degrees; pitch positive looks **down**; `forward(pitch, yaw) =
  (cos p cos y, cos p sin y, −sin p)`.
- Time: seconds; server tick 0.015 s (66.67 Hz). Times written as
  "now + T" become the tick `round((now + T)/0.015)` (Source rounds
  think times to the nearest tick); comparisons are "strictly after" where
  noted, which matters for the tick counts below.
- Damage in hit points; impulse kg·in/s (physics_props.md).
- `U(a, b)` uniform float, `I(a, b)` uniform integer (inclusive).

## Constants

### Weapon and throw (template, CS-derived; CS:S inferred)

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| max_flashbang / max_he / max_smoke | 2 / 1 / 1 | grenades | cvars `ammo_flashbang_max`, `ammo_hegrenade_max`, `ammo_smokegrenade_max` (measured, weapons.md M17) |
| price_flash / price_he / price_smoke | 200 / 300 / 300 | $ | `WeaponPrice` (weapons.md) |
| max_speed_flash / he / smoke | 250 / 250 / 245 | in/s | `MaxPlayerSpeed` while held |
| armor_ratio_he / flash / smoke | 1.475 / 1 / 1.0 | – | `WeaponArmorRatio` in the scripts (use for HE unknown, Q4) |
| throw_delay | 0.1 | s | from button release to projectile spawn (strictly after) |
| throw_pitch_lift | −10 | deg | pitch offset at level view |
| throw_pitch_scale | 100/90 | – | pitch stretch (signed pitch form) |
| throw_speed_per_deg | 6 | in/s per deg | `speed = 6·(90 − p′)` |
| throw_speed_max | 750 | in/s | cap |
| spawn_forward | 16 | in | spawn point = eye + 16·forward(p′, yaw) |
| spin_roll | 600 | deg/s | initial roll rate |
| spin_pitch | I(−1200, 1200) | deg/s | initial pitch rate |
| hidden_window | 0.5 | s | others don't draw the projectile while the thrower's throw gesture is in its first 25 % (client) |

### Projectile (template, "all CS grenades")

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| box | ±2 | in | axis-aligned collision box (4 × 4 × 4), not standable |
| gravity_scale | 0.4 | × | of `sv_gravity` (800) → 320 in/s² |
| elasticity | 0.45 | – | grenade elasticity |
| elasticity_world | 1.0 | – | every non-player surface |
| elasticity_player | 0.3 | – | hitting a player |
| elasticity_cap | 0.9 | – | product clamped to [0, 0.9] |
| friction | 0.2 | – | stored but unused by the CS bounce rule |
| stop_speed | 30 | in/s | contact leaving it slower than this stops it |
| floor_normal_z | 0.7 | – | contact normal z above this is "floor" |
| breakable_damage | 10 | hp (club) | dealt to `func_breakable` / `func_breakable_surf` it hits |
| breakable_slowdown | 0.4 | × | velocity after breaking through glass |
| water_slowdown | 0.5 | × | velocity multiplier per 0.2 s check while in water |
| water_entry_vz | 0.5 | × | vertical velocity on entering water |
| max_velocity | 3500 | in/s per axis | `sv_maxvelocity` clamp |
| check_interval | 0.2 | s | fuse / water check period (13 ticks) |
| net_initial_velocity | 20 bits over ±3000 | in/s | sent once so clients extrapolate from spawn |

### HE (template frag grenade; CS:S inferred)

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| he_fuse | 1.5 | s | detonation time after spawn (first check strictly after: 1.56 s) |
| he_damage | 100 | hp | at the blast centre |
| he_radius | 350 | in | 3.5 × damage; linear falloff to 0 |
| he_src_lift | 1 | in | blast point raised 1 in for the damage traces |
| he_ground_probe | +8 → −24 | in | vertical trace from origin to find the ground under the blast |
| he_pull_out | 0.6 | in | blast origin = probe hit + 0.6·normal |
| he_body_target | origin + eye_offset·U(0.7, 1.0) | in | point aimed at on a player |
| he_force_per_damage | 300 | kg·in/s per hp | 75 kg × 4 in/s |
| he_force_cap | 30000 | kg·in/s | 75 kg × 400 in/s |
| he_force_jitter | U(0.85, 1.15) | × | per victim |
| he_force_scale | 1.5 | × | template "explosion scale" |
| ear_ring_distance | 240 | in | base player: ring DSP if closer than this |
| shock_damage | 30 | hp | base player: shock DSP if blast damage ≥ this |

### Smoke (CS:S client code in the SDK)

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| smoke_grid | 4 × 4 × 4 | sprites | 64 sprites |
| smoke_sprite_radius | 80 | in | sprite size (half-width) |
| smoke_overlap | 20 | in | |
| smoke_spacing | (80 − 20)·4·0.5 = 120 | in | grid half-extent: grid offsets −120, −40, 40, 120 |
| smoke_expand_time | 1 | s | growth time |
| smoke_max_radius | 2 × 120 = 240 | in | final cloud radius |
| smoke_core_fraction | 0.3 | – | inner part of a sprite's radial ramp at full alpha |
| smoke_fade_start / end | 17 / 22 | s | default fade window after the cloud entity spawns |
| smoke_color_min / max | 0.5 / 0.6 | grey | per-sprite colour range |
| smoke_roll | U(−6, 6) | rad | initial roll |
| smoke_roll_speed | U(−0.1, 0.1) | rad/s | |
| smoke_trade_time | U(10, 20) | s | duration of one neighbour swap |
| smoke_near_fade | 0 → 10 | in | sprite alpha ramps from 0 at the eye to 1 at 10 in depth |
| smoke_fog_color | (0.3, 0.3, 0.3) | – | full-screen overlay colour |
| smoke_server_box | ±50 | in | collision bounds of the cloud entity after filling |

## Behavior

### 1. Buying and carrying

- Each grenade type is its own weapon in HUD slot 3 (bucket 3), with ammo in
  its own ammo type (`FLASHBANG`, `HEGRENADE`, `SMOKEGRENADE`); the weapon is
  "exhaustible" (removed when its ammo runs out).
- Max carry is the ammo cvar: 2 flashbangs, 1 HE, 1 smoke grenade, all at
  once. *Inferred:* buying when at the max is refused with the centre text
  "You cannot carry any more." (`Cstrike_TitlesTXT_Cannot_Carry_Anymore`)
  and costs nothing; buying a second flashbang adds ammo to the held
  flashbang weapon.
- Holding a grenade sets max speed 250 (smoke 245).

### 2. Pin pull and throw (template; CS:S inferred, Q1–Q3)

Per user command, in the weapon's post-think:

1. **Primary pressed**, no throw pending, pin not pulled, ammo > 0: play
   `ACT_VM_PULLPIN` (`pullpin`, 40 frames at 41 fps = 0.9512 s); mark the pin
   pulled; next primary = now + 0.9512. Its animation event at cycle 0.6923
   (0.6585 s) plays `Default.PullPin_Grenade` (`weapons/pinpull.wav`).
2. **Pin pulled and attack not held** (checked first every command, so a
   release always wins): send the third-person throw gesture, set the throw
   time = now + 0.1, remove one ammo (server), clear the pin, play
   `ACT_VM_THROW` (`throw`, 24 frames at 30 fps = 0.7667 s), idle timer =
   now + 0.7667.
3. **Throw time set and now strictly after it**: spawn the projectile
   (section 3) and enter "redraw" state.
4. **Redraw state**, idle timer passed (the throw animation is over): if out
   of this grenade's ammo, drop and delete the weapon (the player switches to
   the next best weapon); otherwise *inferred for CS:S* (Q2) redraw the same
   grenade (`ACT_VM_DRAW`, `deploy`, 0.6667 s) and allow attacks after it.
5. Otherwise normal weapon idling.

- Secondary: the template's lob/roll paths need view-model activities CS:S's
  grenade models do not have (`v_eq_*` have only `ACT_VM_IDLE`,
  `ACT_VM_PULLPIN`, three unused pull variants `ACT_VM_PULLBACK_HIGH/LOW/
  HAULBACK` identical to `pullpin`, `ACT_VM_THROW`, `ACT_VM_DRAW`). CS:S
  grenades have no secondary throw.
- Deploy: `ACT_VM_DRAW` 0.6667 s; deploy and holster clear pin and throw
  state. While the pin is pulled, auto-switching away (e.g. on pickup) is
  refused. A manual switch with the pin pulled cancels the throw and keeps
  the grenade (template; Q3).
- The `throw` sequence carries event 3005 (weapon "throw") at cycle 0.4348 =
  0.3333 s. The template ignores it and spawns at +0.1 s; which one CS:S uses
  is Q1.
- Tap-to-throw: in this rule a release on the very next command after the
  press throws at once (the pin animation is cut by the throw animation).
  Whether CS:S waits for the pin animation is Q1.
- Radio: on a throw the thrower's team hears "Fire in the hole!"
  (`Radio.FireInTheHole`, chat `Cstrike_TitlesTXT_Fire_in_the_hole`) unless
  `sv_ignoregrenaderadio` is set (*inferred* from data).
- Tick timing: release seen at tick R → throw time R·0.015 + 0.1 → projectile
  at the first tick with curtime strictly greater: R + 7 (0.105 s).

### 3. Throw direction and speed (template)

Inputs: the eye angles (pitch `p`, yaw `y`; view punch **not** included),
the eye position `E` (origin + view offset), the player's velocity `V`.

1. Bend the pitch. With `p` in signed form (−89 … 89):
   `p′ = −10 + p · 100/90`. (If the angle arrives in 0–360 form and is ≥ 90,
   i.e. looking up as 270–360: `u = 360 − p`, `p′ = −10 − u · 80/90`. Which
   form CS:S sees is Q5; the signed form is the default, see Quirks.)
2. Speed `s = min(750, 6 · (90 − p′))`.
3. Direction `f = forward(p′, y)`.
4. Spawn point `S = E + 16 f`; velocity `v₀ = s f + V`; angles zero;
   angular velocity: roll 600 deg/s, pitch `I(−1200, 1200)` deg/s, yaw 0.
5. The spawn point is not checked against walls (Q6).

Consequences (standing still): level view throws at 600 in/s, 10° up;
any view more than 22.5° up hits the 750 cap; straight down (p = 89) gives
6.7 in/s, a drop at the feet.

### 4. Projectile flight (template "CS grenade" rules)

State: position (the centre of a ±2 in box), velocity, angles, angular
velocity, ground flag, fuse time. Collision: everything solid for a moving
entity (world, brush entities, props, players, grates, windows), but never
its thrower. The box is not standable (players can't stand on it).

Each server tick (dt = 0.015), in this order:

1. **Check** (every 13 ticks, starting on the spawn tick): if out of the
   world, remove it. If now is strictly after the fuse time, detonate
   (section 5) and stop. While in water, velocity × 0.5.
2. If moving up, or it has no ground, or its ground is not standable: clear
   the ground flag. If on the ground and the velocity is exactly zero: clear
   angular velocity and do nothing more this tick.
3. Clamp each velocity component to ±3500.
4. **Gravity** (not on ground): `vz′ = vz − 320·dt`; the move is
   `(vx·dt, vy·dt, (vz + vz′)/2·dt)`, i.e. the exact parabola sampled at
   ticks. Then `vz ← vz′`.
5. Angles += angular velocity · dt.
6. Sweep the box along the move. If the sweep starts and ends in solid:
   zero linear and angular velocity, stop.
7. On a hit (fraction < 1) resolve the contact (below).
8. Water transition: entering water halves `vz` and plays
   `BaseEntity.EnterWater`; leaving plays `BaseEntity.ExitWater`.

**Contact resolution** with hit normal `n`, hit entity `H`:

1. `e_s` = 0.3 if `H` is a player, else 1.0.
2. If `H` is a `func_breakable` or `func_breakable_surf`: deal it 10 club
   damage; if that breaks it, `v ← 0.4·v` and the contact ends (no bounce;
   the grenade continues through the hole next tick).
3. `e = clamp(0.45·e_s, 0, 0.9)` (0.45, or 0.135 on players).
4. Mirror: `v_r = v − 2 (v·n) n` (components within ±0.1 snap to 0), then
   `v_r ← e · v_r`. Add the hit entity's base velocity (conveyors) for the
   speed test: `w = v_r + base`.
5. **Floor** (`n_z > 0.7`): set velocity `v_r`. If `|w| < 30`: it comes to
   rest: ground = `H` if standable, velocity 0, angular velocity 0, angles
   = the orientation of `n` (pitch from the normal, so it lies flat) with yaw
   `U(0, 360)`. Otherwise it keeps moving this tick: a second sweep of
   `v_r · (1 − fraction) · dt` (approximately; conveyor terms omitted).
6. **Wall or ceiling**: if `|w| < 30`, velocity 0 (it then falls next tick,
   since it has no ground); else velocity `v_r`. No second sweep: the rest
   of the tick is lost.
7. Every contact plays the bounce sound (section 4.1).

There is no ground friction term: a grenade sliding on a floor loses 55 %
of its speed at every contact that gravity pushes it into, so it stops
within a few contacts rather than rolling.

#### 4.1 Bounce sounds

| Grenade | Entry | Wave | Level |
|---|---|---|---|
| HE | `HEGrenade.Bounce` | `weapons/hegrenade/he_bounce-1.wav` | compat. attenuation 1.0 |
| Flashbang | `Flashbang.Bounce` | `weapons/flashbang/grenade_hit1.wav` | 75 dB |
| Smoke | `SmokeGrenade.Bounce` | `weapons/smokegrenade/grenade_hit1.wav` | compat. attenuation 1.0 |

All on the voice channel, volume 1, normal pitch. *Inferred:* CS:S plays its
entry on a contact and fires the `grenade_bounce` game event (thrower's
userid). Whether every contact (including the small sliding contacts) makes a
sound, or only above some speed, is Q7. The physics `Grenade.*` impact/roll
entries in `game_sounds_physics.txt` belong to the "grenade" surface property
(physics props), not to these projectiles.

#### 4.2 Client side

- The client gets the spawn velocity once and seeds its interpolation with a
  sample one second back at `origin − v₀`, so the grenade moves from its
  first frame.
- Other players' grenades are not drawn during their first 0.5 s while the
  thrower's third-person throw gesture is still in its first 25 % (the
  grenade is "still in the hand"). The thrower always sees his own.
- Models: `models/weapons/w_eq_fraggrenade_thrown.mdl`,
  `w_eq_flashbang_thrown.mdl`, `w_eq_smokegrenade_thrown.mdl` (one frame, no
  attachments). Entity classes: `hegrenade_projectile`,
  `flashbang_projectile`, `smokegrenade_projectile` (*inferred* names,
  check with the probe).

### 5. HE grenade

#### 5.1 Fuse

Fuse 1.5 s from spawn; detonation happens at the first 13-tick check
strictly after it: spawn tick + 104 (1.56 s). From button release: 111 ticks
(1.665 s). *Inferred* for CS:S (Q8).

#### 5.2 Detonation

1. Trace a line from origin + (0, 0, 8) to origin − (0, 0, 24) (shot-hull
   mask, ignoring itself). If it starts in solid, redo it from the origin
   itself.
2. If it hit something, move the grenade to `hit + 0.6 n`.
3. Effects (5.4), the radius damage (5.3), the scorch decal on the probe's
   hit surface (5.4), the explosion sound, an AI "combat" sound; the
   projectile becomes invisible and non-solid and is removed.
4. The game event `hegrenade_detonate` (userid, x, y, z) fires (data).
5. The base grenade has no screen shake for this grenade type (amplitude
   0). Whether CS:S shakes is Q9.

#### 5.3 Damage (template radius damage; CS:S formula Q10)

Inputs: blast origin `B`, damage `D = 100`, radius `R = 350`, inflictor
the projectile, attacker the thrower.

- `src = B + (0, 0, 1)`. "Blast in water" = `B` is in water.
- For each entity whose bounds touch the sphere (`src`, `R`) and that can
  take damage:
  1. Skip if the blast is in water and the entity is not (water level 0), or
     the blast is dry and the entity is fully under water (level 3).
  2. Target point `T`: for players `origin + eye_offset · U(0.7, 1.0)` (a
     random height between 70 % and 100 % of eye height, new per blast and
     victim); for other entities their world-space centre.
  3. Trace `src → T` against **world brushes and solid brush entities only**
     (solid, moveable, window, grate contents; no players, no props, no
     hitboxes). If it starts in solid, treat the end as `src`. The entity is
     hit if the trace is clear or hit the entity itself.
  4. `d = |end − src|`; `damage = D − d · D/R` (linear). If ≤ 0, nothing.
  5. Force: `|J| = min(D·300, 30000) · U(0.85, 1.15) · phys_pushscale · 1.5`
     along `normalize(end − src)`, applied at `src`. With D = 100 this is
     the 30000 cap: 38250–51750 kg·in/s.
  6. Apply as blast damage, then let the trace hit damage-triggers between
     `src` and the end.
- No damage falloff by facing; damage passes through players and props
  (they don't block). Walls block completely; thin brush walls and grates
  too (grates are in the mask).
- Applies to the thrower and teammates as for any damage (friendly fire
  rules: `mp_friendlyfire`; Q10).
- Players (*inferred*): blast damage has no hitgroup (generic, 0), so
  armour protects without a helmet. Formula for HE is Q4. Hypothesis H1: the M7 rule with the
  HE script's ratio, `health = trunc(damage · 1.475 · 0.5)`, `armour =
  trunc((damage − health_raw) · 0.5)`.
- `sv_legacy_grenade_damage` (server cvar, "replicate grenade damage
  behaviour of the original game") switches between two HE damage models;
  both are Q10.
- Physics props get the generic blast push (physics_props.md 5.3).
- Client ragdolls inside the radius get a push of
  `100 − (100/350)·dist` (only if > 1) along the direction from the blast,
  from a point 32 in below the blast (temp-entity rule).

#### 5.4 Effects and sounds

Server: `BaseGrenade.Explode` on the weapon channel (CS:S script): rndwave
`^weapons/hegrenade/explode3/4/5.wav`, volume 1, 100 dB, pitch 80–120 (the
`^` prefix is the distance-variant marker, sounds.md).

Decal: `Scorch` (`decals/scorch1_subrect`, `scorch2_subrect`, random) placed
with the ground-probe trace, i.e. only when the blast is within 24 in above a
surface; a blast in mid-air leaves no scorch.

Temp entity "explosion" (origin `B`, normal and surface material of the
probe hit when there was one) → every client in hearing range runs:

- If `B` is in water: a water-explosion effect instead (not specced here).
- **Flash overlay**: one screen-facing glow at `B`, colour (1, 0.9, 0.7),
  fading linearly to 0 over 0.1 s while growing 16 units/s.
- **Sound**: `BaseExplosionEffect.Sound` at `B` (static channel, 85 dB,
  pitch 80–110, rndwave `weapons/debris1.wav`, `debris2.wav`).
- **Blast direction** `a`: probe 100 in (the magnitude) along ±X, ±Y, ±Z
  against solid; each probe that hits at fraction `f` contributes
  `−dir · (1 − f)`; `a = normalize(sum)`, or straight up when nothing is
  hit. So the effect sprays away from nearby walls and floor.
- **Core** (force fixed at 2, spread `σ = 1 − 0.15·2 = 0.7`). Smoke sprites
  `particle/particle_noisesphere`, coloured by the world light at
  `B + 32 a` (`L` = luminosity 0–255, light colour `c`; colour =
  `c · I(L/2, L)`):
  - 4 big puffs at `B`: life `U(2, 3)`; direction `normalize(U3(−σ, σ) +
    a·U(1, 6))`, speed `U(1, 750)·2·σ·|dir·a|`; size 72 → 144; alpha
    255 → 0; roll `I(0, 360)`, roll speed `U(−2, 2)`.
  - 8 small puffs at `B + U3(−16, 16)`: life `U(0.5, 1)`; speed
    `U(1, 2000)·2·σ·|dir·a|`; size `I(32, 64)` → ×2; alpha `U(128, 255)`;
    roll speed `U(−8, 8)`.
  - Ring of 32 puffs at yaw steps of 11.25° (first at 11.25°), horizontal
    direction `h`, position `B + U3(−4, 4) + h·U(8, 16)`: life `U(0.5, 1.5)`;
    speed `U(500, 2000)·2·σ` (deviation term with itself = σ); size
    `I(16, 32)` → ×4; alpha `U(16, 32)`; roll speed `U(−8, 8)`.
  - 16 embers (`effects/fire_embers1/2`, random) at `B + U3(−32, 32)`: life
    `U(2, 3)`; direction `normalize(U3(−1.4, 1.4) + a)`, with
    `k = σ·|dir·a|`, speed `U(1, 400)·8k²`; grey `I(192, 255)`; size
    `clamp(I(8, 16)·k, 4, 32)` constant; alpha 255 → 0; roll speed
    `U(−8, 8)`.
  - 32 fireballs (`effects/fire_cloud2`, 128 × 128) at `B + U3(−48, 48)`:
    life `U(0.2, 0.4)`; direction `normalize(U3(−0.525, 0.525) + a)`,
    speed `U(400, 800)·8k²`; grey `I(128, 255)`; size
    `clamp(I(32, 85)·k, 32, 85)` → ×1.5; alpha 255 → 0; roll speed
    `U(−16, 16)`.
  - All core sprites: alpha and colour scale with `Bias(1 − t/life, 0.25)`;
    velocity decays by `0.0001^(dt/0.5)` per frame (to 1/10000 in 0.5 s)
    but never below 32 in/s; roll speed decays with factor `(1 − 8 dt)` per
    frame, never below 0.5 rad/s in magnitude.
- **Debris**: 8–16 spark streaks (`effects/fire_cloud2` trails) from `B`:
  life `U(0.1, 0.15)`, direction `normalize(U3(−1, 1) + a)`, speed
  `U(1500, 2500)`, width `U(2, 16)`, length `U(0.05, 0.1)` s of travel,
  gravity 200, velocity damping 8. And 16–32 cement flecks
  (`effects/fleck_cement1/2`) at `B + 16a + U3(−8, 8)`: life 3, size
  `I(1, 3)`, speed `U(64, 256)·(4 − size)·8·(0.8|dir·a|)²`, dark grey
  `min(1, 0.25·U(0.5, 1.5))`, bouncing (as impact_effects.md flecks:
  gravity 800).
- No dynamic light (disabled in the base effect).
- The networked fireball sprite index and scale (`sprites/zerogxplode`,
  scale radius × 0.03 = 10.5, 25 fps) are sent but the client effect above
  does not draw them. Whether CS:S's client does is Q9.

#### 5.5 Hearing effects (base player; CS:S Q11)

When a player takes blast damage: distance `r` = |player origin − grenade
origin|. If damage ≥ 30, set that player's DSP to a random preset 35–37
("shock muffle": diffusor plus a 3 kHz sine LFO at gain 0.25 = ringing; mix
0.2–0.7; 1.6 s, then an exponential fade back). Else if `r < 240`, a random
preset 32–34 ("explosion ring": diffusor plus a 1 kHz low-pass at gain
0.25; 1.6 s, exponential fade). CS:S's `dsp_presets.txt` also has presets
137–139, commented as the HE grenade shock effect (ring plus low-pass at
4000, 2000, 1000 Hz, durations 1, 1.5, 3 s, ring gain 0.01–0.03); CS:S may
use those instead (Q11).

### 6. Flashbang (CS:S code not public; data + hypotheses)

#### 6.1 Detonation

- Fuse: as HE (1.5 s, first check at 1.56 s), *inferred* (Q8).
- `Flashbang.Explode` (voice channel, volume 1, 85 dB, pitch 70–130,
  rndwave `^weapons/flashbang/flashbang_explode1/2.wav`).
- Game events: `flashbang_detonate` (userid, x, y, z) once, then
  `player_blind` (userid) for each blinded player (data).
- No damage.

#### 6.2 Who is blinded (game text + hypothesis)

From the game's own tutor text: it "temporarily blind[s] everyone in the
area who can see them when they explode ... this includes your teammates and
even yourself", and "turning your view away from a flashbang grenade lessens
its blinding effects". An achievement exists for flashing teammates
(`cause_friendly_fire_with_flashbang`).

Hypothesis model to fit (Q12):

- For each living player: line of sight from the grenade to the player's
  eye, blocked by world brushes (whether players, props or smoke block is
  Q12).
- `r` = distance grenade → eye; `k = dot(view forward, normalize(grenade −
  eye))` (1 = looking straight at it, −1 = facing away).
- Two numbers result per player, both networked on the CS player and
  readable by a server plugin: `m_flFlashMaxAlpha` (0–255, peak whiteness)
  and `m_flFlashDuration` (seconds) (netprop names to confirm with
  `sm_dump_netprops`). Hypothesis: both fall with `r` up to a
  maximum radius, and facing away (`k` below some threshold) reduces them
  (the tutor says "lessens", not "prevents"). Fit
  `duration = F(r)·G(k)` and `max_alpha = A(r, k)` from the measurement grid
  in Q12.
- A new flash only raises the current values, never shortens a stronger
  ongoing blindness (hypothesis).

#### 6.3 What the blinded player sees (data + hypothesis)

Two CS:S materials exist for it:

- `effects/flashbang_white`: unlit, additive, plain white.
- `effects/flashbang`: unlit, additive, base texture = the full-frame
  framebuffer copy (`_rt_FullFrameFB`), gamma-correct read, linear write.

Inferred look: at the flash, the current frame is captured; for the
blindness, a full-screen white additive quad at the current flash alpha is
drawn over everything, plus the captured frame added on top (a frozen,
over-bright after-image) that fades with it. Hypothesis for the curve
(Q13): alpha stays at `max_alpha` for most of the duration, then fades
linearly to 0 over the last part (a few seconds). The tutor message
`Cstrike_Tutor_You_Are_Blind_From_Flashbang` shows for new players.

#### 6.4 Hearing (data; mapping Q14)

`dsp_presets.txt` has three presets commented "flashbang muffle": 134 long
(mix 0.2–0.7, 1.6 s, exponential fade), 135 medium (mix 0.2–0.4, 0.2 s),
136 short (mix 0.2–0.2, 0.1 s); each a 2-delay diffusor plus a 3000 Hz sine
LFO at gain 0.05 (the ringing). No ringing wave file exists in the install,
so the ring is this DSP. Hypothesis: the blinded player gets one of them
picked by blindness strength. While a DSP preset runs, all sounds that
player hears are muffled and ring.

### 7. Smoke grenade

#### 7.1 Detonation (CS:S server not public; Q15)

Hypothesis: after a 1.5 s fuse, at each 0.2 s check the grenade pops once
its speed is (near) zero; then the server creates the smoke cloud entity
(`env_particlesmokegrenade`) at the grenade's position, puts it in "fill"
mode at once, fires `smokegrenade_detonate` (userid, x, y, z) and plays
`BaseSmokeEffect.Sound` (static channel, volume 1, 85 dB,
`^weapons/smokegrenade/sg_explode.wav`). The thrown grenade model stays on
the ground (hypothesis) and is removed later.

#### 7.2 Cloud lifetime

The cloud entity sends its spawn time, a fade start (default 17 s) and fade
end (default 22 s) after its spawn, and its stage (trail / fill). `t` =
now − spawn time:

```math
\text{fade}(t) = \begin{cases} 1 & t < 17 \\ \tfrac12 + \tfrac12\cos\!\left(\pi\,\tfrac{t-17}{22-17}\right) & 17 \le t < 22 \\ 0 & t \ge 22 \end{cases}
```

```math
E(t) = 240 \cdot \sin\!\left(\tfrac{\pi}{2}\min(t, 1)\right), \qquad \text{global\_alpha}(t) = \text{fade}(t) \cdot \frac{E(t)}{240}
```

Whether CS:S changes the fade times is Q15 (community: "about 20 s").

#### 7.3 Cloud look (client)

- On fill: base point `C` = the entity's position (re-read every frame, so
  a moving entity drags the cloud). 64 sprites on a 4 × 4 × 4 grid at
  local offsets `{−120, −40, 40, 120}` per axis. No sprite is culled for
  being inside walls (the cloud pokes through thin walls).
- Per sprite: grey interpolation `g = I(0, 255)/255.1`; roll `U(−6, 6)` rad;
  roll speed `U(−0.1, 0.1)` rad/s; light colour `ℓ` = the world lighting at
  its grid point (0–1 per channel, sampled once at fill); per-sprite fade 1.
- **Churn**: each frame, every sprite not swapping picks a random neighbour
  (one of 26, starting from a random offset per axis) that is not swapping,
  and the pair swaps places over `U(10, 20)` s: positions, light colours and
  fades blend with `w = ½ + ½ cos(π·τ/T)` (from 1 to 0); at the end they
  exchange grid slots.
- **Render** each sprite at local position `q` (`ℓ = |q|`):
  - not drawn if `ℓ > E(t)`;
  - drawn at `C + q` if `ℓ ≤ E/2`, else at `C + q·(E/2)/ℓ` (outer sprites
    hug the growing sphere's half radius);
  - alpha `a = 1 − ℓ/E`; `a = 1` if `a > 0.3` else `a/0.3`; times the
    global alpha, the sprite's fade, and a near-camera ramp from 0 at depth
    0 to 1 at depth 10 in;
  - colour `= ((0.5 + 0.1 g)·ℓ + dynamic light) desaturated: (c + 0.5)/2`.
    Dynamic lights (e.g. muzzle flashes) touching the cloud add
    `colour · (1 − d²/(r+80)²) · 0.1`, renormalized if a channel exceeds 1;
  - size 80 (half-width), sorted back to front, material
    `particle/particle_smokegrenade1` (texture `particle_smokegrenade_thick`,
    64 × 64, alpha-blended, vertex colour and alpha).
- With E = 240 the grid's corner sprites (`ℓ` = 207.85) are drawn at radius
  120 with alpha `(1 − 207.85/240)/0.3 = 0.447`; edge sprites (174.36) at
  0.912; face (132.66) and inner (69.28) sprites at 1.
- A "trail" stage (sprites following a moving entity: 40 per second, life
  0.5 s, size 3 → 10, colour = half the world light) exists for a cloud
  entity that follows something; CS:S probably fills at once (Q15).

#### 7.4 Screen fog inside smoke (client)

Each frame the overlay alpha starts at 0; every filled cloud adds, with `d`
= distance from the camera to `C`:

- `d < 0.3E`: + global alpha;
- `0.3E ≤ d < E`: + `(1 − (d − 0.3E)/(0.7E)) · global alpha`.

Then a full-screen quad, colour (0.3, 0.3, 0.3), alpha = clamp(sum, 0, 1),
material `particle/screenspace_fog` (translucent, vertex colour/alpha, no
depth test) is drawn after the view model and before the screen fade.
Several clouds add up.

#### 7.5 Vision blocking (CS:S server; Q16)

The smoke has no collision for bullets or players. The client keeps a list
of the local player's visible smoke clouds (CS:S-only branch), presumably to
hide the target-ID name of players behind smoke. Bots treat smoke as
blocking their vision (bot code not public). Hypothesis: a sight line is
blocked if the length of it inside any active cloud sphere (radius ≈ the
current `E`, or a fixed radius) exceeds a threshold. Q16.

### 8. Third-person gestures

See animation.md §12 (layer 7) and its CS:S data notes: while the pin is
pulled or a throw is pending, the grenade layer plays the prime sequence
(`{Move}_Shoot_GREN1`, held at its end); when a throw is pending and the
prime has finished, it plays the throw sequence (`{Move}_Shoot_GREN2`) once.
The weapon suffix is forced to the grenade one for the throw, because the
thrower may already hold another weapon when the event arrives.

### 9. Death with a primed grenade (Q17)

The template has a "drop" path that spawns the projectile at eye + 16·view
forward with only the player's velocity, as a normal live grenade with a
full fuse, and marks the weapon spent. Hypothesis for CS:S: a player who dies
with the pin pulled (or between release and spawn) drops a live grenade this
way; otherwise the unthrown grenade is lost. Grenades already in flight keep
their thrower and still damage/score after his death (the
`dead_grenade_kill` achievement exists).

## Per-tick order

Server, one tick:

1. Players' user commands (movement, then weapon post-think): pin pull,
   release detection, throw timer, spawning projectiles (sections 2, 3).
2. Entities simulate; for each grenade projectile: its 13-tick check (fuse,
   world bounds, water slowdown) → ground handling → gravity and sweep →
   contact resolution with bounce sound → water transition (section 4).
3. A detonation runs entirely inside the check of step 2: ground probe,
   effects temp entity, damage to all victims in entity-iteration order,
   decal, sound, removal on the next think.

Client, per frame: interpolate the projectile; smoke clouds update
(expansion, churn, fade, light list), then add their fog contribution;
render world, view model, smoke fog overlay, screen fade, flash overlay
(order of the flash overlay relative to the fog is Q13).

## Edge cases

- Thrower: the projectile never collides with its thrower, so throwing while
  running forward or jumping cannot hit yourself.
- Facing a wall flush: the spawn point is exactly at the wall (eye-to-wall =
  16 for a 32-wide hull at 0° yaw); the 4-unit box can start in solid. In the
  template the first sweep then reports all-solid and the grenade stays put
  and detonates there (Q6).
- Players' elasticity 0.135: a grenade hitting a player nearly drops dead at
  his feet.
- Breakable glass: the grenade breaks `func_breakable_surf`/`func_breakable`
  with 10 damage when that is enough, and flies on at 40 % speed.
- A grenade resting on a moving brush (lift) stays "grounded" to it and is
  carried; if the ground moves away it falls again.
- HE through floors: the ground probe only looks 24 in down; a blast in
  mid-air has no scorch and its damage origin is the grenade centre + 1 in.
- HE in water: damage only to entities that are themselves in water; dry
  blast does not reach fully submerged players.
- HE body-target randomness: a player standing exactly at the blast takes
  between 82.2 (target at eye) and 87.7 (target at 70 % eye height) with
  this formula, never 100; if CS:S kills full-health unarmoured players at
  point blank, its formula differs (Q10).

## Quirks

- **Upward throw bend**: aiming level throws 10° upward; aiming straight up
  (signed pitch −89) bends past vertical to −108.9°, so the grenade flies
  slightly **backwards** at 750 in/s. Keep if Q5 confirms the signed form.
- **Running throws** add the full player velocity: running at 250 adds
  ~42 % range. The game's tutor text tells players to do it.
- **No rolling**: grenades stop after a few bounces because of the 0.45
  restitution on every contact and the 30 in/s stop rule. Keep.
- **Lost tick fraction on walls**: after a wall/ceiling contact the
  remaining movement of that tick is dropped. Keep (it changes trajectories
  by at most one tick).
- **Fuse quantization**: the 0.2 s check (13 ticks) turns 1.5 s into
  1.56 s. Keep.
- **Smoke through walls**: no cloud sprite is culled against walls, so the
  cloud shows on the far side of thin walls. Keep.

## Test cases

| Setup | Input | After | Expected |
|---|---|---|---|
| G1 throw, standing still, pitch 0, yaw 0, eye at (0, 0, 64) | release | spawn | p′ = −10; speed 600; spawn (15.757, 0, 66.778); velocity (590.88, 0, 104.19) |
| G2 throw, pitch −22.5 (up) | release | spawn | p′ = −35; speed 750 (exactly the cap); velocity (614.36, 0, 430.18) |
| G3 throw, pitch −45 | release | spawn | p′ = −60; 6·150 = 900 → 750; velocity (375, 0, 649.52) |
| G4 throw, pitch −89 | release | spawn | p′ = −108.889; velocity (−242.8, 0, 709.61) (backwards); spawn offset (−5.18, 0, 15.138) |
| G5 throw, pitch +45 (down) | release | spawn | p′ = 40; speed 300; velocity (229.81, 0, −192.84) |
| G6 throw, pitch 89 | release | spawn | p′ = 88.889; speed 6.667; velocity (0.13, 0, −6.67); spawn offset (0.31, 0, −15.997) |
| G7 throw, pitch 0, running (250, 0, 0) | release | spawn | velocity (840.88, 0, 104.19) |
| G8 flight from G1, no contact | – | 0.3256 s | apex: vz = 0, height +16.96 in above spawn (104.19²/640) |
| G9 flight, 20 ticks from (0,0,0) with (600, 0, 300) | – | 20 ticks | x = 180, z = 300·0.3 − 160·0.09 = 75.6, vz = 300 − 96 = 204 |
| G10 floor hit, n = (0,0,1), v = (590.88, 0, −200) | contact | – | v = (265.90, 0, 90.00), keeps moving (|v| ≈ 280.7 ≥ 30) |
| G11 wall hit, n = (−1,0,0), v = (500, 0, 0) | contact | – | v = (−225, 0, 0) |
| G12 player hit, n = (−1,0,0), v = (500, 0, 0) | contact | – | v = (−67.5, 0, 0) |
| G13 floor hit, v = (40, 0, −50) | contact | – | reflected ×0.45 = (18, 0, 22.5), \|v\| = 28.8 < 30 → rests, velocity 0, lying flat, random yaw |
| G14 wall hit, v = (−50, 0, 0), n = (1, 0, 0) | contact | – | 22.5 < 30 → velocity 0, no ground, falls next tick |
| G15 glass, v = (500, 0, 0), pane with 5 hp | contact | – | pane takes 10 club damage, breaks; v = (200, 0, 0); no bounce |
| G16 fuse ticks, spawn on tick 1000 | – | – | checks at 1000, 1013, …, 1091, 1104; detonates on tick 1104 (1.56 s) |
| G17 release on tick 500 | – | – | projectile spawns on tick 507 (curtime 7.605 > 7.6); HE detonates on tick 611 |
| G18 HE on flat ground, grenade at rest (origin 2 in up), standing victim 200 in away horizontally, eye offset 64 | detonate | – | blast at floor + 0.6; src at floor + 1.6; target height U(44.8, 64); d ∈ [204.61, 209.51]; damage ∈ [40.14, 41.54] (no armour) |
| G19 HE, victim behind a 4-in brush wall at 100 in | detonate | – | trace blocked → 0 damage |
| G20 HE, another player stands between blast and victim | detonate | – | not blocked; damage as if open |
| G21 HE, victim 400 in away | detonate | – | not in the 350 sphere (unless its box reaches it) → none; at d = 350 exactly damage 0 |
| G22 HE force on a victim at d = 100 | detonate | – | \|J\| = 30000·U(0.85, 1.15)·1.5 ∈ [38250, 51750] |
| G23 base-player hearing, blast damage 35 | hurt | – | DSP preset 35, 36 or 37 |
| G24 base-player hearing, damage 20, 200 in away | hurt | – | DSP preset 32, 33 or 34; at 250 in: none |
| G25 smoke radius | t = 0, 0.25, 0.5, ≥ 1 s | – | E = 0, 91.84, 169.71, 240 |
| G26 smoke fade | t = 17, 18, 19.5, 21, 22 s | – | fade = 1, 0.9045, 0.5, 0.0955, 0 |
| G27 smoke sprite alpha, E = 240, fade 1 | corner / edge / face / inner sprite | – | 0.447 / 0.912 / 1 / 1; corner drawn at radius 120 |
| G28 smoke fog, E = 240, global alpha 1 | camera 50 / 120 / 200 / 260 in from C | – | +1 / +0.714 / +0.238 / 0 |
| G29 two clouds, camera at both cores | – | – | overlay alpha clamps to 1 |
| G30 pin pull | press attack | – | `pullpin` 0.9512 s; pin sound at 0.6585 s |
| G31 buy limit | own 2 flashbangs, buy flashbang | – | refused, money unchanged |

## Open questions

Checks below use the dedicated probe server (`tools/css_probe`, SourceMod;
per-tick logging of netprops and events) unless they need the client.

1. **Q1 Release timing.** Projectile spawn at release + 0.1 s (template) or
   at the throw animation's event (0.3333 s)? Does a tap (press then release
   on the next tick) throw immediately or after the pin animation? Check:
   bot presses attack for 1 tick / 30 ticks / 70 ticks then releases; log the
   tick each `*_projectile` entity is created (OnEntityCreated) and its first
   origin and velocity, plus the weapon's `m_flNextPrimaryAttack` per tick.
2. **Q2 Redraw.** With a second flashbang: does the same grenade redraw
   (draw 0.6667 s) or does the player switch to the best weapon? Log active
   weapon and next-attack times per tick after the first throw.
3. **Q3 Switching with the pin pulled.** Is a manual switch allowed, and is
   the grenade kept or thrown/dropped? Probe: `use weapon_knife` while the
   pin is pulled.
4. **Q4 HE vs armour.** Victim with 100 armour (with and without helmet) at
   several distances; log `player_hurt` dmg_health / dmg_armor and hitgroup.
   Fit against H1 (ratio 1.475 × 0.5) and alternatives (ratio 0.5 × 0.5, or
   no armour effect).
5. **Q5 Pitch form.** Throw at pitch −89, −60, −30, 0, 30, 60, 89 (set eye
   angles on the bot); log spawn origin and velocity. Confirms the bend, the
   cap and the backwards throw.
6. **Q6 Spawn in a wall.** Bot flush against a wall facing it, pitch 0;
   does the grenade appear on the far side, stick in the wall, or bounce
   back? Log origin per tick.
7. **Q7 Bounce sounds.** `sv_soundemitter_trace 1` and the `grenade_bounce`
   event while a grenade bounces and slides; count emits per contact.
8. **Q8 Fuse.** Ticks from spawn to `hegrenade_detonate` /
   `flashbang_detonate` (expected 104).
9. **Q9 HE visuals.** Screenshot sequence of an HE blast on the client
   (Windows PC, `host_timescale` 0.1 in a listen server): fireball sprite or
   only the particle core above? Screen shake? Dynamic light?
10. **Q10 HE damage model.** Unarmoured standing victim, clear line, at
    d = 0, 50, 100, 150, 200, 250, 300, 340 in from the blast (log the
    `hegrenade_detonate` point and the victim's origin; repeat 10× each for
    the random target height); both `sv_legacy_grenade_damage` 0 and 1;
    crouched victims; teammate and self damage with `mp_friendlyfire` 0/1.
    Fit linear `100 − d/3.5` against other shapes (e.g. a bell curve). Also
    confirm walls block, players and props don't.
11. **Q11 HE hearing.** Which DSP presets a hurt player gets (32–37 or
    137–139): the client console with `dsp_player` echo or `snd_show`; and
    whether undamaged nearby players get the ring.
12. **Q12 Flash amount.** Grid: victim at r = 0, 100, 250, 500, 750, 1000,
    1500, 2000, 3000 in, view angle to the grenade 0°, 30°, 60°, 90°, 120°,
    180°; log `m_flFlashDuration` and `m_flFlashMaxAlpha` on the tick after
    `flashbang_detonate`, and whether `player_blind` fires. Repeat with a
    wall between, a player between, a prop between, and smoke between. Same
    for the thrower and a teammate.
13. **Q13 Flash overlay curve.** On the client, screenshots every 0.1 s
    after a full flash (alpha 255, known duration) to fit the hold-and-fade
    curve and the after-image's alpha; check draw order with smoke fog.
14. **Q14 Flash hearing.** Which of DSP 134/135/136 a blinded player gets
    and how it maps to the blindness.
15. **Q15 Smoke timing.** Ticks from spawn to `smokegrenade_detonate` for a
    grenade that stops early, one that is still rolling at 1.5 s, and one
    resting on a moving lift; the cloud entity's fade start/end netprops;
    the stage netprop; when the projectile is removed; whether a trail
    shows in flight.
16. **Q16 Smoke vs bots.** Bot behind a cloud centre at offsets 0, 100,
    200, 250 in from the line of sight; does it see/fire at a visible
    target? Also the client target-ID through smoke.
17. **Q17 Death with a primed grenade.** Kill a bot (`sm_slay`) while its
    pin is pulled, and between release and spawn; log new projectiles. Also:
    which grenades drop as pickups on death, and can grenades be dropped with
    the `drop` command?
