# Source engine: physics brushes and physics helpers (func_physbox, phys_thruster, phys_keepupright, map prop_ragdoll)

Source basis: Valve's public Source SDK 2013 only (current GitHub release).
I read the server code for func_physbox and func_physbox_multiplayer, the
func_breakable base they extend, the physics-solid setup shared with props
(default body parameters, mass scale, override script, mass-centre
override), the map compiler's brush-model physics export (how a brush
entity's mass and surface property are computed and stored in the BSP),
the multiplayer push-away code, the constant-force controller behind
phys_thruster, phys_keepupright and its axis-alignment helper, and
prop_ragdoll (map-placed server ragdolls: creation, pose override, inputs,
fade, damage). The physics engine (vphysics: integration, inertia
computation, contact solving, sleeping, how a controller's acceleration is
applied) is a binary library, not in the SDK; rules that depend on it are
marked *inferred*. Ragdoll building from a model's `.phy` is already in
specs/cs_source/ragdolls.md and body parameters, damping, friction,
sleeping and player interaction in specs/cs_source/physics_props.md; this
spec references them instead of repeating. CS:S's game DLL is not public:
anything CS:S might override (pickup by players, push-away details) is an
open question.
Status: draft

## Summary

func_physbox turns a brush entity into a rigid body. Its mass was computed
by the map compiler from the brush volume and the density of its dominant
surface material (or from surface area × thickness for "shell" materials)
and is stored in the BSP; the map can scale or override it. It breaks like
a func_breakable, can start frozen until enough damage or impact force
frees it, and is pushed by bullets and explosions. func_physbox_multiplayer
is the same body in CS:S's soft push-away group (players walk through it
and shove it). phys_thruster applies a constant force and/or torque to a
named body (the engine of mg_ map vehicles, often driven by game_ui);
phys_keepupright applies a torque that turns a body's up axis toward a
goal axis at a capped rate. prop_ragdoll is a server-simulated ragdoll
placed in Hammer, posed by Hammer's saved bone angles.

## Units and conventions

Source units (inches), Z up; mass in kg; force in kg·in/s²; velocity in
in/s; angular velocity in degrees/s about body-local axes (vphysics
convention). Density in kg/m³ (surface properties data), 1 in³ =
1.6387064 × 10⁻⁵ m³. dt = 0.015 s server tick; physics steps once per tick
(physics_props.md). Spawnflag values as Hammer shows them.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| m3_per_in3 | 0.0254³ = 1.6387064e-5 | m³/in³ | compile-time mass conversion |
| max_brush_mass | 50000 | kg | compiler clamps a brush entity's mass to this |
| brush_shrink | 0.5 | in | collision convexes are built from brushes shrunk by this (volume measured after shrinking, *inferred*) |
| default_inertia | 1.0 | × | inertia multiplier |
| default_damping | 0.1 | — | linear damping |
| default_rotdamping | 0.1 | — | angular damping |
| thin_rod_ratio | 9 | — | longest side > 9 × the length of the other two → angular damping forced to ≥ 1 |
| pickup_limits | 35 kg, 128 in | — | base "can pick up" test for the +use pickup cap |
| keepupright_default_limit | 15 | deg/s | default "angularlimit" |
| ragdoll_max_bodies | 24 | — | ragdoll parts |
| ragdoll_fade_default | 1 | s | FadeAndRemove with 0 |

## Behavior

### 1. Brush-entity mass (stored in the BSP)

For every brush model in the map, the compiler writes a physics text block
next to its collision data with "mass", "surfaceprop" and "volume". The
game reads them at spawn; **mashup should read these values from the
BSP's physics collision lump** rather than recompute them. For reference,
the compiler's rule:
1. Surface property per drawn face = its material's `$surfaceprop`
   (unknown → "default"). Faces are the model's visible faces after CSG
   (faces buried between the entity's own brushes are gone).
2. Sum face areas per surface property; start the table with a
   "default"-like entry of area 1 (an artefact; it only wins when the
   model has no faces). A model with no faces at all (all nodraw) uses
   the first brush side's surface property with area 2.
3. Dominant property = the one with the largest area. A = total area of
   all entries (including the extra 1).
4. With that property's density ρ (kg/m³) and thickness t (in, 0 if
   absent):

```math
m = \begin{cases} A \cdot t \cdot \rho \cdot 1.6387064\times10^{-5} & t \ne 0 \text{ (hollow shell)} \\ V \cdot \rho \cdot 1.6387064\times10^{-5} & t = 0 \text{ (solid)} \end{cases}
```

   with V the summed volume of the model's collision convexes, then
   m = min(m, 50000).
5. "surfaceprop" = the dominant property's name (friction, elasticity and
   impact sounds come from it, physics_props.md 3.4).

### 2. func_physbox

Keyvalues (in addition to func_breakable's, specs/source/breakables.md:
"health", "material", "propdata", "explodemagnitude", "minhealthdmg",
"physdamagescale", "nodamageforces", "damagefilter", ...): "massScale",
"Damagetype" (0 blunt, 1 sharp), "overridescript", "damagetoenablemotion",
"forcetoenablemotion", "preferredcarryangles", "notsolid" (0/1).
Spawnflags: func_breakable's (1 "Only Break on Trigger", 2 "Break on
Touch", 4 "Break on Pressure", 512 "Break immediately on Physics", 1024
"Don't take physics damage", 2048 "Don't allow bullet penetration") plus
4096 "Start Asleep", 8192 "Ignore +USE for Pickup", 16384 "Debris", 32768
"Motion Disabled", 65536 "Use Preferred Carry Angles", 131072 "Enable
motion on Physcannon grab", 262144 "Not affected by rotor wash", 524288
"Generate output on +USE", 1048576 "Physgun can ALWAYS pick up", 2097152
"Physgun is NOT allowed to pick this up", 4194304 "Physgun is NOT allowed
to punt this object", 8388608 "Prevent motion enable on player bump".

Inputs: **Wake**, **Sleep**, **EnableMotion**, **DisableMotion**,
**ForceDrop**, **DisableFloating**, and func_breakable's (**Break**,
**SetHealth**, **AddHealth**, **RemoveHealth**, **SetMass**, ...).
Outputs: **OnDamaged** (activator = attacker), **OnAwakened**,
**OnMotionEnabled**, **OnPlayerUse**, **OnPhysGunPickup**,
**OnPhysGunPunt**, **OnPhysGunOnlyPickup**, **OnPhysGunDrop**, and
func_breakable's **OnBreak**, **OnHealthChanged**.

**2.1 Spawn**, in order:
1. Damage modifiers and prop data as func_breakable.
2. Damage mode: flag 1 or health 0 → **events only** (it reports damage
   and is pushed, but never loses health; physics impacts never damage
   it). Else damageable. Max health = health (≥ 1).
3. Collision group: debris with flag 16384, else normal (solid to
   players, bullets, props: physics_props.md 4.1 rules for a normal
   prop). "notsolid" 1 makes it non-solid.
4. Body: start from the defaults (Constants), read the BSP solid block
   (mass, surfaceprop, volume); mass × massScale if massScale > 0; apply a
   mass-centre override if an info_mass_center targets it; apply
   "overridescript" (comma-separated key,value pairs, same as props:
   physics_props.md 3.1 step 6); if angular damping < 1 and the model's
   bounding box is a thin rod (longest side > 9 × length of the vector of
   the other two sides), angular damping := 1. Created at the entity's
   origin and angles; inertia from the collision shape (vphysics,
   *inferred* uniform density).
5. Damagetype 1: impacts from this body do slicing damage.
6. Awake unless flag 4096. **Motion disabled** (pinned in place) if flag
   32768, or damagetoenablemotion > 0, or forcetoenablemotion > 0.
7. Touch: the breakable touch rules (flags 2/4), unless flag 1.

**2.2 Damage** (bullets, explosions, triggers):
1. func_breakable damage rules (modifiers, filter, minhealthdmg, health,
   break at ≤ 0). The body is pushed by the damage force
   (physics_props.md 5.1) **even in events-only mode**, unless
   "nodamageforces" is set. A motion-disabled body does not move.
2. If the damage has an inflictor: fire **OnDamaged**.
3. If it broke (health ≤ 0): stop.
4. forcetoenablemotion > 0 and |damage force| ≥ it: enable motion.
5. damagetoenablemotion > 0 and **current health** < it: enable motion
   and apply this hit's force. (Compares remaining health, not damage
   taken: an events-only physbox has health 0 so its **first** damage of
   any size frees it; a physbox with health 100 and threshold 30 frees
   only once its health falls below 30. Quirk, keep.)

**2.3 Impacts**: on each physics collision, if forcetoenablemotion > 0 and
the other body is not a player with flag 8388608: if collision speed ×
other body's mass ≥ forcetoenablemotion, enable motion. Impact damage as
func_breakable/physics_props.

**2.4 Enable motion** (input, damage or impact): motion on, wake, both
thresholds set to 0 (it never re-freezes by itself), fire
**OnMotionEnabled**. DisableMotion pins it again (no output).

**2.5 Awakened**: with flag 4096, the first time the body is seen awake
(checked after each physics update) fire **OnAwakened** and fire the
"target" keyvalue's entities with a toggle use, then clear flag 4096.

**2.6 +use**: the physbox offers +use if flag 524288, or if flag 8192 is
clear and the base pickup test passes (mass ≤ 35 kg and size ≤ 128).
On a player's +use: fire **OnPlayerUse** (flag 524288); unless flag 8192,
ask the player to pick it up, which the base player does not implement
(HL2 does; CS:S: Open questions). Physgun outputs never fire in CS:S (no
physgun).

**2.7 Sleep/Wake/ForceDrop/DisableFloating**: wake or sleep the body;
ForceDrop makes a carrying player drop it; DisableFloating turns off
buoyancy in water.

### 3. func_physbox_multiplayer

A func_physbox that, at activation, moves to the **push-away** collision
group and records its mass for the clients. Its multiplayer physics mode is
never set from keyvalues, so it is 0 ("auto"), and every push-away rule
that asks for "solid mode" treats it as **not solid**:
- players do not collide with it in their movement (they walk through
  it) and it is shoved away by players moving into it (the push-away force
  rules of physics_props.md 4.2, as for mode 2 props);
- it does **not** push the player back, and players walking into it do
  not break it (the "break by touching" helper requires solid mode);
- bullets, explosions, props and the world still collide with it.

### 4. phys_thruster

Keyvalues: "attach1" (name of the body), "force" (kg·in/s²), "forcetime"
(s, 0 = until deactivated), "angles" (thrust direction = forward(angles)).
Spawnflags: 1 "Start On", 2 "Apply Force", 4 "Apply Torque", 8 "Orient
Locally", 16 "Ignore Mass", 32 "Ignore Pos". Inputs **Activate**,
**Deactivate**, **scale** (float). No outputs.

- **Activation** (map load or template spawn): attached entity = first
  entity named attach1 (resolved once). Thruster offset r = the thruster's
  position in the body's local frame (0 with flag 32). With flag 8, the
  thruster's angles are re-expressed relative to the body (thrust
  direction becomes a body-local axis). Flag 1: turn on.
- **Turn on** (Activate, or scale while off; ignored if already on): if
  the body has no physics object: nothing. Compute once:
  - F = force × forward(thruster angles); with flag 16: F × body mass
    (so the acceleration equals "force" regardless of mass).
  - Linear acceleration a = F/m; angular acceleration α = I⁻¹(r × F)
    (*inferred*: the physics engine's "velocity change from an impulse at
    a point", applied as a per-second rate).
  - Flag 8: both in the body's local frame (they turn with the body).
    Else in world space, **fixed at the moment of turning on** (a
    world-space thruster keeps pushing the same world direction however
    the body turns; Quirks).
  - Without flag 2 the linear part is zeroed; without flag 4 the angular
    part is zeroed (so a thruster with neither does nothing).
  - Attach a controller that adds a to the body's velocity and α to its
    angular velocity every physics step (a·dt per step), wake the body.
  - forcetime > 0: turn off automatically after forcetime seconds.
- **Turn off** (Deactivate): remove the controller, wake the body.
- **scale** s: if off, turn on first; then a and α = the values computed
  at turn-on × s (s = 0 keeps the thruster "on" with zero effect; s = −1
  reverses it). The base values are **not** recomputed, so mass changes
  and body rotation since turn-on are ignored.

### 5. phys_keepupright

Keyvalues: "attach1", "angularlimit" (deg/s, default 15), "angles" (goal:
the entity's up vector at spawn). Spawnflag 1 "Start inactive". Inputs
**TurnOn**, **TurnOff**, **SetAngularLimit** (float). No outputs.

- Activation: find the physics body named attach1; none: the keepupright
  deletes itself. Goal axis g = up(angles of the keepupright) in world
  space, fixed at spawn. Test axis = the body's local +Z.
- Each physics step of length h while active, in the body's local frame:
  g_l = g expressed in body coordinates; axis k = normalize(+Z × g_l);
  angle θ = atan2(|+Z × g_l|, +Z·g_l) in degrees; ω = current angular
  velocity (local, deg/s).

```math
\omega_{des} = k\,\frac{\theta}{h} - k\,(\omega\cdot k), \qquad
\omega_{des} \leftarrow \omega_{des}\,\frac{\min(|\omega_{des}|,\ \text{limit})}{|\omega_{des}|}
```

  and the controller adds ω_des to the angular velocity this step
  (angular acceleration ω_des/h). Net effect: the rotation rate about the
  correcting axis is driven to θ/h (it would fix the tilt in one step) but
  each step's change is capped at "angularlimit" deg/s, and rotation about
  the correcting axis is damped; rotation about other axes (e.g. yaw for
  a car) is untouched.
- Inactive: no effect. SetAngularLimit changes the cap at once.

### 6. prop_ragdoll (map-placed)

Keyvalues: "model", "angleOverride" (pose string written by Hammer),
"StartDisabled", "fademindist"/"fademaxdist"/"fadescale", "skin",
"modelscale" (newer builds), "sequence" (pose source if no override).
Spawnflags: 4 "Debris", 4096 "Use LRU retirement", 8192 "Allow
Dissolve", 16384 "Motion Disabled", 32768 "Allow stretch", 65536 "Start
asleep". Inputs **EnableMotion**, **DisableMotion**, **Enable** (show),
**Disable** (hide), **FadeAndRemove** (float seconds), **StartRagdollBoogie**
(HL2 effect; ignore). Outputs: the base entity's (OnUser, ...).

1. Pose: the model's bones in its current sequence at the entity's origin
   and angles. Then the entity's own angles are reset to zero (the pose
   now lives in the bodies).
2. Bodies and joints from the model's `.phy` ragdoll description exactly
   as specs/cs_source/ragdolls.md (sections 1 and 2.4), with joint friction
   scale 1, stretch allowed only with flag 32768, no start velocity.
3. **angleOverride**: a comma-separated list of pairs "part index" and
   "pitch yaw roll" (world angles). For each pair: that part's rotation
   := those angles; its position := its parent part's transform applied
   to the part's attachment offset from the joint description (the root
   part goes to the entity origin). Parts are processed in list order, so
   a parent must come before its children (Hammer writes them in part
   order). Index ≥ part count: ignored (warning).
4. Activate the ragdoll (joints on), awake unless flag 65536.
5. Collision group: debris with flag 4 (collides with world and static
   things only), else normal (solid to bullets, props and players; how
   CS:S players interact with server ragdolls: Open questions).
6. Damage mode: events only. A bullet hit pushes the **hit part** (the
   damage force goes to the body of the hit bone, physics_props.md 5.1).
   It has no health and never breaks. Physics impacts compute damage
   (events only: ignored).
7. Flag 16384: all parts pinned. EnableMotion: unpin and wake all parts;
   DisableMotion: pin all.
8. StartDisabled / Disable: hidden but still simulated and solid; Enable:
   shown.
9. **FadeAndRemove** T (0 → 1): render alpha = int(255 × (1 − t/T)) each
   tick from t = 0, then the ragdoll is removed at t ≥ T. Ignored if
   already fading.
10. Networking: the positions and angles of up to 24 parts are sent to the
    clients each tick (the clients only draw them).

## Per-tick order

1. Event queue and thinks (Activate/Deactivate/scale, TurnOn/TurnOff,
   EnableMotion, FadeAndRemove ticks) — specs/source/entity_io.md.
2. Physics step (once per tick): controllers run first inside the step
   (thrusters add a·h and α·h; keepupright adds its correction), then
   gravity, integration, contacts; collision callbacks (impact damage,
   forcetoenablemotion) at contact time.
3. After the step: physbox "awakened" checks; positions copied to the
   entities; ragdoll parts networked.
4. Player movement (before or after in the frame) treats func_physbox as a
   solid moving obstacle (normal group) or ignores it and applies
   push-away (multiplayer variant), physics_props.md 4.

## Edge cases

- func_physbox with damagetoenablemotion and health 0: the first damage
  event of any size frees it (a bullet, a knife hit, a blast). A player
  landing on it is not damage to the physbox (only impact force, 2.3).
- Motion-disabled physbox still takes damage and breaks normally.
- phys_thruster whose attach1 is a func_physbox that starts motion
  disabled: the controller is attached but the body does not move until
  motion is enabled.
- phys_thruster attach1 not found at activation: inputs do nothing
  forever (no re-lookup).
- Two thrusters on one body add.
- phys_keepupright with the body upside down (θ = 180°): the rotation
  axis is undefined (cross product 0); it does nothing until the body
  tilts slightly (*inferred* from the math).
- prop_ragdoll whose model has no ragdoll `.phy`: no bodies (the entity
  has nothing to simulate; treat as a static prop, Open questions).
- A brush entity whose BSP physics block is missing (very old compiler):
  fall back to the compiler rule (1) with the default surface property.

## Quirks

- **damagetoenablemotion compares remaining health** (keep).
- **World-space thrusters do not turn with the body**: a "car" thruster
  without flag 8 pushes in a fixed world direction; mg_ map vehicles use
  flag 8 (orient locally) when they turn (keep).
- **scale reuses the turn-on force** (keep).
- **func_physbox_multiplayer is never solid to players** (mode stays
  auto): keep.
- **Brush mass includes an extra 1 in² of area** in the shell formula
  (negligible; reading the stored mass makes it moot).

## Test cases

| Setup | Input | After | Expected |
|---|---|---|---|
| 64×64×64 brush, surface property density 700, thickness 0, volume measured on the shrunk convex 63³ | compile rule | — | m = 250047 × 1.6387064e-5 × 700 = 2868.3 kg (with the unshrunk 64³: 3007.1 kg; Open questions) |
| same, thickness 0.5 | — | — | A = 6·4096 + 1 = 24577; m = 24577 × 0.5 × 700 × 1.6387064e-5 = 140.96 kg |
| 1024×1024×64 slab, density 2700, thickness 0 | — | — | ≈ 2.97 million kg → clamped to 50000 |
| func_physbox, BSP mass 120, massScale 0.5 | spawn | — | mass 60 |
| func_physbox, overridescript "mass,25" and massScale 2 | spawn | — | mass 25 |
| 256×8×8 rod | spawn | — | 256 > 9·√(8²+8²) = 101.8 → angular damping 1 |
| func_physbox health 0, damagetoenablemotion 1, pinned | one bullet | — | OnDamaged; motion enabled (OnMotionEnabled); bullet force applied |
| func_physbox health 100, damagetoenablemotion 30 | 50 damage | — | health 50, still pinned |
| same | further 25 damage | — | health 25 < 30 → enabled |
| func_physbox forcetoenablemotion 5000, 100 kg prop hits it at 60 in/s | impact | — | 6000 ≥ 5000 → enabled |
| same with player bump and flag 8388608 | player walks into it | — | stays pinned |
| func_physbox flag 4096 | woken by a prop | first update awake | OnAwakened once |
| func_physbox_multiplayer, 20 kg | player walks into it | — | player passes through; box shoved away; player not slowed |
| phys_thruster force 1000, flags 2, attach1 = 50 kg box, angles "0 0 0" | Activate | 1 s | box velocity ≈ +20 in/s per second along +X (a = 1000/50), ignoring friction/gravity |
| same flags 2+16 | Activate | 1 s | a = 1000 in/s² regardless of mass |
| flags 2 only, box turned 90° yaw after Activate | — | — | thrust still +X world |
| flags 2+8 | box turns 90° | — | thrust turns with the box |
| thruster on | scale 0.5 | — | a halved |
| thruster off | scale 2 | — | turns on with 2 × base |
| thruster forcetime 2 | Activate | 2 s | turned off automatically (2/0.015 = 133.3 → 133 ticks) |
| phys_keepupright limit 15, body tilted 30°, at rest, h = 0.015 | one step | — | desired rate 30/0.015 = 2000 → capped: angular velocity += 15 deg/s about the correcting axis |
| same, next step (θ ≈ 29.78°, ω_k = 15) | — | — | desired 1985 − 15 = 1970 → capped 15 → ω_k = 30 |
| keepupright TurnOff | — | — | no corrective torque |
| prop_ragdoll FadeAndRemove 2 | t = 1 s | — | alpha 127 |
| same | t ≥ 2 s | — | removed |
| prop_ragdoll FadeAndRemove 0 | — | — | 1 s fade |
| prop_ragdoll flag 16384 | shoot it | — | does not move; EnableMotion then shooting moves the hit part |

## Open questions

1. **Shrunk or unshrunk volume**: is the stored mass of a 64³ physbox
   based on 63³ or 64³ (or something else after vphysics' convex
   rebuild)? Read the "mass" value from a compiled BSP of a test map with
   one cube physbox of a known surface property (or from a community map
   whose physbox sizes are known) and compare.
2. **Inertia and damping laws** (vphysics): same open questions as
   physics_props.md 3.1/3.3; func_physbox uses the default damping 0.1
   rather than the 0 of most props. Probe: push a physbox along a
   near-frictionless surface (a "glass"-like surface property) and record
   its speed decay per tick with the server probe.
3. **Controller application** (thruster/keepupright): confirm a = F/m per
   second and that the angular part from an offset thruster equals
   I⁻¹(r × F) in deg/s². Probe: a thruster at the centre of a 50 kg box in
   the air with sv_gravity unchanged: measure velocity change over 1 s via
   the server probe.
4. **Player +use pickup in CS:S**: can a CS:S player pick up a light
   func_physbox with +use (base player: no)? Test on a listen server.
5. **func_physbox_multiplayer on the CS:S client**: does the client treat
   mode 0 as non-solid in its predicted movement (no jitter)? Walk into one
   on a test map.
6. **Server ragdolls and CS:S players**: do players collide with a
   map-placed prop_ragdoll (normal group) or pass through? Test on a map
   with one.
7. **prop_ragdoll on a model without a ragdoll**: what does the entity do
   (removed? static?) — try with a static-only model.
8. **Thin-rod check uses the collision AABB at spawn orientation in
   model space**: confirm for a rotated brush (brush models are in world
   space, so the AABB is the world box).
