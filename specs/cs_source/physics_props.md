# Counter-Strike: Source: physics props and the .phy collision format

Source basis: Valve's public Source SDK 2013. I read the shared and server
prop entities (static, dynamic, physics and the multiplayer physics prop), the
client-side physics prop, the shared physics setup helpers, the server and
client physics frame code, the player's physics shadow and its controller
interface, the shared "obstacle push-away" code, the damage-force helpers, the
generic radius damage code, the collision group rules, the public vphysics
interface headers (object parameters, performance limits, surface
parameters), the BSP tools' collision lump byte-swapping code (which documents
the per-solid header) and the static prop compile and lighting tools. The
physics engine itself (vphysics / IVP) is not in the SDK, and neither are CS:S's
own game DLL, the engine's static prop manager or its trace code. Byte layouts
past the per-solid header were confirmed against real CS:S files instead, by
reading the bytes and checking them against the model's `.mdl` and the `.phy`
text section (volumes and mass centres match to 1e-3 on 1317 of 1319 solids).
Where behaviour lives in code I could not read, it is marked *inferred* or
listed under Open questions.
Status: draft

## Summary

- A `.phy` file sits next to a `.mdl` and holds one or more rigid bodies
  ("solids"). Each solid is a set of convex pieces stored in Havok/IVP's
  "compact surface" encoding, in metres with IVP's axes (Y down), followed by
  one plain-text key-value section (mass, surfaceprop, damping, etc.) for the
  whole file. To get Source inches: `src = (x, z, -y) / 0.0254`.
- Physics props are rigid bodies simulated by vphysics at one fixed step per
  server tick (0.015 s at CS:S's 66 tick), gravity `(0, 0, -sv_gravity)` =
  `-800` in/s², linear speed capped at 2000 in/s and angular speed at
  3600 °/s.
- Mass comes from the `.phy` text section (times the entity's `massscale`).
  Friction and elasticity come from the surfaceprop in
  `scripts/surfaceproperties*.txt`.
- How players and props interact depends on the classname:
  - `prop_physics` is solid to the player. The player's 85 kg physics
    "shadow" pushes it.
  - `prop_physics_multiplayer` (CS:S's usual barrel or crate) does not collide
    with players at all. The server shoves it with a scripted horizontal
    impulse each tick the player overlaps it (up to 1000 kg·in/s). On the
    client, the player's own move command is nudged away from the heavier
    ("solid" mode) ones, so the player feels a soft push-back, not a wall.
- Bullets and explosions push props with an impulse
  `J = direction × force × phys_pushscale`, applied at the hit point.

## Units and conventions

- Game space: Hammer units (1 unit = 1 inch), Z up, right-handed. Angles in
  degrees.
- `.phy` vertex space (IVP space): metres, Y **down**. The mapping is a proper
  rotation (determinant +1) plus a scale:
  - IVP → Source: `src = (ivp.x, ivp.z, -ivp.y) / 0.0254`
  - Source → IVP: `ivp = 0.0254 · (src.x, -src.z, src.y)`
  Verified on `models/props_junk/plasticcrate01a`: the asymmetric `.mdl` hull
  bounds (Y −13.156..13.101, Z −7.745..7.674) match the `.phy` extents' signs
  only with this mapping.
- Mass in kg. Impulses ("forces" in damage and push code) in kg·in/s, so a
  linear impulse `J` changes velocity by `J / m` in in/s. Angular impulses are
  in kg·deg/s about the mass centre (this unit is stated in the public
  interface header).
- All binary integers and floats are little-endian. Floats are IEEE-754
  32-bit.
- Time: the server tick is `TICK_INTERVAL` = 1/66.67 s ≈ 0.015 s (CS:S default
  tick rate; it is `-tickrate` dependent). The physics step equals the tick
  interval on both server and client.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| inch_to_m | 0.0254 | m/in | `.phy` coordinate scale |
| phy_header_size | 16 | bytes | file header |
| vphy_tag | `56 50 48 59` ("VPHY") | bytes | modern per-solid header tag |
| vphy_version | 0x0100 | int16 | observed in every modern solid |
| ivps_tag | `49 56 50 53` ("IVPS") | bytes | last 4 bytes of the compact surface header |
| sv_gravity | 800 | in/s² | default (cvar, replicated). Physics gravity = (0,0,−sv_gravity) |
| max_linear_speed | 2000 | in/s | world-space clamp on every object |
| max_angular_speed | 3600 | °/s | world-space clamp |
| max_collisions_per_object_per_step | 10 | – | server sets this. An object is frozen for the rest of the step after this many |
| max_collision_checks_per_step | 250 | – | default |
| lookahead_objects_vs_world | 1.0 | s | collision prediction horizon |
| lookahead_objects_vs_objects | 0.5 | s | |
| friction_mass_min / max | 10 / 2500 | kg | masses are clamped into this range for friction solving only |
| mass_min / mass_max | 0.1 / 50000 | kg | valid mass range |
| default_object_mass | 1.0 | kg | if a solid has no `mass` key |
| default_inertia_scale | 1.0 | – | |
| default_damping | 0.1 | – | linear, if a solid has no `damping` key |
| default_rotdamping | 0.1 | – | angular, if no `rotdamping` key |
| default_rot_inertia_limit | 0.05 | – | see Open questions |
| default_drag_coefficient | 1.0 | – | |
| physics_frame_dt_clamp | 0.1 | s | longest physics advance per call |
| player_shadow_mass | 85 | kg | player's physics shadow |
| player_push_mass_limit | 350 | kg | heaviest object the shadow may push |
| player_push_speed_limit | 50 | in/s | see Open questions |
| player_shadow_surfaceprop | `player` | – | friction 0.5, elasticity 0.001, density 1000 |
| rideable_mass_ratio | 2 | – | ground object is "rideable" if its mass > 2 × 85 = 170 kg |
| shadow_max_dist_error | 2 | in | player/shadow position mismatch threshold |
| shadow_max_vel_error | 10 | in/s | player/shadow velocity mismatch threshold |
| sv_pushaway_force | 30000 | – | server push-away numerator |
| sv_pushaway_max_force | 1000 | kg·in/s | cap on that impulse |
| sv_pushaway_min_player_speed | 75 | in/s | below this 2D speed, "solid" mode props are not pushed |
| sv_pushaway_player_force | 200000 | – | push-back on the player (cheat cvar) |
| sv_pushaway_max_player_force | 10000 | – | cap (cheat cvar) |
| sv_pushaway_clientside | 0 | – | 0 off, 1 local player only, 2 all players: who pushes client-only props |
| sv_pushaway_clientside_size | 15 | in | client-only if bounds volume < 15³ = 3375 in³ |
| pushaway_query_expand | 3 | in | server push query box = player box grown by 3 on every side |
| pushback_max_gap | 5 | in | client push-back ignores props farther than this |
| pushback_mass_min / max | 10 / 30 | kg | mass factor clamp for client push-back |
| pushback_client_scale | 0.25 | – | see Quirks |
| auto_solid_min_mass | 8 | kg | lighter props become "non-solid" mode |
| phys_pushscale | 1 | – | multiplies every damage force (cvar, replicated) |
| phys_timescale | 1 | – | server physics time scale (cvar) |
| explosive_force_per_damage | 75 × 4 = 300 | kg·in/s per hp | generic blast impulse |
| explosive_force_cap | 75 × 400 = 30000 | kg·in/s | |
| explosive_force_jitter | 0.85 – 1.15 | – | uniform random multiplier |
| blast_mass_absorb_all | 350 | kg | blocking physics object absorbs mass/350 of blast damage |
| inertia_scale_floor | 0.5 | – | after the `inertiascale` keyvalue is applied |

## Behaviour

### 1. The .phy file

#### 1.1 Overall layout

```
offset 0   file header (16 bytes)
offset 16  solid section 0
           solid section 1
           ...
           solid section N-1
           text section (to end of file, NUL-terminated)
```

File header, four int32:

| Offset | Type | Meaning |
|---|---|---|
| 0 | int32 | header size in bytes. Always 16. The first solid starts at this offset. |
| 4 | int32 | id. Always 0 in shipped files. |
| 8 | int32 | solid count N (1 for props, one per bone for ragdolls; 16 for CS:S player models) |
| 12 | int32 | checksum. Equals the int32 at offset 8 of the matching `.mdl` (the model checksum). A mismatch means the `.phy` is stale. |

Each solid section:

| Offset (rel. to section) | Type | Meaning |
|---|---|---|
| 0 | int32 | `size`: number of bytes that follow this field in this section |
| 4 | `size` bytes | solid data (modern or legacy, below) |

The next section starts at `section + 4 + size`. After N sections the rest of
the file is the text section.

#### 1.2 Modern solid data ("VPHY"), used by all CS:S models

Detect it by bytes 4..7 of the section being `VPHY`.

| Offset (rel. to section) | Type | Meaning |
|---|---|---|
| 4 | char[4] | `VPHY` |
| 8 | int16 | version, 0x0100 |
| 10 | int16 | model type: 0 = polygon soup of convex pieces (the only one in shipped files). 1 = an alternative tree type not supported here. |
| 12 | int32 | surface size in bytes. Equals `size − 28`. |
| 16 | float[3] | drag axis areas (IVP axes; see Open questions) |
| 28 | int32 | axis map size. 0 in every shipped file. |
| 32 | … | compact surface (1.4), `surface size` bytes |

#### 1.3 Legacy solid data (no tag)

The compact surface (1.4) starts directly at section offset 4, and `size`
equals its byte size. Detect it by bytes 4..7 not being `VPHY`. Bytes 44..47
of the surface still read `IVPS`. None of the 1129 `.phy` files in
`cstrike/cstrike_pak_dir.vpk` use it. 3 of the 1952 in
`hl2/hl2_misc_dir.vpk` do (e.g. `models/props_combine/pod_extractor.phy`).
A loader should accept both.

#### 1.4 Compact surface (one per solid)

All offsets below are relative to the surface start S (section + 32 for
modern, section + 4 for legacy). Header, 48 bytes:

| Offset | Type | Meaning |
|---|---|---|
| 0 | float[3] | mass centre, IVP space (m). Equals the volume centroid of all convex pieces. |
| 12 | float[3] | rotational inertia per unit mass, principal, IVP axes (m²). See Open questions. |
| 24 | float | bounding radius about the mass centre (m) |
| 28 | uint32 | low 8 bits: an encoder tolerance value (ignore). High 24 bits: surface byte size (= header surface size) |
| 32 | int32 | offset from S to the root node of the piece tree |
| 36 | int32[2] | reserved, 0 |
| 44 | char[4] | `IVPS` |

After the header come the **piece records** (convex pieces, plus hulls of groups of
pieces), then one shared **point array**, then the **tree nodes**. Do not rely
on that order. Follow the offsets.

#### 1.5 Piece tree

Node, 28 bytes:

| Offset | Type | Meaning |
|---|---|---|
| 0 | int32 | offset from this node to its right child. **0 means leaf.** |
| 4 | int32 | offset from this node to a piece record. 0 means none. |
| 8 | float[3] | bounding sphere centre (IVP, m) |
| 20 | float | bounding sphere radius (m) |
| 24 | uint8[3] | coarse box sizes (ignore) |
| 27 | uint8 | padding |

- The left child of an interior node is the node immediately after it
  (`node + 28`).
- **Every leaf node owns exactly one convex piece** (its piece record).
- An interior node may own a piece record too. That piece record is the convex hull of all
  pieces below it, used only for broad-phase collision. Skip it when
  collecting collision pieces.
- Interior hull piece records appear only when a solid has more than one piece.
  Single-piece solids have a one-node tree (a leaf).

To list a solid's convex pieces, walk the tree from `S + root offset`. At
each node: if right offset = 0, take the piece record at `node + piece record offset`;
otherwise recurse into `node + 28` and `node + right offset`. Equivalently,
the pieces are exactly the piece records whose subtree bits (below) are 0.
Both rules agreed on all 1320 solids in the CS:S VPK.

#### 1.6 Piece record (one convex piece)

Header, 16 bytes, at piece record address L:

| Offset | Type | Meaning |
|---|---|---|
| 0 | int32 | offset from L to the point array (`P = L + this`) |
| 4 | int32 | leaf pieces: owning bone index + 1 (0 = not bone-assigned). Hull piece records: offset from L back to the owning tree node. |
| 8 | uint32 | bits 0–1: subtree flag (0 = real piece, 1 = hull of a subtree). Bits 2–3: always 1. Bits 4–7: 0. Bits 8–31: piece record size / 16 (informational: = 1 + triangles + points used). |
| 12 | int16 | triangle count T |
| 14 | int16 | reserved, 0 |

Then T triangles, 16 bytes each, at `L + 16 + 16·t`. Each triangle is four
uint32 words:

- Word 0 (triangle info): bits 0–11 triangle index within the piece record; bits
  12–23 an internal index (ignore); bits 24–30 material index (0 in
  every shipped file); bit 31 virtual flag (0 in every shipped file).
- Words 1, 2, 3 (edges 0, 1, 2), each:
  - bits 0–15: **start point index** (unsigned) into the point array P
  - bits 16–30: offset to the twin edge (the same edge seen from the
    neighbouring triangle), as a **signed 15-bit** integer in units of 4-byte
    words, counted from this edge's own word. For example, edge 0 of
    triangle 0 with offset +6 points at word 7, which is edge 2 of
    triangle 1.
  - bit 31: virtual flag (0)

Triangle vertices are the three edges' start points in order (edge 0, 1, 2).
**Winding**: counter-clockwise seen from outside, i.e. `(v1−v0)×(v2−v0)`
points out of the piece. This holds in both IVP and Source space, because the
axis mapping is a rotation. Every triangle of the crate was checked. The twin
offsets are only needed for adjacency. A loader can rebuild each piece as
"convex hull of its referenced points" or use the triangles directly.

Points, 16 bytes each, at `P + 16·i`: float x, y, z (IVP space, metres) and
one float that is 0 in every shipped file. A point index means the same
point for every piece record of the solid: all piece records of one solid resolved to the
same P. The array is shared, and the hull piece records reuse the pieces' points.

#### 1.7 Which space the vertices are in

- Solids of a single-solid model (props): **model space**. Converted with
  the mapping in Units and conventions they line up with the `.mdl` origin.
- Solids of multi-solid models (ragdolls, `$collisionjoints`): **bone-local
  space** of the bone named in that solid's `name` key. For example, the calf
  solid of `models/player/t_phoenix` extends 0..22 in along bone-local +X.
  The bone index in each leaf piece record (offset 4, minus 1) names the same bone.
- The stored hulls are a little smaller than the render mesh bounds. The crate
  is 0.29–0.38 in smaller per side than its `.mdl` hull (see Open questions).

#### 1.8 Text section

ASCII, from the end of the last solid to the end of the file, terminated by a
NUL byte. The grammar is KeyValues-like: an unquoted block name, `{`, lines of
`"key" "value"`, `}`. Blocks seen in shipped files: `solid`, `editparams`,
`ragdollconstraint`, `collisionrules`, `break`, `animatedfriction`.
Unknown blocks must be skipped.

`solid` block keys:

| Key | Meaning |
|---|---|
| `index` | which solid section (0-based) this block describes |
| `name` | bone name (ragdolls) or a mesh name (props) |
| `parent` | parent bone's solid name (ragdolls) |
| `mass` | kg. Present in every shipped solid. |
| `surfaceprop` | surface property name (section 3.4) |
| `damping` | linear damping coefficient. 0 in 1117 of 1320 CS:S solids; 0.05 and 0.01 otherwise. |
| `rotdamping` | angular damping coefficient. 0 mostly; ragdolls use 1–6. |
| `inertia` | multiplier on the inertia tensor. 1 for props, 10 typical for ragdoll bones. |
| `volume` | in³. Equals the sum of the convex pieces' volumes (checked on 1317/1319 solids). |
| `massbias` | ragdoll mass distribution weight (tool-time; informational) |
| `drag` | drag coefficient override (rare) |

Keys absent from a block keep the defaults in the Constants table (mass 1,
inertia 1, damping 0.1, rotdamping 0.1, drag 1). Note that props compiled
with damping 0 therefore get **0**, not 0.1.

`editparams` holds tool-only data (`rootname`, `totalmass`, `concave`).
`ragdollconstraint` holds per-joint `parent`, `child`, `xmin` / `xmax` /
`xfriction` (same for y, z) in degrees. `collisionrules` holds
`"collisionpair" "a,b"` entries (solids that may collide with each other) and
`selfcollisions`. `break` describes gibs. These are out of scope for props
but must parse.

Which solid a single rigid body uses: the **first** `solid` block in the text
gives both the parameters and the `index` of the collision solid. In shipped
props that is index 0.

### 2. Choosing a prop's collision

#### 2.1 Static props (`prop_static`)

Static props are compiled into the BSP's static-prop game lump. Each instance
stores a one-byte solid type copied from the entity's `solid` keyvalue:

| Value | Name (Hammer) | Collision |
|---|---|---|
| 0 | Not solid | none |
| 2 | Bounding box | a box from the `.mdl` hull min/max (see Open questions on orientation) |
| 6 | VPhysics (Hammer default) | the model's `.phy` solid 0, placed at the prop's origin and angles, as an immovable physics object and for traces |

- The compile tool refuses models not compiled as static props. It also
  refuses models whose `.mdl` key-values have a `prop_data` section, unless
  that section says `allowstatic 1`.
- If such a model has no `.phy`, the compile and lighting tools build one
  convex hull **per render mesh** from the mesh vertices. They use it only
  for leaf placement and light occlusion. What the game uses at runtime for a
  solid-6 static prop without a `.phy` is not in the SDK (see Open
  questions).

#### 2.2 `prop_dynamic` (and `prop_dynamic_override`)

- `solid` 0 (not solid): no collision. The game switches it to an oriented
  box internally, flagged not solid, so its bounds rotate with it.
- Otherwise, if the model has bone followers, collision is provided per bone.
  This happens when the model's key-values list them, or when the `.phy` has
  more than one solid. In that case each solid follows its animated bone.
  Out of scope here.
- Otherwise: an immovable (static, or "shadow" if parented) physics object
  from `.phy` solid 0 at the entity's origin and angles. With `solid` 2 it is
  a world-axis-aligned box of the model bounds instead.
- No `.phy`: no physics object is created. The entity keeps its declared
  solid type for traces (see Open questions).
- A `prop_dynamic` whose model has `prop_data` (without `allowstatic`) is
  deleted at spawn.

#### 2.3 `prop_physics`, `prop_physics_override`, `prop_physics_multiplayer`

- One dynamic rigid body from the first `solid` block (rule at the end of 1.8), using
  that solid's convex pieces. Multi-solid models still give one body here.
- No `.phy`:
  - `prop_physics` / `prop_physics_override`: body creation fails. The
    entity becomes not solid and does not move (a warning is printed). It
    stays visible.
  - `prop_physics_multiplayer` in auto mode: removed from the world at spawn,
    on the server and on the client.
- Classname rules applied at spawn, before physics:
  - `prop_physics` whose model has **no** `prop_data`: deleted.
    `prop_physics_override` and `prop_physics_multiplayer` are exempt.
  - A model whose `prop_data` is invalid: deleted.

### 3. Physics behaviour of physics props

#### 3.1 Body parameters and order of setup

At spawn, in this order:

1. Start from the defaults (Constants table).
2. Read the first `solid` block from the `.phy` text (mass, inertia, damping,
   rotdamping, drag, surfaceprop, volume).
3. If the entity keyvalue `massscale` > 0: `mass ← mass × massscale`.
4. If `inertiascale` > 0: `inertia ← max(inertia × inertiascale, 0.5)`.
5. Spawnflag 0x200000 ("no collisions") turns collisions off.
6. If `overridescript` is set, it is a comma-separated list
   `key,value,key,value,…` parsed as extra `solid` keys. It overrides
   everything above, e.g. `mass,100` replaces the scaled mass.
7. The body is created at the entity's origin and angles. It starts awake
   unless spawnflag 0x1 ("start asleep") is set.
8. Spawnflag 0x8 ("motion disabled"), `damagetoenablemotion` > 0 or
   `forcetoenablemotion` > 0 makes the body pinned (immovable) until enabled.
9. Spawnflag 0x4 ("debris") puts it in the debris collision group. Debris
   collides only with the world and static things; with spawnflag 0x1000 it
   also touches triggers.

Mass therefore comes only from the `.phy` (the model compiler's `$mass`, or
density × volume) and the map's `massscale` / `overridescript`.
`scripts/propdata.txt` and the model's `prop_data` set health, damage
modifiers, break pieces and the multiplayer physics mode, **not** mass.

The mass centre is the stored one (1.4). For inertia, see Open questions.

#### 3.2 Gravity

- Each environment's gravity is `(0, 0, −sv_gravity)` in/s², read **once**
  when the level loads (server and client both). sv_gravity defaults to 800.
  Changing sv_gravity mid-map changes player movement but not, in this code,
  physics props (see Quirks).
- Gravity is applied to every body, scaled by nothing else. There is no
  per-entity gravity scale for physics props.

#### 3.3 Damping and drag

- `damping` and `rotdamping` are per-body coefficients passed to vphysics.
  For shipped CS:S props both are usually 0, i.e. no damping. The damping law
  is inside vphysics (see Open questions).
- Air drag uses the drag coefficient (1.0), the solid's drag axis areas and a
  global air density (cheat command `air_density`). Formula: Open questions.
  For props at CS:S speeds it is small.

#### 3.4 Friction and elasticity (surface properties)

- Data files: `scripts/surfaceproperties_manifest.txt` lists the files in
  load order. CS:S's manifest (in `cstrike_pak`) lists
  `scripts/surfaceproperties.txt` (shipped in `hl2/scripts/`) then
  `scripts/surfaceproperties_cs.txt`. A later definition of the same name
  overrides earlier keys.
- Format: `"name" { "key" "value" … }`. `"base" "other"` copies another
  entry first and must be the first key. Every entry not overriding a key
  inherits it from `"default"`.
- Physics fields:

  | Field | Meaning |
  |---|---|
  | `friction` | coefficient of friction (0.01 slick … 1.0 rough) |
  | `elasticity` | collision elasticity (0.01 soft … 1.0 hard) |
  | `density` | kg/m³. Tool-time mass computation only at runtime. |
  | `thickness` | in. If present, mass computed as surface area × thickness (tool time). |
  | `dampening` | extra drag while in contact with this surface |

- `"default"`: density 2000, elasticity 0.25, friction 0.8, dampening 0.
  Examples: `Plastic_Box` (and `plastic`, which bases on it): density 500,
  elasticity 0.01, friction 0.8, thickness 0.25. `metal`: elasticity 0.25,
  thickness 0.1. `player`: density 1000, friction 0.5, elasticity 0.001.
  `surfaceproperties_cs.txt` sets `gravel` and `snow` friction 0.8.
- The `surfaceprop` key of the `.phy` solid picks the entry for the whole
  body. Per-triangle materials are unused (material index 0 everywhere). An
  unknown name falls back to `default`.
- How the two contacting materials' friction and elasticity combine is inside
  vphysics (see Open questions).

#### 3.5 Sleeping

- Bodies go to sleep when they have been nearly still for a while. A sleeping
  body is not simulated and does not move until something wakes it: a
  collision with an awake body, an impulse, or the `Wake` input. The `Sleep`
  input forces sleep.
- A prop spawned with "start asleep" (0x1) fires its `OnAwakened` output the
  first time it is seen awake, then loses that flag.
- The sleep velocity and time thresholds are inside vphysics (see Open
  questions).

#### 3.6 Limits

- Each tick, after integration, linear velocity is clamped to 2000 in/s and
  angular velocity to 3600 °/s (world space, magnitude).
- An object involved in more than 10 collisions in one step is frozen for the
  remainder of that step. This shows up as hitching, not a crash.

### 4. Players and physics props

The interaction differs by collision group, which follows the classname and
mode.

#### 4.1 `prop_physics` (normal collision group) — solid, shadow-pushed

1. **Movement collision.** Player movement traces treat the prop's convex
   pieces as solid, like a wall: the player is blocked and their velocity is
   clipped along the contact plane. The player can stand on the prop.
2. **Touch flag.** When the player's movement touches a movable,
   non-trigger physics prop (other than the one they stand on), the player is
   marked as having touched physics this tick.
3. **Physics shadow.** Every player has an invisible physics body, the
   shadow: a box the size of the player's hull (standing or crouched), 85 kg,
   inertia scale 10²⁴ (cannot rotate), no drag, surfaceprop `player`. The
   shadow is driven by a player controller. Each processed user command gives
   it:
   - a target position, and
   - a target velocity (the player's wish velocity from movement), and
   - a time to arrival of `TICK_INTERVAL × (k+1)` for the k-th command
     processed this tick.

   The shadow pushes objects of up to 350 kg (push speed limit 50 in/s, see
   Open questions). The push is a contact between the shadow and the prop
   inside the physics step, so heavier props resist more.
4. **Target when pushing.** If the player touched physics this tick, is on
   the ground, did not step up, and is not standing on a physics object:
   `target = 0.5 · origin + 0.5 · (origin_prev + dt · wishvel)`, where
   `dt = frametime`, replaced by 0.1 if ≤ 0 or > 0.1. The shadow is aimed
   half a step ahead of where movement stopped the player, so it presses into
   the prop. Otherwise `target = origin`.
5. **Velocity target when not touching.** If the player touched no physics
   and stands on no physics, the controller's velocity target is set to
   `(maxspeed, maxspeed, maxspeed)` (the player's max speed, else
   `sv_maxspeed`).
6. **Step-ups.** If movement stepped up by more than 0.1 in this tick, the
   shadow is moved up too: teleported if the step was over 4 in, otherwise
   raised by a trace-limited amount.
7. **Reconciliation, after the physics step.** Compare the player origin
   with the shadow position (and their velocities). If not touching physics
   and airborne, the vertical difference counts half. Thresholds:
   - 2 in and 10 in/s, or
   - 1 in and 5 in/s (i.e. ×0.25 on the squared thresholds) if standing on a
     "rideable" body (mass > 170 kg).

   If either threshold is exceeded, or the player stands on physics without
   having touched it, **and** the player touched physics or stands on it:
   - If the velocity error is over threshold, take the shadow's velocity,
     remove its component along the player's current velocity direction (that
     component clamped to ±|v_player|), and add the rest to the player's
     velocity. This is skipped when standing on a rideable body.
   - Then move the player to the shadow position if that spot is not inside
     solid.

   If no threshold is exceeded but the player touched physics and is now
   inside solid, the player is moved to the shadow position, falling back to
   the previous origin if still stuck.

Net feel: walking into a light `prop_physics` slows the player to roughly
the speed at which the prop is shoved away. Props over 350 kg act as walls.
Bodies over 170 kg can be ridden: the player moves with them.

#### 4.2 `prop_physics_multiplayer` — CS:S's soft push-away

The multiplayer variant has a mode (keyvalue `physicsmode`, or `physicsmode`
inside the model's `prop_data`; **the model's prop_data wins** over the map
because it is applied later):

| Mode | Name | Meaning |
|---|---|---|
| 0 | auto | choose at spawn (below) |
| 1 | solid | server-simulated; pushes the player back (4.2.3) |
| 2 | non-solid | server-simulated; no push-back on the player |
| 3 | client-side | not created on the server; each client simulates its own copy |

Auto mode at spawn, using the model bounds (the `.mdl` hull) size
`(sx, sy, sz)` and the body mass m:

```
if sx·sy·sz < sv_pushaway_clientside_size³   (default 15³ = 3375 in³) → 3 client-side
else if m < 8 kg                                                      → 2 non-solid
else                                                                  → 1 solid
```

Then spawnflag 0x2000 ("force server side") forces mode 2.

- A client-side prop is deleted on the server. In map edit mode it is
  forced to 2 instead.
- On each client, every `prop_physics_multiplayer` in the BSP entity lump is
  also created locally. It is kept only if its final mode is 3 (and not
  forced server-side), so exactly one side owns each prop.

**4.2.1 Collision group.** The server prop (modes 1, 2) and the client-side
prop (mode 3) both go into the **push-away group**, unless spawned as debris
(spawnflag 0x4 → debris group).

Push-away group rules:
- It does **not** collide with players (neither the movement traces nor the
  player shadow).
- It does collide with the world, static props, normal props, other
  push-away props and debris.
- Bullets and other traces still hit it.

So players walk through these props, and both of the following scripted
forces replace collision.

**4.2.2 Server push (prop moves away from player).** Each tick, for each live
player (*inferred*: CS:S's player code calls this; the SDK only defines it
and makes these cvars non-dev-only in the CS build):
1. Query box = player's hull box grown by 3 in on all sides, at the player's
   origin. Collect entities in the push-away group, plus rotating doors that
   are currently opening or closing.
2. For each: if its mode is "solid" (1) and the player's horizontal speed
   `|v_xy| < sv_pushaway_min_player_speed` (75), skip it. This lets a player
   crouch behind a barrel without knocking it over.
3. `d = prop_center − player_center` (world-space bounds centres), then
   `d_z = 0`. `dist = max(|d|, 1)`. `n = d / |d|` (zero vector if |d| = 0).
4. `J = min(sv_pushaway_force / dist, sv_pushaway_max_force)` =
   `min(30000/dist, 1000)` kg·in/s.
5. Apply the impulse `n · J` to the prop **at the player's centre point**.
   That point is generally not the prop's mass centre, so this also spins
   the prop: torque impulse `(p_player − c_mass) × (n·J)`.

The client runs the same routine for client-side props only when
`sv_pushaway_clientside` is 1 (local player only) or 2 (all players). The
query covers client-side props, and the same speed rule applies.

**4.2.3 Client push-back (player moves away from "solid" props).** While
building each user command on the client (*inferred* call site), if the player
is alive, not observing and not in noclip/none move types:
1. Collect push-away entities overlapping the player's hull box (no
   expansion).
2. Skip any prop whose mode is not "solid" (1). Entities without a mode
   (rotating doors) use mass 30.
3. `w = clamp(m, 10, 30) / 30`.
4. `a` = point on the prop's bounds nearest the player centre.
   `b` = point on the player's box nearest `a`. `v = b − a`. `dist = |v|`.
   - If `dist > 5` and `a` is not inside the player's box, skip.
   - If `v` = 0: `v = player_center − a`. If still 0:
     `v = player_center − prop_center`. Recompute `dist`.
   - `n = v / |v|`. `dist = max(dist, 1)`.
5. `F = min(sv_pushaway_player_force / dist · w, sv_pushaway_max_player_force)`
   = `min(200000 · w / dist, 10000)`, then `F ← 0.25 · F` (see Quirks).
6. With the view's forward and right vectors (from the command's view angles,
   pitch included): `forwardmove += F · (n · forward)` and
   `sidemove += F · (n · right)`.

The command then goes through normal movement, which clamps the wish speed to
the player's max speed. In practice any overlap with a solid-mode prop turns
into strong backing-off input.

**4.2.4 Turbo physics (`sv_turbophysics`, replicated, default 0 in the
SDK).**
- When 1, players get **no physics shadow** at all.
- Server multiplayer props re-pick their group after every physics update:
  - awake → push-away group
  - asleep and mode 2 → debris group
  - asleep otherwise → normal group, i.e. a solid wall to players
  - props spawned as debris keep the debris group
- Holding +use pushes the physics object under the crosshair (trace 96 in
  from the eye, physics objects only, not the C4) with the same impulse as
  4.2.2 (horizontal, `min(30000/dist, 1000)`, applied at the player centre).
  This happens each command while +use is held.

#### 4.3 Is the player slowed?

- `prop_physics`: yes. Movement is blocked by the prop, and the player only
  advances as the shadow shoves the prop (4.1).
- `prop_physics_multiplayer`: movement is never blocked by the prop. With
  "solid" mode props, the client push-back input pushes the player away. With
  "non-solid" mode props, nothing acts on the player at all.

### 5. Damage impulses

#### 5.1 Applying a damage force to a prop

When a physics prop takes damage of a type that carries force (anything
except fall, burn, plasma, drown, time-based, crush, physgun, and the
"prevent physics force" type; generic damage carries none), its body gets:

```
J (kg·in/s, world vector) applied at world point p_hit:
  Δv = J / m
  Δω = I_world⁻¹ · ((p_hit − c_mass) × J)      (converted to °/s)
```

- This happens even for props with no health. Those take "events only"
  damage: the force applies and health is ignored.
- It does **not** happen if:
  - the damage is below the prop's minimum-damage threshold (`prop_data`
    / keyvalue `minhealthdmg`, default 0). The prop then ignores the hit
    entirely, force included.
  - the prop's damage filter rejects the hit.
  - the body is pinned (motion disabled).
- Bodies held by a player's physgun are not relevant to CS:S.
- `forcetoenablemotion`: if `|J| ≥` that value, the pinned body is enabled
  first.
- `damagetoenablemotion`: if health drops below it, the body is enabled and
  the force applied.

#### 5.2 Bullets

`J = normalize(bullet_dir) · ammo_impulse · phys_pushscale · per_shot_scale`,
applied at the trace end point. `ammo_impulse` is a per-ammo-type constant:

- In the shared ammo table format it is given directly in kg·in/s, or derived
  as `ft_per_s × 12 × grains × 0.002285/16 lb × 0.45359 kg/lb × exaggeration`.
- CS:S's own table is not in the SDK. The SDK's CS-derived template mod keeps
  one commented CS line giving `.50 AE` an impulse of **2400**. Other CS
  calibres: see Open questions.
- `per_shot_scale` defaults to 1.

#### 5.3 Explosions (generic radius damage; CS:S's HE grenade code is not public)

- `damage_at_target = D − dist · falloff` (≤ 0 → nothing).
- If a physics object blocks the line, it absorbs `m_block / 350` of the
  damage (all if ≥ 350 kg).
- A non-physics, non-world blocker absorbs 25%.
- `J = min(D_base · 300, 30000) · U(0.85, 1.15) · phys_pushscale`, along
  `normalize(target_center − blast_origin)`, applied at the blast origin.
  The base damage is the undecayed damage.
- If the caller supplied a force, it is instead decayed:
  `|J| ← |J_supplied| · falloff`.

#### 5.4 Prop vs prop or world impacts

Collisions can damage breakable props and the player (energy-based tables).
That is out of scope for this spec, except that a prop's impact damage and
its "damage force" (`post-collision velocity × mass`) do not feed back into
its own motion.

### 6. Server-side multiplayer specifics (networking)

- Server props (modes 1, 2) are simulated only on the server. Clients receive
  origin and angles (plus the awake flag, the mode and the mass for
  multiplayer props) and interpolate. There is no client prediction of
  server props.
- Client-side props (mode 3) exist only on clients. Each client's copy moves
  independently, so their positions can differ between players. Client
  physics runs at the same step as the server, but is advanced by real frame
  time (scaled by the client cvar `cl_phys_timescale`) instead of by ticks.

## Per-tick order

Server, one tick (dt = TICK_INTERVAL):

1. **Players**, for each player, for each queued user command k (0-based):
   1. Movement runs. Collisions with `prop_physics` happen here; touched
      props set the touch flag.
   2. Post-think: compute the shadow target position and velocity (4.1 step
      4/5) and handle step-up (step 6).
   3. *Inferred CS:S*: the server push-away of multiplayer props (4.2.2)
      applies impulses, which go straight into those bodies' velocities.
   4. Send the target to the shadow controller with arrival time
      `dt·(k+1)`.
2. Other entities think (damage from bullets fired in step 1 has already
   applied impulses during step 1).
3. **Physics frame**: advance = dt (0 if the game is paused). It is replaced
   by 0 if > 1 s or < 0, clamped to 0.1 s, multiplied by `phys_timescale`,
   and clamped to 0.1 s again. The environment then advances by this time in
   fixed steps of `TICK_INTERVAL`, normally exactly one step. Within a step,
   in order (*inferred* from vphysics behaviour):
   1. Apply gravity and controllers (the player shadow's controller).
   2. Detect and resolve collisions (friction, elasticity).
   3. Integrate.
   4. Clamp velocities.
   5. Put still bodies to sleep.

   Collision and touch callbacks raised during the step are buffered and
   delivered after it.
4. For every **awake** body, copy its position and angles to the entity, and
   mark its bounds dirty.
5. For every entity with a shadow that is not asleep, reconcile (players:
   4.1 step 7).

Client: each rendered frame, the client physics environment advances by
`frametime × cl_phys_timescale`, in fixed steps of TICK_INTERVAL (remainder
carried, *inferred*). Then awake client-side bodies are copied to their
entities. Client push-back (4.2.3) is applied while building each command.

## Edge cases

- **Multi-solid model on `prop_physics`**: only the first `solid` block's
  index is used, giving one rigid body. The other solids are ignored.
- **`.phy` without a `mass` key**: mass 1 kg (no shipped file lacks it).
- **Mass outside 0.1–50000 kg**: invalid for vphysics. Clamp.
- **Zero-length push vector** (player centre exactly over prop centre
  horizontally, 4.2.2): the direction is the zero vector, so no impulse,
  even though `dist` is clamped to 1.
- **Player standing on a prop** is excluded from the touch flag. A prop over
  170 kg is ridden. A lighter prop under the player is pressed down by the
  85 kg shadow.
- **Very small props** under 3375 in³ in auto mode become client-only. Bullets
  and grenades on the server never touch them. Clients each see their own.
- **Pinned props** (motion disabled) ignore impulses until enabled.
  `forcetoenablemotion` compares impulse magnitude, or
  `collision speed × other mass` for collisions. Unless spawnflag 0x400 is
  set, a player bumping the prop also counts.
- **sv_gravity changed mid-map**: physics bodies keep the gravity read at
  level load until the next map (see Quirks).
- **Paused game**: physics advance is 0.

## Quirks

- **Client push-back is always quartered.** The ×0.25 meant for rotating
  doors is applied unconditionally in the client build (the door test only
  exists in the server build). Effective client push-back is
  `min(200000·w/dist, 10000) × 0.25`, i.e. at most 2500. Keep this. It is
  what players feel.
- **"Inverse square" push is actually inverse distance.** The cvar
  description says it falls off with the inverse square, but the formula is
  `1/dist`. Keep `1/dist`.
- **Server push is applied at the player's centre**, so props get spun
  (barrels roll and topple), not just slid. Keep.
- **Push direction ignores Z on the server, but the client push-back uses the
  full 3D view forward vector.** Looking up or down weakens the push-back.
- **Physics gravity is not updated live from sv_gravity.** Keep for fidelity.
  Optional for us.
- **Props compiled with `damping 0` have no damping** (the 0.1 default only
  applies when the key is missing), so shipped props slide and roll on
  friction alone.
- **The model's prop_data `physicsmode` overrides the map's.**

## Test cases

`.phy` parsing, using `models/props_junk/plasticcrate01a.phy` from
`hl2/hl2_misc_dir.vpk` (2555 bytes; read at test time from the user's
install, never committed):

| Setup | Input | After | Expected |
|---|---|---|---|
| crate .phy | read file header | – | header size 16, id 0, solid count 1, checksum −1202982611 (= `.mdl` int32 at offset 8) |
| crate .phy | solid 0 section | – | size 2280, tag `VPHY`, version 0x0100, model type 0, surface size 2252, drag areas (0.98077, 1.0, 0.98077), axis map 0 |
| crate .phy | compact surface at file offset 48 | – | tag `IVPS` at offset 92, byte size 2252, tree root at file offset 2048 |
| crate .phy | walk tree | – | 9 nodes, 5 leaf pieces at file offsets 96, 512, 720, 304, 928 (walk order); each has 12 triangles and 8 distinct points; root has a hull piece record at 1136 (16 triangles, subtree flag = 1) |
| crate .phy | first point of piece at 96 | convert to Source | IVP (−0.2212873, −0.1877670, −0.3075229) m → Source (−8.71210, −12.10720, 7.39240) in |
| crate .phy | all pieces | sum signed tetra volumes | 1144.14 in³ (text `volume` 1144.139648) |
| crate .phy | stored mass centre | convert | Source (−0.0013, −0.0317, −1.8863) in, equal to the volume centroid within 1e-3 in |
| crate .phy | every triangle | `(v1−v0)×(v2−v0) · (v0 − piece_centroid)` | > 0 for all 60 |
| crate .phy | Source-space AABB of all points | – | min (−8.832, −12.875, −7.463), max (8.795, 12.819, 7.392) |
| crate .phy | text section | – | starts at file offset 2300; solid: index 0, mass 10, surfaceprop plastic, damping 0, rotdamping 0, inertia 1, volume 1144.139648; editparams block; ends with NUL |
| crate .phy | triangle 0 of piece at 96 | decode | tri index 0, edges start at points 0, 1, 2; edge 0 twin offset +6 → word 7 = edge 2 of triangle 1, whose start point is 1 |
| `t_phoenix.phy` (cstrike VPK) | header | – | 16 solids. Pelvis solid's leaf piece record bone field = 1 (bone 0 + 1). Calf solid points extend 0..22.2 in along bone-local +X. |
| `pod_extractor.phy` (hl2 VPK) | solid 0 | – | no `VPHY` tag. `IVPS` at section offset 4+44. Section size = surface byte size = 2044. |

Behaviour:

| Setup | Input | After | Expected |
|---|---|---|---|
| prop_physics_multiplayer, crate model (hull 18.24 × 26.26 × 15.42 in, 10 kg), physicsmode 0 | spawn | – | volume ≈ 7385 in³ ≥ 3375 and m = 10 ≥ 8 → mode 1 (solid) |
| same model, 5 kg (massscale 0.5) | spawn | – | mode 2 (non-solid) |
| model with hull 14 × 14 × 14 in | spawn auto | – | 2744 < 3375 → mode 3; server removes it |
| server push: player centre (0,0,36), 2D speed 200, mode-1 prop 10 kg, centre (20,0,10) | 1 tick | impulse | d = (20,0,0), dist 20, J = min(1500,1000) = 1000 → Δv = (100, 0, 0) in/s; torque impulse = (−20,0,26) × (1000,0,0) = (0, 26000, 0) kg·in²/s (if the mass centre is at the bounds centre) |
| same, prop at (40,0,10) | 1 tick | – | J = 750 → Δv = 75 in/s along +X |
| same, prop at (20,0,10), player 2D speed 50 | 1 tick | – | no impulse (50 < 75, solid mode) |
| same, mode-2 prop, player speed 0 | 1 tick | – | impulse applied (speed rule only for mode 1): Δv = 100 in/s |
| server push, prop at (0.5, 0, 10) | 1 tick | – | dist = max(0.5,1) = 1 → J = min(30000, 1000) = 1000 along +X |
| client push-back: player box ±16×±16, 0..72 at origin (centre (0,0,36)), view yaw 0 pitch 0; mode-1 prop 20 kg whose box face is at x = 18 | build cmd | – | a = (18,0,36), b = (16,0,36), dist 2, w = 0.6667, F = min(66667, 10000) × 0.25 = 2500; forwardmove −= 2500, sidemove += 0 |
| same, prop 50 kg | build cmd | – | w = 1 → same 2500 (cap) |
| same, prop face at x = 22 (gap 6) | build cmd | – | skipped (6 > 5 and a not inside player box) |
| same, mode-2 prop | build cmd | – | no change |
| bullet hits 30 kg prop at its mass centre, ammo impulse 2400, phys_pushscale 1 | hit | – | Δv = 80 in/s along the bullet direction, Δω = 0 |
| same, phys_pushscale 2 | hit | – | Δv = 160 in/s |
| same, prop `minhealthdmg` 50, bullet damage 30 | hit | – | no impulse |
| explosion base damage 50 on a 10 kg prop, no supplied force | blast | – | \|J\| = 15000 × U(0.85,1.15) → Δv between 1275 and 1725 in/s |
| explosion base damage 200 | blast | – | \|J\| = 30000 (cap) × U(0.85,1.15) |
| any awake prop | free fall from rest | 1 tick (0.015 s) | v_z = −800 × 0.015 = −12 in/s (no damping) |
| prop falling for 3 s | – | – | \|v\| clamped at 2000 in/s (reached at 2.5 s with g = 800) |
| player (85 kg shadow) stands on a 200 kg prop | – | – | rideable (200 > 170); stands on 150 kg prop → not rideable |
| player walks into a 400 kg prop_physics | – | – | not pushed (> 350 kg push limit); behaves as a wall |

## Open questions

1. **Inertia tensor.** The stored per-unit-mass rotational inertia (crate:
   0.0564, 0.0618, 0.0330 m² on IVP axes) is 74–81% of the uniform-density
   solid inertia I computed from the same hulls (0.0694, 0.0838, 0.0445). So
   either vphysics uses a different approximation, or the field means
   something else. Proposal: compute inertia from hull geometry × `inertia`
   key. Check by measuring spin-up from an off-centre bullet in game.
2. **Rotational inertia limit 0.05** default: its exact meaning (a minimum
   ratio between principal moments?) is inside vphysics.
3. **Damping law.** How `damping` / `rotdamping` reduce velocity per step
   (e.g. `v ← v·(1 − c·dt)` versus `v ← v / (1 + c·dt)`) is not in the SDK.
   It mostly matters for ragdolls, since props ship with 0.
4. **Friction and elasticity combination** between two surfaces (product?
   min? average?) and the restitution model are inside vphysics. Measure a
   prop's bounce height and its slide on a ramp to fit.
5. **Sleep thresholds** (velocity and time before a body sleeps) are inside
   vphysics. Measure with `cl_showpos`-style observation or ent_text on a
   rolling barrel.
6. **Air drag formula and default air density.** `air_density` with no
   argument prints the current value in game (cheat).
7. **Player push limits.** Exact semantics of the shadow controller's push
   mass limit (350 kg) and push speed limit (50 in/s) are inside vphysics.
8. **Hull shrink.** Crate hulls are ~0.3 in smaller per side than the `.mdl`
   hull. A comment in the SDK says vphysics keeps a minimum separation of
   about 0.25 in between objects. It is unclear whether hulls are stored
   shrunk and inflated at collision time (so contacts happen at the original
   surface), or whether the render hull is simply larger than the
   collision mesh. Check by placing a prop on flat ground and measuring the
   gap between the visible model and the floor.
9. **Static props**:
   - whether the bounding-box solid type (2) is oriented with the prop's
     angles or world-aligned;
   - what solid type 6 does without a `.phy` (no collision, bbox fallback,
     or the per-mesh hulls the tools build).

   The engine's static prop manager is not public. Test in game by walking
   into rotated solid-2 props.
10. **SOLID_VPHYSICS entity without a `.phy`** (e.g. `prop_dynamic`): engine
    trace behaviour (no hit vs. bounding box fallback) is not in the SDK.
11. **CS:S ammo impulses.** Only `.50 AE = 2400` is visible in public code.
    Whether CS:S's own bullet code uses this helper at all, and the other
    calibres' values, need checking: shoot a known-mass prop and measure
    Δv with `ent_text` or a demo. CS:S's HE grenade radius-damage code may
    also differ from the generic one in 5.3.
12. **Call sites in CS:S.** The server push (4.2.2) and the client push-back
    (4.2.3) are defined in shared code. CS:S's player code (not public) is
    presumed to call them every tick or command. The cvars become
    player-visible only in the CS build, and the +use push filter excludes
    the C4. Confirm by walking into a barrel at speed < 75 vs > 75 in game.
13. **CS:S default of `sv_turbophysics`.** It is 0 in the SDK. Type
    `sv_turbophysics` in the CS:S console to confirm, since it changes 4.1
    and 4.2 a lot.
14. **Drag axis areas** in the VPHY header (crate: 0.981, 1.0, 0.981) do not
    match the crate's cross-sections in m² or in². Their units and
    normalisation are unknown. They only matter for drag.
15. **Model type 1** (alternative tree) solids are not supported and not
    shipped. Skip them if encountered.
