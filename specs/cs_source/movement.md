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
| water_jump_push | 50 | units/s | horizontal speed toward the wall (onto the ledge) during a water jump |
| water_jump_reach | 24 | units | forward reach of both water-jump sweeps |
| water_jump_eye_extra | 8 | units | second water-jump sweep starts this far above eye height |
| water_jump_drop | 1024 | units | max depth of the ledge-floor sweep |
| water_jump_min_vz | −180 | units/s | no water jump while sinking faster than this |
| water_feet_probe | 1 | units | feet water test point above the box bottom |
| swim_step_extra | 1 | units | swim stepping probes from step size + this above the destination |
| ladder_floor_probe | 1 | units | point below the box bottom tested for "on floor" while on a ladder |
| ladder_dismount_push | 200 | units/s | added along the ladder normal when on floor and pushing away |
| water_current_scale | 50 | units/s per level | base velocity from current contents |
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
- g_scale is the player's gravity multiplier. A stored value of 0 means "default" and is treated as 1. Attaching to a ladder writes 0 to it, which therefore only resets it to the default. Ladder mode has no gravity because the ladder move never applies gravity, not because of this multiplier.

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
- Surface friction 0.25: this applies **only** when the downward sweep actually ran (vz ≤ 140, not on a ladder moving up), found no walkable ground, and vz > 0 (not noclip). In the "moving up rapidly" case (vz > 140) and the "ladder moving up" case, ground is removed without a sweep, and surface friction stays at the 1 set at the start of this step. The value lasts until the next ground detection, which runs at the end of the next tick. So it applies to that next tick's air acceleration.
  - During a jump, ground detection on the jump tick and the following ticks sees vz > 140, so surface friction is 1. Once the move velocity has dropped into (0, 140] (from tick 11 of a standing shared-impulse jump, jump tick = 1, through the apex on tick 21), detection sets 0.25. Air acceleration therefore runs at 0.25 on ticks 12–22 and at 1 otherwise. After the apex vz ≤ 0, so it is 1 again.
- On finding ground: the ground's material sets surface friction and the material (see Surface materials), the water-jump timer is cleared, and vz is set to 0.

At the very start of a walking tick, the full ground test is not run. The ground is only removed if vz > 250, and the test at the end of the previous tick carries over. The exception is when game code has moved the player since the last tick (teleport, spawn): then the full test runs at the start of the tick.

### Slide move (collide and slide)

Moves the box along v for the remaining time `t_left` (starts at dt), resolving contacts:

- Repeat up to 4 times while |v| > 0:
  1. Sweep from origin to origin + v·t_left.
  2. If entirely in solid, set v = 0 and stop.
  3. If the sweep moved any distance (fraction > 0):
     - Only when it covered the full distance (fraction = 1), first run a stationary box test at the end point. If that end point is in solid, set v = 0 and stop, without moving. This guards against a swept box ending inside terrain. Partial sweeps (0 < fraction < 1) are not re-tested.
     - Then move to the end point, remember v as the "pre-clip velocity", and reset the plane list.
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
- **Finish ducking**: does nothing if the player is already flagged ducked. Otherwise switch to the ducked box and set eye height to eye_duck. On ground, the origin does not move (box min z is the same). In the air, the origin moves up by the height difference (72 − 36 = 36 units), so the feet tuck up and the head stays in place. Then, if the box is stuck at the new origin, move up 1 unit at a time, up to 36 times, stopping at the first free position. If none of the 36 is free, the origin goes back to where it was before these nudges (the box stays ducked, at the lifted or unchanged origin from the previous step). Then re-detect ground.
- **Release duck** while ducked: duck timer = 1000. If released mid-transition (not yet ducked), the timer is set so the unduck starts from the matching point. Fraction already ducked = elapsed/400 ms. New timer = 1000 − 200 + fraction × 200.
- **Can-stand test**: the standing box is **swept**, not just placed at the end position. On ground, the sweep goes from the origin to the same origin, which is a stationary test of the standing box. In the air, it sweeps from the current origin down 36 units (the hull difference). The standing box at the start of that sweep reaches 36 units above the ducked head, so the player needs free space both above the head (start not in solid) and below the feet (the whole sweep clear, fraction = 1).
- **Unduck transition** (button not held): if the can-stand test passes, elapsed = (1000 − timer)/1000. If elapsed > 0.2 s, or the player is in the air, finish unducking now. Otherwise eye = S(1 − elapsed/0.2) blended as above, and the in-transition state is set.
- **Blocked unduck**: if the can-stand test fails and the timer is not already 1000, the player is forced fully ducked. The ducked box and flag are set, the in-transition state is cleared, the eye = eye_duck and the timer = 1000. The player retries every tick and stands up as soon as there is room (e.g. on leaving a vent). This also applies when duck is released partway through a duck transition while the standing box no longer fits: the player snaps to a full duck instead of rising.
- **Finish unducking**: the standing box is used, eye = eye_stand, and in the air the origin moves down 36. Re-detect ground.
- **Re-pressing duck during an unduck transition**: the player is still flagged ducked, so the press does not restart the duck timer. The finish-ducking step does nothing because the player is already ducked. So the in-transition state stays set, the box stays small, and the eye stays frozen at its partial height for as long as duck is held. A jump is refused the whole time (flagged ducked and in transition). Releasing again sets the timer to 1000, so the unduck restarts from the beginning: the eye jumps to eye_duck, then rises over 0.2 s.
- **Speed**: the 1/3 input scale is decided at the start of duck processing, from the state before this tick's duck changes: flagged ducked and having ground at that moment. So on the tick the duck completes, the input is not yet scaled. On the tick an unduck completes (from flagged ducked on ground), it still is. It is applied at most once per tick. In the shared code there is no slowdown during the transition, and none in the air.
- **Duck-jump**: in the air, ducking is instant and lifts the feet 36 units. Holding duck at the jump apex therefore lets the player clear obstacles 36 units higher than the plain jump height. To land, the player unducks in the air, which needs 36 units of space below the feet. Otherwise the player stays ducked until on the ground.
- **Jump while ducked** sets vz = impulse instead of adding (see Jumping). A jump is refused while the player is still flagged ducked but in an unduck transition.
- The shared code also has a single-player-only duck-jump path (jump sets a 510 ms timer that auto-ducks the box). It only runs when the server has one player slot, so it is not part of CS:S multiplayer. An implementation can omit it.
- A safety net: if no duck activity is going on, the player is alive and the eye height differs from eye_stand by more than 0.1, it is reset to eye_stand.

### Ladders

**What counts as a ladder.** The ladder probe is a box sweep using the same solid mask as normal player movement (solid, player clip, window, grate, moveable, monster contents). It does not look for a separate ladder volume. The surface hit counts as a ladder if:
- the hit brush carries the ladder contents flag, or
- the hit surface's material is marked climbable.

A ladder must therefore be something the player collides with, such as a func_ladder brush or a solid/clip brush with the ladder flag or a climbable material. A ladder volume that does not block players is never hit. That a compiled func_ladder brush is player-solid is inferred from the mask, see Open questions.

**Detection, every tick** (alive, not on a train, not noclip). It runs after ducking and before the move:
1. Choose a probe direction d.
   - Already in ladder mode: d = −(ladder normal stored last tick).
   - Otherwise: d = normalize(F·f + R·s). F and R are the full 3D view forward and right vectors (pitch included, not flattened). f and s are the move inputs after the max-speed rescale and the ducked 1/3 scale. If f = s = 0, there is no detection and the player is not on a ladder.
2. Sweep the current box (standing or ducked) from the origin to origin + 2·d.
3. If the sweep hit nothing (fraction 1), or what it hit is not a ladder, detection fails. A player who was in ladder mode returns to walking this tick, keeping the current velocity.
4. Otherwise enter or stay in ladder mode. Store the hit plane normal n (pointing out of the ladder toward the player); it is refreshed every tick. Then compute this tick's velocity (below).

So to grab a ladder, be within 2 units of its face and press a direction whose 3D vector points into it. Looking far up or down while pressing forward shrinks the horizontal reach to 2·cos(pitch), and can make the probe hit the floor or ceiling instead.

**On floor**: the point 1 unit below the box bottom (box centre x/y) has contents exactly equal to "solid", or the player currently has ground.

**Climb input** comes from the held buttons, not the analog move amounts:
- fwd = +200 if forward is held, −200 if back is held (0 if both or neither).
- side = +200 if right is held, −200 if left is held.

Max speed, the diagonal rescale and the ducked 1/3 scale do not affect climbing.

**Velocity on the ladder** (replaces v each tick, so there is no momentum):
- **Jump held** (no fresh press needed): leave ladder mode (walking from this tick on) and set v = 270·n. That tick then runs the normal walking move, see Per-tick order.
- **No climb buttons held**: v = 0. The player hangs in place, with no gravity.
- **Otherwise**:
  1. u = F·fwd + R·side, using the 3D view vectors.
  2. a = u·n (negative when pushing into the ladder). Into-face part c = a·n. Lateral part L = u − c.
  3. perp = normalize(Z × n), horizontal and along the ladder face. ladder_up = n × perp, which is +Z for a vertical ladder.
  4. CS:S-only dampening (in the shared file, compiled only for CS:S): let t = ladder_up·L and p = perp·L. Normalize (perp·p + c). If its dot with n is < sv_ladder_angle (default −0.707), replace L with ladder_up·t + perp·(sv_ladder_dampen·p), where sv_ladder_dampen defaults to 0.2. This applies when the push is mostly into the ladder (within about 45° of straight into it): the sideways part is cut to 20%. Pure sideways strafing is not damped.
  5. v = L − ladder_up·a. The push into the face becomes climbing along ladder_up.
  6. If on floor and a > 0 (pushing away from the ladder), add 200·n so the player can back off the bottom.

**Consequences of the pitch mapping** (vertical ladder, facing it squarely, only forward held):
- vertical speed = 200·(cos p − sin p), where p is the pitch (positive = looking down).
- Level view climbs at 200. Looking up 45° climbs at 282.84, the maximum (200·√2).
- Looking down 45° does not move. Looking further down while holding forward descends: at p = 89 the speed is −196.5.
- Holding back reverses all of this.

**The ladder move** (ladder move type):
1. Water check.
2. Jump check: only reachable when jump is not held, so it just clears the held flag.
3. Add base velocity, slide move, subtract base velocity.

There is no gravity, no friction, no stair stepping, no stick-to-ground, no landing check and no end-of-tick ground detection. In the slide move, the "airborne first plane" bounce path never applies to the ladder move type. Ground detection runs at the start of each ladder tick instead, because the move type is not walking. Moving up (vz > 0 from last tick) on a ladder always counts as airborne. Fall speed (−vz) is still recorded at the start of each tick without ground.

**Entering and leaving:**
- **Stepping on from the bottom**: walk at the ladder holding forward. When the box is within 2 units, detection succeeds and that tick's velocity is the ladder velocity. The walking move, friction and gravity do not run that tick.
- **Getting off at the bottom**: holding back while on floor gives a = +200, so v = −ladder_up·200 + 200·n. The downward part is clipped by the floor and the player walks back at 200. In the air, holding back descends at 200 (level view).
- **Getting off at the top**: when the box rises past the top of the ladder brush, the probe along −n hits nothing. The player returns to walking with the last ladder velocity (e.g. vz = 200), then rises about 200²/(2·800) = 25 more units under gravity while forward input carries them onto the ledge. If the forward probe still reaches ladder surface within 2 units, they re-attach instead.
- **Jumping off**: v = 270·n, purely horizontal for a vertical ladder. After one tick the player is 4.22 units from the face, beyond the 2-unit reach, so detection fails even while still pressing toward it. If the player has ground and jump was not held last tick, the walking move on that same tick also performs a normal jump.

**Ducking on a ladder**: ducking is processed before ladder detection and works as anywhere else. An in-air duck lifts the origin by the hull difference even on a ladder. The ducked box is used for the probe. Climb speed is unchanged, because the 1/3 scale only touches the move inputs, not the buttons.

**Climb sounds** are played by player code not covered here and do not affect movement.

### Water

**Water level.** Three point-contents tests at the box centre x/y. A point counts as "wet" if its contents include water, slime or the "moveable" flag. The "moveable" flag is a quirk of the water mask.
1. Feet point: origin.z + box min z + 1, i.e. 1 unit above the feet. If it is not wet, level 0 and water type "empty".
2. If it is wet: level 1 (feet), and water type = that point's contents (water or slime). Then test the box mid-height point, origin.z + (min z + max z)/2: 36 standing, 18 ducked with the shared boxes. If wet: level 2 (waist).
3. If level 2: test the eye point, origin.z + current eye height (it follows the duck transition). If wet: level 3 (eyes).

"In water" for movement means level ≥ 2. The level is recomputed:
- at the start of each walking tick
- by every ground detection
- after the water-jump move

Within one tick, contents for each of the three points are cached. A test point within 1 unit of the last tested point for that slot reuses the old answer. The cache is cleared each tick.

**Currents.** If the last point tested has current contents flags, base velocity gets 50 × level units/s along each flagged direction: +x for 0°, +y for 90°, −x for 180°, −y for 270°, and ±z for up/down. The "last point tested" is the deepest point reached.

**Level 1 (feet only)**: ordinary walking/air movement with both gravity halves, ground friction and jumping. Water has no other effect.

**Level ≥ 2: swimming.** Gravity is not applied at all. There is no buoyancy force: the only vertical pulls are the input-driven ones below. Per tick:
1. **Water-jump check**, only at exactly level 2 (see Water jump).
2. **Jump button**: if held, vz is set to 100 in water or 80 in slime (replaced, not added) and the ground is removed. This happens every tick it is held, with no release needed, and the swim move below then adjusts it. If not held, the jump-held flag is cleared.
3. **Swim move:**
   a. wishvel = F·f + R·s with the full 3D view vectors, then add an upward part:
      - jump held: + client max speed (the weapon speed, e.g. 250)
      - no forward, side or up input at all: −60 (drift down)
      - otherwise: + up move + clamp(2·f·F_z, 0, client max speed). Looking up while moving forward exaggerates the climb, but looking down adds nothing extra (the clamp floor is 0).
   b. wishspeed = |wishvel|, capped at M, then × 0.8. wishdir = wishvel normalized.
   c. **Friction**: speed = |v| (3D). newspeed = speed − dt·speed·sv_friction·surface_friction. Set it to 0 if below 0.1, then scale v to it. This is proportional at all speeds, with no stop-speed term.
   d. **Acceleration**, if wishspeed ≥ 0.1: add = wishspeed − newspeed (3D speed, not projection). If add > 0: amount = min(sv_accelerate·wishspeed·dt·surface_friction, add), and v += amount·wishdir.
   e. Add base velocity. dest = origin + v·dt. Sweep to dest.
      - If clear: sweep down from dest raised by (step size + 1) back to dest. If that does not start in solid, move to its end (this rides the player up onto steps of up to 19 units) and finish. If it starts in solid, do a normal slide move.
      - If blocked: with no ground, slide move. With ground, stair stepping.
   f. Subtract base velocity.
4. Ground detection, then vz = 0 if grounded.

There is no landing check and no second gravity half on swim ticks.

Notes:
- sv_wateraccelerate and sv_waterfriction exist, but the shared swim code does not read them. It uses sv_accelerate and sv_friction.
- Surface friction is whatever the last ground detection left. While rising slowly in water with no ground (0 < vz ≤ 140) it becomes 0.25, which cuts both water friction and water acceleration to a quarter on the next tick. Holding jump in water hits this every tick.
- Terminal speeds (sv_friction 4, sv_accelerate ≥ 4, surface friction 1):
  - swimming with full input: 0.8·M (200 with M = 250)
  - no-input sink: 48 (= 0.8·60)

**Water jump** (climbing out at a ledge). Checked only at exactly level 2, and only when not already water-jumping. All of these must hold:
1. vz ≥ −180.
2. Not backing up: if horizontal speed ≠ 0, the horizontal velocity direction must have a non-negative dot product with the flattened view forward.
3. Sweep the current box from the box centre (origin + (mins + maxs)/2) 24 units along the flattened forward. It must hit something, and the hit object must not be a physics object the player is carrying.
4. Sweep the box from (origin.z + eye height + 8) at the same x/y, 24 units forward. It must hit nothing.
5. From that second sweep's end, sweep down 1024 units. It must hit a plane with normal z ≥ 0.7.

When all hold:
- vz = 256.
- The stored water-jump velocity = −50 × (normal of the wall hit in step 3). That is 50 units/s toward the wall, onto the ledge, not away from it.
- The jump-held flag is set, so the player must re-press to jump.
- The water-jump timer = 2000 ms.

The rest of the tick that triggered it runs the normal swim move. With a level view and forward held, swim friction takes vz from 256 to 240, and no acceleration is added because 240 > 200. Horizontal velocity is not yet changed.

**While water-jumping** (timer > 0), each tick replaces the whole walking movement:
1. Water check. If the level is ≤ 1, apply the first gravity half only (−6.25 at 64 tick). At level ≥ 2, no gravity.
2. Count the timer down by 1000·dt ms. If it reaches ≤ 0, or the water level is 0, the water jump ends: timer 0, flag cleared.
3. Set horizontal velocity to the stored water-jump velocity, whether or not it ended this tick. vz is untouched.
4. Slide move, then water check. Skip everything else: no jump check, friction, acceleration, ground detection, second gravity half or landing check.

So vz holds constant while still waist-deep, then falls by only 6.25 per tick (half gravity) once only the feet are wet. The water jump does not end on landing, because no ground detection runs. It ends when the feet leave the water or after 2 s. A pressed jump does nothing during it.

**Entering and leaving water.**
- When the level changes between 0 and non-zero (either direction) across a tick, a swim sound plays and the server makes a splash.
- The time of entry is recorded; it is not used by movement.
- The shared code does not clamp or cut velocity on entry. A fall into deep water keeps its speed and is then slowed only by the proportional water friction (6.25% per tick).
- Landing: there is no landing check on swim ticks (level ≥ 2), so there is no fall damage. A walking-tick landing at level 1 with fall speed ≥ 350 skips the damage and volume tiers. It still plays the landing sound and kicks the view roll at volume 0.5 (see Falling and landing).

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
- Each ground detection first resets surface friction to 1, so it only stays below 1 while you are on that material. It is 0.25 whenever detection runs its sweep while airborne and rising slowly (0 < vz ≤ 140; see Ground detection). This includes the slow upper part of every jump, and it cuts air acceleration to a quarter there.
- Material max-speed factor scales M. Material jump factor scales the jump impulse.

### Base velocity (conveyors, moving ground)

- Base velocity is an extra velocity from the environment (conveyors, water currents, the ground's motion).
- Friction runs before base velocity is added, so standing on a conveyor carries you with no slowdown.
- Each move adds base velocity to v before the slide move and subtracts it after. It moves you but is never stored in v.
- Its z is folded into vz once at the start of the tick (× dt) and then cleared.
- On leaving the ground, the ground entity's velocity is added to base velocity, with z taken directly from the ground's vz. On landing on an entity, its velocity is subtracted the same way.

### Stuck recovery (brief)

- **When it runs:** on ticks where (command number + player index) is a multiple of the interval in ticks: int(1.0 s / dt), so 64 at 64 tick (0.2 s in single player). While a nudge sequence is in progress (the nudge index is not 0), it runs every tick.
- **Test:** the box at the current origin. If it is free, reset the nudge index to 0 and continue normally.
- **If stuck (server):** at most one nudge is tried per attempt, and attempts are at least 0.05 s apart. Take the next entry of a fixed 54-entry nudge table (the index advances and wraps). Test origin + nudge. If it is free, move there, reset the index, and the tick continues. Otherwise the whole movement for this tick is skipped (no friction, no gravity, no move). This also happens on ticks inside the 0.05 s gap.
- **The client prediction side, when stuck against the world:** tries all 54 entries in order at once before falling back to the above.
- **Nudge table, in order:**
  1. z ∈ {−0.125, 0, +0.125}
  2. y ∈ {−0.125, 0, +0.125}
  3. x ∈ {−0.125, 0, +0.125}
  4. the 8 corners (±0.125, ±0.125, ±0.125), x outermost, then y, then z, each from − to +
  5. z ∈ {0, 1, 6}
  6. y ∈ {−2, 0, 2}
  7. x ∈ {−2, 0, 2}
  8. for z in {0, 1, 6}: x ∈ {−2, 0, 2} × y ∈ {−2, 0, 2} (x outer)
  9. one final (0, 0, 0) entry, which brings the count to 54
- There is no "last good position" fallback. A player stuck where no nudge helps stays frozen, retrying every tick. Low priority for the prototype.

## Per-tick order

One user command, walking move type, not in water:

1. Scale dt by the per-player time scale.
2. Compute the effective max speed M. Rescale f, s, u to |(f,s,u)| ≤ M. Zero them if frozen or dead.
3. Decay the punch angle. View angles = command angles + punch.
4. Count down the duck, duck-jump, jump and swim-sound timers by 1000·dt ms.
5. Compute the forward/right/up vectors from the view angles.
6. Stuck check: on the 1 s interval tick, or every tick while a nudge sequence is in progress. If still stuck after this tick's nudge (or nudging was rate-limited), end the tick.
7. Ground: full ground detection runs here if the move type is not walking, or game code moved the player since the last tick (teleport, spawn, trigger push that sets position). Otherwise only: if vz > 250, remove ground.
8. Remember the water level. If airborne, fall speed = −vz.
9. Update the duck-jump eye offset (single-player only), then Ducking (incl. the 1/3 input crop when ducked on ground).
10. Ladder detection (see Ladders). This runs after ducking. It may enter ladder mode and set v. It may leave ladder mode, keeping v. A jump off a ladder leaves ladder mode with v = 270·n.
11. By move type. For ladder mode: water check, (jump-held flag cleared if not held), add base velocity, slide move, subtract base velocity, end. Ground detection for ladder mode already ran in step 7, because the move type was not walking then. For walking (including the tick a player leaves a ladder):
    a. Water check. If below waist depth, apply the first gravity half (and fold base vz in), then clamp velocity.
    b. If water-jumping: count down the timer (end it if ≤ 0 or level 0), set horizontal v to the water-jump velocity, slide move, water check, end. Gravity this tick is only the half from (a), and only if level ≤ 1.
    c. If at waist depth or deeper: water-jump check (level exactly 2), jump button (sets vz 100/80), swim move (friction, acceleration, step-or-slide), ground detection, vz = 0 if grounded. End: no gravity, no landing check.
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
| (all air rows below unless stated: airborne with surface friction 1, i.e. falling, or rising with vz > 140) | | | |
| airborne at rest horizontally, airaccelerate 10 | hold forward | 1 tick | horizontal speed 30 (add capped at 30, not 39.0625) |
| same | hold forward | 2+ ticks | stays 30 |
| airborne, v = (250,0), view yaw 0 | hold right only (wishdir ⟂ v) | 1 tick | speed = √(250² + 900) = 251.794; velocity turns 6.843° |
| same, turning view each tick to keep wishdir ⟂ v | hold right | 10 ticks | 267.395 |
| same | – | 64 ticks | 346.554 |
| airborne, rising with 0 < vz ≤ 140 at the last detection (surface friction 0.25), v = (250,0) | hold right only (wishdir ⟂ v) | 1 tick | amount 39.0625 × 0.25 = 9.766 (< 30); speed = √(250² + 9.766²) = 250.191 |
| standing jump from 250 horizontal, perfect perpendicular strafe every tick | hold right, turn each tick | ticks 1–42 (to landing) | friction 1 on ticks 1–11 and 23–42 (31 ticks, +900 to |v|² each); 0.25 on ticks 12–22 (11 ticks, +95.37 each); landing speed ≈ √(62500 + 31·900 + 11·95.37) = 302.41 (±0.1). The jump tick itself counts as an air tick, because the ground is removed before friction. |
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
| standing on ground, not ducked | press duck and hold 26 ticks, moving forward | tick 26 (duck completes) | input on tick 26 not yet scaled; first scaled (×1/3) tick is 27 |
| ducked on ground, room to stand | release duck | tick 13 (unduck completes) | input on tick 13 still scaled ×1/3; tick 14 unscaled |
| ducked on ground, releasing duck, unduck in progress (eye partway, e.g. tick 5: 28 + 36·(1 − S(1 − 5/12.8))) | press duck again and hold | each tick | eye stays at the tick-5 value, box stays ducked, jump presses do nothing |
| same, then release | – | release tick | eye snaps to 28, then the normal 13-tick unduck |
| standing, duck transition at tick 10, obstruction now 50 units above floor | release duck | that tick | forced full duck: ducked box, eye 28 |
| ducked in air, 40 units of free space below the feet but solid 10 units above the head | release duck | each tick | stays ducked (the standing box at the start of the sweep is in solid) |
| ducked in air, solid 20 units below the feet, room above | release duck | each tick | stays ducked (sweep down 36 blocked) |
| server, player box overlapping a wall by 0.1 units on its +x side | any | from the next stuck-check tick | one nudge per attempt, attempts ≥ 0.05 s apart (every 4th tick at 64 tick); z ±0.125/0, y ±0.125/0 and x −0.125 are tried in order; the 7th attempt (x −0.125) frees it; movement is skipped on every tick until then |
| player teleported by game code onto a floor | – | next tick | full ground detection runs at tick start (grounded before friction) |
| airborne, standing | press duck | same tick | ducked box, origin z + 36, eye 28 |
| ducked, airborne, 20 units above floor | release duck | each tick | stays ducked (needs 36 units below) until ground |
| walking at 250 into an 18-unit step | hold forward | contact tick | climbs: z rises by 18, horizontal progress continues |
| walking at 250 into a 19-unit ledge | hold forward | contact tick | blocked: z unchanged, slides along the wall |
| walking off an 18-unit stair edge going down | hold forward | each tick | stays on ground (stick-to-ground snap), no fall |
| airborne into a vertical wall at 45° | none | contact tick | velocity component into the wall removed, tangential kept |
| on ground into a 2-wall 90° corner | hold forward into it | contact tick | velocity → 0 (slides along the first wall into the second, whose clip leaves nothing) |
| sv_maxvelocity 3500 | vz would be −3510 | after clamp | vz = −3500 |
| ground material friction 0.25 (ice-like, × 1.25 = 0.3125) | hold forward from rest, accel 5 | 1 tick | speed 6.1035 |
| **Ladders**: vertical ladder face 1 unit in front of the player, ladder normal n = (−1,0,0), player yaw 0 (facing it), in the air (not on floor), already in ladder mode (so the probe is along −n and pitch does not matter for attaching), unless stated | | | |
| not on ladder, no keys | – | 1 tick | not attached (no probe without input); falls normally |
| not on ladder, ladder 3 units away | hold forward | 1 tick | not attached (2-unit reach) |
| pitch 0 | hold forward | 1 tick | attached; v = (0,0,200); z +3.125; no gravity |
| pitch −45 (looking up) | hold forward | 1 tick | v = (0,0,282.843) |
| pitch +45 | hold forward | 1 tick | v = (0,0,0) |
| pitch +60 | hold forward | 1 tick | vz = 200·(0.5 − 0.866025) = −73.205 |
| pitch +89 | hold forward | 1 tick | vz = −196.48 |
| pitch −89 | hold forward | 1 tick | vz = +203.46 |
| pitch 0 | hold back | 1 tick | v = (0,0,−200) |
| pitch 0 | hold right only | 1 tick | v = (0,−200,0) (sideways along the face, no dampening: angle test gives dot 0) |
| pitch 0 | hold forward + right | 1 tick | CS:S dampening applies (dot −0.70711 < −0.707): v = (0,−40,200). Without dampening it would be (0,−200,200). |
| pitch −45 | hold forward + right | 1 tick | dot −0.57735, so no dampening: v = (0,−200,282.843) |
| on ladder, mid-climb | release all keys | 1 tick | v = (0,0,0), hangs (no gravity, no momentum) |
| on ladder | hold forward and duck | 1 tick | climb speed unchanged (200 at pitch 0) |
| on ladder, in the air | press jump (no keys) | that tick | leaves ladder; v = (−270,0,·); walking air move: vz −6.25 during the move, −12.5 at the end; x −4.21875 |
| same | hold forward toward the ladder | next tick | not re-attached (4.22 > 2 units) |
| on ladder, on floor at the bottom | hold back, pitch 0 | 1 tick | pre-slide v = (−200,0,−200); floor clips it to (−200,0,0) |
| climbing at vz 200, box rises past the ladder top | hold forward | the tick the probe misses | walking mode with v = (0,0,200); rises about 25 more units under gravity |
| **Water**: sv_friction 4, M = client max speed = 250, surface friction 1 at start, no ground, water deep enough for level 3, unless stated | | | |
| standing box, water surface at height H above the feet | – | level | 0 if H ≤ 1; 1 if 1 < H ≤ 36; 2 if 36 < H ≤ 64; 3 if H > 64 (an exact-surface point is ambiguous, avoid it in tests) |
| ducked box, surface H | – | level | 2 if 18 < H ≤ 28; 3 if H > 28 |
| level 3, at rest, sv_accelerate 5 | no input | ticks 1 / 2 | vz = −3.75 / −7.265625 (wish −60·0.8 = −48) |
| same | no input | ticks 23 / 24 / 25 | −46.40 / −47.25 / −48.0, then stays −48 |
| level 3, at rest, sv_accelerate 10 | no input | ticks 1 / 2 / 7 / 8 | −7.5 / −14.53 / −43.62 / −48.0 |
| level 3, at rest, sv_accelerate 5, pitch 0 | hold forward | ticks 1 / 2 | speed 15.625 / 30.273 (horizontal) |
| same | hold forward | ticks 24 / 25 | 196.88 / 200.0, then stays 200 |
| level 3, at rest, sv_accelerate 5, pitch −30 | hold forward | tick 1 | v = 15.625 × (0.5, 0, 0.866025) = (7.8125, 0, 13.5316): it swims 60° up (forward·250 plus an extra 250 upward, then normalized) |
| same | hold forward | tick 2 | speed 19.287, same direction: surface friction is 0.25 after tick 1 (rising, no ground), so friction −0.244 and accel +3.906 |
| level 3, at rest, sv_accelerate 5, pitch +30 | hold forward | tick 1 | v = 15.625 × (0.866025, 0, −0.5): no extra downward exaggeration |
| level 2 or 3, at rest, sv_accelerate 5, no move keys | hold jump | tick 1 | vz 100 → friction 93.75 → +15.625 → 109.375 |
| same | hold jump | tick 2 and every tick after | 102.34375 (vz reset to 100; surface friction 0.25: −1.5625, +3.90625) |
| same, sv_accelerate 10 | hold jump | tick 1 / tick 2+ | 125.0 / 106.25 |
| level 2, facing a vertical wall 10 units ahead whose top is below eye height + 8, with walkable ground on top, and open space above it; horizontal speed 0, pitch 0 | hold forward | trigger tick | water jump starts: vz set to 256, then swim friction → 240 (no accel, 240 > 200); horizontal still 0 |
| same | – | following ticks while level ≥ 2 | v = 50 toward the wall horizontally, vz stays 240 (no gravity) |
| same | – | ticks at level 1 | vz drops 6.25 per tick (half gravity only); horizontal 50 toward the wall |
| same | – | feet leave water (level 0) or 2000 ms pass (128 ticks) | water jump ends; normal walking/air movement from the next tick |
| water jumping | press jump | – | no effect |
| level 2, sinking at vz −200 | facing a ledge, hold forward | – | no water jump (vz < −180) |
| falling at 600 into level-3 water | no input | first swim tick | no fall damage; speed × 0.9375 per tick toward the sink terminal (no velocity cut on entry) |

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

**Hulls, eyes and ducking (measured with the movement probe, reading the player's collision box and view offset every tick):**
- Standing box (−16,−16,0)–(16,16,**62**), eye **64**. Ducked box top **45**, eye **47**.
- Ducking in the air moves the feet up by **half** the height difference (8.5 units); unducking in the air moves them down by 8.5. The shared code moves them by the whole difference.
- Duck and unduck timings are as in the shared code: the ducked box is in use 27 ticks after the press at 66.67 tick (0.405 s > 0.4 s), the standing box 14 ticks after release (0.21 s > 0.2 s), with the same eye curve between 64 and 47.
- Ducked walking: the input scale is **0.34** (wish speed 85 at max speed 250), not 1/3.

**Jump stamina (measured; CS:S-only, not in the shared code):**
- Each jump sets a stamina value to 1315.79 ms. It counts down by 1000·dt every tick (15 per tick at 66.67 tick) and stops at 0, so it lasts about 88 ticks (1.3 s).
- Jump speed: if stamina is above 0 when a jump starts, the jump's vertical speed is multiplied by 1 − 0.00019 × stamina. Then stamina is set again. Measured: a jump 50 ticks after the previous one (stamina 565.79) gives r = 0.8925.
- Ground speed: on each tick on the ground, after friction and before acceleration, horizontal velocity is multiplied by 1 − 0.000199 × stamina. Fit over 30+ ticks of landing traces to ~0.1 units/s. In the air it does nothing.
- Effects: landing from a running jump with forward held dips to about 148 units/s and recovers to 250 in about 40 ticks. Releasing forward on landing stops the player in about 18 ticks, against about 45 with friction alone.

**Bunny-hop cap (measured):**
- With sv_enablebunnyhopping 0 (the default), a jump scales the velocity down to a 3D speed of 286 when it is faster. The 3D speed includes the −6 vz of the jump tick's first gravity half. The cap doesn't change with the held weapon (knife and AWP both give 286; 286 = 1.1 × 260, the fastest weapon's speed).
- With sv_enablebunnyhopping 1 there is no cap.
- sv_autobunnyhopping 1 (present in the current CS:S) lets a held jump button jump again on landing.

**Bots:** CS:S bots (even with bot_zombie or bot_stop) are ducked by the game from their jump tick until about 44 ticks later, with no duck button. Humans aren't: a human standing jump measures as an unducked jump. Probe comparisons of jump heights must allow for this; horizontal measurements are unaffected.

Knife script: MaxPlayerSpeed 250, Damage 50, WeaponArmorRatio 1.7, Range 4096 (the bullet-weapon field; the knife's swing reach is in code), Penetration 1.

### Measured with movecmp fuzz (2026-10-05, ladders on de_nuke, water on de_nuke)

- **Swimming lift**: holding jump in water (and the look-up exaggeration's clamp) uses 260, the base player speed, not the weapon speed. The wish speed is still capped at the weapon's 250. Checked tick-exact (e.g. (12.2885, 0.0936, 101.3716) after a jump-held tick).
- **Duck input scale** (0.34): applies while duck is held, or when the tick started ducked or mid-duck: from the tick duck is pressed until the tick after it's released, on the ground, in the air and in water. Measured as a wish speed of exactly 85 on the press tick while still unducked.
- **Ladders**: ladder velocity gets the same duck scale (e.g. 46.2 vs 135.8 sideways).
- **Contents cache**: confirmed at the water surface: the eye point tested dry at the start of a tick keeps answering dry after sinking 0.6 units that tick (level 2 for one more tick).
- **Coincident faces**: where a player-clip face coincides with a ladder brush's face, the trace reports the clip, so the ladder isn't found from that side (de_nuke, ladder at (856..864, -1448..-1422)).
- **Remembered ladder** (to check further): after using a ladder, CS:S grabbed the same ladder later with no input while falling past it; on a fresh map it doesn't. Not modelled.
- **Bots** duck on their own when they jump (inside the game, not through their buttons). Humans don't.
- Still open from the fuzz: a one-tick-earlier landing on a ledge edge under water, and a few ladder runs (de_nuke) where on-ladder velocity or attachment differs after many ticks.

## Open questions

1. **Scoped speeds:** weapon speeds are now read from the scripts. Still to check: scoped speeds (AWP/scout/autos are slower zoomed; that is in CS:S code, not in scripts).
2. **Answered:** sv_accelerate 5 and sv_stopspeed 75 come from CS:S's stock cfg/skill1.cfg, which the engine runs on every map load. They are CS:S's effective defaults; the registered defaults 10 and 100 are the shared code's.
3. **Answered:** sv_stopspeed is 75 (see CS:S values).
4. **Answered:** the CS:S jump impulse is 301.993 (see CS:S values).
5. **Answered (see CS:S values):** hulls 62/45, eyes 64/47, air duck shift 8.5. Was: **CS:S hulls and eye heights:** CS:S is believed to use a 32×32×62 ducked box (not 36) and ducked eye ≈46, standing eye 64. The air-duck lift and the in-air unduck clearance both equal (standing height − ducked height), so they change with it (36 with the shared boxes). Measure with `cl_showpos`/getpos while standing vs ducked, and by finding the smallest vent you can enter.
6. **CS:S duck behavior:** duck speed (CS:S slows while ducking/ducked: is it ×1/3 of input or a fixed fraction of max speed?), duck spam limit (sv_timebetweenducks or similar), whether the transition is still 0.4 s / 0.2 s, and any air-duck differences.
7. **Answered (see CS:S values: jump stamina).** Was: **Landing slowdown / stamina:** CS:S reduces horizontal speed after landing and after jumps (a "stamina" or velocity modifier). This is not in shared code. Measure speed in the ticks after landing from walking jumps, bhop chains, and a fall.
8. **Answered (see CS:S values: bunny-hop cap, 286).** Was: **Bunny-hop limit:** with sv_enablebunnyhopping 0, CS:S caps speed on jump (believed: if speed > 1.1 × max speed, scale down to 1.1 × max speed). Measure by jumping at > 275 units/s with a knife.
9. **Walk key (+speed):** CS:S walking speed. Believed to be a fraction of max speed (≈0.52). Measure.
10. **Backpedal:** whether sv_backspeed 0.6 is applied in CS:S. Measure backward top speed.
11. **Fall damage formula as applied by CS:S:** the shared header gives the slope 100/(1024 − 580). CS:S applies the actual damage (and may scale it, e.g. ×1.25 is sometimes cited). Measure HP lost from known drop heights.
12. **Client move key speeds:** cl_forwardspeed/cl_sidespeed/cl_backspeed in CS:S (believed 400 or 450). Only matters if they are below max speed, which they are not.
13. **Ladder climb speed in CS:S:** shared is 200. Confirm.
14. **func_ladder solidity:** the ladder probe uses the player-solid mask, so a ladder must block the player to be found. Confirm how CS:S maps compile func_ladder brushes (contents seen by the probe) by reading the BSP brush contents of a ladder in a map with our loader, and confirm that the player rests against the ladder face.
15. **Water currents and base velocity:** the shared code adds the current's 50 × level push to base velocity on every water check. That happens several times per tick, and the shared movement never resets base velocity. The engine or player code presumably resets it each tick. Measure drift speed in a current if any CS:S map has one; otherwise ignore.
16. **CS:S ladder and water overrides:** confirm in the live game the 200 climb speed, the pitch-dependent climb speed (282.8 at 45° up), the 270 jump-off, the swim terminal of 0.8 × max speed, and the 256/50 water jump. CS:S code could override any of these.
17. **Tick rate:** the reference listen server runs 66.67 tick. Bhop, surf and KZ community servers commonly run 66 or 100. The test cases above assume 64.
