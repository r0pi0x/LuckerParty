# Counter-Strike: Source: player ragdolls

Source basis: Valve's public Source SDK 2013 only. I read the shared ragdoll
code (building bodies and joints from a `.phy`, the death force, animation
velocity, collision rules, animated friction, the "rigid attachment" bone
read-back, separation repair, the ragdoll fade queue), the client ragdoll
object (creation, bounds, forced sleep), the client animating entity (how a
dying entity becomes a ragdoll, how bones without physics are posed, the
client-only ragdoll's impact response, fade), the client physics environment
(time step, gravity, collision filtering, self-penetration), the collision
group rules, the client impact effect that pushes ragdolls, the death-pose
helpers used by NPCs, the generic damage-force helpers, the SDK template mod's
player ragdoll (client and server; it is derived from CS:S and keeps comments
naming CS classes) and the public vphysics constraint headers. The physics
engine (vphysics / IVP), the bone-derivative helper and CS:S's own game DLLs
are **not** public. CS:S-specific behaviour is marked *inferred* and listed
under Open questions.

Checked against the user's install with throwaway scripts outside the repo:
every CS:S player `.phy` text section, the bone tables, hitboxes and the
`ragdoll` sequence of `ct_urban.mdl` and `t_phoenix.mdl`, and the death-pose
animations of `cs_player_shared.mdl`. Console variable names (and one help
string) were confirmed present in the CS:S client and server binaries; their
defaults were not read from the binaries and come from the SDK.
Status: draft

## Summary

- When a CS:S player dies, the server spawns a small networked record (class
  `cs_ragdoll`) that carries the player's model, origin, velocity, the hit
  physics bone, a force vector and a death pose. Every client then builds its
  **own** ragdoll from the dead player's current animated pose and simulates
  it in its client-side physics world. The server never simulates player
  ragdolls, bullets on the server pass through them, and each client sees the
  body land in a slightly different place. (*Inferred* from the networked
  fields and the CS-derived SDK template.)
- The ragdoll is the model's `.phy`: 16 convex bodies (one per major bone),
  15 ball-and-socket joints with per-axis angle limits in degrees, and a list
  of body pairs allowed to collide with each other. Bodies start exactly at
  the animated bone transforms, get each bone's velocity from the pose 0.05 s
  earlier (or toward a hit-dependent "death pose"), plus the killing hit's
  impulse on the hit body and a mass-weighted share on all others.
- The mesh follows the bodies: each simulated bone takes its body's rotation
  but is pinned to its parent body at the bind-pose offset (joints never
  visibly stretch); the other 34 bones keep their bind-pose local transform
  under their parent.
- Ragdolls collide with the world, static props and client-side props, not
  with players, not with server-simulated props and not with each other.
  They are forced to sleep after 5 s of near-stillness and are removed at the
  next round restart; there is no fade timer for CS:S player bodies.

## Units and conventions

- Game space: inches, Z up, right-handed; angles in degrees. `.phy` vertex
  space and the IVP axis mapping are as in physics_props.md ("Units and
  conventions", §1.7: ragdoll solids are stored in the **bone-local space**
  of the bone named in their `name` key).
- Model and bone conventions (bind pose, local transform = child-to-parent
  rotation q then translation p, quaternions (x, y, z, w)) are as in
  animation.md. The animated character faces +X in model space, its left is
  +Y.
- Mass kg; impulses ("forces") kg·in/s, so impulse J changes a body's
  velocity by J/m in/s. Angular velocities in °/s.
- Time: client physics advances in fixed steps of one server tick interval
  (0.015 s at CS:S's default 66 tick) regardless of frame rate; the client
  feeds it the real frame time.
- "Body i" = solid i of the `.phy` = the i-th `solid` block in text order
  (the `index` key equals the order in all shipped player files).

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| ragdoll_max_bodies | 24 | – | a `.phy` with more solids makes no ragdoll at all |
| player_bodies | 16 | – | every CS:S player model |
| player_joints | 15 | – | every CS:S player model |
| player_collision_pairs | 24 | – | every CS:S player model |
| ragdoll_rot_inertia_limit | 0.1 | – | set on every ragdoll body (props use 0.05; meaning: physics_props.md Open question 2) |
| joint_friction_scale | 1.0 | – | client multiplier on the `.phy` joint friction |
| bone_dt_player | 0.05 | s | look-back for bone velocities, player ragdolls (template; *inferred* for CS:S) |
| bone_dt_generic | 0.1 | s | the same for generic client ragdolls (not players) |
| death_pose_frames | 6 | – | frame f of a death pose is sampled at cycle f / 6 |
| group_error_tolerance | 3 | in | joint error before a ragdoll counts as "in error" |
| group_min_error_ticks | 15 | ticks | error must persist this long |
| group_extra_iterations | 0 | – | extra solver iterations |
| separation_fix_distance | 1 | in | child farther than this from its joint anchor is a fix candidate |
| separation_mass_ratio | 0.5 | – | child lighter than half its parent is always fixed |
| ragdoll_sleepaftertime | 5.0 | s | cvar: stationary this long → forced to sleep |
| settle_tolerance | 1.0 | in | per-axis root movement per check below which the ragdoll counts as stationary |
| cl_ragdoll_collide | 0 | – | cvar: ragdoll-ragdoll collisions (see Quirks: no effect for player bodies) |
| cl_phys_timescale | 1.0 | × | cvar (cheat): scales client physics time |
| cl_ragdoll_physics_enable | 1 (*inferred*) | – | CS:S-only cvar, help text "Enable/disable ragdoll physics." |
| bullet_hit_impulse | 4000 | kg·in/s | push of a later bullet on a ragdoll (template; *inferred* for CS:S) |
| blast_hit_scale_template | 4000 × trace length | kg·in/s | template blast push (see Quirks) |
| blast_hit_scale_client_copy | 500 × trace length | kg·in/s | generic client ragdoll blast push |
| g_ragdoll_maxcount | 8 | – | fade queue size for client-only ragdoll copies (not CS player bodies, *inferred*) |
| g_ragdoll_fadespeed | 600 | alpha/s | fade speed of queued client ragdolls |
| max_linear_speed / max_angular_speed | 2000 / 3600 | in/s, °/s | per body, as for props |

There is **no** `cl_ragdoll_fade_time` in CS:S (the name is absent from both
binaries); it belongs to later Source games.

## Behaviour

### 1. The player `.phy` as a ragdoll description

Binary layout and the `solid` keys are in physics_props.md §1. What a
ragdoll adds:

#### 1.1 Bodies

- One body per `solid` block. The body's collision is that solid's convex
  pieces in the bone-local space of the bone named by `name`. A bone name
  that the model does not have drops that body (a warning, no failure).
- `parent` names the parent body's bone (informational; the joint blocks are
  what connect bodies).
- Body parameters: `mass`, `inertia` (multiplies the inertia tensor; 10 for
  every player body), `damping` (0.05 for every player body), `rotdamping`
  (1 to 7, per body), `surfaceprop` (`flesh` for all), `volume`. `massbias`
  is a compile-time weight and is ignored at runtime. The rotational inertia
  limit is forced to 0.1 for ragdoll bodies.
- The editor block's `totalmass` (100) is informational: the real total is
  the sum of the body masses (102.64 kg, because the two clavicles and two
  hands were floored at 1 kg each).

#### 1.2 Joints (`ragdollconstraint` blocks)

- Keys: `parent`, `child` (body indices), and for each axis a, `amin`,
  `amax` (degrees) and `afriction`.
- A joint whose parent equals its child is discarded. In shipped files each
  body is the child of exactly one joint, except the root (body 0, pelvis).
- Joint frame: the joint's reference frame is the **child body's bone
  frame**. Where that frame sits on the parent is fixed at creation as the
  child bone's bind-pose transform expressed in the parent bone's space,
  even when unsimulated bones lie between them (e.g. Spine1's parent body is
  the pelvis although the skeleton has Spine in between). Call its
  translation the child's **anchor** `a_child` (parent-bone-local inches).
- Limits are rotations of the child relative to the parent about the child
  bone's local X, Y and Z axes, measured from the bind-pose relative
  orientation (0° = the bind pose's relative orientation). In ValveBiped,
  local X runs along the bone, so X is twist and Z is the main hinge.
  Sign, checked against the shipped animations (*inferred*, the solver is
  not public): positive Z on the calf is knee flexion (the crouch idle has
  the left calf at +48° and the right at +71° about local Z, inside
  −10..115); the forearm flexes toward negative Z (walk: left forearm −24°,
  inside −120..10).
- How the three axis limits combine (Euler order, twist/swing split) is
  inside vphysics (Open questions).
- `xfriction`, `yfriction`, `zfriction`: a resisting torque on that axis,
  multiplied by the joint friction scale (1.0 on clients). All are 0 in every
  CS:S player `.phy`, so CS:S joints are frictionless within their limits.
- All joints of one ragdoll form one constraint group (error tolerance 3 in,
  15 ticks, no extra iterations; see §6.3).

#### 1.3 Self-collision (`collisionrules` block)

- The rule set is per model and built once (the first ragdoll of that model
  creates it; later ones reuse it).
- With a `collisionrules` block: every pair of bodies starts **not**
  colliding; each `"collisionpair" "a,b"` enables bodies a and b. A
  `"selfcollisions" "0"` key disables all self-collision (and later pairs
  are ignored).
- Without the block: every pair collides except each child with its joint
  parent.
- Two bodies of the same ragdoll that are allowed to collide but end up
  interpenetrating are separated by the constraint group's own penetration
  solver, not by the normal contact response.

#### 1.4 Animated friction (`animatedfriction` block)

Absent from all CS:S player files; listed so a parser can handle it. Keys
`animfrictionmin`, `animfrictionmax` (integers), `animfrictiontimein`,
`animfrictiontimeout`, `animfrictiontimehold` (s). If min or max is non-zero,
from creation the joints' angular motor torque ramps linearly from min to max
over time-in, holds max for time-hold, ramps back down over time-out, then
stays as last set; the ragdoll is woken every frame while ramping.

#### 1.5 Which bones are simulated

Bone index of each body (`ct_urban.mdl` and `t_phoenix.mdl` share the
skeleton): body 0 → bone 0 Pelvis, 1 → 10 Spine1, 2 → 11 Spine2,
3 → 28 R_Clavicle, 4 → 15 L_Clavicle, 5 → 16 L_UpperArm, 6 → 17 L_Forearm,
7 → 18 L_Hand, 8 → 29 R_UpperArm, 9 → 30 R_Forearm, 10 → 31 R_Hand,
11 → 5 R_Thigh, 12 → 6 R_Calf, 13 → 1 L_Thigh, 14 → 2 L_Calf, 15 → 14 Head1.
The other 34 of the 50 bones (Spine, Spine4, Neck1, feet, toes, fingers,
wrists, ulnas, weapon bones, `forward`) are not simulated (§5).

Each bone record in the `.mdl` also stores its **physics bone** (bone offset
+172): the body that bone's hitbox belongs to (feet and toes → calf body,
Neck1 → Spine2 body, fingers → hand body). A trace that hits a hitbox
reports this as the hit body.

### 2. When and where the ragdoll is created

#### 2.1 Server side (CS:S, *inferred*)

On death the server creates one `cs_ragdoll` entity per dead player and
networks it to every client regardless of visibility. Fields (the template
sends the first five; CS:S's network table also has a death pose and a
death frame):

- the dead player (handle), so a client that has the player can copy its
  pose;
- the player's model, origin and velocity at death;
- the hit physics bone (8 bits; the physics bone of the killing hit's
  hitbox, see 1.5);
- a force vector (the template sends zero; see Open questions);
- the death pose (a sequence) and death frame (1–6), chosen as in 2.3.

The dead player entity itself is hidden. The record lives until the
server removes it, which for CS:S is the next round restart (template
comment: "removed on round restart automatically"). There is no server-side
physics for it.

#### 2.2 Client side: the poses that seed the ragdoll

When a client first receives the record:

1. If the client has the dead player entity (not dormant):
   - Remote player: the ragdoll copies the player's interpolation history
     and render angles, and its sequence, cycle and playback rate.
   - Local player: the ragdoll is placed at the networked origin with the
     player's render angles.
   - In both cases the three bone sets below are computed **from the player
     entity**, so they reflect what the client was drawing.
2. Otherwise (player dormant or unknown): the ragdoll is placed at the
   networked origin with flat (reset) interpolation history, and the bone
   sets are computed from the ragdoll entity itself. Its animation then has
   no history, so bone velocities are zero (see Edge cases).

The three bone sets (each a full set of world-space bone matrices):

- **B0, previous pose**: the bones at time `t_now − bone_dt` (bone_dt =
  0.05 s), using the entity's interpolated origin, angles and animation at
  that time.
- **B1, current pose**: the bones at `t_now`. If the server chose a death
  pose (2.3), B1 is instead the death pose sequence sampled at cycle
  `frame / 6`, placed at the entity's origin (see Quirks for a small origin
  nudge).
- **Bs, seed pose**: the bones at the time the next client physics step
  begins if that is before the end of this frame (so the body reaches the
  drawn pose at `t_now` instead of popping ahead), else the bones at
  `t_now`. Bs is always the normal animated pose, never the death pose.

#### 2.3 Death pose choice (*inferred* for CS:S; the generic rule)

From the killing hit's damage force F and the dead entity's facing:

- d = −normalize(F) (pointing back toward where the hit came from).
- f = d · forward, r = d · right (forward and right from the entity's
  angles).
- If |r| > |f|: r < 0 → "left side", else "right side". Otherwise f < 0 →
  "back side", else "front side". Ties go to front/back.
- Frame from the hit group: head 1, chest 2, stomach 2, left arm 3, right
  arm 4, left leg 5, right leg 6. Any other hit group (generic, gear) keeps
  the side but uses frame 1 (see Quirks).
- The sequence is the model's sequence for the side's activity:
  `deathpose_front/back/right/left` (activities `ACT_DIE_FRONTSIDE`,
  `ACT_DIE_BACKSIDE`, `ACT_DIE_RIGHTSIDE`, `ACT_DIE_LEFTSIDE`). The crouched
  variants `deathpose_crouch_*` (`ACT_DIE_CROUCH_*SIDE`) presumably apply
  when the player died crouched.
- No death pose is chosen if the damage type carries no physics force.

The shipped death poses are 7-frame, 1 fps animations: frame 0 is a neutral
stand (crouch for the crouch set), frames 1–6 are the per-hit-group poses,
and cycle f/6 lands exactly on stored frame f. They are not meant to be
seen; they exist to give each bone a velocity toward a plausible reaction
(head-shot from the front: head snaps back about 22 in).

#### 2.4 Building the bodies (order matters)

1. Read the text section in order: create each body (2.4.1), each joint, the
   collision rule set, the animated friction.
2. For each body i: create it with its `.phy` parameters at Bs[bone(i)]
   (position and rotation of the bone's world matrix). The body's collision
   pieces are in bone-local space, so they line up with the mesh at once.
   Collisions stay off until step 6.
3. Each joint is created between child and parent at their current
   (seed) relative placement, with the bind-pose frame of 1.2. The child's
   anchor `a_child` is stored for §5 and §6.3.
4. Death impulse (2.5).
5. Bone velocities (2.6) are **added** to each body.
6. Activate: apply the collision rule set, mark every body as part of a
   ragdoll, enable collisions, wake every body, activate the joint group
   (which also wakes everything).
7. The ragdoll entity takes the debris collision group (§4), uses body 0
   (pelvis) as its main physics object, and becomes visible to client
   traces even though it is not solid.
8. The entity switches to its model's `ACT_DIERAGDOLL` sequence (in CS:S
   merged sequence 0, `ragdoll`) at playback rate 0; the server can no
   longer change its sequence. This sequence poses the unsimulated bones
   (§5).
9. The player's model instance moves to the ragdoll, so existing blood and
   bullet decals stay on the body.

#### 2.5 Death impulse

Inputs: force vector F (kg·in/s, world) and hit body k (from the record).

- If 0 ≤ k < body count: body k receives F at its mass centre. Let p be
  body k's **origin** (its bone origin, not its mass centre). Every other
  body j receives the impulse `(m_j / M) · F` applied at the world point p
  (so also an angular impulse `(p − c_j) × (m_j/M)·F` about its mass centre
  c_j). M = total mass, at least 1 kg.
- If k is out of range (e.g. −1): no impulse at all (the shared point stays
  at the world origin, which disables the distribution).
- Linear momentum added: `F · (2 − m_k / M)`. Every body except k gets the
  same linear Δv = F / M; body k gets F / m_k.

#### 2.6 Bone velocities

For each body i with bone b:

- Linear velocity: `v = (o1 − o0) / bone_dt`, where o0, o1 are the world
  positions of bone b in B0 and B1. This is the bone origin's velocity, set
  as the body's (mass-centre) velocity.
- Angular velocity: the rotation that takes the bone's B0 orientation to its
  B1 orientation, as axis × angle (°), divided by bone_dt, in world space;
  it is converted to the body's local frame before being added. (The exact
  helper is not public; this is the natural reading, *inferred*.)
- Velocities are added on top of the death impulse.

Consequences: a player running at 250 in/s whose pose does not change gives
every body +250 in/s; with a death pose the bones additionally lunge toward
the pose (the front head-shot example in Test cases gives the head about
450 in/s backward).

### 3. Simulation

- Client physics environment: gravity `(0, 0, −sv_gravity)` read when the
  level loads, fixed step = one server tick, advanced each frame by
  `frame_time × cl_phys_timescale`.
- Bodies obey the same rules as physics props (physics_props.md §3: damping
  law, friction from surface properties, speed clamps 2000 in/s and
  3600 °/s, at most 10 collisions per body per step). Player bodies use
  surfaceprop `flesh`, damping 0.05, rotdamping 1–7, inertia ×10.
- Joints keep the anchor points together and the relative rotation inside
  the limits. Self-collision follows 1.3.
- `cl_ragdoll_physics_enable 0` disables ragdoll physics (behaviour when off
  unknown, Open questions).

### 4. What a ragdoll collides with

Collision filtering for two bodies of different entities, in order:

1. Collision-group rules. A ragdoll is in the **debris** group, which
   collides only with the "none" group (world, static props, doors that have
   client collision, most client-side props) and with the multiplayer
   push-away group (client-side `prop_physics_multiplayer`).
2. Two ragdoll bodies of different ragdolls collide only if
   `cl_ragdoll_collide` is 1 (but rule 1 already rejects debris vs debris).
3. Contents masks and explicit "don't collide" pairs.

What exists in the client physics world at all: the BSP's world collision
(model 1), static props, client-side physics props (and their gibs), the
client shadows of rotating prop doors, and ragdolls. Players,
server-simulated physics props (`prop_physics`, `func_physbox`,
`prop_physics_multiplayer` in modes 1 and 2) and brush entities other than
the world (func_door, func_brush, func_breakable, …) are **not** in it, so
ragdolls pass through them (*inferred* from the client code; worth an
in-game check).

Bodies of the same ragdoll: 1.3.

### 5. How the mesh follows the bodies

Every frame the ragdoll is drawn, bone matrices are built in bone order
(parents before children):

1. Simulated bone b of body i: rotation = body i's current world rotation.
   Position: for the root body, body 0's world position; for every other
   body, the parent body's **bone** matrix applied to `a_child` (the
   rigid-attach rule), not the body's own simulated position. So joints
   never visibly stretch even if the solver lets them drift.
2. Unsimulated bone: world = parent bone's world matrix × the bone's local
   transform from the current sequence (`ragdoll`, playback 0). That
   sequence animates only the pelvis, so every unsimulated bone uses its
   **bind-pose local transform** (the model's bone default position and
   rotation). Feet stay at their bind angle to the calves, fingers at their
   bind curl, Spine4/Neck1 rigid with Spine2, etc.
3. No IK and no interpolation are applied to ragdolls; positions are read
   from the physics state.

The entity's origin follows body 0 every frame. Its culling bounds are a
cube of half-size `|bounds_max − bounds_min| / 2` (the model's bounds)
around it while
awake, and the exact union of the bodies' bounds once every body sleeps.

### 6. Life of a ragdoll

#### 6.1 Forced settling

Checked at most once per client frame, when the physics engine reports the
ragdoll moved:

- Let Δ = body 0's position now minus at the previous check. If any axis
  |Δ| > 1 in, the ragdoll is "moving": remember the time.
- Otherwise, if not every body is already asleep and at least
  `ragdoll_sleepaftertime` (5 s) passed since it last moved, every body's
  velocity is zeroed and it is put to sleep.
- A bullet push (6.2) resets the timer.

The physics engine's own sleep thresholds also apply (physics_props.md Open
question 5).

#### 6.2 Being shot after death

A client-side bullet impact effect traces from the shot start to the
impact point. For each ragdoll on that line (skipped if the ragdoll was
created this tick, because the killing force was already applied):

- Non-blast: impulse `4000 · normalize(end − start)` applied at the hit
  point, on **body 0 (pelvis)** (see Quirks).
- Blast: impulse `4000 · (end − start)` (not normalized) at body 0's mass
  centre.
- The settle timer is reset.

These pushes exist only on the client that saw the impact.

#### 6.3 Separation repair

If the joint group reports an error (some joint off by more than 3 in for
15 ticks), once per frame while awake, for each body with a parent:

- target = parent body's world transform applied to `a_child`; gap =
  target − child body position.
- If the parent was repaired this frame, repair the child too.
- Else if |gap| > 1 in and either the child's mass × 2 < the parent's mass,
  or the child touches something in the gap direction and a world trace
  from target back to the child hits something: repair.
- Repair = move the child body so its origin is at target (rotation kept),
  linear velocity = the parent's velocity at target, angular velocity 0.
- If nothing needed repair, the error state is cleared.

#### 6.4 Removal

CS:S player bodies stay until the server removes the record (round restart)
and then vanish at once (*inferred*). The generic client fade queue (keep at
most `g_ragdoll_maxcount` = 8, remove the oldest not-burning ones first,
fading alpha at `g_ragdoll_fadespeed` 600/s) applies to client-only ragdoll
copies, not to these networked bodies (*inferred*).

## Per-tick order

Client, per frame:

1. Network updates: a new ragdoll record runs §2.2 to §2.4 (seed, impulse,
   velocities, activate).
2. Client physics simulates `frame_time × cl_phys_timescale` in whole
   steps of one tick (gravity, joints, contacts, damping, clamps).
3. For each ragdoll that moved: bounds update, separation repair (6.3),
   settle check (6.1).
4. Client think: animated friction ramp (none in CS:S).
5. Bullet impact effects push ragdolls (6.2) as they arrive.
6. Rendering: bone matrices per §5.

## Edge cases

- A model with no `.phy`, more than 24 solids, or no `ACT_DIERAGDOLL`
  sequence gets no ragdoll.
- Dormant or unknown player when the record arrives: no animation history,
  so B0 = B1 and all bone velocities are 0; only the death impulse moves the
  body. The networked velocity is stored on the entity but does not reach
  the bodies (template behaviour).
- Seed poses outside the joint limits (several upper-body crouch poses bend
  the right forearm to about +50° about Z, outside −120..10) are corrected by
  the joint solver on the first steps: the arm snaps into range.
- Hit body k = the physics bone of the hitbox: feet and toes push the calf
  body, the neck hitbox pushes Spine2, fingers do not exist as hitboxes.
- Bodies created inside world geometry are pushed out by the contact solver;
  a child stuck behind a wall while its parent is free is teleported back by
  separation repair.
- If the respawn comes before round restart (not normal in CS:S) the old body
  stays; a second death of the same player makes a second body.

## Quirks

- **Death impulse roughly doubles.** The hit body gets the whole force and
  every other body also gets its mass share, so total momentum is
  F·(2 − m_k/M) (≈ 1.96 F for a head hit). Keep: it is part of how far
  bodies fly.
- **Shared impulse point is the hit body's bone origin**, not its mass
  centre or the bullet's hit point, so the mass-share impulses also spin the
  other bodies about that point.
- **Later bullets push the pelvis.** The post-death push always goes to body
  0 with the lever arm from the pelvis mass centre to the hit point, as if
  the whole body were the pelvis. A 4000 kg·in/s push on a 1.91 kg pelvis is
  a 2091 in/s Δv before the joints share it out (and above the 2000 in/s
  clamp). Keep: this is why shot corpses twitch hard.
- **Blast push is not normalized**: it scales with the trace length.
- **Generic hit group still picks a death pose** (side from the force,
  frame 1), although the intent was "no pose".
- **Death-pose origin nudge**: the death pose is placed at the entity origin
  moved along its last interpolation step direction by (step length)² × frame
  time. The step is between the drawn and the interpolated origin at the same
  instant, so this is about 0.
- **Settle check depends on frame rate.** "Stationary" means body 0 moved at
  most 1 in on every axis since the last check, and checks happen per frame
  only when physics ran. With one physics step between checks (frame rate
  ≥ tick rate) a body slower than about 1 in / 0.015 s ≈ 67 in/s on every
  axis counts as still; at 30 fps (two steps) the bar is about 30 in/s.
  A slowly sliding body is therefore frozen 5 s after its last fast moment.
- **`cl_ragdoll_collide` does nothing for player bodies** (both are debris,
  rejected earlier), unless CS:S's game rules differ (Open questions).
- **Collision pairs are asymmetric**: the left forearm (6) may collide with
  the right upper arm (8) but the right forearm (9) never with the left upper
  arm (5). The upper arms never collide with the pelvis or thighs; the hands
  do.
- **Bind-pose extremities**: feet, fingers and the neck are not simulated and
  stay locked in their bind pose relative to their parent body.

## Test cases

All numbers from the user's install (`cstrike/cstrike_pak_dir.vpk`), read at
test time, never committed.

`.phy` data:

| Setup | Input | After | Expected |
|---|---|---|---|
| `ct_urban.phy` | parse | – | 16 solids, 15 joints, 24 collision pairs, no `animatedfriction` block; editparams `totalmass` 100, `rootname` `valvebiped.bip01_pelvis` |
| `ct_urban.phy` | sum body masses | – | 102.636687 kg (`t_phoenix`: 102.636752) |
| `ct_urban.phy` | body masses | – | pelvis 1.912537, Spine1 23.843636, Spine2 31.036686, clavicles 1.0, upper arms 1.315652, forearms 1.855846, hands 1.0, thighs 9.428412 / 9.428411, calves 6.050971 / 6.050970, head 4.542068 |
| `ct_urban.phy` | body params | – | all `flesh`, damping 0.05, inertia 10; rotdamping: pelvis 3, Spine1 5, Spine2 5, clavicles 6, upper arms 2, forearms 4, hands 1, thighs 7, calves 5, head 3 |
| `ct_urban.phy` vs `ct_gign`, `ct_gsg9`, `ct_sas`, `t_arctic`, `t_leet` | text sections | – | identical; `t_guerilla` and `t_phoenix` differ only in mass (≤ 0.005 kg) and volume |
| `ct_urban.mdl` + `.phy` | body → bone index | – | 0, 10, 11, 28, 15, 16, 17, 18, 29, 30, 31, 5, 6, 1, 2, 14; 34 bones unsimulated |
| all joints (both models) | xfriction, yfriction, zfriction | – | 0 |

Joint limits (degrees, child-bone axes; identical in both models):

| Joint (parent ← child) | x min, max | y min, max | z min, max |
|---|---|---|---|
| Pelvis ← Spine1 (0 ← 1) | −10, 10 | −16, 16 | −20, 30 |
| Spine1 ← Spine2 (1 ← 2) | −10, 10 | −10, 10 | −20, 20 |
| Spine2 ← R/L Clavicle (2 ← 3, 2 ← 4) | −15, 15 | −10, 10 | 0, 45 |
| Clavicle ← UpperArm (4 ← 5, 3 ← 8) | −15, 20 | −40, 32 | −80, 25 |
| UpperArm ← Forearm (5 ← 6, 8 ← 9) | −40, 15 | 0, 0 | −120, 10 |
| Forearm ← Hand (6 ← 7, 9 ← 10) | −25, 25 | −35, 35 | −50, 50 |
| Pelvis ← Thigh (0 ← 11, 0 ← 13) | −25, 25 | −10, 15 | −55, 25 |
| Thigh ← Calf (11 ← 12, 13 ← 14) | −10, 25 | −5, 5 | −10, 115 |
| Spine2 ← Head (2 ← 15) | −50, 50 | −20, 20 | −26, 30 |

Anchors (child bone origin in parent bone space, bind pose, inches,
tolerance 0.002):

| Joint | `ct_urban` | `t_phoenix` |
|---|---|---|
| Pelvis ← Spine1 | (0, 7.620, −3.446) | (0, 7.767, −3.513) |
| Spine1 ← Spine2 | (3.661, 0, 0) | (3.732, 0, 0) |
| Spine2 ← L_Clavicle | (11.195, 1.396, 2.015) | (11.410, 1.423, 2.054) |
| Spine2 ← R_Clavicle | (11.195, 1.396, −2.015) | (11.410, 1.423, −2.054) |
| L_UpperArm ← L_Forearm | (12.160, 0, 0) | (12.394, 0, 0) |
| Pelvis ← R_Thigh | (−4.046, 0, 0) | (−4.124, 0, 0) |
| L_Thigh ← L_Calf | (18.562, 0, 0) | (18.919, 0, 0) |
| Spine2 ← Head1 | (15.765, 2.729, 0) | (16.068, 2.782, 0) |

Collision rules (`ct_urban`):

| Setup | Input | After | Expected |
|---|---|---|---|
| rule set | pairs | – | (14,12) (13,11) (2,10) (2,7) (1,10) (1,7) (2,5) (2,8) (9,15) (9,1) (6,15) (6,1) (6,9) (6,8) (10,11) (10,13) (10,0) (7,11) (7,13) (7,0) (15,10) (15,7) (15,3) (15,4) |
| rule set | L hand (7) vs pelvis (0) | – | collide |
| rule set | L upper arm (5) vs pelvis (0) | – | do not collide |
| rule set | R forearm (9) vs L upper arm (5) | – | do not collide; L forearm (6) vs R upper arm (8) collide |
| rule set | thigh vs its calf (11, 12) | – | do not collide |

Model data:

| Setup | Input | After | Expected |
|---|---|---|---|
| `ct_urban.mdl` sequences | – | – | one sequence `ragdoll`, activity `ACT_DIERAGDOLL`; its animation `@ragdoll` (21 frames, 30 fps) has a record only for the pelvis |
| `ct_urban.mdl` hitboxes | bone → physics bone | – | Pelvis 0, Spine1 1, Spine2 2, Neck1 2, Head1 15, L_Thigh 13, L_Calf/L_Foot/L_Toe0 14, R_Thigh 11, R_Calf/R_Foot/R_Toe0 12, L_UpperArm 5, L_Forearm 6, L_Hand 7, R_UpperArm 8, R_Forearm 9, R_Hand 10 |
| `cs_player_shared.mdl` | `@deathpose_*` (8 animations) | – | 7 frames each, 1 fps |
| `@deathpose_front` with the shared model's bone defaults, root at the model origin | frames 0 and 1 | FK | Head1 origin (6.208, −2.116, 62.414) → (−16.242, 2.050, 60.962); pelvis (−0.410, 0.003, 37.436) → (4.746, 0.952, 41.065) |
| `@deathpose_back` | frame 1 | FK | Head1 ≈ (14.9, −4.2, 51.8): head thrown forward |
| `@Crouch_Idle_Lower` frame 0 | calf rotation relative to bind, about calf local Z | – | left +47.95°, right +71.39° (inside −10..115) |
| `a_WalkN` frame 10 | L_Forearm relative to bind, local Z | – | −24.39° |

Behaviour:

| Setup | Input | After | Expected |
|---|---|---|---|
| any ragdoll | build | – | every body's world transform = its bone's seed matrix; t_phoenix calf body points span 0..22.2 in along that bone's X |
| `ct_urban`, force F = (1000, 0, 0), hit body 15 (head) | build | Δv | head 1000 / 4.542068 = 220.16 in/s; every other body 1000 / 102.636687 = 9.743 in/s along +X; total momentum 1955.75 kg·in/s |
| same, hit body −1 | build | – | no impulse on any body |
| previous pose = current pose translated by (12.5, 0, 0), bone_dt 0.05 | build | – | every body v = (250, 0, 0) in/s, ω = 0 |
| previous pose = `@deathpose_front` frame 0, death pose frame 1 (front head shot) | build | head body | v ≈ (−449.0, 83.3, −29.0) in/s |
| death pose choice: entity yaw 0, damage force (−1, 0, 0) (shot from the front) | – | – | front side; head hit → frame 1, cycle 1/6 |
| entity yaw 0 (right = −Y), force (0, 1, 0) | – | – | right side |
| entity yaw 0, force (0, −1, 0) | – | – | left side |
| entity yaw 0, force (1, 0, 0) | – | – | back side |
| hit group stomach / left leg / generic | – | – | frame 2 / 5 / 1 |
| unsimulated L_Foot, calf body at identity at the origin | draw | – | foot origin (17.186, 0, 0), foot local rotation = bind (−0.0208, −0.0115, −0.5106, 0.8595) |
| Spine1 body drifted 2 in from its anchor | draw | – | Spine1 bone position = pelvis bone matrix × (0, 7.620, −3.446) (drift invisible) |
| body 0 moving at 30 in/s, 66 tick, 100 fps | 5 s | – | forced asleep at 5 s (0.45 in per step < 1 in) |
| body 0 moving at 80 in/s along X | – | – | never forced asleep (1.2 in per step) |
| bullet hits a settled `ct_urban` body | impact | pelvis | impulse 4000 at the hit point; pelvis Δv 2091.5 in/s before joints and clamps; settle timer reset |
| joint error, Head (4.542) vs Spine2 (31.037), gap 2 in | repair | – | 9.08 < 31.04 → head moved to its anchor, ω = 0 |
| joint error, Spine1 (23.84) vs pelvis (1.91), gap 2 in, no contact | repair | – | not repaired |
| two player ragdolls, `cl_ragdoll_collide 1` | overlap | – | do not collide (debris vs debris) |

## Open questions

1. **CS:S's own ragdoll code is not public.** Confirm the template model:
   server record only, client simulation, per-client results. Check: two
   clients watch the same kill; bodies should rest in different places, and
   a bullet fired into a body on the server leaves no server-side effect
   (`sv_showimpacts 1` shows the server trace passing through).
2. **Force vector.** The template sends zero; CS:S bodies visibly react to
   the killing shot. Likely the killing hit's damage force (bullet:
   direction × the ammo's impulse × `phys_pushscale`, physics_props.md §5.2;
   HE: §5.3), perhaps accumulated over the shots of the killing tick. The
   CS:S ammo impulses are unknown. Check: kill a standing bot with one
   Desert Eagle shot vs one knife stab vs an AWP and compare the throw; vary
   `phys_pushscale` (replicated) and see whether the throw scales.
3. **Death pose for players.** CS:S networks a death pose and frame and ships
   the crouch variants, so it very likely uses the 2.3 rule. Unknown: how it
   picks crouch variants, which facing it uses (feet yaw or eye yaw), and
   whether it applies to the local player. Check: record a demo of head-shot
   kills from the front and back; the head should snap back or forward in
   the first frames.
4. **bone_dt.** Template uses 0.05 s for player ragdolls; CS:S may use the
   generic 0.1 s. Halving bone_dt doubles bone velocities. Check: kill a bot
   running sideways and measure how far the body slides.
5. **Joint limit convention inside vphysics**: Euler order or twist/swing
   split of the three limits, whether the limits are measured from the bind
   relative orientation exactly (the code comment admits the axes are not
   transformed), and the meaning and unit of the friction torque.
   Implementation proposal: express child-relative-to-parent rotation
   (relative to bind) as X-then-Y-then-Z Euler angles in the child frame and
   clamp each axis. Check: `vcollide_wireframe 1` (cheat) on a body bent at
   the knee and elbow.
6. **`cl_ragdoll_physics_enable`**: CS:S-only. Default and the "off"
   behaviour (frozen death frame? no body?) unknown. Check: `cl_ragdoll_physics_enable 0`
   then kill a bot.
7. **Collision with server props, players and brush entities.** The client
   physics world has none of them, so bodies should fall through a
   `prop_physics` barrel or a func_door. Check on `cs_office` (server
   props) and any map with func_door. Also whether CS:S game rules override
   the debris rules (`cl_ragdoll_collide 1` with two bodies on a pile).
8. **Water**: whether ragdoll bodies float or sink in client water volumes
   (buoyancy is inside vphysics and depends on the `flesh` density). Check
   on a map with deep water.
9. **Removal**: confirm bodies vanish instantly at round restart and not at
   respawn, and whether a disconnecting player's body is removed.
10. **Bone derivative helper**: whether the linear velocity is that of the
    bone origin (as assumed) and how the angular velocity is computed (world
    axis-angle over dt assumed).
11. **Physics position interpolation**: whether the client reads body
    positions at the last fixed step or interpolated to the frame time
    (affects the settle check and visual smoothness above 66 fps).
12. Damping law, sleep thresholds, inertia tensor and friction combination:
    shared with physics props (physics_props.md Open questions 1–5). Ragdolls
    are where the damping law matters most (rotdamping up to 7).
