# Source engine: point_viewcontrol cameras, point_template and env_entity_maker

Source basis: Valve's public Source SDK 2013 only (current GitHub release).
I read the server code for point_viewcontrol (the trigger camera), the
player's view-entity, control-freeze and button handling and how a frozen
player's commands are treated, the multiplayer "local player" helper,
point_template, the template store (capturing an entity's map text at load,
the name fix-up pass and per-instance suffixes), the map-load entity parser
(how template members are pulled out of the spawn list), the hierarchical
spawn of a new group of entities, and env_entity_maker. The client side of
the camera is plain entity rendering. CS:S's own player and HUD code is not
public; where CS:S could differ (HUD, view model, respawn reset) it is an
open question. Episodic-only camera options compiled out of non-episodic
games (CS:S) are left out. VScript hooks on templates (the current SDK
calls optional script functions before and after spawning) do not exist in
CS:S and are left out.
Status: draft

## Summary

point_viewcontrol takes over one player's view: the player sees from the
camera entity, can optionally be frozen, cannot be damaged while viewing,
and the camera can turn smoothly toward a target and travel along a
path_corner chain. It is strictly per player: the player is the input's
activator, and with no player activator in multiplayer nothing happens.
point_template remembers the map text of the entities it names (taking
them out of the map unless told not to) and, on ForceSpawn, re-creates
them from that text, keeping their relative placement around the template.
Output connections are just keyvalues in that text, so they come back
with the copies. When the members refer to each other by name, every
spawn gives those names a unique "&NNNN" suffix so each copy talks to its
own partners. env_entity_maker spawns a point_template's group at the
maker's own position and angles, optionally only when there is room or
nobody is looking, optionally repeatedly, optionally throwing the new
entities.

## Units and conventions

Source units, Z up; angles (pitch, yaw, roll) in degrees, pitch positive
down; angle math wraps to [−180, 180]. dt = 0.015 s server tick; "think
at now" = next tick. Names compare case-insensitively. Activator/caller
as in specs/source/entity_io.md. Spawnflag values are what Hammer shows.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| cam_turn_gain | 40 | 1/s per s | angular velocity = error × 40 × dt (deg/s) |
| cam_vel_blend | 2 | 1/s | per-tick path velocity blend fraction = 2·dt (0.03) |
| cam_free_damp | 0.8 | per tick | camera velocity multiplier per tick when the player is not frozen |
| cam_free_stop | 10 | units/s | below this (after damping) the velocity is zeroed |
| cam_default_accel | 500 | units/s² | acceleration and deceleration 0 become this |
| fixup_suffix | "&" + 4 digits | — | per-instance name suffix, e.g. "&0012" |
| fixup_wrap | 10000 | — | instance counter wraps to 0 here |
| template_slots | 16 | — | Template01 .. Template16 |
| maker_autospawn_period | 0.5 | s | env_entity_maker autospawn re-check (→ 33 ticks) |

## Behavior

### 1. point_viewcontrol

Keyvalues: "target" (entity to look at), "targetattachment" (attachment
name on the target's model), "moveto" (first path_corner), "speed"
(initial speed, units/s), "acceleration", "deceleration" (0 → 500),
"wait" (s; Hammer default 10). Spawnflags: 1 "Start At Player", 2 "Follow
Player", 4 "Freeze Player", 8 "Infinite Hold Time", 16 "Snap to goal
angles", 32 "Make Player non-solid", 64 "Interruptable by Player". Inputs
**Enable**, **Disable**. Output **OnEndFollow** (activator = caller = the
camera). The camera is invisible, non-solid, and moves by its own velocity
and angular velocity (no collision: it flies through walls).

**1.1 Enable** (input; the activator is the player):
1. State := on. If the activator is not a player, the game uses the
   single-player "local player", which in multiplayer is **none**: the
   camera is marked on but **nothing else happens** (no view change, no
   think). So an Enable fired from logic_auto, a timer or any chain whose
   activator is not a player does nothing in CS:S.
2. If that player's current view entity is another point_viewcontrol,
   that camera is disabled first (1.3). If it is this camera already: do
   nothing more (the "player" field has however been overwritten with this
   activator).
3. Remember the player's held buttons; remember its damage mode and make
   it **invulnerable** (takes no damage) while viewing. Flag 32: player
   becomes non-solid.
4. Return time = now + wait; current speed = target speed = "speed".
5. Flag 16: snap angles on the first follow step. Look target: the player
   itself with flag 2, else the "target" entity (first match). With
   "targetattachment" and a target with a model, aim at that attachment.
6. Flag 4: **freeze** the player: every command from it has movement,
   buttons and impulse zeroed and its view angles held (the player cannot
   move, shoot, jump or look; the server ignores its mouse).
7. Path: first entity named "moveto" (activator-relative names allowed);
   if it exists and has a nonzero "speed", target speed := its speed; stop
   time = now + its "wait".
8. Flag 1: camera moves to the player's eye position, angles (player
   pitch, player yaw, 0), velocity = player's velocity. Else velocity 0.
9. The player's view entity := this camera (the client renders from the
   camera's origin and angles); the player's active weapon world model is
   hidden.
10. If there is a look target: think every tick (1.2). Move once now
    (1.4). Become networked to everyone.

**1.2 Each tick while on with a look target:**
1. No player: stop. Target gone: Disable.
2. Unless flag 8, if now > return time: Disable (so the camera holds for
   "wait" seconds, rounded to whole ticks).
3. Goal angles = the angles of the vector camera → target (or → the
   attachment). With the snap flag (first step only): set angles to the
   goal at once. Otherwise: wrap yaw to [0, 360); errors dx = goal pitch −
   pitch, dy = goal yaw − yaw, each wrapped to [−180, 180]; angular
   velocity = (dx·40·dt, dy·40·dt, unchanged roll). Integrated over one
   tick, the camera turns 40·dt² = 0.009 of the remaining error per tick
   (exponential approach, time constant 1/(40·dt) ≈ 1.67 s; it never quite
   arrives).
4. If the player is **not** frozen (flag 4 clear): velocity ← 0.8 ×
   velocity; if its length < 10: velocity := 0.
5. Move (1.4).

**1.3 Disable:**
- If the player exists and is alive: restore solidity (flag 32), view
  entity := the player, **unfreeze** (the frozen state is cleared even if
  this camera did not set it), show the weapon, restore its damage mode.
  A dead player is not restored.
- State := off, return time = now, stop thinking, angular velocity 0,
  fire **OnEndFollow** (also when the camera was already off: Disable on
  an off camera still fires it). Stop being networked.

**1.4 Move** (once at Enable, then every tick from 1.2):
1. Flag 64: if the player's buttons changed since the last check **and**
   at least one button is now held: Disable and stop. Remember the
   buttons. (A frozen player's buttons are all zero, so flag 64 never
   triggers together with flag 4.)
2. No path: stop here.
3. Remaining distance −= current speed × dt. When it is ≤ 0 (always on the
   first Move, since it starts at 0): send **InPass** to the current
   corner (path_corner fires its OnPass), advance to the corner's next
   target ("target" keyvalue). None: velocity := 0 (and the path ends).
   Else: if that corner has a nonzero "speed", target speed := it;
   direction = normalize(corner − camera), remaining distance = that
   length; stop time = now + the corner's "wait".
4. Speed: if now < stop time: approach 0 by deceleration × dt; else
   approach target speed by acceleration × dt (approach = move toward
   without overshoot).
5. Velocity ← direction × speed × (2·dt) + velocity × (1 − 2·dt).

Consequences (kept as quirks, see Quirks): the camera never travels to the
"moveto" corner itself (the first Move passes it immediately and heads for
the corner after it); without flag 4 the per-tick damping in 1.2 step 4
keeps the camera far below the path speed; without a look target the
camera does not think, so after the single Move it keeps whatever velocity
it got forever and the hold time never expires.

**1.5 What the player sees** (client): the scene from the camera origin and
angles (interpolated like any moving entity), with the normal HUD (the
base code does not hide it; CS:S, Open questions). The view model is not
drawn when the view entity is not the player (*inferred*, Open questions).
Other players still see the viewer's body standing (invulnerable) where it
was.

### 2. point_template

Keyvalues: "Template01" .. "Template16" (entity names; each may match
several entities). Spawnflags: 1 "Don't remove template entities", 2
"Preserve entity names (Don't do name fixup)". Input **ForceSpawn**.
Output **OnEntitySpawned** (activator = caller = the template).

**2.1 At map load** (before any other entity spawns; the templates are
handled after the whole entity lump is parsed):
1. Each point_template spawns first, then collects every map entity whose
   targetname matches any TemplateNN (in slot order, each slot in map
   order). A name with no matches is just skipped (warning).
2. For each collected entity: store its **original map text** (all its
   keyvalues, including classname, origin, angles, model, spawnflags and
   every output connection) and its placement **relative to the
   template**: the 4×4 transform M_rel = inverse(T_template) × T_entity,
   built from origins and angles at load time. An entity with no
   targetname cannot be a template member.
3. Unless flag 1: the original entity is **removed** before it spawns (it
   never exists in the map, never runs Spawn/Activate, and name lookups
   cannot find it). With flag 1 it also spawns normally.
4. Name fix-up preparation (unless flag 2), over the group of this
   template's members:
   - For every keyvalue of every member except "targetname", take the
     value up to its first field separator (outputs are
     "target,input,parameter,delay,times", the separator being an ASCII
     comma, or the escape character 0x1B in newer compiled maps; a plain
     keyvalue is used whole). If it equals (case-insensitive) the
     targetname of **any** member of the group, the value gets the marker
     "&0000" inserted right after that name (before the first separator),
     and that member is flagged "rename".
   - Every flagged member's own targetname gets "&0000" appended.
   - This covers outputs and plain keys alike: "parentname", "target",
     "filtername", "LaserTarget", "LightningStart", even "message" if its
     text happens to equal a member's name.
   - Only exact whole-name matches count: wildcard targets ("name*"),
     "!activator"-style names and names of entities outside the group are
     left alone.

**2.2 ForceSpawn:**
1. Advance the global instance counter (shared by all templates for the
   whole server session; it also advances once per template at level
   precache, and wraps to 0 at 10000). Suffix = "&" + the counter in 4
   digits.
2. For each stored member, in stored order: take its text; if fix-up
   applies to it (2.1 step 4 found something and flag 2 is clear), every
   "&" followed by four digits is replaced by the current suffix. Parse
   the text into a **new entity** (keyvalues applied, outputs connected),
   then place it at T_now × M_rel, where T_now is the template's current
   origin and angles (origin from the matrix, angles extracted from its
   rotation).
3. Spawn all new entities together, parents before children (so
   "parentname" pointing at a fellow member attaches to that member's new
   copy), then activate them all. They are now ordinary entities.
4. Fire **OnEntitySpawned**.

Name results:
- Members referenced inside the group: every copy has a unique name
  ("door&0003", "door&0004", ...), and the group's internal outputs point
  at their own copy. Outside entities targeting "door" find **no** copy
  (use flag 2, or a wildcard "door*" target).
- Members never referenced inside the group: all copies share the plain
  name, so an input to that name reaches every copy.

### 3. env_entity_maker

Keyvalues: "EntityTemplate" (name of a point_template), "PostSpawnSpeed",
"PostSpawnDirection" (angles), "PostSpawnDirectionVariance",
"PostSpawnInheritAngles" (0/1). Spawnflags: 1 "Enable AutoSpawn", 2
"AutoSpawn: Wait for entity destruction", 4 "AutoSpawn: Even if the
player is looking", 8 "ForceSpawn: Only if there's room", 16 "ForceSpawn:
Only if the player isn't looking". Inputs **ForceSpawn**,
**ForceSpawnAtEntityOrigin** (string: entity name; newer builds, Open
questions). Outputs **OnEntitySpawned**, **OnEntityFailedSpawn**
(activator = caller = the maker).

- No EntityTemplate at activation: the maker deletes itself.
- **Spawn** (at position p, angles a; default the maker's own):
  1. Find the template (first entity of that name that is a
     point_template; none: nothing).
  2. Create an instance as in 2.2 steps 1–3 but with T_now built from p
     and a: members keep their offsets from the template, now relative to
     the maker.
  3. If nothing was created: stop (no output).
  4. Remember the **first** created entity as the "current instance" and
     the "blocker", and its position. The first time ever, also remember
     its world bounding box relative to its origin (the "spawn box").
  5. Fire the maker's **OnEntitySpawned** (the template's own
     OnEntitySpawned does **not** fire).
  6. Flag 1: (re)start the autospawn check 0.5 s later.
  7. If PostSpawnSpeed ≠ 0, for every created entity that can move
     (movetype not none): direction angles = PostSpawnDirection (+ the
     maker's world angles, or its parent's, with PostSpawnInheritAngles);
     f, r, u = forward/right/up of that; dir = normalize(f + r·U(−1,1)·v +
     f·U(−1,1)·v + u·U(−1,1)·v) with v = PostSpawnDirectionVariance (new
     randoms per entity); velocity = dir × PostSpawnSpeed, **added** to a
     physics object's velocity, or **set** as the entity's velocity
     otherwise.
- **Room test**: no room if the blocker still exists and has not moved at
  all (exactly the same position); else trace the spawn box at the maker's
  origin (zero-length box trace against everything solid except the
  maker): any overlap → no room, and that entity becomes the blocker.
- **Looking test**: true if **any** player (alive, dead or spectating,
  regardless of walls) has dot(eye direction, normalize(maker − eye)) > 0,
  i.e. the maker is anywhere in front of their eye plane.
- **ForceSpawn**: template missing: nothing. Flag 8 and no room: fire
  OnEntityFailedSpawn. Flag 16 and looking: fire OnEntityFailedSpawn.
  Else spawn at the maker.
- **ForceSpawnAtEntityOrigin** n: first entity named n (activator-relative
  names allowed); spawn at its origin and angles (no room/looking tests).
- **Autospawn** (flag 1): one spawn at activation (map load), then every
  0.5 s: flag 2 and the current instance still exists → wait; no room →
  wait; flag 4 clear and someone looking → wait; else spawn.

## Per-tick order

1. Entity thinks (entity order): cameras with targets turn and move;
   env_entity_maker autospawn checks.
2. Movement integration of cameras (origin += velocity·dt, angles +=
   angular velocity·dt) happens when the camera is simulated, after its
   think in the same tick.
3. Event queue (entity_io.md): Enable/Disable/ForceSpawn take effect; a
   ForceSpawn creates, spawns and activates its entities immediately, and
   their outputs with zero delay (and the spawner's OnEntitySpawned) run
   later in the same queue pass.
4. Player commands (any time in the frame): a frozen player's commands are
   neutralised (1.1 step 6); flag 64 cameras read the buttons on their
   next Move.

## Edge cases

- Two different players enabling the same camera: the second overwrites
  the camera's player; Disable then restores only the second; the first
  stays viewing (and invulnerable) until another camera or respawn resets
  their view (Open questions). Map logic that uses one camera for all
  players needs per-player handling; we should reproduce the per-player
  rule, not invent a broadcast.
- Enable on a camera already viewed by the same player: ignored (warning),
  no re-hold.
- Camera target removed while viewing: Disable on the next tick.
- Player dies while viewing: Disable does not restore anything for a dead
  player; the view stays on the camera until something else resets it.
- point_template with no members found: ForceSpawn does nothing and fires
  nothing.
- A template member whose class removes itself at spawn (lights, nodes)
  cannot be templated (they are spawned during parsing).
- Brush-entity members (func_*): the copy uses the same brush model
  ("model" "*N"), placed by the transform; several copies overlap if the
  template does not move.
- Fix-up only inserts the suffix after the first field of a value; an
  output whose *parameter* names a member (e.g. AddOutput or SetParent
  with "name" as parameter) is **not** fixed up.
- Template spawned while a copy with the same unsuffixed name exists: both
  answer to that name.

## Quirks

- **Enable needs a player activator in multiplayer** (keep: maps that
  fire it from logic_auto simply do nothing in CS:S).
- **The camera skips its "moveto" corner** (heads for the corner after it;
  keep, measure, Open questions).
- **Unfrozen cameras crawl**: with flag 4 clear the 0.8 damping and the
  10 units/s cutoff combine with the 0.03 blend: for a path speed P below
  about 417 units/s the camera settles at 0.03·P per second (e.g. P = 100:
  3 units/s); above that it settles near 0.134·P. Frozen cameras (flag 4)
  settle at P. Keep, verify (Open questions).
- **Disable always unfreezes**, even if a game_ui or another system froze
  the player.
- **Invulnerable while viewing** (keep: minigame intro cameras rely on it).
- **The global suffix counter** makes exact "&NNNN" values unpredictable
  (depends on how many templates precached and previous spawns). Tests
  must only check uniqueness and consistency inside one instance.

## Test cases

| Setup | Input | After | Expected |
|---|---|---|---|
| point_viewcontrol, Enable fired by logic_auto (no activator) | — | — | no player's view changes; OnEndFollow not fired |
| camera, target info_target, wait 3, flags 0; Enable from a trigger by player P | tick 0 | — | P views from the camera; P takes no damage |
| same | — | tick 201 | Disable at the first tick with now strictly after the return time (3.0 s = tick 200 is not after it; float rounding may move it by one tick): P's view restored, OnEndFollow fired |
| same with flag 8 | — | 10 s | still viewing |
| camera without target, wait 3 | Enable | 10 s | still viewing (no think, no timeout) |
| camera yaw 0 aiming at a target 90° to the left (goal yaw 90), no snap | 1 tick | — | yaw ≈ 0.81 (90 × 0.009) |
| same | 167 ticks (2.5 s) | — | remaining error ≈ 90·(0.991)^167 ≈ 20° |
| same with flag 16 | first tick | — | yaw = 90 exactly |
| flag 4 | P holds +forward and fires | while viewing | P does not move or shoot; view angles fixed |
| flag 64, flag 4 clear | P presses jump | next tick | Disable |
| flag 64 + flag 4 | P presses jump | — | nothing (buttons zeroed) |
| flag 32 | Enable then Disable | — | P non-solid while viewing, solid after |
| two cameras A, B | Enable A, then Enable B (same P) | — | A disabled first (A's OnEndFollow), P views B |
| camera off | Disable | — | OnEndFollow fires anyway |
| camera with moveto c1 → c2 → c3, speed 100, flag 4, target set | Enable | — | c1 receives InPass at once; camera heads toward c2 |
| camera path speed 100, flag 4 | long straight leg | — | velocity → 100 (blend 3 %/tick: 63 % after 33 ticks) |
| same, flag 4 clear | — | — | velocity settles at 3 units/s |
| point_template T at (0,0,0) yaw 0, member "box" (prop) at (64,0,0) | ForceSpawn | — | a new "box" at (64,0,0); OnEntitySpawned fired |
| T moved (SetParent/teleport) to (100,0,0) yaw 90 | ForceSpawn | — | new "box" at (100,64,0), yaw +90 |
| T, flag 0 | map load | — | original "box" absent from the map |
| T, flag 1 | map load | — | original "box" present; ForceSpawn adds a copy |
| members "btn" (func_button, OnPressed "door,Open,,0,-1") and "door" | ForceSpawn twice | — | copies "btn&A"/"door&A" and "btn&B"/"door&B" (A ≠ B); pressing btn&A opens only door&A |
| same with flag 2 | ForceSpawn twice | — | two "btn" and two "door"; pressing either button opens both doors |
| member "lamp" with parentname "cart", "cart" also a member | ForceSpawn | — | lamp copy parented to the cart copy (both suffixed) |
| outside logic_relay targets "door" (fix-up active) | fire | — | no door responds |
| outside relay targets "door*" | fire | — | all door copies respond |
| env_entity_maker M at (0,0,100) yaw 180, template T at (0,0,0) yaw 0 with member at (64,0,0) | ForceSpawn on M | — | member copy at (−64,0,100), yaw 180; M's OnEntitySpawned fires, T's does not |
| M flag 8, copy still sitting at M | ForceSpawn | — | OnEntityFailedSpawn (blocker has not moved) |
| M flag 16, a player facing M through a wall | ForceSpawn | — | OnEntityFailedSpawn |
| M PostSpawnSpeed 500, direction "0 0 0", variance 0, member prop_physics | ForceSpawn | — | prop velocity += (500, 0, 0) |
| M flags 1+2+4 | map load | — | one copy at once; a new one 0.5 s after that copy is destroyed |

## Open questions

1. **CS:S HUD and view model under a camera**: does CS:S hide the HUD or
   the view model while a player views a point_viewcontrol? Probe: a test
   map with a camera enabled by a trigger; screenshot.
2. **View reset on respawn**: does a CS:S player's view entity (and the
   freeze) reset on respawn or round restart if a camera was never
   disabled (two-player overwrite case, death while viewing)? Probe on a
   listen server with two clients.
3. **Camera path behaviour**: confirm the first corner is skipped and the
   unfrozen-camera crawl (speed 100: ~3 units/s) on a test map with a
   two-corner path; record origin per tick with a server-side probe.
4. **ForceSpawnAtEntityOrigin** exists only in newer builds: does the
   CS:S build accept it? Probe with a test map and the console
   (ent_fire maker ForceSpawnAtEntityOrigin target).
5. **Separator in compiled CS:S maps**: are output fields in the community
   maps separated by commas or by 0x1B? Read the entity lumps of the maps
   in the sweep (mashup's BSP reader already sees the raw text).
6. **Instance counter start**: what suffix does the first ForceSpawn on a
   fresh CS:S server produce? Only matters for maps that hard-code
   "name&0001" (rare but possible); check with ent_text / ent_dump after
   one ForceSpawn.
7. **Activation of spawned logic_auto**: a logic_auto inside a template
   fires OnMapSpawn when activated after ForceSpawn? (Depends on logic_auto
   rules in entity_io.md; test with a templated logic_auto.)
