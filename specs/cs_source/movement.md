# Counter-Strike: Source: player movement

Source basis: Valve's public Source SDK 2013 only. I read the shared player movement code (walking, air, ladder, water, ducking, jumping, ground detection, sliding collision, stepping), its header, the shared console variable definitions, the default player hull/eye table in the shared game rules, and the shared constants header (duck timings, fall speeds). CS:S's own game DLL is not public. Where CS:S overrides shared behavior (jump impulse, hulls, stamina, landing slowdown, walk key), I say so and list it under Open questions. No leaked CS:S/CS:GO code was used.
Status: draft

## Summary

- Quake-lineage kinematic controller. Each tick: half of gravity, then jump, ground friction, acceleration toward a wish direction, a swept-box slide move with stair stepping, ground re-detection, then the other half of gravity.
- Acceleration only adds speed along the wish direction until the velocity's projection on that direction reaches the wish speed. In the air the projection target is capped at 30 units/s, but the amount added per tick is not. That mismatch is why air-strafing (turning while holding a strafe key) gains speed.
- Ground friction removes a fixed fraction per tick above sv_stopspeed and a constant amount below it. It is skipped on the tick a jump starts, which makes bunny-hopping possible.

## Units and conventions

- Distances are Hammer units (1 unit ≈ 1 inch). Z is up. Velocities are units/s.
- Angles are degrees: pitch (positive looks down), yaw (counter-clockwise from +X, seen from above), roll. Forward from angles is (cos p·cos y, cos p·sin y, −sin p). With zero roll, right is (sin y, −cos y, 0).
- The player origin is at the bottom centre of the collision box (the box's min Z is 0 relative to the origin).
- Movement runs once per user command. The time step `dt` is one server tick: 1/64 s = 0.015625 s on 64-tick servers (1/66 s on 66-tick, the CS:S default for older servers; all numbers below use 64). Nothing depends on render frame rate. A per-player time scale (normally 1) multiplies `dt`.
- Timers in the duck/jump logic count in milliseconds. Each tick they go down by 1000·dt (15.625 ms at 64 tick) and stop at 0.
- Collision is a swept axis-aligned box. Results: fraction of the sweep completed, end position, hit plane normal, "started in solid" and "entirely in solid".
- Move inputs: forward, side and up amounts in units/s. Positive side means right. The client sends ±(cl_forwardspeed, cl_sidespeed, cl_backspeed) for the keys, which can be larger than the max speed. The server rescales them (see Wish direction).

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| hull_stand | (−16,−16,0) to (16,16,72) | units | standing box, shared default |
| hull_duck | (−16,−16,0) to (16,16,36) | units | ducked box, shared default (CS:S differs, see Open questions) |
| eye_stand | 64 | units | eye height above origin, standing (shared default) |
| eye_duck | 28 | units | eye height above origin, ducked (shared default) |
| hull_observer | (−10,−10,−10) to (10,10,10) | units | spectator box |
| air_wish_cap | 30 | units/s | max wish speed used for the "how much more" test in air |
| walkable_normal_z | 0.7 | – | a plane is ground if its normal z ≥ 0.7 (≈45.57° max slope) |
| ground_probe | 2 | units | ground test sweeps the box this far down |
| leave_ground_vz | 140 | units/s | upward speed above which the ground is never kept |
| optimized_unground_vz | 250 | units/s | at tick start, a walking player with vz > this loses ground before anything else |
| jump_impulse_shared | √(2·800·45) = 268.3281573 | units/s | shared-code jump speed (a fixed constant, not recomputed from sv_gravity) |
| clip_planes_max | 5 | planes | max contact planes per slide move |
| bump_iterations | 4 | – | max sweeps per slide move |
| step_epsilon | 0.03125 | units | added to step height when stepping up/down |
| stay_on_ground_up | 2 | units | lift before the stick-to-ground trace |
| snap_min | 1/64 | units | stick-to-ground only moves if the gap exceeds this |
| duck_timer_start | 1000 | ms | value the duck timer is set to on a duck/unduck event |
| time_to_duck | 0.4 | s | duck transition length |
| time_to_unduck | 0.2 | s | unduck transition length |
| duck_speed_frac | 1/3 (0.33333333) | – | move-input multiplier while fully ducked on ground |
| duck_jump_down_probe | 36 | units | used only by the single-player duck-jump path |
| ladder_reach | 2 | units | how far toward a ladder the box is swept to attach |
| climb_speed | 200 | units/s | ladder climb speed per held key |
| ladder_jump_off | 270 | units/s | speed along ladder normal when jumping off |
| water_jump_up | 256 | units/s | upward speed of a water jump |
| water_jump_push | 50 | units/s | speed away from the wall during a water jump |
| water_jump_time | 2000 | ms | water-jump duration |
| water_wish_scale | 0.8 | – | wish speed multiplier in water |
| water_sink | 60 | units/s | downward wish when no input in water |
| swim_up_water / slime | 100 / 80 | units/s | vz set when holding jump at waist depth or deeper |
| fall_punch_threshold | 350 | units/s | min landing speed for view punch and landing sound |
| fall_safe | 580 | units/s | landing speed above which fall damage applies |
| fall_fatal | 1024 | units/s | landing speed that does 100 damage |
| fall_damage_per_speed | 100 / (1024 − 580) = 0.225225 | hp per unit/s | damage slope above fall_safe |
| fall_min_bounce | 200 | units/s | below this, no landing sound |
| floating_landing_bonus | 200 | units/s | subtracted from landing speed on floating objects |
| land_punch_scale | 0.013 | deg per unit/s | landing view roll kick |
| punch_damping | 9 | 1/s | punch angle spring damping |
| punch_spring | 65 | 1/s² | punch angle spring constant |
| surface_friction_scale | 1.25 | – | material friction × this, capped at 1 |
| upward_air_friction | 0.25 | – | surface friction when leaving ground upward without a jump |
| stuck_check_interval | 1.0 | s | how often (multiplayer) to test for being stuck |

## Behavior

### Wish direction and the speed cap

Inputs: view angles, forward move `f`, side move `s`, up move `u`, the player's current max speed (in CS:S this comes from the held weapon, see CS:S values), the client's max speed and the ground material.

1. Effective max speed `M` = min(player max speed, client max speed if nonzero) × the ground material's max-speed factor (1 for normal materials). A map constraint can lower it further (not used in CS:S maps).
2. If √(f²+s²+u²) > M, scale f, s and u by M/√(f²+s²+u²). So holding forward+strafe is not faster than forward alone.
3. If the player is frozen, on a train or dead, f = s = u = 0.
4. If the player is fully ducked and on the ground, multiply f, s, u by 1/3 (once per tick; see Ducking).
5. For walking and air movement, take forward and right from the view angles, zero their z, and renormalize them. Looking up or down does not slow you.
6. wishvel = forward·f + right·s with z = 0. wishspeed = |wishvel|, wishdir = wishvel/wishspeed.
7. If wishspeed > M, set wishspeed = M. Step 2 normally makes this a no-op.

The view angles used are the command's angles plus the current punch angle. Yaw above 180 is wrapped to (−180, 180].

### Ground friction

Applied only when the player has a ground entity at that point in the tick, after the jump check. A jump on this tick removes the ground, so friction is skipped. Skipped during a water jump.

- speed = |v| (3D, but vz has just been set to 0 on ground).
- If speed < 0.1, nothing happens.
- control = max(speed, sv_stopspeed).
- drop = control × sv_friction × surface_friction × dt.
- newspeed = max(speed − drop, 0). v is scaled by newspeed/speed.

So above sv_stopspeed, speed is multiplied by (1 − sv_friction·dt) each tick (0.9375 at 64 tick with friction 4). Below it, speed drops by a constant sv_stopspeed·sv_friction·dt each tick (6.25 units/s per tick at stopspeed 100, 4.6875 at 75).

### Ground acceleration

Inputs: wishdir, wishspeed, sv_accelerate, surface friction.

- current = v · wishdir (projection, not speed).
- add = wishspeed − current. If add ≤ 0, nothing happens.
- accel = sv_accelerate × dt × wishspeed × surface_friction, capped at add.
- v += accel × wishdir.

Before accelerating, vz is set to 0, and it is set to 0 again after. If the resulting speed (with base velocity added) is under 1 unit/s, velocity becomes exactly 0 and the player does not move this tick.

### Air acceleration

Same as ground acceleration, with two differences:

- The target for the "add" test is min(wishspeed, 30): add = min(wishspeed, 30) − v·wishdir.
- The amount is still sv_airaccelerate × wishspeed × dt × surface_friction, using the uncapped wishspeed. It is then capped at add.

Consequences:
- Holding forward from a standstill in the air reaches only 30 units/s.
- If wishdir is perpendicular to v, each tick adds min(30, sv_airaccelerate·wishspeed·dt) = 30 (with 250 wish speed and airaccelerate 10, the uncapped amount is 39.0625). So |v|² grows by 900 per tick: |v_n| = √(|v_0|² + 900n).
- The best gain per tick is with wishdir exactly perpendicular to v. Any wishdir with projection ≥ 30 gains nothing. A wishdir pointing back against v slows the player down.

Air movement uses no friction. Dead players and water-jumping players get no air acceleration.

### Gravity

Gravity is split into two half-steps around the move. This is exact for constant acceleration (trapezoid rule).

- First half (at the start of walking movement, when not in water at waist depth or deeper): vz −= g_scale × sv_gravity × dt/2. The base velocity's z × dt is also added to vz, and the base velocity's z is cleared.
- Second half (after the move and ground re-detection, when not in water at waist depth or deeper, and not water-jumping): vz −= g_scale × sv_gravity × dt/2.
- If the player is on ground at the end of the tick, vz = 0.
- g_scale is the player's gravity multiplier (0 is treated as 1; ladders set it to 0 while attached).

At 800 gravity and 64 tick, each half-step is 6.25 units/s.

### Jumping

Checked every tick while the jump button is held, before friction. When the button is not held, the "jump was held last tick" flag is cleared. A jump happens only if all of these hold:

1. Not dead. Not water-jumping. Water level is below waist depth (at waist depth or deeper, holding jump instead sets vz = 100 in water or 80 in slime and removes the ground).
2. The player has a ground entity. In the air, the "held" flag is set, so a jump buffered in the air does not fire on landing.
3. Jump was not held last tick. You must release and press again. There is no auto-hop in shared code.
4. Not in the middle of an unduck transition while still flagged as ducked, and the single-player duck-jump eye timer is 0.

Effect:
- The ground entity is cleared. Base velocity gets the old ground's velocity (see Base velocity).
- Impulse = ground material's jump factor (1 normally) × 268.3281573 units/s.
- If the player is ducked or in a duck transition, vz = impulse. This replaces vz, so the half-gravity already applied this tick is lost. Otherwise vz += impulse.
- Then an extra gravity half-step is applied right away (vz −= sv_gravity·dt/2). Gravity on the jump tick is therefore 1.5 half-steps for a standing jump, 1 for a ducked jump, plus the normal second half at the end of the tick.
- The "held" flag is set.

The shared code says 268.33 ≈ √(2·800·45), a 45-unit jump. With the discrete steps above, a standing jump peaks at 42.93 units (see Test cases). CS:S uses its own jump code (see Open questions).

### Ground detection

Run at the end of every walking tick, and at the start of the tick for non-walking move types:

- Clear surface friction to 1 and update the water level.
- If vz > 140 (relative to the ground entity's own vz when standing on something), or if on a ladder and moving up, the player has no ground.
- Otherwise sweep the current box from the origin to 2 units below. If it hits something whose plane normal z ≥ 0.7, that is the ground. If not, or the plane is too steep, sweep each of the four quadrant sub-boxes (the box split at its centre in X and Y, full height). The first quadrant that finds a plane with normal z ≥ 0.7 counts as ground. This lets a player stand on the peak of a ridge or on a thin edge. The origin is not moved by this test, so a player can rest up to 2 units above a floor until the next ground move snaps them down (see Staying on the ground).
- If no ground is found and vz > 0 (not noclip), surface friction is set to 0.25 for the rest of the tick.
- On finding ground: the ground's material sets surface friction and the material (see Surface materials), the water-jump timer is cleared, and vz is set to 0.

At the very start of a walking tick, the full ground test is not run. The ground is only removed if vz > 250. The test at the end of the previous tick carries over.

### Slide move (collide and slide)

Moves the box along v for the remaining time `t_left` (starts at dt), resolving contacts:

- Repeat up to 4 times while |v| > 0:
  1. Sweep from origin to origin + v·t_left.
  2. If entirely in solid, set v = 0 and stop.
  3. If the sweep moved any distance (fraction > 0): run a stationary test at the end point. If that is in solid, set v = 0 and stop, without moving. Otherwise move to the end point, remember v as the "pre-clip velocity", and reset the plane list.
  4. If fraction = 1, stop.
  5. t_left −= t_left × fraction.
  6. If 5 planes are already stored, set v = 0 and stop. Otherwise store this plane's normal.
  7. If this is the first stored plane, the player is walking-type and has no ground (airborne): clip the pre-clip velocity against it. Use overbounce 1 for planes with normal z > 0.7, and 1 + sv_bounce × (1 − surface_friction) otherwise (which is 1 in CS:S, sv_bounce 0). The result becomes v. There is no "reversed" check on this path.
  8. Otherwise (on ground, or 2+ planes): for each stored plane i, clip the pre-clip velocity against plane i with overbounce 1. If the result does not point into any other stored plane (dot < 0 counts as into), use it. If no single plane works: with exactly 2 planes, v = (dir · v) dir, where dir = normalize(n₀ × n₁), so you slide along the crease. With any other count, v = 0 and stop.
  9. Still on that path: if v · (velocity at the start of the whole move) ≤ 0, v = 0 and stop. This prevents jitter in acute corners.
- If no sweep made any progress (sum of fractions = 0), v = 0.

**Clip velocity** with normal n and overbounce b: out = in − n·(in·n)·b. If out·n < 0 after that (float error), out −= n·(out·n). There is no small-number snapping to zero in this version.

### Stair stepping

Used by ground movement when the direct sweep to the destination is blocked (fraction < 1), the player had ground at the start of the move, and is not water-jumping:

1. Save origin P₀ and velocity V₀.
2. "Down" try: slide move from P₀ with V₀. Remember the result (P_d, V_d). Reset to P₀, V₀.
3. "Up" try: sweep straight up by sv_stepsize + 0.03125. If that sweep did not start or stay in solid, move to its end. Slide move from there. Then sweep straight down by sv_stepsize + 0.03125.
4. If the down sweep's plane normal z < 0.7 (it did not land on walkable ground, or hit nothing), use the "down" result (P_d, V_d).
5. Otherwise move to the down sweep's end (unless it started in solid). Compare the horizontal squared distance travelled from P₀: if the "down" try went strictly farther, use (P_d, V_d). If not, keep the stepped position and velocity, but replace vz with V_d's z.

The step-up height is a property of the player, default 18 (sv_stepsize). Steps up to 18 units are climbed without jumping. A 19-unit ledge blocks. Stepping only happens on ground. An airborne player hitting a step just slides.

### Ground movement order (one walking-on-ground tick)

1. Build wishdir/wishspeed. Set vz = 0, accelerate, set vz = 0.
2. Add base velocity.
3. If |v| < 1: v = 0, subtract base velocity back out, done.
4. Destination = origin + (vx, vy)·dt, same z. Sweep there. If fraction = 1, move there.
5. Otherwise, if the player had no ground at the start of the move (and is not in water) or is water-jumping, stop here (v stays as is). If not, run Stair stepping.
6. Subtract base velocity.
7. Stay on ground (below). It runs after both the clear path and the stepped path.

### Staying on the ground

After a ground move: sweep up from the origin 2 units (to a known free spot), then sweep down from there to origin.z − step size. If the sweep moved some distance and hit something (0 < fraction < 1), did not start in solid, and hit a plane with normal z ≥ 0.7, then snap the origin to the hit point when the vertical change is more than 1/64 unit. This keeps the player glued to downhill slopes and lets them walk down stairs up to step size without becoming airborne.

### Air movement

wishdir/wishspeed as above (horizontal only), air acceleration with sv_airaccelerate, add base velocity, slide move, subtract base velocity. No stepping, no friction.

### Velocity clamp

After the first gravity half, before the move, after the move and after the second half: each velocity component is clamped to [−sv_maxvelocity, +sv_maxvelocity] (per axis, not by length). NaN components are set to 0, and NaN origin components to 0.

### Ducking

State: "ducked" (small box in use), "ducking" (in transition), a duck timer (ms), plus the duck button now and last tick. Runs every tick before movement, after the timers have been reduced.

- **Press duck** (pressed this tick, not already ducked): duck timer = 1000, transition starts.
- **During the duck transition** with the button held: elapsed = (1000 − timer)/1000 s. If elapsed > 0.4 s, or the player is in the air, finish ducking now. Otherwise eye height = lerp(eye_stand, eye_duck, S(elapsed/0.4)), with S(x) = 3x² − 2x³.
  - Eye height for fraction f: z = (eye_duck − (duck box min z − stand box min z))·f + eye_stand·(1 − f). With the shared boxes, that is 28f + 64(1 − f).
- **Finish ducking**: switch to the ducked box and set eye height to eye_duck. On ground, the origin does not move (box min z is the same). In the air, the origin moves up by the height difference (72 − 36 = 36 units), so the feet tuck up and the head stays in place. Then, if the box is stuck, try moving up 1 unit at a time, up to 36 times. Then re-detect ground.
- **Release duck** while ducked: duck timer = 1000. If released mid-transition (not yet ducked), the timer is set so the unduck starts from the matching point. Fraction already ducked = elapsed/400 ms. New timer = 1000 − 200 + fraction × 200.
- **Unduck transition**: only if the standing box fits. On ground, test the standing box at the same origin. In the air, test it at origin − 36 in z (the feet drop back down). If it does not fit, stay fully ducked (eye = eye_duck, timer reset to 1000) and retry each tick, so you stand up as soon as you leave a vent. If it fits: elapsed = (1000 − timer)/1000. If elapsed > 0.2 s, or the player is in the air, finish unducking now. Otherwise eye = S(1 − elapsed/0.2) blended as above.
- **Finish unducking**: the standing box is used, eye = eye_stand, and in the air the origin moves down 36. Re-detect ground.
- **Speed**: fully ducked (small box) on ground multiplies the move inputs by 1/3. In the shared code there is no slowdown during the transition, and none in the air.
- **Duck-jump**: in the air, ducking is instant and lifts the feet 36 units. Holding duck at the jump apex therefore lets the player clear obstacles 36 units higher than the plain jump height. To land, the player unducks in the air, which needs 36 units of space below the feet. Otherwise the player stays ducked until on the ground.
- **Jump while ducked** sets vz = impulse instead of adding (see Jumping). A jump is refused while the player is still flagged ducked but in an unduck transition.
- The shared code also has a single-player-only duck-jump path (jump sets a 510 ms timer that auto-ducks the box). It only runs when the server has one player slot, so it is not part of CS:S multiplayer. An implementation can omit it.
- A safety net: if no duck activity is going on, the player is alive and the eye height differs from eye_stand by more than 0.1, it is reset to eye_stand.

### Ladders

- Each tick (alive, not on a train, not noclip): if already on a ladder, probe toward −(last ladder normal). Otherwise probe along forward·f + right·s, normalized (3D view vectors, not flattened). This is skipped when there is no move input. The probe sweeps the box 2 units. If it hits a surface marked as ladder (ladder contents, or a climbable material), the player enters ladder mode. Otherwise a player in ladder mode returns to walking.
- While on a ladder: gravity multiplier = 0. "On floor" = the point 1 unit below the box bottom is solid, or the player has ground.
- Climb input comes from the buttons, not the analog move amounts: forward/back give ±200, left/right give ±200.
- Jump held: leave ladder mode, v = ladder normal × 270.
- Otherwise, with input: intended = forward_view × fwd + right_view × side. Split it into the part into the ladder face, normal_amount = intended · n, and the rest, lateral = intended − n·normal_amount. Let perp = normalize(up × n) (horizontal along the ladder) and ladder_up = n × perp. The velocity is lateral − ladder_up × normal_amount. So pushing into the ladder climbs. The direction depends on view pitch, and you can face up while moving down.
  - CS:S-specific (in the shared file, compiled only for CS:S): build the desired direction in the normal/perp plane as perp·(perp·lateral) + n·normal_amount, normalized. If its dot with n is below sv_ladder_angle (−0.707), the perp part of lateral is scaled by sv_ladder_dampen (0.2) before use. This makes it hard to slide sideways off a ladder while climbing.
  - If on floor and moving away from the ladder (normal_amount > 0), add n × 200 so the player can step off.
- No input: v = 0 (hangs in place).
- Ladder movement is: water check, the jump check as usual, add base velocity, slide move, subtract base velocity. No gravity, no friction.
- Ground detection treats "on ladder and vz > 0" as airborne.

### Water (brief)

- Water level: 0 if the point 1 unit above the feet (box centre XY) is not water. 1 (feet) if it is. 2 (waist) if the box mid-height point is also water. 3 (eyes) if the eye point is also water.
- At waist depth or deeper, the gravity halves are not applied. Movement is swimming:
  - wish = forward·f + right·s using the full 3D view vectors. Plus: holding jump adds the client max speed upward. No input at all sinks at 60. Otherwise up move + clamp(2·f·forward_z, 0, client max speed) is added upward.
  - The wish speed is capped at max speed, then multiplied by 0.8.
  - Friction: speed × (1 − dt × sv_friction × surface_friction), zeroed below 0.1. (sv_waterfriction is defined but not used here.)
  - Acceleration uses sv_accelerate (not sv_wateraccelerate). It adds along the wish direction up to wishspeed − newspeed (speed, not projection).
  - Move: if the straight sweep is clear, try to "step" by sweeping down from the destination raised by step size + 1. Otherwise slide.
- **Water jump** (only at exactly waist depth, vz ≥ −180, not backing into the wall): if a sweep 24 units forward from the box centre hits something, a sweep 24 forward at eye height + 8 is clear, and below that there is walkable ground within 1024 units, then: vz = 256, horizontal velocity is locked to 50 units/s away from the wall, the "jump held" flag is set, and this lasts 2000 ms (or until out of water). During it there is no friction, no acceleration and no gravity halves.
- Water currents (contents flags) add 50 × water level units/s to base velocity in their direction.

### Falling and landing

- At the start of each tick, if the player has no ground, fall speed = −vz.
- At the end of a walking tick, if the player has ground and fall speed > 0, a landing happens:
  - If fall speed ≥ 350 and alive: if in water, nothing extra. Otherwise, on floating objects subtract 200. If the ground entity is moving down, add its vz (min 0.1). Then:
    - fall speed > 580: fall damage is applied by game code. The volume is 1.
    - else > 290: volume 0.85.
    - else < 200: volume 0.
    - otherwise volume 0.5.
  - If the volume > 0: landing sound, step-sound timer = 400 ms, and the punch angle's **roll** = fall speed × 0.013 degrees (then pitch is clamped to ≤ 8, which has no effect since only roll was set).
  - Fall speed resets to 0.
- The damage slope is 100 hp per (1024 − 580) units/s, so 0.2252 hp per unit/s above 580 (inferred: the shared header defines the slope, the game DLL applies it; see Open questions).
- Slamming into a wall: if horizontal speed lost during one slide move > 1160, a rough landing effect at volume 1. If > 580, volume 0.85. These play sound and set the view roll from the current fall speed. There is no damage in shared code.

### Punch angle decay

Runs once per tick while |punch|² > 0.001 or |punch velocity|² > 0.001. Otherwise both are zeroed.
- punch += punch_vel × dt
- punch_vel × = max(0, 1 − 9·dt)
- punch_vel −= punch × clamp(65·dt, 0, 2)
- clamp punch to pitch ±89, yaw ±179, roll ±89.

Punch is added to the view angles used for movement (step 1 of Wish direction).

### Surface materials

- When ground is found, surface friction = clamp(material friction × 1.25, ≤ 1). Normal materials (friction 0.8) give 1.0. Ice-like materials give less. It scales both friction and acceleration (ground and air) for the tick.
- Each tick ground detection first resets surface friction to 1, so it only stays below 1 while you are on that material. It is also 0.25 for a tick when the player leaves the ground upward without a jump.
- Material max-speed factor scales M. Material jump factor scales the jump impulse.

### Base velocity (conveyors, moving ground)

- Base velocity is an extra velocity from the environment (conveyors, water currents, the ground's motion).
- Friction runs before base velocity is added, so standing on a conveyor carries you with no slowdown.
- Each move adds base velocity to v before the slide move and subtracts it after. It moves you but is never stored in v.
- Its z is folded into vz once at the start of the tick (× dt) and then cleared.
- On leaving the ground, the ground entity's velocity is added to base velocity, with z taken directly from the ground's vz. On landing on an entity, its velocity is subtracted the same way.

### Stuck recovery (brief)

Once per stuck interval (1 s in multiplayer, staggered per player), test the box at the current origin. If it is stuck, try a table of small nudges, and if none works, reuse the last good position. Movement is skipped on a tick where the player is still stuck. Low priority for the prototype.

## Per-tick order

One user command, walking move type, not in water:

1. Scale dt by the per-player time scale.
2. Compute the effective max speed M. Rescale f, s, u to |(f,s,u)| ≤ M. Zero them if frozen or dead.
3. Decay the punch angle. View angles = command angles + punch.
4. Count down the duck, duck-jump, jump and swim-sound timers by 1000·dt ms.
5. Compute the forward/right/up vectors from the view angles.
6. (Every 1 s) Stuck check. If stuck, end the tick.
7. Ground: if vz > 250, remove ground. Other move types run full ground detection here.
8. Remember the water level. If airborne, fall speed = −vz.
9. Update the duck-jump eye offset (single-player only), then Ducking (incl. the 1/3 input crop when ducked on ground).
10. Ladder check. It may switch to or from ladder mode and set v.
11. By move type. For walking:
    a. Water check. If below waist depth, apply the first gravity half (and fold base vz in), then clamp velocity.
    b. If water-jumping: apply water-jump velocity, slide move, water check, end.
    c. If at waist depth or deeper: water-jump check, jump button, swim, ground detection, vz = 0 if grounded. End.
    d. Jump button (or clear the held flag if not held).
    e. If on ground: vz = 0, then friction.
    f. Clamp velocity.
    g. Ground movement (with stepping and stick-to-ground) if on ground, else air movement.
    h. Ground detection.
    i. Clamp velocity.
    j. If below waist depth: second gravity half (+ clamp).
    k. If on ground: vz = 0.
    l. Landing check.
12. Store this tick's buttons as "last tick's buttons".

## Edge cases

- **Slopes:** walkable when normal z ≥ 0.7. Steeper planes are walls. On ground the slide move clips with overbounce 1. You can walk up a 45° ramp at full horizontal input speed, because the ground move is horizontal and the step/slide logic lifts you. Walking down, stick-to-ground snaps you down up to 18 units per tick.
- **Surfing:** on a slope steeper than 0.7 you are airborne. Air acceleration plus slide clipping lets you surf. Gravity pulls you along the slope through the clip.
- **Ceiling hit while jumping:** the first plane in the air is clipped with overbounce 1, so vz becomes 0 and the player falls from there.
- **Landing between tick boundaries:** the slide move stops at contact and clips vz to 0, and the rest of the time is spent sliding horizontally. Ground is detected that tick. The second gravity half is then cancelled by "on ground ⇒ vz = 0".
- **Hovering:** the ground test reaches 2 units down without moving you. You can be "on ground" up to 2 units above the floor until the next ground move's stick-to-ground snaps you down. A player standing still does not move at all (step 3 of Ground movement), so the gap remains.
- **Jump on the landing tick:** the landing tick ends with ground. If jump is pressed (newly) on the next tick, the jump runs before friction, so no friction is ever applied. If jump is held through the landing, nothing happens until it is released and pressed again.
- **Pitch ±90°:** the flattened forward vector has zero length. CS:S clamps the client pitch to ±89 so this does not occur. An implementation should clamp too.
- **Ducking in the air near the ground:** finishing a duck in the air raises the origin 36. Unducking in the air requires 36 units of free space below. Otherwise you stay ducked.
- **Unduck under a ceiling:** remains ducked and retries every tick.
- **Too many planes:** 5 planes in one move, or 3+ planes with no valid single clip, stops the player dead for the rest of the tick.

## Quirks

- **Air-strafe gain** (keep): the air "add" test uses a 30-unit cap while the amount uses the full wish speed. This is the defining CS/Quake feel.
- **Bunny-hop without friction** (keep in shared form; CS:S adds its own limit, see Open questions): friction is skipped on the jump tick.
- **1.5× gravity on the jump tick** (keep): an extra half-step right after the impulse makes the real apex 42.93, not 45.
- **Ducked jump replaces vz** (keep): it ignores the first gravity half, so a ducked jump is slightly higher (44.98) than a standing one.
- **Landing punch goes to roll, and the clamp hits pitch** (keep the roll kick; the clamp is a no-op).
- **Hovering up to 2 units** on ground until moved (keep; affects eye height by ≤ 2).
- **Water friction uses sv_friction, water accel uses sv_accelerate** (keep): sv_waterfriction and sv_wateraccelerate are defined but the shared swim code does not read them.
- **Velocity clamp is per axis**, so diagonal speed can reach √2·sv_maxvelocity (keep).

## Test cases

All at 64 tick (dt = 1/64), flat floor at z = 0 unless stated, sv_gravity 800, sv_friction 4, max speed M = 250 (knife), surface friction 1, input keys ±450 rescaled to M. "Speed" is horizontal speed after the tick's move. Tolerance ±0.01 unless stated (the original is 32-bit float).

| Setup | Input | After | Expected |
|---|---|---|---|
| at rest on ground, sv_accelerate 5, sv_stopspeed 100 | hold forward | 1 tick | speed = 19.53125, x moved 0.30518 |
| same | hold forward | 2 ticks | speed = 32.8125 (friction 19.53→13.28, +19.53) |
| same | hold forward | 3 ticks | 46.09375 |
| same | hold forward | 7 ticks | 99.21875 (below stopspeed: +13.28125 per tick) |
| same | hold forward | 8 ticks | 112.5 |
| same | hold forward | 9 / 10 ticks | 125.0 / 136.71875 (above stopspeed: v' = 0.9375v + 19.53125) |
| same | hold forward | 25 ticks | 245.73 |
| same | hold forward | 26 ticks | 249.91 (first tick ≥ 99% = 247.5) |
| same | hold forward | 27 ticks | exactly 250, and stays 250 every tick after (friction 234.375 then accel capped at add 15.625) |
| same | hold forward | distance after 27 ticks | ≈ 67.84 (±0.05) |
| at rest, sv_accelerate 10, sv_stopspeed 100 | hold forward | 1 / 2 / 3 ticks | 39.0625 / 71.875 / 104.6875 |
| same | hold forward | 4 / 5 / 6 / 7 ticks | 137.207 / 167.694 / 196.276 / 223.071 |
| same | hold forward | 8 ticks | 248.19 (first ≥ 99%) |
| same | hold forward | 9 ticks | exactly 250 |
| at rest, any accel | hold forward + right | 1 tick | f = s = 176.777 each; wishspeed 250; direction 45° right of view |
| moving 250 on ground, sv_stopspeed 100 | no input | 1 / 2 ticks | 234.375 / 219.727 |
| same | no input | 14 / 15 ticks | 101.28 / 94.95 (15th tick still ×0.9375 because 101.28 ≥ 100) |
| same | no input | 16 ticks | 88.70 (−6.25 per tick from here) |
| same | no input | 30 / 31 ticks | 1.20 / 0 (stopped on tick 31) |
| same | no input | total distance | ≈ 46.875 (±0.05) |
| moving 250 on ground, sv_stopspeed 75 | no input | 18 / 19 ticks | 78.24 / 73.35 (then −4.6875 per tick) |
| same | no input | stop tick, distance | stopped on tick 35; total ≈ 49.80 (±0.05) |
| moving 0.9 on ground | no input or input giving < 1 | 1 tick | velocity exactly 0, no movement |
| standing on ground, jump not held last tick | press jump (standing) | jump tick | vz during move = 255.828; z after = 3.9973; vz at end = 249.578 |
| same | no further input | ticks 2..21 | vz used for the move on tick k (k ≥ 2) = 243.328 − 12.5(k−2) |
| same | – | apex | z = 42.928 at end of tick 21 (jump tick = 1) |
| same | – | landing | ground regained on tick 42 (0.65625 s airborne); fall speed recorded 250.42; no punch (< 350) |
| ducked on ground | press jump | jump tick | vz during move = 262.078 (impulse replaces vz); apex z = 44.98 at end of tick 21 |
| on ground, jump held since before landing | keep holding | any | no jump until released and pressed again |
| airborne | press jump | – | no effect; and pressing earlier does not queue a jump |
| airborne at rest horizontally, airaccelerate 10 | hold forward | 1 tick | horizontal speed 30 (add capped at 30, not 39.0625) |
| same | hold forward | 2+ ticks | stays 30 |
| airborne, v = (250,0), view yaw 0 | hold right only (wishdir ⟂ v) | 1 tick | speed = √(250² + 900) = 251.794; velocity turns 6.843° |
| same, turning view each tick to keep wishdir ⟂ v | hold right | 10 ticks | 267.395 |
| same | – | 64 ticks | 346.554 |
| airborne, v = (250,0) | wishdir 85° from v | 1 tick | 250.85 (projection 21.79, adds 8.21) |
| airborne, v = (250,0) | wishdir 80° from v | 1 tick | 250.0 (projection 43.4 ≥ 30, nothing added) |
| airborne, v = (250,0) | wishdir 95° from v | 1 tick | 249.65 (adds the full 39.0625 against v, loses speed) |
| step-off ledge, vz = 0, height h above floor | no input | each tick k | z = h − k²/10.24 (exact) |
| drop from 64 | – | landing | lands tick 26, fall speed 312.5, no punch, no damage |
| drop from 128 | – | landing | lands tick 36, fall speed 437.5, roll punch 5.6875°, volume 0.85, no damage |
| drop from 256 | – | landing | lands tick 51, fall speed 625, damage (shared slope) 10.14 |
| drop from 512 | – | landing | lands tick 73, fall speed 900, damage 72.07 |
| drop heights in general | – | landing | tick m = ceil(√(10.24·(h − 2))), fall speed = 12.5·(m − 1); damage if fall speed > 580 (h ≳ 218) |
| standing, ground | press and hold duck | tick 0 (press) | eye 64, standing box |
| same | hold | tick 13 | eye 45.58 (fraction S(13/25.6) = 0.5117) |
| same | hold | tick 25 | still standing box (390.6 ms elapsed) |
| same | hold | tick 26 | ducked box and eye 28 (406.25 ms > 400) |
| ducked on ground | release duck, room to stand | tick 0 / 13 | eye 28 / standing box and eye 64 (203.1 ms > 200) |
| ducked on ground | hold forward | 1 tick from rest, accel 5 | wishspeed 83.33; speed 6.5104 |
| airborne, standing | press duck | same tick | ducked box, origin z + 36, eye 28 |
| ducked, airborne, 20 units above floor | release duck | each tick | stays ducked (needs 36 units below) until ground |
| walking at 250 into an 18-unit step | hold forward | contact tick | climbs: z rises by 18, horizontal progress continues |
| walking at 250 into a 19-unit ledge | hold forward | contact tick | blocked: z unchanged, slides along the wall |
| walking off an 18-unit stair edge going down | hold forward | each tick | stays on ground (stick-to-ground snap), no fall |
| airborne into a vertical wall at 45° | none | contact tick | velocity component into the wall removed, tangential kept |
| on ground into a 2-wall 90° corner | hold forward into it | contact tick | velocity → 0 (slides along the first wall into the second, whose clip leaves nothing) |
| sv_maxvelocity 3500 | vz would be −3510 | after clamp | vz = −3500 |
| ground material friction 0.25 (ice-like, × 1.25 = 0.3125) | hold forward from rest, accel 5 | 1 tick | speed 6.1035 |

## CS:S values

Read over RCON from the reference install (Linux, version 11003710) on 2026-10-05, with a listen server on de_dust2. The weapon speeds come from the game's own weapon scripts, decrypted locally (files and key kept out of the repo).

| Console variable | CS:S value | Note |
|---|---|---|
| sv_accelerate | **5** | set by CS:S's stock cfg/skill1.cfg; registered default 10 |
| sv_friction | 4 | |
| sv_airaccelerate | 10 | |
| sv_stopspeed | **75** | set by cfg/skill1.cfg; registered default 100 (the shared code's value) |
| sv_gravity | 800 | |
| sv_maxspeed | 320 | |
| sv_stepsize | 18 | |
| sv_maxvelocity | 3500 | |
| sv_bounce | 0 | |
| sv_backspeed | 0.6 | not read by the shared movement code |
| sv_wateraccelerate / sv_waterfriction | 10 / 1 | not read by the shared swim code |
| sv_ladder_dampen / sv_ladder_angle | 0.2 / −0.707 | |
| sv_enablebunnyhopping | 0 | CS:S-only |
| sv_timebetweenducks | 0 | CS:S-only (duck spam limit, off) |
| cl_forwardspeed / cl_sidespeed / cl_backspeed | **400** | key input before rescaling to max speed |
| server tick rate | **66.67** (sv_maxupdaterate 66) | CS:S's default tick interval of 0.015 s |

**Jump (measured):** a standing jump on flat ground peaks at 54.75 units. With the per-tick order above at 66.67 tick, an impulse of √(2·800·57) = 301.993 units/s gives 54.748. The shared 268.33 gives 42.9. So CS:S's jump speed is 301.993 (a 57-unit jump), not the shared value.

Weapon move speed (MaxPlayerSpeed, units/s):

| Weapon | Max speed | Weapon | Max speed |
|---|---|---|---|
| knife | 250 | ak47 | 221 |
| glock, usp, p228, deagle, elite, fiveseven | 250 | m4a1 | 230 |
| mac10, tmp, mp5navy, ump45 | 250 | sg552 | 235 |
| p90 | 245 | aug | 221 |
| m3 | 220 | scout | 260 |
| xm1014 | 240 | awp | 210 |
| galil | **215** | g3sg1, sg550 | 210 |
| famas | 220 | m249 | 220 |
| hegrenade, flashbang, c4 | 250 | smokegrenade | 245 |

Knife script: MaxPlayerSpeed 250, Damage 50, WeaponArmorRatio 1.7, Range 4096 (the bullet-weapon field; the knife's swing reach is in code), Penetration 1.

## Open questions

1. **Scoped speeds:** weapon speeds are now read from the scripts. Still to check: scoped speeds (AWP/scout/autos are slower zoomed; that is in CS:S code, not in scripts).
2. **Answered:** sv_accelerate 5 and sv_stopspeed 75 come from CS:S's stock cfg/skill1.cfg, which the engine runs on every map load. They are CS:S's effective defaults; the registered defaults 10 and 100 are the shared code's.
3. **Answered:** sv_stopspeed is 75 (see CS:S values).
4. **Answered:** the CS:S jump impulse is 301.993 (see CS:S values).
5. **CS:S hulls and eye heights:** CS:S is believed to use a 32×32×62 ducked box (not 36) and ducked eye ≈46, standing eye 64. The air-duck lift and the in-air unduck clearance both equal (standing height − ducked height), so they change with it (36 with the shared boxes). Measure with `cl_showpos`/getpos while standing vs ducked, and by finding the smallest vent you can enter.
6. **CS:S duck behavior:** duck speed (CS:S slows while ducking/ducked: is it ×1/3 of input or a fixed fraction of max speed?), duck spam limit (sv_timebetweenducks or similar), whether the transition is still 0.4 s / 0.2 s, and any air-duck differences.
7. **Landing slowdown / stamina:** CS:S reduces horizontal speed after landing and after jumps (a "stamina" or velocity modifier). This is not in shared code. Measure speed in the ticks after landing from walking jumps, bhop chains, and a fall.
8. **Bunny-hop limit:** with sv_enablebunnyhopping 0, CS:S caps speed on jump (believed: if speed > 1.1 × max speed, scale down to 1.1 × max speed). Measure by jumping at > 275 units/s with a knife.
9. **Walk key (+speed):** CS:S walking speed. Believed to be a fraction of max speed (≈0.52). Measure.
10. **Backpedal:** whether sv_backspeed 0.6 is applied in CS:S. Measure backward top speed.
11. **Fall damage formula as applied by CS:S:** the shared header gives the slope 100/(1024 − 580). CS:S applies the actual damage (and may scale it, e.g. ×1.25 is sometimes cited). Measure HP lost from known drop heights.
12. **Client move key speeds:** cl_forwardspeed/cl_sidespeed/cl_backspeed in CS:S (believed 400 or 450). Only matters if they are below max speed, which they are not.
13. **Ladder climb speed in CS:S:** shared is 200. Confirm.
14. **Tick rate:** the reference listen server runs 66.67 tick. Bhop, surf and KZ community servers commonly run 66 or 100. The test cases above assume 64.
