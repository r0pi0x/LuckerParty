# Source engine: +use, doors, buttons and moving brushes

Source basis: Valve's public Source SDK 2013 only. I read the shared player
use code (finding the use target, press/hold/release handling), the server
door code (func_door, func_door_rotating, func_water), the shared
"toggle" mover helpers (linear and angular moves, axis selection, lip,
wait), the button code (func_button), func_movelinear, func_rotating,
func_tracktrain and path_track, the model-door code (prop_door_rotating),
the server physics code for pushers (how a moving brush carries, pushes and
is blocked by players), the player command runner (moving-ground handling)
and the shared movement's ground-entity velocity handling. The door code
contains CS:S-specific lines (a bot event), so CS:S uses this same door
code. CS:S's own player class is not public (it may override use finding).
Real examples: de_nuke (func_door_rotating pairs, func_rotating),
cs_office (func_door, func_button), de_prodigy and de_inferno (func_door,
func_door_rotating), cs_assault (prop_door_rotating), all from the user's
install.
Status: draft

## Summary

Moving brushes ("pushers") move by setting a velocity (or angular velocity)
and a "move done" time; each tick the engine advances them by velocity × dt
(the last step shortened so they stop exactly at the goal), drags along
players standing on them, shoves players in their way, and, if a player
cannot be moved out of the way, refuses to move that tick and reports
"blocked" (doors then deal crush damage and usually reverse). Doors open
along a direction by their own size minus a lip, wait, and come back.
Rotating doors swing away from whoever opened them. Buttons are short
doors that fire outputs. The +use key finds a usable entity with a line
straight ahead (up to 1024 units, but it must be within 80 units of the
player's body) and with a few downward-angled box traces and a radius
search, and sends it "Use" once per key press.

## Units and conventions

Source units, Z up. Angles (pitch, yaw, roll) degrees; pitch positive =
down, so movedir "−90 0 0" is straight up. Linear speeds units/s, angular
speeds degrees/s. dt = 0.015 s. Player box CS:S 32×32×62 standing, eye
height 64 (specs/cs_source/movement.md). Spawnflag values are what Hammer
shows. "Brush size" means the brush model's local bounding box size along
each axis.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| use_radius | 80 | units | maximum +use distance (see distance rule) |
| use_trace_long | 1024 | units | first (straight) use trace length |
| use_trace_box | 72 | units | length of the angled box traces |
| use_box_half | 16 | units | half-size of the angled trace box |
| use_cone_dot | 0.8 | — | radius search: cos of max angle off the view (≈ 36.9°) |
| door_default_speed | 100 | units/s or deg/s | func_door, func_door_rotating, prop_door_rotating, func_movelinear |
| door_size_shrink | 2 | units | subtracted from the brush size before the lip |
| door_portal_close_delay | 0.5 | s | areaportals close this long after a door fully closes |
| door_lock_sound_wait | 1 | s | min time between door locked/unlocked sounds |
| button_lock_sound_wait | 0.5 | s | same for buttons |
| button_default_speed | 40 | units/s | func_button |
| button_default_wait | 1 | s | func_button |
| button_default_lip | 4 | units | func_button |
| button_uselocked_wait | 0.5 | s | min time between OnUseLocked outputs |
| rot_default_maxspeed | 100 | deg/s | func_rotating |
| rot_step | 0.1 | s | func_rotating spin-up/down step |
| train_default_maxspeed | 100 | units/s | func_tracktrain "startspeed" 0 |
| train_lookahead | 0.1 | s | train steers toward the path point this far ahead |
| train_controls_height | 72 | units | train control box extends this far above the train |
| propdoor_default_distance | 90 | deg | prop_door_rotating |

## Behavior

### +use: finding what to use

Runs every player command before movement (in the pre-think), whenever the
use key is down, was just pressed or just released.

Usable entities: those that declare "impulse use" (doors with "Use opens",
buttons with "Use activates", prop_door_rotating unless "Ignore use",
func_rotating, weapons/C4 in CS:S...), "continuous use" (momentary
buttons), "on/off use", or "directional use" (trains). Most also declare
"usable in radius". If a hit entity is not usable, its parent, grandparent
... is tried.

Search (from the eye position E along the view direction f, with the
player's own up vector u):
1. **Trace 0**: a line from E to E + 1024·f against solid geometry,
   players/NPCs and clip brushes (CS:S's mask: brush-only NPC solids plus
   opaque-and-NPC contents; see Open questions). If it hits a usable entity
   (or usable ancestor) and the **use distance** of the hit point is < 80,
   return it immediately.
2. **Traces 1..7**: box traces (box ±16) from E, length 72, along
   normalize(f − t·u) for t = 1, 0.57735, 0.36397, 0.26795, 0.17633,
   −0.17633, −0.26795 (45°, 30°, 20°, 15°, 10° down, then 10°, 15° up). Each
   usable hit within use distance < 80 becomes the candidate (later traces
   overwrite earlier ones).
3. If the player stands on a usable entity that allows "use on ground",
   that becomes the candidate.
4. Candidate score: distance from the candidate's nearest point (to E) to
   the view line.
5. **Radius search**: every entity within 80 of E that is usable in radius:
   take its nearest point P to E; skip if dot(normalize(P − E), f) < 0.8;
   score = distance from P to the view line; if lower than the best, check
   a line E → P is clear (or hits that entity) and take it.
6. Return the best (or nothing).

**Use distance** of a hit point H: horizontal components of H − E as is,
vertical component replaced by how far H.z is outside the player's own box
height range [feet, head] (0 if within). So anything level with the
player's body counts only its horizontal distance; a button at foot level
30 units ahead is at distance 30, not √(30² + 64²).

**Sending use**:
- If found and (key held and the entity is "continuous") or (key just
  pressed and the entity is "impulse" or "on/off"): send it Use with
  activator = caller = player; type "on" for on/off entities, else
  "toggle". Continuous use also marks the player as "using".
- Else if found, key just released and the entity is "on/off": Use with
  type "off".
- Else if nothing found and the key was just pressed: the player's "use
  denied" sound. The shared player plays nothing; HL2 overrides it. What
  CS:S plays is an open question (the install's sound scripts define
  "Player.UseDeny" twice: common/use_deny.wav at volume 1, and
  common/wpn_select.wav at 0.4).
- Press handling also covers trains: pressing use while standing on a
  func_tracktrain's control area (and not jumping) takes control of it
  instead (out of scope unless a target map needs it).

So doors and buttons react **once per press** (holding the key does not
re-trigger), and the first trace takes priority: looking straight at a
button within reach always uses that button.

### Moving brush basics ("pushers")

A door, button, func_movelinear, func_rotating, func_tracktrain is a
**pusher**: solid to players, moved only by its own logic.

- A **linear move** to a goal G at speed s: if already at G, finish
  immediately. Else velocity = (G − origin)/T with T = |G − origin|/s, and
  "move done" time = local time + T.
- An **angular move** to angles A at speed s (deg/s): T = |A − angles|/s
  (length of the angle difference vector), at least 0.01 s; angular
  velocity = (A − angles)/T; same "move done" time.
- Each tick, a pusher's step time = min(dt, time left until move done)
  (time left only counts while a move-done time is set). It moves (or
  rotates) by velocity × step time, then if its local time has reached the
  move-done time, it snaps exactly to the goal, stops, and runs its
  "arrived" logic. So a move of T = 0.58 s takes 38 full ticks plus a
  39th partial tick, and arrives at the goal exactly at tick 39.
- A pusher's local time only advances when its step succeeded; a blocked
  step is undone and does not count (the arrival moves later).
- A waiting period with zero velocity also uses the move-done time (e.g. a
  door's "wait"), counted in the same local time with partial last steps:
  wait 4 s ends on the 267th tick after arrival (266 × 0.015 = 3.99, then a
  0.01 s step).
- Thinks (sound loops, areaportal closing) run separately on the normal
  nearest-tick think schedule.

### Pushing, riding and blocking (one pusher step)

For a pusher (and everything parented to it) moving by vector m this tick
(or rotating by angle a):
1. Move/rotate the pusher (and its children) to the new position.
2. Collect candidates: entities in the pusher's bounding box (expanded
   backwards along m for linear moves), that are solid, not themselves
   pushers/static/physics/noclip, whose collision group collides with the
   pusher's, not in the pusher's hierarchy, and that either **stand on**
   the pusher (their ground entity is the pusher) or **overlap** the
   pusher at its new position. A candidate that is parented is replaced by
   its top-most parent.
3. For each candidate (in reverse collection order), with the pusher made
   temporarily non-solid:
   - Push vector: m for a linear move. For a rotation: where the
     candidate's reference point ends up when carried by the rotation,
     minus where it was (the reference point is the candidate's centre for
     brush-solid pushers, or the corner of its box facing the motion for
     physics-solid pushers).
   - Trace the candidate's box from its origin along the push vector
     against everything except the pusher; move it to the end of the trace
     (if it moved at all).
   - Linear push that went the full distance: accepted (the candidate is
     not re-checked against the pusher).
   - Otherwise (partial trace, or any rotational push): test whether the
     candidate's box at its new spot is inside anything solid (the pusher
     included, movable physics props ignored). If not: accepted. If yes:
     **blocked**, unless the pusher is "unblockable by player" (some trains,
     doors with "Non-solid to Player") and the candidate is a player: then
     the player is placed at the full push destination, or nudged ±half
     the pusher's local X or Y axis if that frees them, and the push is
     accepted anyway.
4. If any candidate blocked: put the pusher and **all** candidates back,
   don't advance the pusher's local time, and report the blocking
   candidate. Else: re-link pusher and candidates against triggers, notify
   touched entities, and for rotations turn riders: players get the yaw
   change added to their view (and angular velocity yaw set), other
   entities get their yaw rotated.
5. Blocked reporting: if the blocker changed since last tick, the pusher
   gets "end blocked" (old) and "start blocked" (new); then "blocked" (the
   class's blocked behaviour) runs **every tick** the push fails.

Riding: a player standing on a pusher is a candidate every tick and is
carried by the push vector before their own movement runs (or after,
depending on simulation order in the frame). Their own velocity is not
changed by being carried. When the player **leaves the ground** of a moving
pusher (jumps or walks off), the shared movement adds the pusher's velocity
(z set to the pusher's z velocity) to the player's base velocity, which the
next command turns into real velocity (×(1 + dt/2)); when **landing** on a
pusher, the pusher's velocity is subtracted in the same way. So jumping off
a moving platform keeps its speed; landing on one cancels it.

Players are pushed sideways by a door swinging into them the same way (a
partial trace moves them as far as possible; if they end up inside the
door or the world, the door is blocked).

### func_door

Keyvalues: "movedir" (angles; the door's travel direction), "speed"
(units/s, 0 → 100), "lip" (units, may be negative), "wait" (s; −1 = never
return), "spawnpos" (0 closed, 1 open), "dmg" (crush damage per blocked
tick), "forceclosed" (0/1), "noise1" (moving sound), "noise2" (arrive
sound), "startclosesound", "closesound" (optional separate closing
sounds), "locked_sound", "unlocked_sound", "loopmovesound" (0/1),
"chainstodoor" (name of another door to use/touch along), "ignoredebris",
"health" (unused by this code), "locked_sentence"/"unlocked_sentence" (HL1
voice lines, ignore).

Spawnflags: 1 "Starts Open" (obsolete form), 4 "Non-solid to Player", 8
"Passable" (completely non-solid), 32 "Toggle" (no auto return), 256 "Use
Opens", 512 "NPCs Can't", 1024 "Touch Opens", 2048 "Starts locked", 4096
"Silent", 65536 "New use rules" (newer builds).

Positions: closed P1 = spawn origin; distance D = |dir.x|·(sx − 2) +
|dir.y|·(sy − 2) + |dir.z|·(sz − 2) − lip with dir = forward(movedir) and
(sx, sy, sz) the brush size; open P2 = P1 + dir·D. "spawnpos" 1 (or flag 1)
starts the door at P2 in the open state.

States: closed, opening, open, closing.

- **Open** (going up): play the moving sound (unless already moving or
  Silent; loops for its sound length while moving if loopmovesound),
  linear move to P2 at speed, fire **OnOpen** (activator = door). Areaportals
  that target this door's name are opened.
- **Arrived open**: stop moving sound, play arrive sound. If "Toggle" (32):
  wait for the next activation (touch re-enabled). Else: after "wait"
  seconds of door local time, close (wait −1: never). Fire **OnFullyOpen**
  (activator = door). (With the obsolete "Starts Open" flag, Open/Closed
  outputs are swapped.)
- **Close** (going down): moving sound (or startclosesound), linear move to
  P1, fire **OnClose** (activator = door).
- **Arrived closed**: arrive sound (closesound if set), touch re-enabled,
  fire **OnFullyClosed** (activator = the last user), close areaportals
  0.5 s later.

Activation:
- **Touch** (flag 1024, players only): if locked: fire OnLockedUse and play
  the locked sound; else activate. While moving the door ignores further
  touches until it arrives somewhere.
- **Use** by a player: if flag 256 is not set, play the locked sound and do
  nothing. Allowed only when closed (or with "New use rules": closed or
  closing), or open with "Toggle". Locked: OnLockedUse + locked sound.
  Else activate.
- **Activate**: if "Toggle" and open: close. Else play the unlocked sound
  and, unless already open or opening, open.
- chainstodoor: a Use or touch also uses/touches every door with that name
  (not recursively back).

Inputs: **Open** (if not open/opening and not locked: unlocked sound, open;
locked → ignored), **Close** (if not closed: close; ignores the lock;
restarts the closing move and fires OnClose again if already closing),
**Toggle** (only when fully open or fully closed and not locked),
**Lock**, **Unlock**, **SetSpeed** (float; applies to the next move),
**SetToggleState** (teleport to open/closed position without moving).
Inputs Open/Close/Toggle do **not** change the stored "last user", so a
rotating door opened by input swings using the previous user (or its
default direction).

Blocked (every blocked tick):
1. If dmg ≠ 0: damage the blocker by dmg, type crush (1), attacker = door.
   (Physics objects that can't take damage, with forceclosed or wait −1,
   are instead pushed through by a physics solver.)
2. If forceclosed: stop here (the door keeps pushing until free).
3. If wait ≥ 0: reverse: closing → open, opening → close (the direction
   flips on the first blocked tick, so a door with dmg hurts once per
   reversal in practice).
4. Doors that share this door's targetname and move the same way are
   snapped to this door's position and reversed too.
5. Outputs: when blocking starts **OnBlockedOpening** / **OnBlockedClosing**
   (activator = blocker); when it ends **OnUnblockedOpening** /
   **OnUnblockedClosing**.

A wait −1 door that is blocked neither reverses nor moves: it pushes
(and damages, if dmg) every tick until the blocker is gone.

Sounds default, if empty: moving "DoorSound.DefaultMove", arrive
"DoorSound.DefaultArrive", locked "DoorSound.DefaultLocked", unlocked
"DoorSound.Null" (rotating doors use the "RotDoorSound." versions). All
door sounds are on the static channel at normal level, reliable.

### func_door_rotating

Same keyvalues/flags as func_door, plus "distance" (degrees), "solidbsp".
Axis by spawnflag: default yaw (around Z); 64 "X axis" = roll; 128 "Y axis"
= pitch; 2 "Reverse Dir" negates; 16 "One-way" disables the "away from
user" choice. Closed angles A1 = spawn angles; open A2 = A1 + axis ×
distance. "spawnpos" 1 starts open. The rotation centre is the brush's
origin (the origin brush in Hammer).

**Opening away from the user** (yaw doors only, not one-way, with a known
user U): let N = the point of the door's box nearest to U's origin, a =
(door origin − U origin) and b = (N − U origin), both flattened to z = 0.
If the z of a × b is > 0 the door opens to A1 − axis·distance, else to A2
(it closes back to A1 either way). This is evaluated each time the door
starts opening (including reversals after blocking).

de_nuke's doors: spawnflags 1280 (use opens + touch opens), distance 90,
speed 200 (deg/s), wait 4, dmg 0, paired with chainstodoor so both halves
open together. They are brush doors (func_door_rotating), not model doors.

### prop_door_rotating (model doors; cs_assault, de_port, cs_compound)

Keyvalues: "model", "hardware" (handle type; 0 = none), "distance" (deg, 0
→ 90), "speed" (deg/s, 0 → 100), "returndelay" (auto close after open; −1
never), "opendir" (0 both ways, 1 forward only, 2 backward only),
"spawnpos", "ajarangles", "axis" (Hammer helper; the hinge is the model
origin, rotation about Z), "forceclosed", "slavename" (doors that move
with it), sound override keyvalues. Spawnflags: 1 starts open, 2048 starts
locked, 4096 silent, 8192 "Use closes", 16384 silent to NPCs, 32768
"Ignore +use".

- Open forward = closed yaw − distance, open back = closed yaw + distance
  (swapped if the model's hinge is on the left).
- **Use**: if closed (or open with "Use closes"): locked → locked sound,
  OnLockedUse; else open. If opening and "Use closes": close. If closing
  or ajar: open again.
- **Direction** (opendir 0): opens "back" if the user's origin is in front
  of the door (dot(door forward, user) > dot(door forward, door origin)),
  else forward. If the chosen side's swept volume is blocked when a player
  opens it, the other side is used.
- After fully open: OnFullyOpen, then if returndelay ≠ −1, try to close
  after returndelay + 0.1 s; if something is in the way, retry after the
  same delay.
- Inputs Open (if unlocked; fires OnOpen twice: once in the input and once
  when the motion starts, quirk), OpenAwayFrom (name), Close (fires OnClose
  twice similarly), Toggle, Lock, Unlock.
- Blocked: the door stops (no damage) and fires OnBlockedOpening/Closing;
  it resumes when unblocked. With forceclosed, physics objects are pushed
  through and damageable physics objects take their full health as crush
  damage.

### func_button

Keyvalues: "movedir", "speed" (0 → 40), "lip" (0 → 4), "wait" (0 → 1;
−1 = stays pressed), "sounds" (index N → sound entry "Buttons.sndN"),
"locked_sound", "unlocked_sound" (indices, same table), "health" (> 0 makes
it shootable), plus sentences (ignore). Spawnflags: 1 "Don't move", 32
"Toggle", 256 "Touch Activates", 512 "Damage Activates", 1024 "Use
Activates", 2048 "Starts locked", 4096 "Sparks".

Positions as func_door: P2 = P1 + dir·(Σ|dir_i|·(size_i − 2) − lip); if
|P2 − P1| < 1 or "Don't move", P2 = P1 (moves of zero length finish
instantly).

States: out (rest), going in, in, going out.

- **Press** (use with flag 1024; touch by a player with flag 256; damage
  with flag 512 or health > 0):
  - ignored while moving (going in/out);
  - at rest: fire **OnPressed** (activator = presser), then go in (play
    the "sounds" sound; if locked: locked sound and stop, else unlocked
    sound; linear move to P2);
  - in, with "Toggle": play sound, fire OnPressed, go out;
  - in, without Toggle: ignored (a use) / nothing (touch).
  - Use while locked: locked sound; OnUseLocked at most every 0.5 s.
- **Arrived in**: fire **OnIn**. If wait = −1 or Toggle: stay in (touch
  re-enabled if touch-activated). Else after wait seconds (a nearest-tick
  think: round(wait/dt) ticks) go out.
- **Arrived out**: fire **OnOut**; touch re-enabled.
- Damage: **OnDamaged** fires on any damage (activator = the *previous*
  presser, quirk), then acts as a press if damage-activated.
- Inputs: **Press** (as a press, ignored while moving), **PressIn**
  (press only if out/going out), **PressOut** (return only if in/going
  in), **Lock**, **Unlock**.

cs_office's soda buttons: spawnflags 1025 (don't move + use activates),
wait 3.5. A use fires OnPressed and OnIn at once (zero-length move); uses
in the next 233 ticks (round(3.5/0.015)) are ignored; then OnOut.

### func_movelinear

Keyvalues: "movedir", "speed" (≤ 0 → 100), "MoveDistance" (≤ 0 → computed
like a door from the brush size minus "lip"), "StartPosition" (0..1: where
the brush as placed sits along its track), "BlockDamage", "StartSound",
"StopSound". Spawnflag 8 "Not Solid".

- Track: P1 = spawn origin − dir·D·StartPosition; P2 = P1 + dir·D.
- Inputs: **Open** (move to P2 if not there), **Close** (to P1),
  **SetPosition** f (move to P1 + f·(P2 − P1); f is not clamped, so 2.0
  moves past P2), **SetSpeed** (store; if not at the current goal, restart
  the move to the goal at the new speed).
- On arrival: **OnFullyOpen** if exactly at P2, **OnFullyClosed** if exactly
  at P1 (exact equality); the stop sound plays 0.1 s later.
- Blocked: BlockDamage crush damage per blocked tick; it does **not**
  reverse, it keeps trying.

### func_rotating

Keyvalues: "maxspeed" (deg/s, abs, 0 → 100), "fanfriction" (percent; 0 →
100 %), "volume" (0..10), "message" (looping sound), "dmg" (crush damage
per blocked tick), "solidbsp". Spawnflags: 1 "Start ON", 2 "Reverse
Direction", 4 "X Axis" (roll), 8 "Y Axis" (pitch), default yaw; 16
"Acc/Dcc"; 32 "Hurt on touch"; 64 "Not solid"; 128/256/512 small/medium/
large sound radius.

- Speed s (signed) with |s| ≤ maxspeed; angular velocity = axis × s.
- **Without Acc/Dcc**: speed changes jump to the target at once.
- **With Acc/Dcc**: every 0.1 s of local time (exact, via the move-done
  mechanism: steps land on ticks 7, 14, 20, 27, 34, 40, ... i.e.
  ceil(k·6.667)): spin-up adds 0.2 × maxspeed × friction to |s|; spin-down
  subtracts 0.1 × maxspeed × friction; reversing spins down to 0 first,
  then up. friction = fanfriction/100.
- Inputs: **Start** (target = maxspeed in the current direction), **Stop**
  (target 0), **Toggle** (stop if s > 0, else start; note s < 0 counts as
  stopped, quirk), **Reverse** (flip direction, keep magnitude),
  **SetSpeed** f (direction from the sign, target = clamp(|f|, 0, 1) ×
  maxspeed), **StartForward**, **StartBackward**, **StopAtStartPos** (slow
  down and stop exactly at the spawn angle). Use (any source) toggles
  between 0 and maxspeed. "Start ON" starts 0.2 s after spawn.
- Sound: pitch 30 + 70 × |s|/maxspeed, volume = volume/10 × |s|/maxspeed.
- Hurt on touch (32): anything that can take damage touching it takes
  |angular velocity|/10 crush damage per touch and gets velocity set to
  (its origin − rotator centre) normalized × that damage.
- Blocked: dmg crush damage per blocked tick; the rotator holds still that
  tick (does not reverse).
- de_nuke's fans: spawnflags 72 (not solid + Y axis), maxspeed 180,
  fanfriction 20.

### func_tracktrain and path_track (minigame platforms)

path_track keyvalues: "targetname", "target" (next node), "altpath"
(alternate next), "speed" (new train speed at this node, see below),
"radius", "orientationtype". Spawnflags: 1 "Disabled", 4 "Branch Reverse",
8 "Disable train" (removes user control), 16 "Teleport to THIS path_track".
Nodes link at activation: each node's next = first entity named "target"
(must be a path_track), and that node's previous = this one (unless this
one is that node's altpath). Inputs **EnablePath**, **DisablePath**,
**TogglePath**, **EnableAlternatePath**, **DisableAlternatePath**,
**ToggleAlternatePath**, **InPass**, **InTeleport**; outputs **OnPass**,
**OnTeleport** (activator = train). Next in a direction: forward = the
alternate if enabled (and not branch-reverse), else "target"; backward =
the alternate if enabled with branch-reverse, else previous.

func_tracktrain keyvalues: "target" (first node), "speed" (initial speed),
"startspeed" (max speed; 0 → speed, or 100 if both 0), "height" (train
origin above the path), "wheels" (look distance for facing), "bank"
(roll degrees in turns), "dmg", "velocitytype" (0 instantaneous, 1 linear
blend, 2 ease in/out), "orientationtype" (0 fixed, 1 at path tracks, 2
linear blend, 3 ease in/out), sound keys, "volume". Spawnflags: 1 "No
pitch", 2 "No user control", 4 "Forward only", 8 "Passable", 16 "Fixed
orientation", 128 "HL1 train" (brush-solid), 512 "Unblockable by player".

- **Spawn** then next tick: find the first node; teleport the train's
  origin to node origin + (0,0,height), facing the point "wheels" units
  along the path (pitch 0 with "No pitch"). Arrive at that node. If the
  initial speed ≠ 0, start moving 0.1 s later.
- **Every tick while moving**: path point Q = origin − (0,0,height);
  look-ahead point L = the point |speed| × 0.1 units further along the path
  from Q (in the travel direction, through enabled nodes). Velocity =
  normalize((L + height) − origin) × |speed|. So the train aims slightly
  ahead and cuts corners a little.
  - velocitytype 1/2: speed is interpolated between the previous node's
    "speed" and the next node's "speed" (0 = keep) by the fraction along
    the segment (2: smoothstep).
  - orientation: faces along the path (yaw always; pitch unless "No
    pitch"), turning with angular velocity = (target − current)/dt per
    axis (differences < 0.1° ignored); bank rolls toward ±bank when turning
    faster than 5°/s.
  - The node *behind* L changes when L passes a node: the train "arrives"
    there: that node's **OnPass** fires (≈ 0.1 s before the train's
    origin actually reaches it); with flag 8 on the node, the train loses
    user control; if the train has no user control and the node's "speed"
    ≠ 0, the train takes that speed. If the node after it has "Teleport",
    the train jumps there.
  - **OnNextPoint** fires every tick the train moves (in this SDK; Open
    questions).
- **Dead end** (no enabled next node): the train heads for the last node,
  arrives there exactly (speed kept until arrival), stops, and that node's
  OnPass fires.
- Inputs: **StartForward**, **StartBackward** (direction then max speed),
  **Stop**, **Toggle** (0 ↔ max speed), **Resume** (previous speed),
  **Reverse**, **SetSpeed** f (f × max, f clamped 0..1), **SetSpeedDir** f
  (sign = direction), **SetSpeedReal** (units/s, clamped 0..max). Outputs
  **OnStart**, **OnNextPoint**.
- Blocked: if the blocker stands on the train: give it an upward velocity
  of min(|speed|, 50) if its vertical velocity is 0, and nothing else.
  Otherwise: set the blocker's velocity to (blocker − train) normalized ×
  dmg (yes, dmg as a speed), then deal dmg crush damage (unless 512 and the
  blocker is a player). The train holds still that tick.
- Riders are carried as for any pusher; rotations turn riders' view.

## Per-tick order

Within the server frame each entity simulates once, in think-list order:
1. A pusher runs its thinks (sound loops, delayed returns that use thinks
   such as func_button), then takes one push step (move or rotate, with
   riders/blockers, possibly blocked callbacks with damage and reversal),
   then if its move is done: snap, arrival logic (outputs queued).
2. A player runs its commands: use search and Use (outputs queued, door
   movement starts this tick but the door's first step happens when the
   door simulates, which may be later this frame or next frame), then
   movement against the pushers' current positions, then trigger touch.
3. Untouch, event queue (specs/source/entity_io.md).

## Edge cases

- Door already open receiving Open: nothing. Locked door receiving Close:
  closes.
- Rotating door with "wait" 0: closes as soon as it is fully open (after
  one partial step of zero length; it starts closing in the arrival tick).
- Toggle-flagged door (32) touched while open: closes.
- A door blocked by a player when closing with dmg 0 and wait ≥ 0: reverses
  without damage (classic "door bounces off you").
- Button with zero travel ("Don't move") fires OnPressed and OnIn in the
  same tick.
- Trains with "Passable" are non-solid: no riding.
- A pusher moving into a player standing in a corner: the player is moved
  as far as possible; if still overlapping, blocked.
- Use through thin glass or grates: trace 0 uses a mask that includes
  windows/grates as blockers? The radius search does check a line to the
  candidate (Open questions).
- Use while dead or spectating: observer use (out of scope).

## Quirks

- **Use distance ignores height within the player's body span** (keep).
- **Use is per press; holding does nothing for doors/buttons** (keep).
- **Door OnOpen/OnClose/OnFullyOpen use the door as activator** (keep; map
  logic sometimes relies on !activator being the door).
- **Inputs don't update the rotating door's user** (keep).
- **prop_door_rotating fires OnOpen/OnClose twice on inputs** (keep).
- **func_movelinear SetPosition is unclamped** (keep).
- **Train OnPass fires 0.1 s early** (keep).
- **func_rotating Toggle treats reverse spin as stopped** (keep).
- **func_button OnDamaged activator is the previous presser** (keep).

## Test cases

dt = 0.015. Brush sizes are the model bounding box.

| Setup | Input | After | Expected |
|---|---|---|---|
| func_door 64×8×128, movedir "0 0 0", lip 4, speed 100, wait 4, flags 256 | player uses at tick 0 | — | OnOpen at tick 0; D = 58; position 1.5 per tick, 57 after tick 38, 58 (P2) at tick 39; OnFullyOpen at tick 39; closing starts 267 ticks after arrival; OnClose then; OnFullyClosed 39 ticks later |
| cs_office slidingdoor: movedir "−90 0 0", lip 16, wait −1, speed 100, brush height 128 | trigger_once Open | — | moves up 110 units in 1.1 s (74 ticks, last partial); never returns |
| same door open | Open | — | nothing |
| func_door locked (2048) | Open | — | nothing; Unlock then Open opens |
| func_door locked, flags 256 | player use | — | OnLockedUse, "DoorSound.DefaultLocked" |
| func_door flags 0 (no use) | player use | — | locked sound only |
| func_door wait 4, dmg 10, closing onto a player | blocked | — | player takes 10 crush damage on the first blocked tick; door reverses (opens); OnBlockedClosing |
| func_door wait −1, dmg 10, closing onto a player standing still | 10 ticks blocked | — | 100 damage total; door does not move or reverse |
| func_door forceclosed 1, dmg 0, wait 4 | blocked by player | — | no damage, no reversal, door waits until free |
| two func_door named "gate" moving together | one blocked | — | both reverse together |
| func_door_rotating at origin (0,0,0), brush x 0..64, y −2..2, distance 90, yaw | user at (32, −40) uses | — | opens to yaw +90 (swings toward +Y, away) |
| same | user at (32, +40) uses | — | opens to yaw −90 |
| same, spawnflag 16 (one-way) | user at (32, +40) | — | opens to +90 |
| de_nuke door pair (speed 200, distance 90) | use one | — | both halves open, 90° in 0.45 s (30 ticks); close after wait 4 |
| func_door_rotating opened by Open input after a user at (32,40) used it once | Open | — | swings to −90 (old user) |
| prop_door_rotating cs_assault (returndelay 20, distance 90, speed 100, flags 8192) | player in front uses | — | opens away (back) over 0.9 s (60 ticks), closes after 20.1 s; second use while open closes it |
| prop_door_rotating | Open input | — | OnOpen delivered twice |
| func_button flags 1024, speed 40, lip 4, depth 8 (movedir into the wall), wait 1 | use at tick 0 | — | OnPressed tick 0; travels 2 units (8 − 2 − 4) in 0.05 s (4 ticks); OnIn tick 4; returns after round(1/0.015) = 67 ticks, OnOut 4 ticks later |
| cs_office soda button (flags 1025, wait 3.5) | use at tick 0, again at tick 100 and tick 240 | — | OnPressed and OnIn at 0; tick 100 ignored; OnOut at tick 233; tick 240 press accepted |
| func_button toggle (32 + 1024) | use, use | — | 1st: OnPressed, OnIn; 2nd: OnPressed, OnOut |
| func_button locked, use 3 times in 0.3 s | — | — | one OnUseLocked |
| func_button health 10 (no flag 512) | shot for 5 | — | OnDamaged, then pressed |
| func_movelinear D 100, StartPosition 0 | SetPosition 0.5 at speed 100 | — | moves 50 units in 0.5 s; no Fully* output |
| same | SetPosition 2 | — | moves to 200 units past P1 |
| same, at P2 | Close | — | OnFullyClosed on arrival |
| func_rotating maxspeed 180, fanfriction 20, flags 16 | Start | — | |s| rises 7.2 deg/s every 0.1 s; full speed after 25 steps (2.5 s) |
| same, at 180 | Stop | — | falls 3.6 per step, 0 after 50 steps (5 s) |
| func_rotating maxspeed 100, no Acc/Dcc | SetSpeed −0.5 | — | angular speed −50 at once |
| func_rotating spinning −100 | Toggle | — | starts to +100 (negative counts as stopped) |
| func_rotating flags 32, spinning 200 deg/s | player touches | — | 20 crush damage, velocity 20 away from centre |
| func_tracktrain on straight path A→B (1000 units along +X), startspeed 200 | StartForward | — | velocity (200,0,0) each tick; B's OnPass when the train origin is 20 units short of B (0.1 s); dead end at B: stops exactly at B + height |
| train with "No user control", node B speed 50 | passes B | — | speed 50 after B's OnPass |
| platform door moving up 1.5/tick, player standing on it | 10 ticks | — | player rises 15 units, own velocity unchanged |
| same with a ceiling 10 units above the player's head | — | — | blocked on the tick the player would hit the ceiling; door crush damage / reversal |
| player standing on a train moving +X 200, jumps | — | — | after leaving ground, base velocity += (200,0,0); next tick velocity.x += 201.5 |
| rotating platform (func_rotating yaw 90 deg/s), player standing on it | 1 tick | — | player carried along the arc, view yaw +1.35° |
| player eye at (0,0,64), looking +X, button face at x = 79 | press use | — | button used (trace 0, distance 79) |
| same, button face at x = 81 | press use | — | nothing used; deny sound (if any) |
| player looking +X, button on the floor 30 ahead at z = 0..2 | press use | — | used by an angled trace (distance 30) |
| player holds use for 2 s facing a door with "Use Opens" | — | — | door activated once |

## Open questions

1. **CS:S use search**: does CS:S override the use search (it has use
   interactions for hostages, C4 defuse and weapons)? Measure: on the probe
   server, place a func_button with flag 1024 and test distances 79/81
   straight ahead and at foot level 30 ahead; note which register.
2. **Use deny sound in CS:S**: which, if any, sound plays when pressing use
   on nothing? Listen on the reference client.
3. **Use trace mask**: does +use go through grates/windows in CS:S (the
   CS:S mask is "brush-only NPC solid plus opaque-and-NPC")?
4. **Ride timing**: is a rider moved before or after their own command in
   the same frame (depends on the think-list order of the platform vs the
   player)? Visible as one tick of jitter. Measure the player's z per tick
   on an elevator.
5. **func_tracktrain OnNextPoint**: does it fire every tick in CS:S's build
   (as in the current SDK) or only per node? Connect it to a math_counter
   Add 1 and read the count after 1 s.
6. **Door move sound timing and channel**: confirm "DoorSound.DefaultMove"
   starts on the activation tick and stops on arrival (listen).
7. **Rotating door "away" rule edge**: user exactly on the door plane or
   at the hinge (cross product 0 → opens to A2). Confirm with a probe
   placement.
8. **prop_door_rotating sounds**: which entries the hardware type and
   model surface pick (not traced in this pass); and whether the CS:S
   model doors behave the same (only cs_assault/de_port/cs_compound use
   them).
