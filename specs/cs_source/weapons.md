# Counter-Strike: Source: weapons, bullets, hitboxes and melee

Source basis: Valve's public Source SDK 2013. I read the shared weapon base
(lifecycle, attack gating, reload, deploy and holster), the shared player
command and think order, the weapon script parser, the shared bullet-firing
routine and its spread helper, the shared-random and command-seed helpers,
damage-info and damage-force helpers, the generic player and character damage
paths (hitgroup scaling, damage accumulation), the hitbox and model header
layouts and the animating entity's hitbox ray test, weapon drop and pickup on
the player and character, view punch and its decay, the HL2MP weapon bases
(machine gun, shotgun, melee and crowbar) and the SDK's template mod, which is
a stripped-down CS-derived example (its bullet routine, MP5, shotgun, melee
base and ammo table). CS:S's own game DLL (its weapons, firing, inaccuracy,
recoil, penetration, armour and knife) is **not** in the SDK; everything
CS:S-specific is either data read from the CS:S install (weapon scripts and
model files, values only) or listed as a hypothesis with a measurement plan.
The vphysics engine, the engine trace code (including the hitbox ray-box
test) and the random number generator body are not in the SDK either.
Status: draft

## Summary

- A weapon is a state machine driven once per user command, after movement.
  Every action (fire, reload, deploy, swing) sets "next primary/secondary
  attack" times; the player also has a "next attack" time that blocks all
  weapon logic. Most of those times are either a fixed per-weapon number
  (`CycleTime` from the script) or the **duration of a view-model animation
  sequence** read from the `.mdl`. Draw and reload times in CS:S come from the
  view model (e.g. AK-47 draw 1.0 s, reload 2.4324 s).
- A shot is a straight trace from the eye along the view angles (plus the
  punch angle in the CS family), bent by a random offset seeded from the user
  command so client and server agree. What it hits is decided per hitbox: a
  model's hitbox set is a list of oriented boxes on bones, each tagged with a
  hitgroup (head, chest, stomach, arms, legs). Damage is scaled by hitgroup,
  then by game rules (in CS:S: distance falloff `RangeModifier^(d/500)`,
  armour, penetration).
- What makes CS:S feel like CS:S (inaccuracy growth while moving/jumping,
  recoil pattern and its decay, penetration, armour, knife damage) lives in
  code we cannot read. The weapon scripts give all the per-weapon numbers
  (tables at the end); the formulas around them have to be measured on our
  dedicated server.

## Units and conventions

- Distance: inches ("units"), Source axes: X forward, Y left, Z up.
- Angles: degrees; pitch positive looks **down**; punch angles are added to
  view angles component-wise (pitch, yaw, roll).
- Time: seconds. Game time during a user command is
  `curtime = tickbase × tick_interval`. CS:S servers run
  `tick_interval = 0.015 s` (66.67 Hz; `-tickrate` is ignored). All weapon
  timers are absolute game times compared with `curtime`, so every event is
  quantised to ticks.
- Mass kg, impulse kg·in/s (same as specs/cs_source/physics_props.md).
- "Spread" values in the generic code are offsets along the unit right/up
  vectors, roughly sin(half-angle) of the cone (e.g. the generic "3 degree
  cone" is 0.02618 ≈ sin 1.5°).

## Constants

### Generic (SDK shared code)

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| tick | 0.015 | s | CS:S server tick |
| max_trace_length | 56755.84 | in | √3 × 32768; generic bullet trace length |
| bullet_default_distance | 8192 | in | default shot length when a weapon gives none |
| tracer_freq_default | 4 | shots | one tracer every N shots (base weapon uses 2) |
| empty_sound_interval | 0.5 | s | dry-fire click at most this often |
| underwater_refire | 0.2 | s | delay after trying to fire a non-underwater weapon underwater (eyes under water) |
| autoswitch_delay | 0.3 | s | next primary attack after auto-switching away from an empty weapon |
| seed_mask | 255 | – | the seed used for shots is the command's random seed & 255 |
| pickup_bloat | 36 | in | dropped weapon's touch box grows 36 in on X and Y, 18 in up, 0 down |
| drop_height | −12 | in | player-dropped weapon starts 12 in below the eye |
| drop_speed | 400 | in/s | player drop throw speed along the view direction (capped at 400 when given) |
| drop_spin | (200, 200, 200) | deg/s (angular impulse) | added to a dropped weapon's physics body |
| pickup_delay | 1.0 | s | dropped weapon becomes touchable after this |
| dropped_remove_mp | 30 | s | generic multiplayer: dropped (non-respawning) weapon removed after this, counted from when it becomes touchable |
| playback_rate_cap | 12 | × | max animation speed-up when an activity is forced to a duration |
| sk_player_head | 2 | × | generic (HL2) hitgroup multiplier, cvar |
| sk_player_chest | 1 | × | cvar |
| sk_player_stomach | 1 | × | cvar |
| sk_player_arm | 1 | × | cvar |
| sk_player_leg | 1 | × | cvar |
| ai_shot_bias_min / max | −1 / 1 | – | generic spread-shape cvars (replicated) |
| shotgun_hull_pellet | ±3 | in | generic: every odd pellet of a multi-pellet player shot is a 6×6×6 box trace |
| blood_offset | 4 | in | generic: blood spawns 4 in back along the shot from the hit point |
| bullet_force_scale | phys_pushscale (1) | × | cvar multiplying all damage forces |
| melee_force_per_damage | 75 × 4 = 300 | kg·in/s per hp | generic melee push |
| melee_hull | ±16 | in | generic melee fallback hull |
| melee_facing_cos | 0.70721 | – | hull hit only counts if target origin is within ≈45° of the view |
| hl2mp_crowbar_range / refire / damage | 75 / 0.4 / 25 | in / s / hp | HL2MP example |
| template_melee_range | 32 (crowbar 64) | in | SDK template example |
| punch_damping | 9 | 1/s | punch spring damping (see movement.md) |
| punch_spring | 65 | 1/s² | punch spring constant |
| punch_impulse_scale | 20 | (deg/s)/deg | a view punch of A degrees adds 20·A deg/s to the punch velocity |
| lag_comp_max | 1.0 | s | `sv_maxunlag`; server rewinds other players up to this far for hit tests |
| lag_comp_teleport | 64 | in | a player moved further than this between records is not rewound past it |

### CS-derived template (public SDK; strong hints for CS:S, verify)

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| tmpl_range_falloff_base | 0.85 | – | template: damage × 0.85^(distance/500) (CS:S uses the script's `RangeModifier` instead, hypothesis) |
| tmpl_falloff_step | 500 | in | distance unit of the falloff exponent |
| tmpl_max_range | 8000 | in | template bullet length (CS:S uses the script's `Range`, hypothesis) |
| tmpl_spread_sum | U(−0.5,0.5)+U(−0.5,0.5) | – | per-axis spread factor (triangular on [−1, 1]) |
| tmpl_shotgun_spread | 0.0675 | – | template pump shotgun spread |
| tmpl_shotgun_aim_punch | 2 × punch | deg | template shotgun aims along view + 2 × punch |
| tmpl_shotgun_cycle | 0.875 | s | template shotgun refire (CS:S script: 0.88) |
| tmpl_shotgun_punch_ground | 4–6 (int) | deg pitch up | added to punch per shot on ground |
| tmpl_shotgun_punch_air | 8–11 (int) | deg pitch up | in the air |
| tmpl_shotgun_reload_start | 0.5 | s | before the first shell |
| tmpl_shotgun_reload_insert | 0.45 | s | per shell |
| tmpl_shotgun_idle_after_shot | 2.5 (0.875 if empty) | s | |
| tmpl_rifle_spread | 0.01 ground / 0.05 air | – | template MP5 spread (CS:S replaces with its inaccuracy model) |
| tmpl_rifle_idle | 5 | s | template idle animation delay after a shot |
| tmpl_50ae_impulse | 2400 | kg·in/s | commented CS line in the template ammo table; splash size 10–14 |

## Behavior

### 1. Weapon script data

Each weapon `weapon_<name>` has a key-value script `scripts/weapon_<name>.txt`
(in CS:S stored encrypted as `.ctx` in the VPK; we decrypt at runtime from the
user's install, never commit the key). The generic parser reads:

- `printname`, `viewmodel`, `playermodel` (world model), `anim_prefix`,
  `bucket` (HUD slot), `bucket_position`.
- `clip_size` (default −1 = no clip: ammo is used straight from the reserve),
  `clip2_size` (−1), `default_clip` (default = `clip_size`),
  `default_clip2`, `weight` (auto-switch priority), `item_flags` (default 8 =
  "limit in world") and the named flag keys (`ITEM_FLAG_EXHAUSTIBLE` 1 sets
  bit 16: the weapon is removed when its ammo runs out, used by grenades),
  `autoswitchto`/`autoswitchfrom` (1), `BuiltRightHanded` (1),
  `AllowFlipping` (1), `MeleeWeapon` (0), `primary_ammo`/`secondary_ammo`
  ("None" = no ammo type), `SoundData` (sounds by category, e.g.
  `single_shot`, `special1..3`).
- CS:S adds (parsed by its own DLL; values listed in the tables at the end):
  `MaxPlayerSpeed`, `WeaponType`, `FullAuto`, `WeaponPrice`,
  `WeaponArmorRatio`, `CrosshairMinDistance`, `CrosshairDeltaDistance`,
  `Team`, `PlayerAnimationExtension`, `MuzzleFlashScale`, `MuzzleFlashStyle`,
  `CanEquipWithShield`, `Penetration`, `Damage`, `Range`, `RangeModifier`,
  `Bullets`, `CycleTime`, `AccuracyDivisor`, `AccuracyOffset`,
  `MaxInaccuracy`, `AccuracyQuadratic`, `TimeToIdle`, `IdleInterval`,
  `Spread`, `InaccuracyCrouch/Stand/Jump/Land/Ladder/Fire/Move`, the same
  eight with an `Alt` suffix, `RecoveryTimeCrouch/Stand`, `SilencerModel`,
  `AddonModel`, `DroppedModel`, `shieldviewmodel`.
- The `Alt` set exists exactly for the ten weapons with a second mode:
  silencer (`weapon_usp`, `weapon_m4a1`), burst (`weapon_glock`,
  `weapon_famas`) and scope (`weapon_aug`, `weapon_sg552`, `weapon_scout`,
  `weapon_awp`, `weapon_g3sg1`, `weapon_sg550`). Inference: the Alt values
  apply while that mode is on.

### 2. Who runs weapon logic, and when

Per user command, in order (server; the client predicts the same):

1. `curtime = tickbase × tick`; the shared random seed for this command is set
   from the command (section 4.2).
2. A weapon selection carried in the command switches weapons now
   (section 3.6).
3. Button state is updated (pressed / released edges this command).
4. **Pre-think**: other carried weapons get their "holstered" frame; if
   `curtime ≥ player_next_attack`, the active weapon's pre-frame runs (it only
   advances transition animations).
5. Movement (punch angle decays here first, see movement.md).
6. **Post-think**: if `curtime < player_next_attack` the weapon gets only a
   "busy" frame (nothing fires, reloads or idles); otherwise the weapon's
   main frame runs (3.x below). Shots therefore start from the **post-move**
   eye position and use the punch angle already decayed this tick.
7. Tickbase +1.

Three timers gate everything: `player_next_attack` (blocks the whole frame),
`next_primary` and `next_secondary` (per weapon). A weapon picked up or
equipped has both per-weapon timers set to `curtime`.

### 3. Lifecycle (generic shared weapon base)

#### 3.1 Main weapon frame

Order inside one frame:

1. Fire-duration counter: +tick while attack is held, else 0.
2. Reload completion check (3.4) for clip weapons.
3. **Secondary attack has priority.** If attack2 is held:
   - if it uses secondary ammo and the reserve is 0: click (at most every
     0.5 s) and `next_secondary = curtime + 0.5`;
   - else if eyes are under water and the weapon can't alt-fire under water:
     click, `next_secondary = curtime + 0.2`, **end the frame**;
   - else if `next_secondary ≤ curtime`: do the secondary attack. (It may
     block the primary attack this frame; whether it does is per weapon.)
4. Primary attack if attack is held, not blocked by step 3, and
   `next_primary ≤ curtime`:
   - not melee and the clip is ≤ 0 (or, clipless, the reserve is ≤ 0):
     **fire on empty** (3.5);
   - else under water and not allowed: click, `next_primary = curtime + 0.2`,
     end the frame;
   - else: if attack was **pressed this command** (or attack2 released this
     command) set `next_primary = curtime` (drops any accumulated lag, see
     3.2), then do the primary attack.
5. If reload is held, `next_primary ≤ curtime`, the weapon uses a clip and is
   not already reloading: start a reload (3.4) and zero the fire duration.
6. If none of attack, attack2, reload are held: try the auto-reload /
   auto-switch rule (3.5); if that did nothing and not reloading, idle (3.7).

#### 3.2 Fire timing

Two patterns exist in public code; which one CS:S uses must be measured
(test cases T6/T7):

- **Accumulate** (generic base and HL2MP machine guns): while
  `next_primary ≤ curtime`, add one shot and `next_primary += CycleTime`.
  Shots per tick can exceed 1 if the timer fell behind; the long-run rate is
  exactly `1/CycleTime`. A fresh press resets `next_primary` to `curtime`
  first, so a press never fires a burst of stored shots.
- **Set** (CS-derived template): one shot, then
  `next_primary = next_secondary = curtime + CycleTime`. Held fire then
  happens every `ceil(CycleTime / tick)` ticks (exact multiples can come out
  one tick longer from float rounding). AK-47 (0.1 s): every 7 ticks = 0.105
  s (571 rounds/min instead of 600).

Shots never exceed the clip: shots fired = min(shots due, clip), clip −=
shots.

#### 3.3 Deploy (draw)

When a weapon becomes active:

1. Refused if it has no ammo at all and allows auto-switching away (grenades,
   empty guns); refused for a dead owner.
2. The view model is set to the weapon's `viewmodel`, the draw activity
   (`ACT_VM_DRAW` in the model) is played, and
   `player_next_attack = next_primary = next_secondary = curtime + D`,
   where `D` = duration of the chosen draw sequence (section 3.8).
3. Deploy sound, weapon becomes visible.

CS:S draw durations (from the view models): most 1.0 s; exceptions in the
timing table. Whether CS:S keeps exactly this rule (e.g. some games allow
firing slightly before the draw ends) is measurement M8.

#### 3.4 Reload

Start (reload key, or an empty-clip trigger):

- Refused if the reserve is 0, or the clip needs nothing
  (`min(clip_size − clip, reserve) = 0`).
- Play the reload activity (`ACT_VM_RELOAD`, or the silenced variant) and the
  third-person reload animation; with `R` = that sequence's duration:
  `player_next_attack = next_primary = next_secondary = curtime + R`; mark
  "in reload".
- The clip is **not** changed yet; the magazine is "swapped" only at the end.

Completion (checked each frame, step 2 of 3.1; note it runs only when the
player's next attack time has passed, i.e. at `curtime ≥ start + R`):

- `n = min(clip_size − clip, reserve)`; `clip += n`; `reserve −= n`.
- `next_primary = next_secondary = curtime`; not in reload.
- So a reload keeps the old clip's rounds (no ammo lost), and the weapon can
  fire on the same tick the reload completes.

Interruption: switching weapons (holster) cancels a reload with no ammo
moved. Nothing else in the generic base cancels it (attack is blocked by
`player_next_attack`).

Reload one at a time (shotguns; generic flag "reloads singly"):

- Generic form: after each `R`, if attack/attack2 is held and the clip > 0,
  stop reloading (the shot can follow); else if reserve is 0, finish; else if
  the clip is not full, move 1 round from reserve to clip and start another
  insert (`R` again); else finish and allow attacks at once.
- CS-derived template form (matches CS:S's three-sequence shotgun models):
  start sequence 0.5 s (attack blocked 0.5 s) → then every 0.45 s one shell
  goes in (the insert animation restarts each time) → when the clip is full
  or the reserve empty, play the finish sequence. Firing is allowed between
  inserts once `next_primary` has passed and interrupts the reload. CS:S's
  own M3/XM1014 timings: model durations are start 0.375 / 0.6667 s, insert
  0.4909 / 0.3889 s, finish 0.875 / 0.4 s; which numbers the game uses is
  measurement M9.

#### 3.5 Empty clip, dry fire, auto reload and auto switch

- Attack with an empty clip (not melee), first time: click (at most every
  0.5 s) and remember "fired on empty". Holding attack on later frames, with
  that flag set, runs the auto rule below immediately (so holding attack on
  an empty gun reloads it as soon as allowed).
- Auto rule (also runs every frame with no buttons held):
  - if the weapon has no ammo at all and both attack timers are in the past,
    and it isn't flagged "no auto switch when empty": switch to the next
    best weapon (game rules pick it, by `weight`); if the switch happened,
    the old weapon's `next_primary = curtime + 0.3`;
  - else, if it uses a clip, the clip is 0, it isn't flagged "no auto
    reload", and both timers have passed: reload.
  The "fired on empty" flag is cleared whenever the auto rule runs.
- The generic primary attack also starts a reload itself when called with an
  empty clip.
- A dry-fire **animation** (`ACT_VM_DRYFIRE`, present in several CS:S pistol
  models: Deagle 0.575 s, P228 0.8824 s, Five-seveN 0.6389 s, Glock 0.5714 s,
  USP 0.9375 s, Elite left/right) is chosen by CS:S code for the last round
  or an empty trigger pull: measurement M10.

#### 3.6 Switching and holstering

- Selecting a weapon you already hold that is visible and not holstered: no
  change. If it is hidden, deploy it again.
- A switch is allowed only if the target has some ammo (or ammo of its type
  in reserve), can deploy, and the current weapon can holster.
- Holster: cancels any reload, plays `ACT_VM_HOLSTER` if the model has it,
  and sets `player_next_attack = curtime + holster duration`. **CS:S view
  models have no holster sequence** (none of the 29 has one), so the holster
  time is 0 and the old weapon hides at once.
- Then the new weapon deploys (3.3). Total switch time = new weapon's draw
  duration.
- The previous weapon is remembered for "last weapon" (`lastinv`); switching
  back uses the same path.
- Pickups never auto-switch for players in the generic base (an HL2-specific
  rule does; CS:S behaviour is measurement M14).

#### 3.7 Idle

When nothing else happened and the idle timer has elapsed, play the idle
activity. Idle animations do not affect timing. CS:S scripts carry
`TimeToIdle` and `IdleInterval` (probably: idle starts this long after a
shot, and repeats every `IdleInterval`); cosmetic only.

#### 3.8 Animation durations

- Playing an activity picks a sequence of the view model tagged with that
  activity (weighted random when several, e.g. `ak47_fire1..3`), restarts it
  at cycle 0, playback rate 1.
- Sequence duration (inferred; the formula is engine code):
  `duration = (frames − 1) / fps` of the sequence's animation;
  0 if it has a single frame. Checked against CS:S data: AK-47 reload 91
  frames at 37 fps → 2.4324 s, which matches the well-known AK reload time.
- When a caller forces an activity to last `T` seconds the playback rate
  becomes `duration / T`, capped at 12.
- Animation events in view-model sequences carry timing for effects only:
  muzzle flash and brass ejection at cycle 0, sounds (e.g. AK-47 reload:
  clip-out at cycle 0.144 = 0.35 s, clip-in at 0.633 = 1.54 s), and the
  grenade release event at cycle 0.435 of the 0.7667 s throw = 0.3335 s.

#### 3.9 Dropping and picking up (generic)

Drop (player):

- Spawn point: eye position − (0, 0, 12); angles: the player's yaw (horizontal
  facing).
- Velocity: given velocity clamped to 400 in/s, or by default forward (view
  direction) × 400. With a physics body, angular impulse (200, 200, 200) is
  added too.
- The weapon stops following the player, falls under gravity, and only
  becomes touchable 1.0 s later. In multiplayer, a dropped weapon is removed
  30 s after that (generic; CS:S keeps dropped weapons for the round,
  measurement M14).

Pickup (player touches a weapon):

- Touch volume: the weapon's box grown 36 in on X and Y, 18 in on top, none
  below.
- Refused if the weapon has an owner, the player can't use it, game rules
  refuse it (CS:S: one weapon per slot), or the player's eye can't see the
  weapon (line of sight with solid geometry blocking; windows also block).
- If the player already has that weapon type: take only ammo. The weapon's
  clip is given to the reserve (up to the ammo type's max carry); if that
  took nothing it stays; if the dropped gun is empty afterwards it is removed.
- Otherwise equip: added to the inventory, not drawn, not solid; if the
  weapon is clipless it gives `default_clip` as reserve ammo; if
  `default_clip > clip_size`, the clip is filled and the excess goes to the
  reserve. `next_primary = next_secondary = curtime`.
- Newly spawned weapons start with `clip = default_clip` (clip weapons).

### 4. Bullet firing

#### 4.1 Shot origin and direction

- Origin: the eye position (`origin + view offset`, see movement.md) after
  this tick's movement.
- Direction: the eye angles. In the CS family (template) the **punch angle is
  added** to the eye angles before firing; the template shotgun adds 2 × punch.
  So recoil (punch) moves where bullets go, not only the camera. CS:S's exact
  factor per weapon: measurement M3.
- Generic HL2 code instead aims along the eye direction (or an auto-aim
  vector; auto-aim is off in multiplayer) and applies punch only to the view.

#### 4.2 Random seed shared by client and server

- Each user command carries its sequence number `n` and a random seed
  `S = H(n) & 0x7FFFFFFF`, where `H(n)` = MD5 of the 4 little-endian bytes
  of `n`, then the
  32-bit little-endian integer at digest bytes 6–9. The server recomputes it
  from the command number, so the client cannot pick its own seed.
- Shots use `seed = S & 255` (256 possible spread patterns per
  command).
- Generic: before each pellet `i`, the global uniform generator is reseeded
  with `seed + i`. The template does the same but starts at `seed + 1`
  (pellet `i` uses `seed + 1 + i`).
- Other shared randoms (e.g. punch kicks) seed the generator with
  `CRC32(S as int32 LE ‖ extra as int32 LE ‖ label bytes)`, where the
  label is a fixed per-use text and `extra` an optional integer (0 by
  default), so each use gets a different but reproducible value.
- The generator itself (header only in the SDK): a 32-entry shuffle table
  with two integers of state (Park–Miller "minimal standard" with
  Bays–Durham shuffle, i.e. Numerical Recipes `ran1`, *inferred*). Float in
  `[lo, hi)` = `lo + (hi − lo) × u`. Reproducing CS:S spread **patterns**
  exactly needs this generator bit-exact: open question Q1.

#### 4.3 Spread

Generic (HL2 family), with forward `f`, right `r`, up `u` from the aim
direction and spread vector `(sx, sy)`:

- `r`, `u`: if `f` is vertical, `r = (0, −1, 0)`, `u = (−f.z, 0, 0)`;
  otherwise `r = normalize(f × (0,0,1))`, `u = normalize(r × f)`.
- `bias = 1` (player); `shot_bias = (max − min) × bias + min` with the cvars
  `ai_shot_bias_min/max` (−1/1) → 1; `flatness = |shot_bias| / 2 = 0.5`.
- Repeat: `x = U(−1,1)·flatness + U(−1,1)·(1 − flatness)`, same for `y`; if
  `shot_bias < 0`, fold `x → ±1 − x` (inverse Gaussian-ish); until
  `x² + y² ≤ 1`.
- Direction (not normalised) `d = f + x·sx·r + y·sy·u`.
- A multi-pellet shot may flag "first pellet exact" (no spread on pellet 0).

CS template (CS family): per pellet,
`x = U(−0.5,0.5) + U(−0.5,0.5)`, `y` likewise (triangular on [−1,1], no
rejection), `d = normalize(f + x·s·r + y·s·u)` where `s` is one scalar spread
and `r`, `u` are the right/up vectors of the aim angles (with roll). CS:S's
newer model (with the `Inaccuracy*` keys) is not public; hypothesis and
measurement in section 7.

#### 4.4 The trace and what a shot hits

Generic:

- Line trace from origin to `origin + d × distance`; mask: solid, moveable,
  monster, window, debris and **hitbox** contents; the shooter is ignored.
  Distance: the weapon's (generic base: 56755.84; template: 8000; CS:S:
  script `Range`, hypothesis).
- Multi-pellet player shots (generic HL2 only): every odd pellet is a
  6 × 6 × 6 box trace instead of a line, to make shotguns easier. The CS
  template has no such rule (every pellet is a line); assume CS:S has none
  (measurement M4).
- If the trace starts in solid, it is treated as hitting at the start.
- Triggers that react to shots are hit along the ray first (e.g. breakable
  triggers).
- If it hit something (`fraction < 1`):
  - a bullet-impact sound/AI event is made;
  - water: if the end point is in water/slime and the shot started dry,
    trace again including water; if the water surface is found, a splash
    effect (size from the ammo type) plays there and, unless the shot allows
    underwater hits, the shot does not damage what is under the water
    (generic flag; CS:S: measurement);
  - damage (4.5) is applied through the hit entity's trace-attack;
  - impact effect: unless it hit water (and didn't start in water), an impact
    effect is sent with the hit point, start point, surface property and
    hitbox; the client re-traces 8 in past the hit point and places a decal
    chosen by the surface's game material (decal list
    `scripts/decals_subrect.txt`), on static props via the prop index, on
    entities (including players, studio decals) otherwise. Sky, nodraw, hint
    and skip surfaces get no decal (template). The template also skips the
    effect on teammates when friendly fire is off.
- Tracers: one every `tracer_freq` shots (counter global across shots), from
  the weapon's muzzle attachment in multiplayer, unless the shot broke glass.
- Glass: if the surface is glass on a `func_breakable` without the "no
  bullet penetration" flag, the shot continues through it (generic HL2
  rule); CS:S has its own penetration (section 7, Q7).

#### 4.5 Damage per hit (generic plumbing)

- Damage type: bullet; "always gib" if damage > 16 else "never gib" (cosmetic).
- Base damage: the shot's damage (CS:S: script `Damage`, then falloff etc.
  in CS code, section 7). Generic HL2 uses per-ammo-type player/NPC damage
  cvars instead.
- Force: `F = normalize(d) × ammo_impulse × phys_pushscale × per_shot_scale`,
  applied at the hit point (physics props: specs/cs_source/physics_props.md
  section 5.2). HL2MP's derivation: `impulse = ft_per_s × 12 ×
  grains × 0.002285/16 lb × 0.45359237 × 3.5` (e.g. 200 grain at 1225 ft/s →
  666.57). CS:S's table is not public except the template's `.50 AE` = 2400
  (Q8).
- Players: the hitgroup multiplier (section 5) is applied in the player's
  trace-attack: head × `sk_player_head`, chest × `sk_player_chest`, stomach
  × `sk_player_stomach`, left/right arm × `sk_player_arm`, left/right leg ×
  `sk_player_leg`, generic (0) and gear (10) × 1. Defaults 2,1,1,1,1. The last
  hitgroup is remembered on the victim (used for death animations etc.).
  Team damage is refused before blood if the rules say so (`mp_friendlyfire`).
  CS:S replaces this whole step with its own (section 7).
- Accumulation: damage to the same target within one firing call is summed
  (damage, force summed; position = last hit) and applied once when the
  target changes or the call ends. So all pellets of one shotgun shot that hit
  one player arrive as **one** damage event.
- Health: generic characters subtract the integer part and carry the
  fraction in an accumulator; when it reaches 1, one extra point is
  subtracted. (CS:S rounding: Q5.)
- Physics objects get the force as an impulse at the hit point
  (physics_props.md 5.1–5.2).

#### 4.6 Lag compensation

Before a player's shot or melee swing is traced on the server, other players
are moved back to where that player saw them: command time minus the
client's interpolation, at most `sv_maxunlag` (1.0 s) back, not across a
teleport > 64 in, then restored. Bots have no latency, so measurements with a
bot shooter are unaffected.

### 5. Hitboxes and hitgroups

#### 5.1 Data in the model

- A `.mdl` holds one or more **hitbox sets**; each set is a name and a list
  of hitboxes. Each hitbox: bone index, **group** (the hitgroup id),
  bone-local box min and max (3 floats each), optional name. (68 bytes per
  hitbox in the file: two ints, six floats, a name offset, eight unused ints.)
- The entity chooses which set to use (index, normally 0).
- Hitgroup ids: 0 generic, 1 head, 2 chest, 3 stomach, 4 left arm, 5 right
  arm, 6 left leg, 7 right leg, 10 gear.
- Header fields: eye position, hull and view boxes, bone count and offset,
  hitbox set count and offset (byte offsets 172 and 176 in the header).

#### 5.2 CS:S player models

All eight CS:S player models (`models/player/ct_gign|ct_gsg9|ct_sas|
ct_urban|t_arctic|t_guerilla|t_leet|t_phoenix.mdl`, version 44) have one set
called `cstrike` with the same 19 hitboxes. Six are identical; `t_guerilla`
is uniformly ≈2.9 % larger and `t_phoenix` ≈1.9 % larger. Bone → group
(boxes in bone space, inches, from `ct_urban`, rounded to 0.01; for tests
only, the game reads them from the model at runtime):

| # | Bone | Group | Min | Max |
|---|---|---|---|---|
| 0 | Pelvis | 3 stomach | (−8.30, −7.73, −5.43) | (8.30, 2.01, 6.58) |
| 1 | L Thigh | 6 left leg | (4.58, −5.38, −3.43) | (22.88, 3.78, 4.58) |
| 2 | L Calf | 6 | (−0.29, −4.30, −3.43) | (20.88, 3.15, 3.43) |
| 3 | L Foot | 6 | (−2.29, −1.14, −2.58) | (5.72, 4.58, 2.01) |
| 4 | L Toe | 6 | (−2.86, −2.81, −2.86) | (4.00, 1.20, 1.72) |
| 5 | R Thigh | 7 right leg | (4.58, −5.38, −3.43) | (22.88, 3.78, 4.58) |
| 6 | R Calf | 7 | (−0.29, −4.30, −3.43) | (20.88, 3.15, 3.43) |
| 7 | R Foot | 7 | (−2.29, −1.14, −2.01) | (5.72, 4.58, 2.58) |
| 8 | R Toe | 7 | (−2.86, −2.81, −1.72) | (4.00, 1.20, 2.86) |
| 9 | Spine1 | 3 stomach | (−9.15, −1.26, −8.01) | (4.58, 11.33, 8.01) |
| 10 | Spine2 | 2 chest | (−3.07, −3.43, −9.69) | (13.57, 10.32, 9.69) |
| 11 | Neck | 1 head | (−0.10, −4.58, −2.86) | (5.51, 2.29, 2.86) |
| 12 | Head | 1 head | (−0.42, −6.24, −3.64) | (9.36, 4.44, 3.02) |
| 13 | L UpperArm | 4 left arm | (−1.14, −2.58, −2.29) | (14.87, 2.58, 2.29) |
| 14 | L Forearm | 4 | (−1.72, −2.40, −2.40) | (13.16, 2.40, 2.40) |
| 15 | L Hand | 4 | (0.29, −2.46, −1.72) | (6.58, 1.55, 2.86) |
| 16 | R UpperArm | 5 right arm | (−1.14, −2.58, −2.29) | (14.87, 2.58, 2.29) |
| 17 | R Forearm | 5 | (−1.72, −2.40, −2.40) | (13.16, 2.40, 2.40) |
| 18 | R Hand | 5 | (0.29, −2.46, −2.86) | (6.58, 1.55, 1.72) |

Notes: the **neck counts as head**; the pelvis and lower spine are stomach;
only the upper spine box is chest. There is no "generic" box on players, so
a player hit always has a hitgroup 1–7. Player collision hull: (−13, −13, 0)
to (13, 13, 72), eye in the model header (0, 0, 73) (the game's real eye
height comes from movement code, see movement.md).

#### 5.3 How a trace picks a hitbox

- The entity's current bone transforms (from its animation this tick,
  lag-compensated for shots) place each hitbox as an oriented box:
  `world = bone_matrix × local`, scaled by the model scale.
- A ray with the hitbox content bit tests these boxes when it reaches an
  entity that uses hitbox ray tests (players and other animated models); the
  hit is the box with the **smallest entry fraction** along the ray (ray–OBB
  slab test, *inferred*: the engine routine is not public). The result
  carries the hit point, the hitbox index, its group as the hitgroup, and the
  hit bone's surface property and a hitbox surface flag.
- The ray first has to reach the entity's surrounding bounds; players'
  hitboxes (e.g. arms, head) can stick out of the 26 × 26 × 72 collision
  hull, so the surrounding bounds must cover the hitboxes, not just the hull
  (Q6).
- Whether the solid player hull itself blocks bullets that miss every hitbox:
  in Source shots pass through the hull and only hitboxes count (*inferred*
  from the shot mask including the hitbox bit and players using custom ray
  tests; measurement M12).

### 6. Melee

#### 6.1 Generic swing (HL2MP crowbar / template melee)

On attack (no ammo, no clip; fires under water):

1. Lag-compensate other players (primary attack only in HL2MP).
2. `start` = eye, `f` = view forward, `end = start + f × range`.
3. Line trace start → end (mask: shot-hull contents: solid, moveable,
   monster, window, debris, grate; owner ignored).
4. If it missed: move `end` back by `1.732 × 16 = 27.712` and sweep a
   32 × 32 × 32 box from start to that end. If the box hits an entity:
   - if `normalize(target_origin − start) · f < 0.70721` (target origin more
     than ≈45° off the view), treat as a miss;
   - otherwise refine the hit point: re-trace a line to twice the box hit
     distance; if that misses, trace lines to the 8 corners of the box placed
     at that doubled end point and keep the nearest hit.
5. Miss: play the miss activity (`ACT_VM_MISSCENTER`, or `…MISSCENTER2` for a
   secondary swing); splash if the full-range line enters water.
6. Hit: damage `D` of type "club" along the view direction, with melee force
   `D × 300 × phys_pushscale` kg·in/s (75 kg × 4 in/s per point); the
   target's trace-attack applies hitgroups as for bullets; impact effect (or
   water splash). Hit activity `ACT_VM_HITCENTER`.
7. `next_primary = curtime + refire`; `next_secondary = curtime + duration
   of the sequence just played`.

HL2MP crowbar: range 75, refire 0.4 s, damage 25, view punch pitch U(1,2)
and yaw U(−2,−1) degrees (shared random). Template melee: range 32
(template crowbar 64), damage = script `Damage`.

#### 6.2 What CS:S's knife needs

CS:S's knife has two attacks with different animations. Data we have:

- Script: `Damage` 50 (probably unused or a base), `WeaponArmorRatio` 1.7,
  `MaxPlayerSpeed` 250, slot 2, no ammo.
- View model `models/weapons/v_knife_t.mdl`: draw 1.0 s; `stab` (hit) 1.2857
  s, `stab_miss` 1.5 s, `midslash1`/`midslash2` (hit) 1.1818 s; there is no
  separate slash-miss sequence. No animation events (timings are in code).

Unknown (Q9, measurement M11): primary and secondary damage, hit and miss
refire times for each, ranges, whether a hull fallback exists and its size,
the backstab rule (facing test angle, damage), whether slashes alternate,
armour handling, and whether the first slash after a pause differs.
Community-reported values to test against, **unverified**: slash 15 (first)
/ 20, stab 65, backstab stab 195; slash range ≈48 in, stab ≈32 in; slash
refire 0.4 s on hit, 0.5 s on miss, stab 1.0 s / 1.1 s.

### 7. CS:S-specific rules (not in the SDK)

Each item: what we know, a hypothesis, and how to measure (measurement plan
below). Nothing here may be implemented from the hypothesis alone.

1. **Inaccuracy model.** Keys per weapon: `Spread`, `Inaccuracy{Stand,
   Crouch, Move, Jump, Land, Ladder, Fire}`, `RecoveryTime{Stand,Crouch}`.
   Hypothesis (shape of the later Source CS model): total cone =
   `Spread + inaccuracy`, where the steady part is Stand or Crouch, plus
   Move scaled by horizontal speed between some fraction of max speed and
   max speed, plus Jump while airborne, plus Ladder on ladders; each shot
   adds `InaccuracyFire`; landing adds `InaccuracyLand`; the accumulated
   part decays exponentially toward the steady value with time constant
   tied to `RecoveryTime{Stand,Crouch}` (e.g. to 10 % in RecoveryTime).
   Direction offset: random angle θ = U(0, 2π), radius = U(0,1) × cone,
   separately for the inaccuracy and spread parts. Measure M1, M2.
2. **Legacy accuracy keys** (`AccuracyDivisor`, `AccuracyOffset`,
   `MaxInaccuracy`, `AccuracyQuadratic`) exist only on automatic weapons
   (snipers have divisor −1). CS 1.6 (community knowledge) used
   `accuracy = shots³/divisor + offset` (shots² if quadratic), capped. In
   CS:S they may now only drive recoil. Measure M3.
3. **Recoil (view punch per shot) and decay.** Not in scripts. Expect a
   per-weapon kick that grows with consecutive shots and is random in yaw;
   decay may use the SDK spring (movement.md) or a CS-specific rule. Aim =
   view + punch (template). Measure M3.
4. **Damage falloff by distance.** Hypothesis from the template:
   `damage = Damage × RangeModifier^(distance / 500)` with distance = length
   travelled by the bullet (eye to hit point, summed across penetrations),
   max distance = `Range`. Measure M5.
5. **Hitgroup multipliers.** CS:S overrides the SDK cvars. Community values
   (unverified): head 4, chest 1, stomach 1.25, arms 1, legs 0.75.
   Measure M6.
6. **Armour.** `WeaponArmorRatio` per weapon. Hypothesis: if the victim has
   armour and the hit is not a leg (and for head, only with a helmet):
   `health_damage = damage × ArmorRatio × 0.5`,
   `armour_damage = (damage − health_damage) × 0.5`; if armour is short,
   the uncovered part goes to health. Measure M7.
7. **Wall penetration.** Script `Penetration` (0–3, number of surfaces?)
   plus per-ammo power and max distance (not in scripts) plus per-material
   modifiers (surface game material: concrete, metal, wood, grate, glass,
   etc.). Damage after each wall reduced; total travelled distance still
   feeds falloff. Measure M13.
8. **Weapon movement speeds.** Max speed = script `MaxPlayerSpeed`
   (already used by movement.md). Scoped snipers are slower while zoomed
   (value unknown). Measure M15.
9. **Fire modes.** Glock burst (3 rounds), FAMAS burst (3 rounds) via
   attack2; silencer toggles on USP (3.0811 s anim) and M4A1 (2.0 s anim);
   scopes with zoom levels (FOVs unknown). Alt keys apply in the other mode.
   Damage, range modifier and sound when silenced: unknown. Measure M16.
10. **Reload and deploy times.** Hypothesis: equal to the view-model
    sequence durations in the timing table; shotguns use start/insert/finish
    sequences. Measure M8, M9.
11. **Ammo types** (penetration power, max penetration distance, max carry,
    impulse). Max carry likely exposed as cvars `ammo_<type>_max`
    (template naming). Measure M17.

## Per-tick order

One user command on the server (client prediction mirrors it):

1. Set `curtime` from tickbase; set the command's random seed.
2. Apply a weapon selection (holster old, deploy new).
3. Update button edges.
4. Pre-think: holstered-weapon frames; active weapon pre-frame if
   `curtime ≥ player_next_attack`.
5. Movement: punch decays (spring), then the normal move.
6. Post-think: weapon main frame (secondary → primary → reload → idle/auto)
   or busy frame if `curtime < player_next_attack`. Shots trace from the
   post-move eye with the post-decay punch; damage from one firing call is
   applied at its end.
7. Tickbase +1.

Inside the main frame: fire-duration → reload completion → attack2 →
attack → reload key → auto-reload/auto-switch/idle.

## Edge cases

- Holding attack across a reload: the reload completes, `next_primary =
  curtime`, and the same frame's primary-attack step fires (completion is
  step 2, attack is step 4).
- Reload with a full clip or empty reserve does nothing (no animation, no
  delay).
- An empty weapon with no reserve auto-switches only when no buttons are held
  (or via the fire-on-empty path while attack is held).
- Pressing attack after holding attack2 for a burst/zoom: the
  "attack2 released" edge also resets `next_primary` to `curtime` (generic).
- Deploy of an unusable weapon is refused; the switch fails and the old
  weapon stays.
- A shot whose trace starts inside solid hits at its start point.
- Several pellets hitting one player = one damage event (summed), so armour
  and rounding apply to the sum (generic accumulation; CS:S: verify).
- A target behind the shooter's own body: the shooter is always ignored by
  his own shots.
- Weapons that are carried are not solid and do not block traces.
- The hitbox test uses the victim's lag-compensated pose for human shooters.

## Quirks

- **Fire rate is tick-quantised** if CS:S uses the "set" timing: AK-47 fires
  every 7 ticks (0.105 s), M4A1 every 6 (sometimes 7), MP5/M249 every 6
  (0.09 s instead of 0.08), P90/TMP every 5 (0.075 s instead of 0.07). Keep
  whatever M4 measures; this is part of the feel.
  *Measured (M4): held automatic fire accumulates, so the long-run rate is
  exactly 1/CycleTime (AK-47 7,7,7,6 … ticks); the quantised intervals
  apply only to tapping and semi-automatic weapons.*
- **Neck hitbox is head**: neck shots get the head multiplier.
- **`weapon_aug` uses 7.62 mm ammo** in the script (real AUG: 5.56); keep the
  data as is.
- **`weapon_mp5navy` lists a secondary ammo** (9 mm) though it has no
  secondary fire; harmless.
- **The C4 script** has `Damage` 50 and `clip_size` 30, unused for its real
  behaviour.
- Shared spread seed has only 256 values per command (`& 255`).
- The HL2MP "half the pellets are 6-inch boxes" rule must **not** be copied to
  CS:S unless M4 shows it.

## Test cases

Generic-code tests (exact). CS:S-dependent expectations are marked
"(hyp.)" and must be confirmed by the matching measurement first.

| Setup | Input | After | Expected |
|---|---|---|---|
| T1 seed: command number 1 | – | – | S = 1997239609; shot seed = 57 |
| T1b command number 2 / 100 / 12345 | – | – | 1318994272 (seed 96) / 236147958 (246) / 36020103 (135) |
| T2 template spread, aim yaw 0 pitch 0, s = 0.01, x = 0.5, y = 0 | one shot | – | r = (0, −1, 0); d = normalize(1, −0.005, 0); bullet yaw = −0.2865° (to the right) |
| T3 generic spread defaults | – | – | shot_bias = 1, flatness 0.5, x,y = average of two U(−1,1), rejected if x²+y² > 1 |
| T4 right/up basis, f = (0,0,−1) (looking straight down) | – | – | r = (0, −1, 0), u = (1, 0, 0) |
| T5 deploy AK-47 at curtime t | switch | – | next attack = t + 1.0; with ticks at 0.015 the first allowed shot is 67 ticks later |
| T6 AK-47 held fire, "set" rule | hold attack 30 shots | – | shot interval 7 ticks; 30 shots span 29 × 7 = 203 ticks = 3.045 s |
| T7 AK-47 held fire, "accumulate" rule | hold | 3 s | 30 shots due at t, t+0.1, … ; per-tick fire pattern 7,7,6 ticks repeating (6.667 avg) |
| T8 reload AK-47 clip 10, reserve 90 | reload at t | t + 2.4324 → 163 ticks | clip 30, reserve 70; can fire on tick 163 |
| T9 reload clip 10, reserve 5 | reload | 163 ticks | clip 15, reserve 0 |
| T10 reload with clip 30 or reserve 0 | reload | – | nothing happens, no delay |
| T11 switch AK-47 → USP during reload at 1.0 s | switch | – | reload cancelled, clip unchanged; USP fires 1.0 s after switch (draw 1.0) |
| T12 falloff (hyp.) AK-47, 1000 in, no armour, chest | 1 shot | – | 36 × 0.98² = 34.5744 |
| T13 falloff (hyp.) Deagle at 500 in | – | – | 54 × 0.81 = 43.74 |
| T14 falloff (hyp.) M3 pellet at 3000 in | – | – | 26 × 0.70⁶ = 3.0589 |
| T15 generic hitgroups, 30 damage | head / chest / leg | – | 60 / 30 / 30 |
| T16 generic fractional damage | two hits of 34.5744 | – | health −34 then −35 (accumulator 0.5744 → 0.1488) |
| T17 HL2MP impulse, 200 grain at 1225 ft/s | – | – | 0.0129557 kg × 14700 in/s × 3.5 = 666.57 kg·in/s |
| T18 bullet force, 30 kg prop, impulse 2400 (hyp. for .50 AE) | hit at mass centre | – | Δv = 80 in/s along the shot |
| T19 view punch (2, 0, 0) | 1 call | – | punch velocity += (40, 0, 0) deg/s; then decays per movement.md |
| T20 melee hull fallback, range 75 | line misses | – | box ±16 swept to 47.288 in; target origin at 50° off view → miss (cos 50° = 0.643 < 0.70721); at 40° → hit |
| T21 drop, eye at (0,0,64), looking along +X | drop | – | weapon at (0,0,52), velocity (400,0,0), touchable after 1.0 s |
| T22 pickup volume, weapon box (−9,−9,−9)–(30,9,7) at origin, yaw 0 | – | – | trigger box (−45,−45,−9)–(66,45,25) |
| T23 hitbox groups (ct_urban) | – | – | 19 boxes; neck → 1, head → 1, pelvis → 3, spine1 → 3, spine2 → 2; legs 6/7, arms 4/5 |
| T24 grenade throw | throw | – | release event at 0.435 × 0.7667 = 0.3335 s after the throw animation starts |
| T25 shotgun reload, template timings (hyp. for M3) clip 5, reserve 32, no buttons held | reload at t | – | start anim; insert anim at t + 0.5; shell 1 added at t + 0.95; each later insert starts one tick after the previous shell and adds its shell 0.45 s later (period 0.465 s at 0.015 tick, before rounding of 0.5/0.45 up to whole ticks); after shell 3 (clip 8) the finish anim plays |
| T26 pickup same weapon type, dropped clip 20, reserve 80 of max 90 | touch | – | reserve 90, dropped gun keeps 10 and stays |

## Weapon data (CS:S scripts, values only)

Decrypted from the user's install (`cstrike_pak_dir.vpk`, `scripts/
weapon_*.ctx`). Values recorded as data; the scripts themselves and the key
are not committed. Implementation should read the live scripts at runtime and
use these tables only for tests.

### Economy, handling, damage

Slot = `bucket` (0 primary, 1 pistol, 2 knife, 3 grenade, 4 C4). Ammo types
are `BULLET_PLAYER_<x>` / `AMMO_TYPE_<x>` in the scripts. Clip −1 = no clip.
Grenades have `default_clip` 1 and are exhaustible.

| weapon | team | slot | price | clip | ammo type | max speed | weight | auto | bullets | damage | range | range mod | pen. | armor ratio | cycle |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| weapon_ak47 | TERRORIST | 0 | 2500 | 30 | 762MM | 221 | 25 | 1 | 1 | 36 | 8192 | 0.98 | 2 | 1.55 | 0.1 |
| weapon_aug | CT | 0 | 3500 | 30 | 762MM | 221 | 25 | 1 | 1 | 32 | 8192 | 0.96 | 2 | 1.4 | 0.09 |
| weapon_awp | ANY | 0 | 4750 | 10 | 338MAG | 210 | 30 | 0 | 1 | 115 | 8192 | 0.99 | 3 | 1.95 | 1.5 |
| weapon_c4 | TERRORIST | 4 | 0 | 30 | None | 250 | 0 | – | 1 | 50 | 4096 | 0.99 | 1 | 1.0 | – |
| weapon_deagle | ANY | 1 | 650 | 7 | 50AE | 250 | 7 | 0 | 1 | 54 | 4096 | 0.81 | 2 | 1.5 | 0.225 |
| weapon_elite | TERRORIST | 1 | 800 | 30 | 9MM | 250 | 5 | 0 | 1 | 45 | 4096 | 0.75 | 1 | 1.05 | 0.12 |
| weapon_famas | CT | 0 | 2250 | 25 | 556MM | 220 | 75 | 1 | 1 | 30 | 8192 | 0.96 | 2 | 1.4 | 0.09 |
| weapon_fiveseven | CT | 1 | 750 | 20 | 57MM | 250 | 5 | 0 | 1 | 25 | 4096 | 0.885 | 1 | 1.5 | 0.15 |
| weapon_flashbang | ANY | 3 | 200 | -1 | FLASHBANG | 250 | 1 | – | 1 | 50 | 4096 | 0.99 | 1 | 1 | – |
| weapon_g3sg1 | TERRORIST | 0 | 5000 | 20 | 762MM | 210 | 20 | 1 | 1 | 80 | 8192 | 0.98 | 3 | 1.65 | 0.25 |
| weapon_galil | TERRORIST | 0 | 2000 | 35 | 556MM | 215 | 25 | 1 | 1 | 30 | 8192 | 0.98 | 2 | 1.55 | 0.09 |
| weapon_glock | ANY | 1 | 400 | 20 | 9MM | 250 | 5 | 0 | 1 | 25 | 4096 | 0.75 | 1 | 1.05 | 0.15 |
| weapon_hegrenade | ANY | 3 | 300 | -1 | HEGRENADE | 250 | 2 | – | 1 | 50 | 4096 | 0.99 | 1 | 1.475 | – |
| weapon_knife | ANY | 2 | 0 | -1 | None | 250 | 0 | – | 1 | 50 | 4096 | 0.99 | 1 | 1.7 | – |
| weapon_m249 | ANY | 0 | 5750 | 100 | 556MM_BOX | 220 | 25 | 1 | 1 | 35 | 8192 | 0.97 | 2 | 1.6 | 0.08 |
| weapon_m3 | ANY | 0 | 1700 | 8 | BUCKSHOT | 220 | 20 | 1 | 9 | 26 | 3000 | 0.70 | 1 | 1.0 | 0.88 |
| weapon_m4a1 | CT | 0 | 3100 | 30 | 556MM | 230 | 25 | 1 | 1 | 33 | 8192 | 0.97 | 2 | 1.4 | 0.09 |
| weapon_mac10 | TERRORIST | 0 | 1400 | 30 | 45ACP | 250 | 25 | 1 | 1 | 29 | 4096 | 0.82 | 1 | 0.95 | 0.075 |
| weapon_mp5navy | ANY | 0 | 1500 | 30 | 9MM | 250 | 25 | 1 | 1 | 26 | 4096 | 0.84 | 1 | 1.0 | 0.08 |
| weapon_p228 | ANY | 1 | 600 | 13 | 357SIG | 250 | 5 | 0 | 1 | 40 | 4096 | 0.8 | 1 | 1.25 | 0.15 |
| weapon_p90 | ANY | 0 | 2350 | 50 | 57MM | 245 | 26 | 1 | 1 | 26 | 4096 | 0.84 | 1 | 1.5 | 0.07 |
| weapon_scout | ANY | 0 | 2750 | 10 | 762MM | 260 | 30 | 0 | 1 | 75 | 8192 | 0.98 | 3 | 1.7 | 1.25 |
| weapon_sg550 | CT | 0 | 4200 | 30 | 556MM | 210 | 20 | 1 | 1 | 70 | 8192 | 0.98 | 2 | 1.45 | 0.25 |
| weapon_sg552 | TERRORIST | 0 | 3500 | 30 | 556MM | 235 | 25 | 1 | 1 | 33 | 8192 | 0.955 | 2 | 1.4 | 0.09 |
| weapon_smokegrenade | ANY | 3 | 300 | -1 | SMOKEGRENADE | 245 | 2 | – | 1 | 50 | 4096 | 0.99 | 0 | 1.0 | – |
| weapon_tmp | CT | 0 | 1250 | 30 | 9MM | 250 | 25 | 1 | 1 | 26 | 4096 | 0.84 | 1 | 1.0 | 0.07 |
| weapon_ump45 | ANY | 0 | 1700 | 25 | 45ACP | 250 | 25 | 1 | 1 | 30 | 4096 | 0.82 | 1 | 1.0 | 0.105 |
| weapon_usp | ANY | 1 | 500 | 12 | 45ACP | 250 | 5 | 0 | 1 | 34 | 4096 | 0.79 | 1 | 1.0 | 0.15 |
| weapon_xm1014 | ANY | 0 | 3000 | 7 | BUCKSHOT | 240 | 20 | 1 | 6 | 22 | 3000 | 0.70 | 1 | 1.0 | 0.25 |

### Accuracy keys (normal mode)

Units: probably radians-like cone fractions (same scale as Spread); RecoveryTime in seconds. Grenades, knife and C4 have none.

| weapon | Spread | InaccuracyCrouch | InaccuracyStand | InaccuracyJump | InaccuracyLand | InaccuracyLadder | InaccuracyFire | InaccuracyMove | RecoveryTimeCrouch | RecoveryTimeStand |
|---|---|---|---|---|---|---|---|---|---|---|
| weapon_ak47 | 0.00060 | 0.00687 | 0.00916 | 0.43044 | 0.08609 | 0.10761 | 0.01158 | 0.09222 | 0.34868 | 0.48815 |
| weapon_aug | 0.00060 | 0.00412 | 0.00549 | 0.36936 | 0.07387 | 0.09234 | 0.01090 | 0.07268 | 0.30263 | 0.42368 |
| weapon_awp | 0.00020 | 0.06060 | 0.08080 | 0.54600 | 0.05460 | 0.13650 | 0.14000 | 0.27300 | 0.24671 | 0.34539 |
| weapon_deagle | 0.00400 | 0.00975 | 0.01300 | 0.34500 | 0.06900 | 0.02300 | 0.05500 | 0.02070 | 0.32236 | 0.38683 |
| weapon_elite | 0.00400 | 0.00600 | 0.00800 | 0.29625 | 0.05925 | 0.01975 | 0.03162 | 0.01778 | 0.24753 | 0.29703 |
| weapon_famas | 0.00060 | 0.00412 | 0.00549 | 0.36527 | 0.07305 | 0.09132 | 0.01186 | 0.06980 | 0.30328 | 0.42460 |
| weapon_fiveseven | 0.00400 | 0.00600 | 0.01000 | 0.25635 | 0.05127 | 0.01709 | 0.05883 | 0.01538 | 0.18628 | 0.22353 |
| weapon_g3sg1 | 0.00030 | 0.01935 | 0.02580 | 0.46557 | 0.04656 | 0.11639 | 0.04989 | 0.23279 | 0.22245 | 0.31142 |
| weapon_galil | 0.00060 | 0.00939 | 0.01253 | 0.45434 | 0.09087 | 0.11358 | 0.00984 | 0.10561 | 0.35197 | 0.49275 |
| weapon_glock | 0.00400 | 0.00750 | 0.01000 | 0.27750 | 0.05550 | 0.01850 | 0.03167 | 0.01665 | 0.21875 | 0.26249 |
| weapon_m249 | 0.00200 | 0.00763 | 0.01017 | 0.70830 | 0.14166 | 0.13281 | 0.00427 | 0.10618 | 0.55920 | 0.78288 |
| weapon_m3 | 0.04000 | 0.00750 | 0.01000 | 0.42000 | 0.08400 | 0.07875 | 0.04164 | 0.04320 | 0.29605 | 0.41447 |
| weapon_m4a1 | 0.00060 | 0.00525 | 0.00700 | 0.34151 | 0.06830 | 0.08538 | 0.01266 | 0.06872 | 0.26973 | 0.37762 |
| weapon_mac10 | 0.00100 | 0.01425 | 0.01900 | 0.13704 | 0.02741 | 0.03426 | 0.00845 | 0.00620 | 0.25263 | 0.35368 |
| weapon_mp5navy | 0.00100 | 0.01289 | 0.01718 | 0.23025 | 0.04605 | 0.05756 | 0.00638 | 0.01785 | 0.27960 | 0.39144 |
| weapon_p228 | 0.00400 | 0.00825 | 0.01100 | 0.28500 | 0.05700 | 0.01900 | 0.03318 | 0.01710 | 0.23026 | 0.27631 |
| weapon_p90 | 0.00100 | 0.01463 | 0.01951 | 0.16494 | 0.03299 | 0.04124 | 0.00732 | 0.01062 | 0.23289 | 0.32605 |
| weapon_scout | 0.00030 | 0.02378 | 0.03170 | 0.38195 | 0.03819 | 0.09549 | 0.06667 | 0.19097 | 0.17681 | 0.24753 |
| weapon_sg550 | 0.00030 | 0.01928 | 0.02570 | 0.43727 | 0.04373 | 0.10932 | 0.03829 | 0.21864 | 0.20970 | 0.29358 |
| weapon_sg552 | 0.00060 | 0.00405 | 0.00540 | 0.33464 | 0.06693 | 0.08366 | 0.01227 | 0.06132 | 0.27631 | 0.38683 |
| weapon_tmp | 0.00100 | 0.01500 | 0.02000 | 0.11180 | 0.02236 | 0.02795 | 0.01594 | 0.00389 | 0.15131 | 0.21184 |
| weapon_ump45 | 0.00100 | 0.01439 | 0.01919 | 0.16941 | 0.03388 | 0.04235 | 0.01129 | 0.01366 | 0.21710 | 0.30394 |
| weapon_usp | 0.00400 | 0.00600 | 0.00800 | 0.28725 | 0.05745 | 0.01915 | 0.03495 | 0.01724 | 0.23371 | 0.28045 |
| weapon_xm1014 | 0.04000 | 0.00750 | 0.01000 | 0.41176 | 0.08235 | 0.07721 | 0.03644 | 0.03544 | 0.32894 | 0.46052 |

### Accuracy keys, alternate mode (silenced, burst or scoped)

RecoveryTime has no Alt variant.

| weapon | SpreadAlt | InaccuracyCrouchAlt | InaccuracyStandAlt | InaccuracyJumpAlt | InaccuracyLandAlt | InaccuracyLadderAlt | InaccuracyFireAlt | InaccuracyMoveAlt |
|---|---|---|---|---|---|---|---|---|
| weapon_aug | 0.00060 | 0.00288 | 0.00385 | 0.36936 | 0.07387 | 0.09234 | 0.01090 | 0.07268 |
| weapon_awp | 0.00020 | 0.00150 | 0.00200 | 0.54600 | 0.05460 | 0.13650 | 0.14000 | 0.27300 |
| weapon_famas | 0.00060 | 0.00412 | 0.00549 | 0.36527 | 0.07305 | 0.09132 | 0.00593 | 0.06980 |
| weapon_g3sg1 | 0.00030 | 0.00150 | 0.00200 | 0.46557 | 0.04656 | 0.11639 | 0.04989 | 0.23279 |
| weapon_glock | 0.00400 | 0.00750 | 0.01000 | 0.27750 | 0.05550 | 0.01850 | 0.02217 | 0.01665 |
| weapon_m4a1 | 0.00054 | 0.00525 | 0.00700 | 0.34846 | 0.06969 | 0.08712 | 0.01165 | 0.07039 |
| weapon_scout | 0.00030 | 0.00300 | 0.00400 | 0.38195 | 0.03819 | 0.09549 | 0.06667 | 0.19097 |
| weapon_sg550 | 0.00030 | 0.00150 | 0.00200 | 0.43727 | 0.04373 | 0.10932 | 0.03829 | 0.21864 |
| weapon_sg552 | 0.00060 | 0.00284 | 0.00378 | 0.33464 | 0.06693 | 0.08366 | 0.00859 | 0.06132 |
| weapon_usp | 0.00300 | 0.00600 | 0.00800 | 0.29625 | 0.05925 | 0.01975 | 0.02504 | 0.01778 |

### Legacy accuracy and idle keys (automatic weapons and snipers)


| weapon | AccuracyDivisor | AccuracyOffset | MaxInaccuracy | AccuracyQuadratic | TimeToIdle | IdleInterval |
|---|---|---|---|---|---|---|
| weapon_ak47 | 200 | 0.35 | 1.25 | – | 1.9 | 20 |
| weapon_aug | 215 | 0.3 | 1.0 | – | 1.9 | 20 |
| weapon_awp | -1 | 0 | 0 | – | 2 | 60 |
| weapon_famas | 215 | 0.3 | 1.0 | – | 1.1 | 20 |
| weapon_g3sg1 | -1 | 0 | 0 | – | 1.8 | 60 |
| weapon_galil | 200 | 0.35 | 1.25 | – | 1.28 | 20 |
| weapon_m249 | 175 | 0.4 | 0.9 | – | 1.6 | 20 |
| weapon_m4a1 | 220 | 0.3 | 1.0 | – | 1.5 | 60 |
| weapon_mac10 | 200 | 0.6 | 1.65 | – | 2 | 20 |
| weapon_mp5navy | 220 | 0.45 | 0.75 | – | 2 | 20 |
| weapon_p90 | 175 | 0.45 | 1.0 | 1 | 2 | 20 |
| weapon_scout | -1 | 0 | 0 | – | 1.8 | 60 |
| weapon_sg550 | -1 | 0 | 0 | – | 1.8 | 60 |
| weapon_sg552 | 220 | 0.3 | 1.0 | – | 2 | 20 |
| weapon_tmp | 200 | 0.55 | 1.4 | – | 2 | 20 |
| weapon_ump45 | 210 | 0.5 | 1 | 1 | 2 | 20 |

### View-model sequence durations (animation-driven timings)

`duration = (frames − 1) / fps` (section 3.8), from each weapon's `viewmodel`
in the install. "Ticks" = first tick at or after the duration at 0.015 s.
Fire sequences are cosmetic (refire uses `CycleTime`). Where several
sequences share an activity the game picks one at random (all listed values
equal unless noted).

| weapon | view model | draw s | reload s (ticks) | fire anim s | other |
|---|---|---|---|---|---|
| weapon_ak47 | v_rif_ak47 | 1.0 | 2.4324 (163) | 0.75 | |
| weapon_aug | v_rif_aug | 1.0 | 3.7714 (252) | 0.8571 | |
| weapon_awp | v_snip_awp | 1.0 | 3.6667 (245) | 1.3667 | |
| weapon_c4 | v_c4 | 1.0 | – | press button 2.7667 | drop (secondary) 1.0 |
| weapon_deagle | v_pist_deagle | 1.0 | 2.1667 (145) | 0.675 | dry fire 0.575 |
| weapon_elite | v_pist_elite | 1.3333 | 3.76 (251) | 0.8333 | left = primary, right = secondary activity; last-round left/right dry fire 0.8333 |
| weapon_famas | v_rif_famas | 1.0 | 3.3333 (223) | 0.8571 | |
| weapon_fiveseven | v_pist_fiveseven | 1.0 | 3.2 (214) | 0.6389 | dry fire 0.6389 |
| weapon_flashbang | v_eq_flashbang | 0.6667 | – | – | pull pin 0.9512, throw 0.7667 (release at 0.3335) |
| weapon_g3sg1 | v_snip_g3sg1 | 1.0 | 4.6667 (312) | 0.5 | |
| weapon_galil | v_rif_galil | 0.7812 | 2.9524 (197) | 0.8571 | |
| weapon_glock | v_pist_glock18 | 1.0667 | 2.1429 (143) | single 0.5 | burst (secondary activity) 0.75; last round 0.5714 |
| weapon_hegrenade | v_eq_fraggrenade | 0.6667 | – | – | as flashbang |
| weapon_knife | v_knife_t | 1.0 | – | – | stab hit 1.2857, stab miss 1.5, slash hit 1.1818 |
| weapon_m249 | v_mach_m249para | 0.96 | 5.7 (380) | 0.9211 / 1.0 | |
| weapon_m3 | v_shot_m3super90 | 1.0 | start 0.375, insert 0.4909, finish 0.875 | 1.125 / 0.973 | |
| weapon_m4a1 | v_rif_m4a1 | 0.975 | 3.0541 (204) | 1.5 (0.4 silenced) | attach / detach silencer 2.0 |
| weapon_mac10 | v_smg_mac10 | 1.0 | 3.1429 (210) | 0.6 | |
| weapon_mp5navy | v_smg_mp5 | 0.8571 | 3.0526 (204) | 0.6667 | |
| weapon_p228 | v_pist_p228 | 1.0 | 2.7143 (181) | 0.7059 | dry fire 0.8824 |
| weapon_p90 | v_smg_p90 | 1.0 | 3.375 (225) | 0.4–0.4667 | |
| weapon_scout | v_snip_scout | 1.0 | 2.9 (194) | 1.2857 | |
| weapon_sg550 | v_snip_sg550 | 1.0 | 3.75 (250) | 1.0 | |
| weapon_sg552 | v_rif_sg552 | 0.8108 | 2.7568 (184) | 0.4286 / 0.5714 | |
| weapon_smokegrenade | v_eq_smokegrenade | 0.6667 | – | – | as flashbang |
| weapon_tmp | v_smg_tmp | 0.8333 | 2.12 (142) | 0.5333 / 0.8 | |
| weapon_ump45 | v_smg_ump45 | 1.0 | 3.4545 (231) | 0.4375 | |
| weapon_usp | v_pist_usp | 1.0 | 2.6757 (179) | 1.0 | attach / detach silencer 3.0811; dry fire 0.9375 |
| weapon_xm1014 | v_shot_xm1014 | 1.0 | start 0.6667, insert 0.3889, finish 0.4 | 0.5 | |

Silenced and unsilenced USP/M4A1 sequences have equal durations (the silenced
set is tagged with the `_SILENCED` activity variants). No CS:S view model has
a holster sequence.

### Fire interval in ticks (if the "set" rule holds)

(Measured: this applies to fresh presses only; held automatic fire
accumulates, see "CS:S values (measured)", M4.)

Simulated in 32-bit floats over 600 start tickbases (`next = curtime +
CycleTime`, fire at the first tick with `curtime ≥ next`):

| CycleTime | weapons | ticks (share) | effective interval |
|---|---|---|---|
| 0.07 | p90, tmp | 5 | 0.075 s |
| 0.075 | mac10 | 5 (93 %), 6 (7 %) | ≈0.075 s |
| 0.08 | m249, mp5navy | 6 | 0.09 s |
| 0.09 | aug, famas, galil, m4a1, sg552 | 6 (81 %), 7 (19 %) | ≈0.093 s |
| 0.1 | ak47 | 7 | 0.105 s |
| 0.105 | ump45 | 7 (96 %), 8 (4 %) | ≈0.106 s |
| 0.12 | elite | 8 (84 %), 9 (16 %) | |
| 0.15 | fiveseven, glock, p228, usp | 10 (85 %), 11 (15 %) | |
| 0.225 | deagle | 15 (84 %), 16 (16 %) | |
| 0.25 | g3sg1, sg550, xm1014 | 17 | 0.255 s |
| 0.88 | m3 | 59 | 0.885 s |
| 1.25 | scout | 84 | 1.26 s |
| 1.5 | awp | 100 | 1.5 s |

(Semi-automatic weapons also need the attack button released and pressed
again, measurement M16.)

## CS:S values (measured)

Measured on a local CS:S dedicated server (2026-10, build 11003710, Linux,
0.015 s tick) by observing the running game only: a bot shooter driven per
tick through `tools/css_probe` (plugin `mashup_probe.sp`, driver
`weapcmp.py` / `weapmeas.py`), a second bot as target (health reset to 1000
and armour re-applied after every hit), logging the networked weapon state
(`m_iClip1`, `m_flNextPrimaryAttack`, `m_flNextAttack`, `m_iShotsFired`,
`m_vecPunchAngle`, `m_fAccuracyPenalty`, `m_iFOV`) every tick and the
`weapon_fire`, `bullet_impact` and `player_hurt` events. "Ticks" count server
ticks of 0.015 s. Each item says how it was measured; anything not listed
here is still a hypothesis.

### M11 Knife

Method: target bot standing on flat ground in front of the attacker
(dust2 CT spawn), view aimed at the target's chest, one attack per run after
the knife was fully drawn; ranges by bisection on the target distance;
facing by rotating the target.

| attack | front | back (backstab) | armour 100 (health / armour) | refire on hit | refire on miss |
|---|---|---|---|---|---|
| slash (attack) | 20 first, 15 follow-up | same as front (20 / 15) | 20 → 17 / 1 | 0.5 s both timers (34 ticks held) | 0.4 s primary, 0.5 s secondary (27 ticks held) |
| stab (attack2) | 65 | 195 | 65 → 55 / 4, 195 → 165 / 14 | 1.1 s both timers (74 ticks held) | 1.0 s both timers (67 ticks held) |

- Slash damage: 20 if at least 0.9 s (60 ticks) passed since the previous
  slash, else 15 (59 ticks → 15, 60 → 20). Holding attack on a target gives
  20, 15, 15, 15, … every 34 ticks.
- Knife hits report hitgroup 0 (generic): no hitgroup multiplier, armour
  always applies (`WeaponArmorRatio` 1.7, formula in M7). The script's
  `Damage` 50 is unused.
- Backstab (stab only): taken when the angle between the target's facing yaw
  and the horizontal direction from the attacker's origin to the target's
  origin is ≤ 36°; 37° and more is a front stab (symmetric left/right). This
  fits `dot ≥ 0.8` (36.87°). Verified to depend on the attacker→target
  direction, not the attacker's view direction (target offset sideways by
  12 units, view straight ahead: windows shift by the 16.7° bearing).
- Reach (target facing the attacker, view horizontal through the target's
  eye height, so no line hit on a hitbox): along a world axis the knife hits
  up to an origin distance of 79.8 (slash) and 63.9 (stab), at 20° off-axis
  81.0–82.0 and 65.1–66.0. Both fit a 32 × 32 × 32 box (±16) swept from the
  eye for **48** (slash) / **32** (stab) units hitting the target's
  axis-aligned ±16 collision hull: reach = range + 32 on-axis, range +
  32/cos 20° off-axis. With the view aimed down at the chest the ray to the
  hitbox at the reach limit is only 54.6 / 40.3, so the box sweep, not the
  line, sets the reach. Some off-centre aims at 40 units missed depending
  on the target's rotation (3 of 9), so hitboxes matter somewhere in the
  test; not resolved.

### M5, M6, M7 Damage, hitgroups, falloff, armour

Method: target hovering (move type none) on a 4269-unit clear line, shooter
crouched, single shots aimed at each hitgroup (aim points found with a
server-side bullet-mask trace), 2–3 hits per group at 100, 500, 1000, 2000
and 3000 units (AK-47, USP; no armour, armour 100 without and with helmet),
200 and 1500 (Deagle, AWP, Glock; no armour and armour+helmet). 491 hits.
`d` = distance from the shooter's eye to the `bullet_impact` point.

- `damage = Damage × RangeModifier^(d / 500) × group`, with group = head 4,
  chest 1, stomach 1.25, left/right arm 1, left/right leg 0.75 (knife:
  generic, 1).
- Armour protects when the victim has armour > 0 and the hit is chest,
  stomach, an arm or generic, or the head **with** a helmet; legs never, the
  head without a helmet not.
- Protected: `health_damage = damage × WeaponArmorRatio × 0.5`,
  `armour_damage = (damage − health_damage) × 0.5`; unprotected:
  `health_damage = damage`, no armour damage.
- Both are truncated to integers (35.86 → 35, 15.75 → 15).

- 482 of 491 hits match exactly. The other 9 are all within one point, all on
  the strongly falling pistols (USP, Deagle, Glock) right at an integer
  boundary, and all need a slightly **shorter** distance (≈ 1 %: e.g. USP leg
  at 995 units measured 16, formula 15.95). Using `0.99 d` fixes them but
  breaks one AK-47 head hit at 2000 (needs ≥ 0.991 d). The game's internal
  distance is therefore within about 1 % of eye → impact; its exact
  definition is not resolved.
- No random damage, no fraction carried between hits (the same shot repeated
  gives the same number).
- Examples (no armour): AK-47 at 96 units: head 143, chest 35, stomach 44,
  arm 35, leg 26. USP at 1000: head 85, chest 21, stomach 26, leg 16.
  AWP at 190 with armour+helmet: head 446 / 5, chest 111 / 1,
  stomach 139 / 1, leg 85 / 0.
- Armour ratio check: AK-47 1.55, USP 1.0, Deagle 1.5, AWP 1.95, Glock
  1.05, knife 1.7 all follow the same formula. A victim with less armour
  than `armour_damage` was not tested.

### M4, M8, M10 Fire timing, deploy, reload, empty clip

Method: per-tick `weapon_fire` ticks and weapon timers, attack held, or
tapped (pressed every N ticks).

- **Automatic weapons, attack held: "accumulate".** After each shot
  `next_primary = previous next_primary + CycleTime` (not `curtime +
  CycleTime`), at most one shot per tick. AK-47 (0.1): ticks between shots
  7, 7, 7, 6, 7, 7, 6, … (mean 6.67, i.e. exactly 600 rounds/min);
  M4A1 (0.09): 6 every time; M3 held (0.88): 59, 59, 58. The timer after a
  shot reads 0.100, 0.095, 0.090, 0.085, 0.095, … for the AK-47.
- **A fresh press** sets `next_primary = curtime + CycleTime` (AK-47 tapped
  every 8 ticks: 8-tick spacing, timer exactly 0.1 after each shot).
- **Semi-automatic** (pistols, AWP, scout): holding attack fires once; each
  new press fires when `next_primary` has passed, `next_primary = curtime +
  CycleTime`: Deagle 15 ticks (16 when float rounding lands just past),
  USP and Glock 10, AWP 100.
- **Deploy** = the view model's draw duration (section 3.8):
  `next_attack = next_primary = curtime + draw`; first shot
  `ceil(draw / tick)` ticks later: AK-47, AWP, Deagle, USP 1.0 s → 67 ticks,
  M4A1 0.975 s → 65 ticks.
- **Reload** = the view model's reload duration: AK-47 2.4324 s → clip
  refilled 163 ticks after the start, and with attack held the first shot
  fires on that same tick; M4A1 3.0541 s → 204 ticks. No ammo is lost
  (clip 0 + reserve 90 → 30 / 60).
- **Empty clip, attack held**: one extra `weapon_fire` without a bullet
  (dry fire) one cycle after the last round, then nothing while attack stays
  held; the automatic reload starts on the first tick with no buttons held.

### M1, M2 Inaccuracy

Method: `m_fAccuracyPenalty` per tick while standing, crouching, walking,
running, jumping, landing and firing (AK-47); plus single AK-47 shots at a
wall 415 units away, one every 40 ticks (punch back to 0, penalty logged at
each shot), 250 shots each standing, crouched, running (221), walking
(114.9) and 150 each at speeds 78.7, 150 and 190.

- `m_fAccuracyPenalty` is the stored inaccuracy `I` (same scale as the script
  keys). Standing still it equals `InaccuracyStand` (AK-47 0.009159),
  crouched (`FL_DUCKING`) it settles at `InaccuracyCrouch` (0.006871).
  Moving does **not** change it (the move term is added at shot time).
- Recovery per tick (steady = Crouch when ducked, else Stand):
  if `I > steady`: `I = steady + (I − steady) × 0.1^(tick / RT)`, with
  `RT = RecoveryTimeCrouch` when ducked else `RecoveryTimeStand` (fitted
  0.488 s vs. 0.48815 standing, 0.348 vs. 0.34868 crouched: "back to 10 %
  of the excess in RecoveryTime"). If `I < steady` (standing up from a
  crouch) `I` jumps to the steady value at once.
- Firing: on the shot tick, after that tick's decay, `I += InaccuracyFire`
  (exact to 1e-6: 0.00917 → 0.020749, then 0.016740 → 0.027802 seven ticks
  later). No cap reached in 5 shots.
- Jumping: on the jump tick `I += InaccuracyJump` (0.43044), and the same
  tick already decays. While airborne `I` decays by a factor 0.96755 per
  tick (≈ 10 % of the excess in 1.05 s), toward a value that could not be
  pinned down (fits between 0.006 and 0.009).
- Landing raised `I` from 0.0930 to 0.1633 in one tick (+0.0702 net) for
  `InaccuracyLand` 0.08609; the exact landing rule (scaling, order) is not
  resolved. Afterwards the standing recovery applies.
- Movement term at shot time, AK-47 (max speed 221), from the spread of
  shots: speed 78.7 → ≈ 0.003, 114.9 → ≈ 0.027, 150 → ≈ 0.050,
  190 → ≈ 0.076–0.086, 221 → ≈ 0.090 (`InaccuracyMove` 0.09222). Fits
  `InaccuracyMove × clamp((v − vmax/3) / (vmax − vmax/3), 0, 1)` within the
  statistical error (about ±15 %).
- Shot direction: offset in tangent units along the aim's right/up vectors
  (`d = f + x·r + y·u`), isotropic (mean |x| = mean |y|), with radius
  uniform: standing, offsets/(Spread + I) have quartiles 0.24 / 0.45 / 0.68
  and max 0.92; mean radius 0.00491 for I = 0.00994 (uniform radius
  predicts I/2 = 0.00497); crouched 0.00351 for I = 0.00712. Consistent
  with `U(0,1)·I` at a uniform random angle plus `U(0,1)·Spread` at a second
  random angle (the template/hypothesis shape). The random generator itself
  (Q1) was not checked.

### M3 Recoil (AK-47)

Method: full-clip bursts (attack held) at the wall, 5 standing and 1
crouched, per-tick punch angle and the per-shot bullet offset.

- Punch velocity stays 0: CS:S does not use the SDK spring for weapon recoil.
- Decay per tick, before the weapon fires: with `L = |punch|` (pitch, yaw),
  `L ← max(L − (10 + 0.5 L) × 0.015, 0)`, direction unchanged (matches every
  logged tick to 1e-5).
- Kick per shot, standing (shot number n = `m_iShotsFired` after the shot):
  up (pitch more negative) by `1.0` for n = 1, `1.0 + 0.175 n` after;
  sideways by `0.375` for n = 1, `0.375 + 0.0375 n` after; pitch clamped at
  −5.75, yaw at ±1.75. The sideways direction is random at the first shot
  and flips with probability ≈ 1/8 per later shot (17 flips in 145 shots).
- Crouched: up `0.9`, then `0.9 + 0.15 n`; sideways `0.35`, then
  `0.35 + 0.025 n` (clamps not reached in the logged part).
  Moving and airborne kick sets were not measured.
- **Bullets go along view + 2 × punch**, using the punch after this tick's
  decay and before this shot's kick: residuals after removing `2 × punch` are
  all inside the inaccuracy cone; with `1 × punch` they are up to 5° outside.
- `m_iShotsFired` keeps counting while attack is held; after the last shot it
  stays for about 0.45 s, then falls to 0 within about 0.1 s (5 → 3 → 2 → 0
  in 3-tick samples).

### M15 Zoom

Method: attack2 presses 100 ticks apart, `m_iFOV` per tick (0 = default
90), then a shot while zoomed; top speed with forward held for 80 ticks.

| weapon | zoom steps (FOV) | after a shot | zoomed max speed |
|---|---|---|---|
| weapon_awp | 40, 10, off | unzooms; re-zooms when `next_primary` passes (100 ticks) | 150 (unzoomed 210) |
| weapon_scout | 40, 15, off | unzooms; re-zooms after 84 ticks | 220 (script max speed 260; unzoomed not re-measured) |
| weapon_sg550, weapon_g3sg1 | 40, 15, off | stays zoomed | sg550 150 |
| weapon_aug, weapon_sg552 | 55, off | stays | aug 221 (unchanged) |

Each zoom toggle sets `next_secondary = curtime + 0.3`; it does not touch
`next_primary`. The FOV value changes on the toggle tick.

### M16 Fire modes and pellets

- Glock burst (attack2 toggles, 0.3 s secondary delay): one trigger press
  fires 3 bullets on ticks 0, 4 and 8 (0.06 s apart), with a single
  `weapon_fire` event; 3 rounds used.
- FAMAS burst: 3 bullets 5–6 ticks apart (≈ 0.08 s); with attack held a new
  burst every 36–37 ticks (≈ 0.55 s).
- Silencer toggle: USP blocks both attacks for 3.0 s, M4A1 for 2.0 s
  (`next_primary = next_secondary = curtime + time`). Silenced damage and
  range: not measured (the bot AI toggles the silencer back on its own).
- Shotguns: one `bullet_impact` per pellet, 9 for the M3, 6 for the XM1014,
  one `weapon_fire` per shot.

### M17 Ammo cvars

`cvarlist ammo_` (max carry, replicated server cvars): 338mag 30, 357sig 52,
45acp 100, 50AE 35, 556mm_box 200, 556mm 90, 57mm 100, 762mm 90, 9mm 120,
buckshot 32, flashbang 2, hegrenade 1, smokegrenade 1. Penetration power and
impulse per ammo type are not exposed as cvars.

### Not measured yet

M2's dependency on shots fired beyond 5, M9 shotgun reload, M12 hitbox reach,
M13 penetration (no thin wall of known material found quickly on dust2; the
64-unit wall at the CT spawn stops everything), M14 drop/pickup, silenced
damage, the landing rule, moving/airborne recoil sets, the knife's
hitbox/hull interplay, and the RNG (Q1).

## Open questions

- **Q1 Random generator.** The uniform generator body is not in the SDK
  (only its state layout: 32-entry table, two ints). Exact CS:S spread
  patterns need it bit-exact. Either implement the inferred `ran1` and
  verify by comparing predicted vs. logged `bullet_impact` points for known
  seeds (M2), or accept statistically equal spread.
- **Q2 Inaccuracy formula** (section 7.1): measurement M1, M2. *Mostly
  resolved: "CS:S values (measured)", M1/M2 (recovery, fire, jump, move
  term, cone shape); landing rule and airborne target still open.*
- **Q3 Recoil kick and decay**, and how much punch adds to the aim: M3.
  *Resolved for the AK-47 standing/crouched (measured section, M3): linear
  decay rule, kick formula, aim = view + 2 × punch. Other weapons and
  moving/airborne kick sets not measured.*
- **Q4 Fire timing rule** ("set" vs. "accumulate"): M4. *Resolved (measured
  section, M4): accumulate while held (one shot per tick max), set on a
  fresh press; the tick table above applies to tapping only.*
- **Q5 Damage rounding** (truncate, round, or accumulate fractions) and
  falloff formula: M5.
  *Resolved (measured section, M5–M7): truncate, no carried fraction,
  falloff `RangeModifier^(d/500)`; the exact distance is within ~1 % of
  eye → impact.*
- **Q6 Hitbox surrounding bounds**: are hits on arms/head outside the
  collision hull registered? M12.
- **Q7 Penetration** model, per-ammo power/distance, material table: M13.
- **Q8 Ammo impulses** for physics props (shared with physics_props.md Q11):
  shoot a known-mass prop and measure Δv.
- **Q9 Knife** damages, ranges, refire, backstab: M11.
  *Resolved (measured section, M11): 20/15 slash, 65/195 stab, refire, reach
  48/32 box sweep, backstab at ≤ 36° (dot ≥ 0.8).*
- **Q10 Hitgroup multipliers and armour**: M6, M7.
  *Resolved (measured section, M6, M7): head 4, chest 1, stomach 1.25, arm 1,
  leg 0.75; armour formula as hypothesised, helmet needed for the head.*
- **Q11 Deploy/reload exceptions** (e.g. faster deploy, reload start delay,
  fire allowed before the reload animation ends): M8.
  *Partly resolved (measured section, M8): deploy and reload equal the
  view-model durations, firing allowed on the completion tick; shotgun
  reload (M9) open.*
- **Q12 Shotgun pellets**: line vs. box traces, pattern, per-pellet damage
  event or summed: M4.
- **Q13 Drop / pickup** CS rules: drop velocity, which weapon is dropped on
  death, pickup by walking only into an empty slot, dropped-weapon lifetime:
  M14.
- **Q14 Zoom** levels (FOV), zoomed speed, unzoom after a sniper shot and
  re-zoom timing: M15.
  *Resolved for FOV, toggle delay, unzoom/re-zoom and zoomed speed
  (measured section, M15).*
- **Q15** `sv_legacy_grenade_damage` exists on current CS:S servers ("replicate
  grenade damage behaviour of the original game"): grenade damage changed in
  an update; measure both settings when grenades are specced.

### Measurement plan (CS:S dedicated server + SourceMod probe)

Setup: `tools/css_probe` (a bot driven per tick from an input file, logging
per tick). Extend the plugin (it is our code, not game code) with:

- **Per-tick weapon log** (shooter): active weapon, clip (`m_iClip1`), reserve
  (`m_iAmmo[type]`), `m_flNextPrimaryAttack`, `m_flNextSecondaryAttack`,
  `m_flNextAttack`, `m_iShotsFired`, punch (`m_vecPunchAngle`,
  `m_vecPunchAngleVel`), FOV (`m_iFOV`), silencer/burst state, eye position and
  angles, velocity, flags (ground, duck), ladder/water state. Confirm netprop
  names with `sm_dump_netprops` / `sm_dump_datamaps` first; record any
  accuracy-penalty-like float the CS weapon networks.
- **Events**: `weapon_fire`, `bullet_impact` (x, y, z per bullet),
  `player_hurt` (dmg_health, dmg_armor, hitgroup, weapon), `weapon_reload`,
  `weapon_zoom`, `item_pickup`, `player_death`. Log with the tick number.
- **Commands**: give weapon/ammo, set armour and helmet (`m_ArmorValue`,
  `m_bHasHelmet`), set health, teleport target bots to exact positions and
  angles, freeze targets (`bot_stop 1`, `bot_zombie 1`), and a server-side
  `TR_TraceRay` from the shooter's eye along the view to log the ideal hit
  point and hitgroup for each shot.
- Server cvars for runs: `sv_cheats 1`, `mp_friendlyfire 1` when needed;
  `host_timescale`
  only for RCON sampling, never for timing tests.

Measurements:

- **M1 Inaccuracy by state.** For each weapon and each state (stand, crouch,
  walk at several speeds, run, jump apex, just landed, ladder), fire single
  shots spaced ≥ 2 × RecoveryTimeStand apart at a wall 1024 in away, N ≥ 300
  shots per state. From `bullet_impact` and the ideal point compute angular
  offsets; fit: max radius vs. `Spread + Inaccuracy*`, radial distribution
  (uniform radius vs. uniform area), angle distribution. Repeat with speed
  sweeps (0 → max) to get the move term's shape.
- **M2 Fire penalty and recovery.** Fire 1–10 rapid shots then one probe shot
  after delay Δ ∈ {0, 0.05, …, 1.0 s}; fit the decay law and its time
  constant vs. `RecoveryTime*`; crouched vs. standing. Also log seeds
  (command numbers) to test Q1.
- **M3 Recoil.** Hold fire for a full clip, log punch angle and punch velocity
  per tick and each bullet's offset; repeat 20 times to separate the fixed
  part from random yaw; release and log the decay to zero (compare with the
  spring in movement.md). Check whether bullets follow view + punch or
  view + k × punch.
- **M4 Fire timing.** Hold fire, log the tick of each `weapon_fire`; compare
  with the tick table above. For shotguns, count `bullet_impact` events per
  shot and check pellet traces (line vs. box) by shooting just past an edge.
- **M5 Range falloff and rounding.** Unarmoured target, chest hits
  (`player_hurt` hitgroup 2) at distances 64, 250, 500, 1000, 2000, 4000 in;
  compare `dmg_health` with `Damage × RangeModifier^(d/500)`; deduce rounding.
- **M6 Hitgroup multipliers.** Same distance, aim at each hitgroup (use the
  hitbox table to aim; confirm with the logged hitgroup).
- **M7 Armour.** Armour 100 with and without helmet, all hitgroups, several
  weapons (different `WeaponArmorRatio`); log health and armour damage; also
  armour < needed.
- **M8 Deploy and reload.** Switch and reload every weapon; log
  `m_flNextPrimaryAttack` / `m_flNextAttack` and the tick clip changes; compare
  with the duration table. Try firing on the tick the reload ends; try
  switching mid-reload (ammo kept?).
- **M9 Shotgun reload.** Log clip per tick during M3/XM1014 reloads; fire
  mid-reload.
- **M10 Empty and dry fire.** Empty clip with/without reserve: tick of the
  auto reload, click interval, auto-switch behaviour.
- **M11 Knife.** Primary/secondary on a target at distances 16–80 in (step 4),
  front and back (rotate the target in 15° steps), with and without armour;
  log damage, hit/miss and next-attack times; repeat consecutive slashes.
- **M12 Hitbox reach.** Shoot along lines that pass through a hitbox but
  outside the 26 × 26 × 72 hull, and through the hull missing all hitboxes.
- **M13 Penetration.** Use walls of known thickness and material (measure
  thickness with two opposite `TR_TraceRay`s) on stock maps or a small test
  map; per weapon, shoot a target behind walls of 1–32 in; log damage and
  whether `bullet_impact` appears on the far side.
- **M14 Drop / pickup.** `drop` command: log the weapon entity's origin and
  velocity for 2 s; walk the bot over weapons with the slot full and empty;
  check lifetime of dropped weapons; death drops.
- **M15 Zoom and speed.** Toggle zoom on each scoped weapon, log `m_iFOV`
  and the time until the next shot; measure top speed zoomed/unzoomed with
  the movement probe.
- **M16 Fire modes.** Glock/FAMAS burst: tick spacing of burst rounds and
  the delay after a burst; USP/M4A1 silencer: toggle time and damage/range
  change; semi-auto: holding attack fires once.
- **M17 Ammo cvars.** `cvarlist ammo_` on the server: max carry per type.
