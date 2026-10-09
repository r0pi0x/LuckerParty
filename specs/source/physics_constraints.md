# Source engine: physics constraints (phys_constraint, phys_ballsocket, phys_hinge, phys_slideconstraint, phys_lengthconstraint, phys_pulleyconstraint, phys_ragdollconstraint, phys_spring, phys_constraintsystem, info_constraint_anchor)

Source basis: Valve's public Source SDK 2013 only (current GitHub
release). I read the server code shared by all two-object constraint
entities (keyvalues, spawnflags, how the two bodies are found, world
fallback, anchors, break parameters, inputs, outputs, break handling,
teleport following), each constraint class's setup (hinge, ball-and-socket,
fixed, sliding, length, pulley, ragdoll), the constraint-system entity,
info_constraint_anchor, phys_spring, the map loader's spawn priorities,
the physics helpers they use (name lookup of a physics body, mass-centre
override along an axis, the "both pinned to the world don't collide"
rule, direction snapping) and the public physics-engine headers that
define the constraint parameter blocks and their documented units. The
physics engine (vphysics: how each constraint is solved, how breaking is
measured, motors, friction, spring force law) is a binary library, not in
the SDK; rules that depend on it are marked *inferred*. The SDK compiles
some constraint features only for HL2 episodic (motion sounds on hinges
and sliders, a stricter missing-entity rule); CS:S is not episodic, so
the non-episodic behaviour is given. CS:S's own DLLs are not public; it
compiles this shared server code, but could differ (Open questions).
Body parameters, damping, sleeping and player interaction are in
specs/cs_source/physics_props.md and specs/source/physics_brushes.md and
are not repeated.
Status: draft

## Summary

A constraint entity joins two physics bodies (or one body and the world)
at map start: a weld (phys_constraint), a ball joint (phys_ballsocket), a
hinge with optional friction and motor (phys_hinge), a rail with optional
end stops, friction and motor (phys_slideconstraint), a rope of maximum
(and optional minimum) length (phys_lengthconstraint), a rope over two
pulleys (phys_pulleyconstraint), a ball joint with per-axis angle limits
(phys_ragdollconstraint), or a spring (phys_spring). The bodies are found
by name when the map has finished spawning; a missing name means "the
world". Constraints can start off, be turned on and off, and break when
the force or torque they must apply exceeds a limit (given in Hammer in
pounds), firing OnBreak and deleting themselves. mg_ map contraptions
(swinging doors, pendulums, wrecking balls, chained platforms, carts on
rails) are built from these.

## Units and conventions

Source units (inches), Z up; mass kg; force kg·in/s²; torque
kg·in²/s²; angles degrees; angular velocity degrees/s (vphysics
convention). Hammer's break limits are in **pounds** and are converted
with 1 kg = 2.2 lb (factor 1/2.2 exactly). Server tick Δt = 0.015 s;
physics steps once per tick. "Point" keyvalues ("hingeaxis",
"slideaxis", "attachpoint", "position2", "springaxis") are world
positions written by Hammer's helper; directions are derived from them
and the entity's origin. Spawnflag values are bits; Hammer labels are
quoted as Hammer shows them (the FGD is not in the SDK).

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| kg_per_lb | 1/2.2 = 0.454545 | kg/lb | forcelimit and torquelimit conversion |
| hinge_friction_scale | 1000 | — | "hingefriction" × 1000 = the hinge axis friction torque |
| axis_snap_eps | 0.002 | — | a unit direction with a component of magnitude > 0.998 is snapped to that exact axis |
| constraint_spawn_priority | 8 | — | constraint classes spawn before props (7) and other entities (−1) |
| group_default_iterations | 0 | — | extra solver iterations of a constraint system |
| group_error_ticks | 15 | ticks | a constraint group reports an error after this many ticks out of tolerance (*inferred* use) |
| group_error_tolerance | 3 | units | constraint-group error tolerance |
| spring_default_constant | 150 | — | phys_spring "constant" if absent |
| spring_default_damping | 2.0 | — | phys_spring "damping" if absent |
| spring_default_relative_damping | 0.01 | — | phys_spring "relativedamping" if absent |
| pulley_default_gear | 1 | — | gear ratio when "gearratio" is 0 |

## Behavior

### 1. Common keys, flags, inputs and outputs (all two-object constraints)

Classes: phys_constraint, phys_ballsocket, phys_hinge,
phys_slideconstraint, phys_lengthconstraint, phys_pulleyconstraint,
phys_ragdollconstraint. (phys_spring is separate, section 9.)

Keyvalues: "attach1", "attach2" (entity names; empty = world),
"constraintsystem" (name of a phys_constraintsystem), "forcelimit" (lb,
0 = never breaks by force), "torquelimit" (lb·in, 0 = never breaks by
torque), "breaksound" (sound name), "teleportfollowdistance" (units,
default 0), plus base keys ("targetname", "origin", "angles").

Spawnflags:
- 1 "No Collision until break": the two bodies do not collide with each
  other while the constraint exists (re-enabled when it is turned off,
  breaks or is deleted).
- 2 class-specific: phys_slideconstraint "Limit Endpoints", phys_length-
  constraint "Keep Rigid", phys_pulleyconstraint "Is Rigid",
  phys_ragdollconstraint "Only limit rotation" (free translation).
- 4 "Start inactive": created switched off.
- 8 "Change mass to keep stable attachment to world": phys_hinge only
  (section 4.4); ignored by the others.
- 16 "Do not connect entities until turned on": bodies are looked up and
  the constraint created only by the first TurnOn.

Inputs: **Break**, **ConstraintBroken** (same effect as Break),
**TurnOn**, **TurnOff**; per class: phys_hinge **SetAngularVelocity**
(float), **SetHingeFriction** (float); phys_slideconstraint
**SetVelocity** (float). (There is no "SetVelocity" on phys_hinge in this
code: the hinge's motor input is SetAngularVelocity.) Output: **OnBreak**
(activator and caller = the constraint).

**1.1 Spawn order.** Map entities spawn by priority (higher first):
func_wall 10, scripted_sequence 9, the seven constraint classes above
plus info_mass_center and trigger_vphysics_motion 8 (phys_spring and
phys_constraintsystem are not in the list: −1), prop_physics and
prop_ragdoll 7, everything else −1 (ties by parent-hierarchy depth, then
map order, *inferred* for the tie rule). Constraints only record keys at
spawn (phys_hinge also registers a mass-centre override, 4.4); bodies are
looked up at **activation**, which runs for every map entity after all
have spawned, in map order (entity_io.md).

**1.2 Finding a body for a name** (each of attach1, attach2 separately):
1. If an info_constraint_anchor of that exact name (case-sensitive, no
   wildcards) was recorded (section 10; the last one recorded wins) and
   its parent still exists: body = the parent's physics body, anchor
   point = the anchor's offset in the parent's frame, mass scale = the
   anchor's "massScale". If the anchor was attached to a model attachment
   of an animated parent with several bodies (a ragdoll), the body is the
   one of the bone that attachment follows and the anchor point is the
   attachment's position in that body's frame.
2. Else: search entities by name (wildcard `*` allowed; `!`-names do not
   resolve here) and take the **first** one that has a physics body;
   anchor point = that body's origin (0 offset); mass scale 1. More than
   one named entity with a body: the first is used (developer warning).
   An entity without a physics body (point entities, a brush entity
   without collision) counts as not found. A multi-body entity (ragdoll)
   gives its main body (*inferred*: the first part).

Physics bodies that count: props, func_physbox, map ragdolls (moving
bodies); func_brush, func_wall, static props (static bodies); doors,
func_movelinear, func_rotating, func_tracktrain and other movers
(kinematic "shadow" bodies that follow their scripted motion and push
what is joined to them).

**1.3 Pairing and the world** (body A from attach1, B from attach2):
- A and B found: reference = A, attached = B.
- Only B found: reference = **world**, attached = B. If attach1 was a
  non-empty name that did not resolve, a warning is logged but the world
  is still used (episodic HL2 refuses instead; CS:S is not episodic).
  Both mass scales := 1.
- Only A found: reference = world, attached = A ("swapped"). Same warning
  rule; mass scales := 1.
- Neither: the constraint cannot be made: the entity **deletes itself**
  at activation.
- Both bodies static (world + a static body, or two static bodies), or
  both kinematic movers: warning, not created, entity deleted.
- If either body belongs to a map ragdoll, the constraint joins that
  ragdoll's constraint group, unless "constraintsystem" names an existing
  phys_constraintsystem, which takes precedence.

A misspelled attach name therefore silently pins the other body to the
world (Quirks).

**1.4 Creation** (at activation unless flag 16): build the joint (sections
2–8) from the bodies' **current** poses; break limits
F_max = forcelimit/2.2, τ_max = torquelimit/2.2 (0 = unbreakable),
strength 1, per-body mass scales (1.2), active unless flag 4. If it is in
a constraint system, the system's group is (re)activated. Then: register
both entities for teleport notices (1.7) if both are real bodies; with
flag 1, disable collisions between the two bodies' entities.

**1.5 Breaking.** Each physics step the solver measures the force (and,
where applicable, torque) the joint applies; when it exceeds F_max (or
τ_max) the joint is deactivated and the game is told (*inferred*: the
exact measure and the gravity used to turn the pound limit into a force
are inside vphysics; public documentation says to set forcelimit to the
weight in pounds that should break it when resting on it; Open
questions). Then, and also on the **Break** or **ConstraintBroken**
input:
1. Deactivate (as TurnOff, 1.6, including re-enabling collisions under
   flag 1).
2. If "breaksound" is set: play it once, static channel, normal volume,
   static attenuation, at the midpoint of the two bodies' positions (the
   world counts as the other body's position).
3. Fire **OnBreak**.
4. Delete the constraint entity after the current physics step.

A broken constraint is gone: later TurnOn does nothing. Deleting one of
the two bodies (a broken prop, a killed func_physbox) also reports the
constraint as broken (*inferred* from the engine's "constraint notify"
switch; Open questions), so OnBreak fires then too.

**1.6 TurnOn / TurnOff.**
- TurnOn: with flag 16, look up the bodies and create the joint now if
  not yet created (if it already exists, re-register teleports and, with
  flag 1, disable collisions again). Then, if the joint exists and has
  both bodies: activate it and wake both bodies.
- TurnOff: deactivate; clear the "pinned to world" mark of both bodies
  (2.1); with flag 1, re-enable collisions between the two. phys_hinge
  with flag 8 also releases the world-hinge mode (4.4). No output.
- A TurnOn without flag 16 does **not** disable collisions again after a
  TurnOff (Quirks).

**1.7 Teleport following.** When either joined entity is teleported (an
explicit teleport: trigger_teleport, point_teleport, the Teleport-style
inputs; not ordinary motion) and the teleport distance (old origin to new
origin) is greater than "teleportfollowdistance", the **other** entity is
moved by the same rigid transform: if the teleport changed angles, the
other entity is rotated about the teleported one as well; otherwise it is
only translated by the same offset. Only if the other entity is a moving
physics body. At most once per tick per constraint (no recursion). With
the default 0 any non-zero teleport is followed.

### 2. phys_constraint (weld)

Joint: keeps the attached body's pose relative to the reference body
fixed as it was at creation (position and orientation). Both force and
torque limits apply. Joined to the world: the attached body is marked
"pinned to world"; **two bodies that are both pinned to the world never
collide with each other** (the mark is cleared on TurnOff/break). The
entity's own origin and angles do not matter.

### 3. phys_ballsocket

Joint: one point, the entity's **origin at creation**, expressed in each
body's local frame, must coincide; rotation is free. Only the force limit
applies (the torque limit is forced to 0, so "torquelimit" is ignored).

### 4. phys_hinge

Keyvalues: "hingeaxis" (point), "hingefriction", "systemloadscale",
common keys. Episodic-only sound keys ("minSoundThreshold",
"maxSoundThreshold", "slidesoundfwd", "slidesoundback",
"reversalsoundSmall/Medium/Large", "reversalsoundthresholdSmall/Medium/
Large") do nothing in CS:S.

**4.1 Axis**: at spawn, hinge point P = entity origin; direction
d = normalize(hingeaxis − P), snapped (a component with |x| > 0.998 makes
d exactly ±that axis). hingeaxis = origin (zero direction): the joint is
not created and the entity deletes itself at activation.

**4.2 Joint**: the attached body may only rotate relative to the
reference body about the line through P along d (both given in world
space at creation). No angle limits. Both force and torque limits apply.

**4.3 Friction and motor**:
- Friction: the hinge axis gets a resisting torque of hingefriction ×
  1000 (vphysics: a motor with target speed 0 and that maximum torque,
  *inferred*). SetHingeFriction f: same with the new f, applied to the
  live joint (*inferred*: the stored parameters are updated; whether the
  running joint picks it up is an Open question).
- **SetAngularVelocity** ω (degrees/s, *inferred* unit): if the joint has
  both bodies: wake every moving body; compute the load
  M = mean over the **moving** bodies of |I| (the length of the body's
  principal inertia vector, kg·in²), computed as: start at 1; the
  reference body, if moving, **replaces** it with |I_ref|; the attached
  body, if moving, **adds** |I_att|; divide by the number of moving bodies
  (if none, M = 1). With L = systemloadscale (0 → 1):

```math
\tau_{max} = \omega \cdot L^2 \cdot M / \Delta t
```

  and the hinge motor drives the relative angular velocity about the axis
  toward ω with at most that much torque (impulse per step, *inferred*)
  until changed. The motor uses the same axis setting as the friction,
  so it **replaces** the friction torque (*inferred*); SetAngularVelocity
  0 gives τ_max = 0, so the hinge then swings freely with no friction at
  all until SetHingeFriction (Open questions). Note L is applied
  **twice** and a world hinge's load includes the stray 1 (Quirks).

**4.4 Flag 8 (world hinge)**, only when exactly one of attach1/attach2 is
set: at spawn, request that the named body's centre of mass be moved
onto the hinge line (its default centre of mass projected onto the line
through P along d, applied when that body is created, which is later
thanks to the spawn priority). At creation, if the reference is the
world and d, expressed in the body's local frame and snapped, is exactly
a body axis, the body is switched into the physics engine's dedicated
"hinged" mode about that local axis (more stable, *inferred*). Otherwise
(both names set, or not axis-aligned) the flag is cleared and ignored.

### 5. phys_slideconstraint

Keyvalues: "slideaxis" (point), "slidefriction", "systemloadscale",
common keys (episodic sound keys as for the hinge: ignored).

- Axis: d = normalize(slideaxis − origin), snapped (4.1).
- Joint: the attached body keeps its orientation relative to the
  reference body (as at creation) and may only translate along d (fixed
  in the reference body's frame).
- Flag 2 "Limit Endpoints": with s(x) = d·x, limits s_min = min(s(origin),
  s(slideaxis)), s_max = the larger; widened to include s(p_att), the
  attached body's position at creation; then made relative:
  [s_min − s(p_att), s_max − s(p_att)]. If both ends come out equal there
  is **no** limit (engine rule). Without flag 2: unlimited.
- Friction: slidefriction (a force, *inferred* kg·in/s²) resists sliding.
- **SetVelocity** v (units/s): load M as for the hinge but with **mass**
  (kg) instead of |I| (same start-at-1, replace, add, average rule);
  L = systemloadscale (0 → 1):

```math
F_{max} = v \cdot L \cdot M / \Delta t
```

  (L applied once here), motor drives the relative speed along d toward v
  with at most F_max. Overrides the friction until changed.

### 6. phys_lengthconstraint

Keyvalues: "attachpoint" (point), "addlength", "minlength", common keys.
Flag 2 "Keep Rigid".

- End points: the entity **origin** belongs to attach1's body, the
  "attachpoint" position to attach2's body (in both pairing cases of
  1.3; with the world on one side, the world end is a fixed world point).
  Each end point is stored in its body's local frame at creation.
- Lengths: L_max = |origin − attachpoint| + addlength; L_min =
  minlength; flag 2: L_min := L_max (a rigid rod).
- Joint: the distance between the two end points is kept ≤ L_max (a
  rope: slack below it) and ≥ L_min. Both limits apply.

### 7. phys_pulleyconstraint

Keyvalues: "position2" (point), "addlength", "gearratio", common keys.
Flag 2 "Is Rigid".

- Pulley wheels: W₀ = entity origin (for the reference body's rope),
  W₁ = position2 (for the attached body's rope).
- Rope ends: each body's anchor point (1.2: the body origin, or an
  info_constraint_anchor's point) — the world as reference has anchor
  (0, 0, 0) in world space (Edge cases). A₀, A₁ = their world positions
  at creation.
- g = gearratio; total length

```math
L = \text{addlength} + |A_0 - W_0| + g\,|A_1 - W_1|
```

  and the joint keeps |A₀ − W₀| + g'·|A₁ − W₁| ≤ L, with g' = g if g ≠ 0,
  else 1 (*inferred* meaning of "gear ratio": the second rope moves g
  times as far; Quirks for g = 0). Flag 2: = L (rope never slack).

### 8. phys_ragdollconstraint

Keyvalues: "xmin", "xmax", "ymin", "ymax", "zmin", "zmax" (degrees),
"xfriction", "yfriction", "zfriction", common keys. Flag 2 "Only limit
rotation", flag 4 start inactive.

- Constraint frame: the entity's origin and angles at creation, stored
  relative to each body.
- Joint: a ball joint at the frame origin (unless flag 2: then the bodies
  may also translate freely, only rotation is limited) with the relative
  rotation about the frame's x, y, z axes limited to [min, max] degrees
  and a friction torque per axis (the raw value, **not** × 1000 as for the
  hinge). Equal min and max (e.g. both 0) lock that axis (*inferred*).
- **forcelimit, torquelimit and anchor mass scales are ignored**: the
  ragdoll constraint never breaks by itself (Quirks). Break/TurnOff
  inputs still work.

### 9. phys_spring

Keyvalues: "attach1", "attach2", "springaxis" (point: the other end),
"constant" (default 150), "length" (natural length, units, ≤ 0 → the
spawn distance), "damping" (default 2), "relativedamping" (default
0.01). Spawnflag 1 "Force only on stretch". Inputs **SetSpringConstant**,
**SetSpringLength**, **SetSpringDamping** (floats, applied to the live
spring). No outputs, never breaks, no TurnOn/TurnOff.

- Spawn: start S = entity origin, end E = springaxis; natural length
  := |E − S| if "length" ≤ 0.
- Activation: start body = body named attach1, end body = attach2 (first
  named entity with a body, 1.2 step 2; anchors are not used). Missing
  start → world as start. Start found but end missing → the found body
  becomes the end body and the world the start. Both missing, or both the
  same body: error, the entity deletes itself. Teleport following (1.7,
  with no distance threshold) only when both bodies were found.
- If E is not nearer than S to the end body's centre, S and E are
  swapped (so the end point nearest a body is attached to it).
- The spring acts between world points S (on the start body) and E (on
  the end body), each fixed in its body's frame at creation: force along
  the line between them

```math
F = k\,(\ell - \ell_0) + c\,\dot\ell \quad (\text{only if } \ell > \ell_0 \text{ with flag 1})
```

  (*inferred* law; "relativedamping" damps the bodies' relative velocity;
  the units of k and c are vphysics's: Open questions).

### 10. info_constraint_anchor and phys_constraintsystem

**info_constraint_anchor**: keys "massScale" (default 1), "parentname"
(required). At spawn, if it has a parent: record (its name, its parent,
the parent attachment if parented to a named attachment, its local offset
from the parent, its mass scale) in a level-wide list and delete itself;
without a parent it stays an inert point entity and is not an anchor.
Constraints use the list (1.2). The anchor's mass scale is applied to that
body's mass **inside this constraint's solver only**, to make a light
body behave as heavier against the joint (*inferred* purpose).

**phys_constraintsystem**: key "additionaliterations" (integer, default
0). At spawn it creates a solver group with that many extra iterations
(error ticks 15, tolerance 3 units). Constraints naming it in
"constraintsystem" are solved together in that group (more stable chains;
*inferred*). No inputs or outputs of its own. It must exist by the time
the constraints activate (always true for map entities, since groups are
made at spawn).

## Per-tick order

1. Event queue: inputs (TurnOn/TurnOff/Break, motor and friction inputs,
   spring inputs) run in firing order (specs/source/entity_io.md).
2. Thinks.
3. Physics step: controllers, gravity, joint solving (motors, friction,
   limits), contacts. A joint exceeding its limit is deactivated inside
   the step and reported; the report runs the break steps 1.5 1–3 at once
   (sound, OnBreak queued like any output), and the entity is deleted
   after the step.
4. Entities copied from the bodies; teleport notices during the tick
   (from inputs or triggers) are handled when the teleport happens.

Map start: all spawns (constraints first by priority, hinge mass-centre
overrides registered) → all activations in map order (bodies looked up,
joints created from the bodies' poses at that moment) → first tick.

## Edge cases

- **attach1 and attach2 both empty**: deleted at activation (nothing to
  join).
- **attach2 names a func_brush and attach1 is empty**: world + static →
  deleted (two static bodies).
- **Body joined to a mover** (func_door, func_rotating,
  func_tracktrain): the mover drives it; the mover is never pushed back.
  Two movers: refused.
- **Constraint whose body starts motion-disabled** (func_physbox flag
  32768, prop with motion disabled): the joint exists but the body does
  not move until motion is enabled; a joint to a pinned body acts like a
  joint to the world.
- **Pulley with attach1 empty**: the reference is the world, whose anchor
  is the world origin (0, 0, 0): the first rope's length is measured from
  (0, 0, 0) to the first wheel. Keep (it only shifts the total length).
- **Pulley with attach2 empty** (swapped pairing): the anchor points are
  not swapped with the bodies: the body's rope end is its origin and the
  attach1 anchor offset (0 without an info_constraint_anchor) is read as
  a world position for the world's end. Without anchors this is the same
  as the case above.
- **Length constraint with addlength < 0**: L_max shorter than the
  current distance: the joint snaps the bodies together on the first
  step (*inferred*).
- **Hinge or slider axis not exactly axis-aligned**: used as is (only
  near-axis directions within 0.002 are snapped).
- **Slide limits when the attached body starts outside them**: the
  limits are widened to include its start, so it never starts violating
  them.
- **Break input on a constraint created with flag 16 and never turned
  on**: no joint; OnBreak still fires and the entity is deleted
  (*inferred*: the break path does not check for a joint before the
  sound; with no joint and a breaksound set it would fail; Open
  questions — treat as: fire OnBreak, no sound).
- **Spring inputs before activation** or on a spring that failed to
  create: the original would crash; ignore the input.
- **Same name in attach1 and attach2**: both resolve to the same body;
  the two-object constraints do not refuse this (vphysics behaviour
  unknown; treat as not created); phys_spring refuses it.
- **Ragdoll bodies**: a constraint to a map prop_ragdoll joins the part
  that the name resolves to (the main part) unless an anchor attached to
  a bone attachment chooses another part.

## Quirks

- **Unresolved names fall back to the world** (warning only). Maps rely on
  this (attach2 left as a typo still "works" as a world pin). Keep.
- **Ballsocket ignores torquelimit; ragdoll constraint ignores both
  limits and mass scales.** Keep.
- **Hinge motor torque uses systemloadscale squared**; slider uses it
  once. Keep (maps are tuned to it).
- **Motor load starts at 1**: with the world as reference (the usual
  case) the load is 1 + |I| (hinge) or 1 + mass (slider), not |I| or mass.
  Keep.
- **TurnOn after TurnOff does not restore "No Collision until break"**
  (without flag 16). Keep.
- **Flag 1 with flag 4**: collisions are disabled from creation even while
  the constraint starts inactive. Keep.
- **Pulley gear ratio 0** removes the second rope from the total length
  but the joint still uses gear 1. Keep.
- **phys_constraint pins mark bodies "pinned to world"**, and two such
  bodies never collide with each other (stacked welded crates pass
  through each other if pushed together). Keep.
- **Joints are built from the bodies' poses at activation**, not from
  Hammer's intent: a body that moved between spawn and activation (none
  normally do) keeps the moved relation.

## Test cases

| Setup | Input | After | Expected |
|---|---|---|---|
| phys_ballsocket attach1 = "" attach2 = "box" (exists), forcelimit 220 | activation | — | reference world, attached box; F_max = 100 kg; τ_max 0 (ignored) |
| phys_ballsocket attach1 = "boxx" (typo), attach2 = "" | activation | — | world + nothing → deleted |
| phys_constraint attach1 = "box" (exists), attach2 = "nope" (missing) | activation | — | warning; box welded to the world (pinned, marked) |
| two boxes each welded to the world by phys_constraint, pushed into each other | — | — | they do not collide with each other |
| phys_hinge origin (0,0,0), hingeaxis (0.5, 0, 100) | spawn | — | d = (0.005, 0, 0.99999) → snapped (0, 0, 1) |
| phys_hinge origin (0,0,0), hingeaxis (10, 0, 100) | spawn | — | d = (0.0995, 0, 0.995), not snapped |
| phys_hinge hingeaxis = origin | activation | — | not created, entity deleted |
| hinge, hingefriction 0.5 | — | — | axis friction torque 500 |
| hinge world + body with principal inertia (10, 20, 20) kg·in², systemloadscale 0 | SetAngularVelocity 90 | — | |I| = 30, M = 1 + 30 = 31, τ_max = 90·31/0.015 = 186000 |
| same, systemloadscale 2 | SetAngularVelocity 90 | — | τ_max = 90·4·31/0.015 = 744000 |
| hinge between two moving bodies, |I| 30 and 50 | SetAngularVelocity 90 | — | M = (30+50)/2 = 40, τ_max = 240000 |
| slide world + 50 kg body, systemloadscale 2 | SetVelocity 100 | — | M = 51, F_max = 100·2·51/0.015 = 680000 |
| slide origin (0,0,0), slideaxis (0,0,100), flag 2, body at z = 30 | creation | — | limits [−30, +70] relative to start |
| same, body at z = −20 | creation | — | limits widened to [−20, 100] → relative [0, 120] |
| slide without flag 2 | — | — | unlimited along +Z |
| length origin (0,0,100), attachpoint (0,0,0), addlength 20 | creation | — | L_max = 120, L_min = minlength (0) |
| same, flag 2 | creation | — | L_min = L_max = 120 |
| pulley origin W₀ (0,0,100), position2 W₁ (100,0,100), body A at (0,0,0), body B at (100,0,50), gear 0 | creation | — | L = 100 + 0·50 = 100, gear used 1 |
| same, gearratio 1 | creation | — | L = 150 |
| same, gearratio 2 | creation | — | L = 200, gear 2 |
| ragdollconstraint forcelimit 10 | heavy load | — | never breaks |
| ragdollconstraint xmin −30 xmax 30, others 0 | — | — | free ±30° about the entity's x axis, y and z locked |
| constraint flags 1 | TurnOff | — | bodies collide again |
| same | TurnOn (no flag 16) | — | joint active, bodies still collide |
| constraint flag 4 | activation | — | joint exists, inactive; TurnOn activates and wakes both |
| constraint flag 16 | activation | — | no joint; first TurnOn looks up bodies and creates it |
| constraint with breaksound "x" | Break | — | sound at midpoint of the bodies, OnBreak fired, entity deleted after the step; later TurnOn: nothing |
| teleportfollowdistance 64, box A teleported 32 units | — | — | B not moved |
| same, A teleported 200 units, no rotation | — | — | B translated by the same 200-unit offset |
| phys_spring origin (0,0,0), springaxis (0,0,64), length 0 | spawn | — | natural length 64 |
| phys_spring attach1 empty, attach2 "box" | activation | — | spring between world point and box |
| phys_spring attach1 "a" = attach2 "a" | activation | — | deleted |
| phys_spring S (0,0,0), E (0,0,64), end body centre at (0,0,−10) | activation | — | E not nearer → S and E swapped: (0,0,0) on the end body |
| info_constraint_anchor "anc" parented to "crate" at local (0,0,16), massScale 4; ballsocket attach1 "anc" | activation | — | body = crate, mass scale 4 for that body in this joint |

## Open questions

1. **Break limit physics**: what exactly is compared to forcelimit/2.2
   (kg)? Probe: hang a prop of known mass from a phys_ballsocket to the
   world with forcelimit F; find the smallest mass that breaks it at rest
   with sv_gravity 800 (CS:S default). If it breaks near F/2.2 kg the limit
   is "weight in kg"; record the factor otherwise. Repeat for torquelimit
   with a phys_constraint weld and a known offset mass.
2. **Motor units**: SetAngularVelocity in deg/s? Measure the steady
   rotation speed of a light door on a phys_hinge after
   SetAngularVelocity 90 (record yaw over 2 s with the server probe).
   Same for SetVelocity in units/s on a slider.
3. **Hinge friction**: how much does hingefriction 1 slow a door swung at
   a known speed? Probe: push with a phys_thruster for one tick, record
   the angular velocity decay. Also whether SetHingeFriction affects a
   running joint.
4. **Spring law and units**: measure the oscillation period of a 10 kg
   box on a vertical phys_spring with constant 150, damping 0, natural
   length 64, to get the effective stiffness (period = 2π√(m/k)).
5. **Body deleted while constrained**: does OnBreak fire? Weld a
   breakable prop to the world with an OnBreak output to a logic_relay
   that prints, then break the prop.
6. **CS:S differences**: CS:S's server is its own build; confirm the
   non-episodic world fallback (typo in attach1 still pins to the world)
   on a test map.
7. **Ragdoll constraint limits with equal min and max**: locked or free?
   Place one with all limits 0 between two boxes and push.
8. **Pulley gear semantics**: verify that with gearratio 2 the second
   body rises 2 units per unit the first falls (watch both bodies).
9. **Break on a never-created joint** (flag 16 + Break): crash, nothing,
   or OnBreak? Try on a listen server with developer 1.
10. **Spawn tie order** for equal priority and depth: map order or not?
    Only matters for which of two same-name bodies is found first; check
    with two props of the same name and a ballsocket.
