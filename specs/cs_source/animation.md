# Counter-Strike: Source: player model animation

Source basis: Valve's public Source SDK 2013 only. I read the studio model
header (bones, animation descriptions, per-bone animation records, sequences,
pose parameters, include models, autolayers), the compressed vector/quaternion
types and half-float conversion, the shared bone-setup code (decoding an
animation at a cycle, blending the animations of a sequence by pose
parameters, accumulating sequences and layers, pose-parameter encoding,
cycle rate), the include-model merging code, the client animating-entity code
(main sequence, sequence cross-fades, overlay layers, frame advance), the
CS-derived base player animation state and the SDK template mod's player
animation state built on it, and the multiplayer player animation state used
by later games. CS:S's own player animation state is not public; everything
in "Player animation state" is the SDK template (CS-derived) and is flagged
under Open questions where CS:S may differ. IK (foot locking, IK rules, IK
locks) is out of scope. No leaked CS:S/CS:GO code was used.

Data was checked against the user's install: `models/player/ct_urban.mdl`,
the seven other player models, `models/player/cs_player_shared.mdl`,
`models/player/ak_anims_ct.mdl` and `ak_anims_t.mdl`, with a throwaway
decoder (not in the repo). All test values below come from that decoder.

Status: draft

## Summary

- A CS:S player model (`ct_urban.mdl`, 50 bones) holds only its mesh and a
  ragdoll sequence. All animation lives in included models: the shared
  `cs_player_shared.mdl` (49 bones, 1382 animations, 723 sequences) and a
  team-specific AK model. They are merged by name into one list of
  sequences, animations and pose parameters; bones are matched by name.
- An animation stores, per bone and frame, a local rotation and position,
  either as one raw constant or as run-length-encoded 16-bit values that are
  scaled and added to the bone's default. A sequence is a 1×1 or 3×3 grid of
  animations addressed by up to two pose parameters; a 3×3 grid is sampled
  by a three-animation triangle blend.
- The body is built bottom-up: model defaults → lower-body sequence (9-way
  move grid on `move_x`/`move_y`) → upper-body weapon sequence(s) whose
  autolayers add a per-weapon static arm pose, a body offset, a 3×3 additive
  aim matrix on `body_yaw`/`body_pitch` and a hand pose → shoot/reload/
  grenade gesture layers. Additive layers are applied in the bone's local
  frame after the base. Feet lag the view: the model's yaw is a "feet yaw"
  that converges toward the eye yaw, and the difference drives `body_yaw`.

## Units and conventions

- Distances in Hammer units (≈ inches). Z up. Right-handed.
- Model space: the animated character faces +X, its left is +Y, up is +Z.
  (The bind/reference pose of the meshes faces −Y; the animations rotate the
  pelvis so the animated character faces +X. Only the animated pose matters
  once animation is on.)
- World placement of the animated root: the entity transform is the
  player origin (bottom centre of the hull) with angles (pitch 0, yaw =
  feet yaw, roll 0). Root bones' local transforms are relative to it.
- Angles: degrees for yaw/pitch and pose parameters; radians for bone
  Euler angles in the file. Eye pitch is positive looking down.
- Quaternions are (x, y, z, w), w real. Product `p·q` is the Hamilton
  product (apply q first, then p).
- Bone local transform: child-to-parent rotation q and translation p;
  world = parent_world ∘ (rotate by q, then translate by p).
- Time: animations are 30 frames/s (1373 of 1382 shared animations; eight
  are 1 fps one-frame poses and one is 15 fps). Bone setup is evaluated per
  rendered frame (client) from the current cycles; cycles advance by real
  time (`dt` = frame time), not by ticks.
- "Cycle" is the normalized position in a sequence, 0 ≤ cycle < 1 for
  looping sequences, 0 ≤ cycle ≤ 1 otherwise.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| studio_version_css | 44 | – | version field of all CS:S player model files |
| max_bones | 128 | bones | per model |
| max_pose_params | 24 | – | per merged model |
| frame_blend_epsilon | 0.001 | – | sub-frame fraction ≤ this samples one frame only |
| grid_epsilon | 0.001 | – | pose fraction < this (or > 1 − this) snaps to a grid column/row |
| diagonal_epsilon | 0.001 | – | triangle-blend middle weight below this is treated as 0 |
| moving_min_speed | 0.5 | units/s | horizontal speed above which the player counts as moving |
| run_threshold | 175 | units/s | template: faster than this standing = run, else walk |
| ground_speed_walk | 100 | units/s | template: full-pose speed for walk (and idle) sequences |
| ground_speed_run | 250 | units/s | template: full-pose speed for run sequence |
| ground_speed_crouchwalk | 85 | units/s | template: full-pose speed for crouch-walk |
| playback_rate_min / max | 0.01 / 10 | × | clamp on speed ÷ ground speed |
| max_body_yaw | 90 | ° | template: torso twist before the feet are dragged along |
| feet_yaw_rate | 720 | °/s | feet yaw convergence rate |
| feet_turn_fade | 45 | ° | below this remaining angle the turn rate scales down linearly |
| feet_turn_min_scale | 0.01 | × | lower bound of that scale |
| face_front_time | 3 | s | standing still this long after the last feet turn → feet turn to face the eyes |
| jump_ground_ignore | 0.2 | s | after a jump starts, landing is not checked for this long |
| default_fade | 0.2 | s | sequence fade in/out time in the data for almost all sequences |
| crossfade_shape | 3s² − 2s³ | – | smoothstep applied to fading weights |

## Behavior

### 1. File layout (what an implementation must read)

All integers little-endian. Every "offset" is a byte offset relative to the
start of the structure that contains it, unless stated. Names here are ours.

Model header (offsets from file start; valid for version 44 files):

| Offset | Type | Meaning |
|---|---|---|
| 4 | i32 | version (44) |
| 12 | char[64] | internal name |
| 156, 160 | i32, i32 | bone count, bone array offset |
| 180, 184 | i32, i32 | animation count, animation-description array offset |
| 188, 192 | i32, i32 | sequence count, sequence array offset |
| 300, 304 | i32, i32 | pose-parameter count, array offset |
| 336, 340 | i32, i32 | include-model count, array offset |
| 348 | i32 | offset of external animation-block file name (0 = none) |
| 352, 356 | i32, i32 | animation-block count, array offset |

Include-model entry (8 bytes): +0 label string offset, +4 file-name string
offset (e.g. `models/player/cs_player_shared.mdl`).

Bone (216 bytes): +0 name string offset; +4 parent index (−1 = root);
+32 default position (3 f32); +44 default quaternion (4 f32); +60 default
Euler angles (3 f32, radians); +72 position scale (3 f32); +84 rotation scale
(3 f32); +96 bind-to-bone 3×4 matrix; +144 alignment quaternion; +160 flags.
The default quaternion equals the default Euler angles converted as in §2
(checked: max difference 7·10⁻⁸). Flag 0x00100000 ("fixed alignment") changes
quaternion alignment (§4); no CS:S player bone has it.

Pose parameter (20 bytes): +0 name offset, +4 flags, +8 start, +12 end,
+16 loop range (0 = none, 360 = wraps).

Animation description (100 bytes):

| Offset | Type | Meaning |
|---|---|---|
| +4 | i32 | name string offset |
| +8 | f32 | frames per second |
| +12 | i32 | flags: 0x1 looping, 0x4 delta (additive), 0x20 "all zeros" |
| +16 | i32 | frame count |
| +20, +24 | i32 | movement-segment count, offset (44-byte segments, §11) |
| +52 | i32 | data block: 0 = inside this file, > 0 = external block, −1 = missing |
| +56 | i32 | data offset (within the block, see below) |
| +80 | i32 | section table offset (8-byte entries: block, offset) |
| +84 | i32 | frames per section (0 = not sectioned) |

Where the per-bone records start for a given integer frame f:
- Not sectioned: block/offset from +52/+56, local frame = f.
- Sectioned (frames per section = S > 0): if the animation has more than S
  frames and f is the last frame, use section ⌊N/S⌋ + 1 with local frame 0
  (the last frame is stored on its own); otherwise section ⌊f/S⌋ with local
  frame f − section·S. Block/offset come from that section entry.
- Block 0: records start at (start of this animation description) + offset.
  Block b > 0: records start at (external file byte `start` of block b) +
  offset, where the block table (8-byte entries: start, end) is at header
  356 and the external file is named by header 348.
- CS:S player files use neither sections nor external blocks (all 1382
  shared animations are block 0, unsectioned). Supporting them is optional.

Per-bone animation record (a linked list, one per animated bone):
- +0 u8 bone index (in the animation's own model; 255 = end of list),
  +1 u8 flags, +2 i16 offset from this record to the next (0 = last).
- Flags: 0x01 raw position (one 6-byte half-float vector for all frames),
  0x02 raw rotation, 48-bit (6 bytes), 0x20 raw rotation, 64-bit (8 bytes),
  0x04 animated position, 0x08 animated rotation, 0x10 delta.
- Payload starts at +4. Raw rotation (6 or 8 bytes) comes first, then raw
  position. For animated data: if 0x08, a 6-byte triple of i16 offsets (x, y,
  z rotation streams), then if 0x04 a 6-byte triple for position. Each
  offset is relative to the start of its own triple; an offset ≤ 0 means
  "no stream: value 0". Note: the position triple's location only accounts
  for an animated-rotation triple before it, not for a raw rotation; mixed
  raw/animated combinations do not occur in CS:S data (flag sets seen: 0x01,
  0x02, 0x03, 0x08, 0x0C, 0x11, 0x12, 0x13, 0x18, 0x1C).
- Bones without a record keep their sequence-model default (non-delta
  animation) or identity/zero (delta animation), see §3.

### 2. Compressed value formats

- Half float (16 bits): IEEE binary16, except exponent 31 with mantissa 0
  decodes to ±65504 and exponent 31 with mantissa ≠ 0 decodes to 0.
- Position vector, 48-bit: three half floats x, y, z.
- Quaternion, 48-bit: three u16 words a, b, c. x = (a − 32768)/32768,
  y = (b − 32768)/32768, z = ((c & 0x7FFF) − 16384)/16384,
  w = √(1 − x² − y² − z²), negated if bit 15 of c is set.
- Quaternion, 64-bit: one u64 v. x = (bits 0–20 − 1048576)/1048576.5,
  y = (bits 21–41 − 1048576)/1048576.5, z = (bits 42–62 − 1048576)/1048576.5,
  w = √(1 − x² − y² − z²), negated if bit 63 is set. (Clamp the radicand at
  0 to be safe.)
- Euler (radians, x, y, z) to quaternion: q = qz(z) · qy(y) · qx(x), i.e.
  rotate about X by x, then about Y by y, then about Z by z (fixed axes).
  With sx = sin(x/2), cx = cos(x/2), and likewise for y and z:
  - x = sx·cy·cz − cx·sy·sz
  - y = cx·sy·cz + sx·cy·sz
  - z = cx·cy·sz − sx·sy·cz
  - w = cx·cy·cz + sx·sy·sz

### 3. Sampling one animation at a cycle

Inputs: the animation, cycle c, and the bone defaults of the model that owns
the animation (for CS:S: `cs_player_shared.mdl`, even when animating
`ct_urban.mdl`, see §8).

1. f = c · (N − 1) with N the animation's frame count; i = ⌊f⌋ (truncation),
   t = f − i. Each animation in a blend uses its own N with the shared c.
2. Locate the records for frame i (§1); i becomes the local frame.
3. Initialise every bone used by the sequence: delta animation → identity
   rotation, zero position; otherwise the bone's default quaternion and
   position (of the animation-owning model).
4. For each record:
   - Rotation: raw 48/64-bit → that quaternion (no default added, even for
     non-delta animations). Not animated → identity if the record's delta flag
     is set, else the bone default. Animated → read each axis stream at
     frame i (and i + 1 if t > 0.001) as value·rotation_scale[axis]; if the
     record is not delta add the default Euler angle of that axis; convert
     each Euler triple to a quaternion (§2). If t > 0.001 and the two triples
     differ, the result is the normalized linear blend of the two
     quaternions (after aligning the second to the first, §4) at t; else the
     first.
   - Position: raw → that vector (no default added). Not animated → zero if
     delta, else default. Animated → per axis value·position_scale[axis],
     linearly interpolated v_i·(1 − t) + v_{i+1}·t when t > 0.001; then add
     the default position if not delta.
5. The 0x10 flag on a record and the 0x4 flag on the animation agree in
   CS:S data (delta animations have only delta records).

Run-length value stream (one per axis): a sequence of spans. Each span is a
16-bit header (low byte = number of stored values V, high byte = number of
frames covered T, V ≤ T) followed by V signed 16-bit values. A span covers T
frames; frames past the V stored values repeat the last stored value.

Reading frame k (k = local frame):
- Special case: if the first span has V = 1 and T = 1, the value is that one
  value for every frame (and for the "next frame" too).
- Walk spans: while T ≤ k, subtract T from k and move to the next span (skip
  header + V values). A span with T = 0 means the stream ran out: value 0.
- In the found span: if k < V, value = stored[k]; else value = stored[V−1].
- The next-frame value (only needed when t > 0.001): if k + 1 < V,
  stored[k+1]; else if k + 1 < T, the same value as frame k; else the first
  stored value of the following span.

### 4. Quaternion operations used by blending

- Align(p, q): if |p − q|² > |p + q|² (component-wise) use −q, else q.
- Normalized blend (nlerp) from p to q at t: q' = Align(p, q);
  r = (1 − t)·p + t·q'; normalize. Used between frames, between grid
  animations, and in the bone-merge cases below.
- Slerp from p to q at t: q' = Align(p, q); cos ω = p·q'. If 1 − cos ω >
  10⁻⁶: standard slerp weights sin((1−t)ω)/sin ω, sin(tω)/sin ω; else linear
  weights (1 − t, t). (The antipodal branch cannot happen after alignment.)
  No renormalization.
- Bones flagged "fixed alignment" skip Align in both (not used by CS:S).
- Scale a rotation by s (used for additive layers): with v = (x, y, z),
  σ = min(|v|, 1), σ' = sin(s·asin σ); result vector = v·σ'/(σ + ε) with
  ε = 1.1920929·10⁻⁷, result w = √(max(0, 1 − σ'²)) with the sign of the
  input w. For w ≥ 0 this scales the rotation angle by s.
- Product with alignment: p·q is computed with q first aligned to p (sign
  only; the rotation is unchanged), then normalized where stated.

### 5. A sequence's pose from its pose parameters

Sequence description (212 bytes), fields used:

| Offset | Type | Meaning |
|---|---|---|
| +4 | i32 | name string offset (e.g. `Run_lower`) |
| +8 | i32 | activity name string offset (e.g. `ACT_RUN`, may be empty) |
| +12 | i32 | flags: 0x1 looping, 0x2 snap (no cross-fade into it), 0x4 delta, 0x10 "post" (with delta: apply after the base, §6) |
| +20 | i32 | activity weight (selection weight) |
| +24, +28 | i32 | event count, offset (80-byte events, §11) |
| +60 | i32 | offset of the animation index grid (i16 entries) |
| +68, +72 | i32 | grid width W, height H |
| +76, +80 | i32 | pose parameter index for the X and Y axes (−1 = none; local to the sequence's own model) |
| +84, +88 | f32 | pose range start X, Y (used only without pose keys) |
| +92, +96 | f32 | pose range end X, Y (same) |
| +104, +108 | f32 | fade-in time, fade-out time (s) |
| +148, +152 | i32 | autolayer count, offset (24-byte entries, §7) |
| +156 | i32 | offset of per-bone weights (f32 per bone of the sequence's model) |
| +160 | i32 | offset of pose keys (f32: W values for X, then H values for Y; 0 = none) |

Grid cell (x, y) → animation index grid[y·W + x] (indices out of range are
clamped to W − 1 / H − 1). Animation indices are local to the sequence's own
model; map them through that model's local-to-merged animation table (§8).

Pose parameter values are stored normalized: n = (value − start)/(end −
start), clamped to [0, 1], after first wrapping looping parameters:
value ← value − L·⌊(value + L − ((start + end)/2 + L/2)) / L⌋ with L the loop
range (this maps into a window of width L centred on the range centre).
Defaults when nothing has set a parameter: if start < 0 < end, the
normalized value of 0; otherwise 0.5.

Per axis (X then Y), from the merged pose-parameter value n:
- No parameter on this axis: fraction s = 0, index i = 0.
- With pose keys (all CS:S grids have them): value v = n·(end − start) +
  start. Start at i = 0; s = (v − key[i])/(key[i+1] − key[i]); while
  s > 1 and i < size − 2, increment i and recompute. Clamp s to [0, 1].
  Keys may be ascending or descending.
- Without pose keys: local start = (range_start − P.start)/(P.end −
  P.start), local end likewise; s = (n − local start)/(local end − local
  start) clamped to [0, 1]; if the axis has more than 2 cells, i = ⌊s·(size −
  1)⌋ capped at size − 2 and s = s·(size − 1) − i, else i = 0.

Then, with (i0, s0) for X and (i1, s1) for Y and cycle c (wrapped into [0,1)
for looping sequences, clamped to [0,1] otherwise), let A(x, y) be the
animation sampled at c (§3). Write "blend(P, Q, s)" for per-bone nlerp of
rotations and lerp of positions from P to Q at s, applied only to bones whose
sequence weight is > 0:

- s0 < 0.001: s1 < 0.001 → A(i0, i1); s1 > 0.999 → A(i0, i1+1); else
  blend(A(i0, i1), A(i0, i1+1), s1).
- s0 > 0.999: same with i0 + 1.
- otherwise, s1 < 0.001 → blend(A(i0, i1), A(i0+1, i1), s0); s1 > 0.999 →
  blend(A(i0, i1+1), A(i0+1, i1+1), s0); otherwise the triangle blend:

Triangle blend of the cell (i0, i1) with fractions (s0, s1). The cell is
split along a diagonal that alternates like a checkerboard:
- (i0 + i1) even (diagonal from cell corner (0,0) to (1,1)):
  - s0 > s1: corners (0,0), (1,0), (1,1); weights w0 = 1 − s0, w1 = s0 − s1.
  - else: corners (1,1), (0,1), (0,0); w0 = s0, w1 = s1 − s0.
- (i0 + i1) odd (diagonal from (1,0) to (0,1)):
  - s0 + s1 > 1: corners (1,0), (1,1), (0,1); w0 = 1 − s1, w1 = s0 + s1 − 1.
  - else: corners (0,1), (0,0), (1,0); w0 = s1, w1 = 1 − s0 − s1.
- If w1 < 0.001 set w1 = 0. w2 = 1 − w0 − w1. Corner offsets add to (i0, i1).
- If w1 < 0.001: result = blend(A(c0), A(c2), w2/(w0 + w2)).
  Otherwise: R = blend(A(c0), A(c1), w1/(w0 + w1)); result = blend(R,
  A(c2), w2).

An animation flagged "all zeros" at the chosen single corner (s ≈ 0 cases)
yields "no pose" and the sequence is skipped; when exactly one of the two
X-neighbours is all-zero, the other is scaled toward identity instead of
blended. No CS:S animation has that flag.

### 6. Accumulating a sequence onto the current pose

Inputs: the current pose (per bone q, p), a sequence, its cycle, a weight
w ∈ [0, 1] (clamped), the pose parameters.

1. Compute the sequence's own pose (§5) into a scratch pose.
2. Merge it into the current pose bone by bone with s = w · bone_weight[b]
   (bone_weight from the sequence; bones of the target model that the
   sequence's model lacks get 0). Skip bones with s ≤ 0; cap s at 1.
   - Non-delta sequence: q ← slerp(scratch q, current q, 1 − s) (equivalently
     from current toward scratch by s, with alignment relative to the
     scratch value); p ← p·(1 − s) + scratch p·s.
   - Delta sequence with the "post" flag (0x14, every CS:S additive layer):
     q ← normalize(q · Scale(scratch q, s)) — the delta is applied in the
     bone's local frame after the base; p ← p + scratch p·s.
   - Delta without "post": q ← normalize(Scale(scratch q, s) · q);
     p ← p + scratch p·s. (Not used by CS:S player data.)
3. Then accumulate the sequence's autolayers (§7) with this weight.

Building a whole body: start from the target model's default pose (every
bone's default q, p), accumulate the main (lower-body) sequence with weight
1, then fading-out previous main sequences (§9), then the overlay layers in
order (§10), each with its own sequence, cycle and weight.

### 7. Autolayers

Autolayer entry (24 bytes): +0 i16 sequence index (local to the owning
model), +2 i16 pose parameter index, +4 i32 flags, +8 f32 start, +12 peak,
+16 tail, +20 end.

For each autolayer of a sequence accumulated with weight w at cycle c, in
file order, after the sequence itself:
- start = end: accumulate the layer sequence with cycle c and weight w.
- otherwise (ramp on the parent cycle): skip if c < start or c ≥ end. Ramp
  s = 1; if c < peak and start ≠ peak, s = (c − start)/(peak − start); else if
  c > tail and end ≠ tail, s = (end − c)/(end − tail). Weight = w·s; layer
  cycle = (c − start)/(end − start).
- Flag variants exist (ramp by a pose parameter, spline ramp, ignore parent
  weight, cross-fade bias, local) but every autolayer in the CS:S player
  files has flags 0.
- Autolayer sequences may have autolayers themselves (recursion).
- NaN ramps: 32 crouch upper-body sequences (e.g. `Crouch_Idle_Upper_PISTOL`)
  store NaN for start/peak/tail/end of their aim autolayer. The comparisons
  all fail, so the layer gets full weight w and a NaN cycle. All the aim
  animations involved are one-frame raw data, so the cycle has no effect.
  Implement as: a NaN ramp is treated like start = end (full weight, parent
  cycle).

### 8. Include models (merged model)

A model with include entries is animated through a merged view built in this
order: the model itself, then each include in order (recursively, depth
first). Matching is case-insensitive by name.
- Sequences: appended in that order; a sequence whose name already exists is
  not added (the first one wins). Sequence indices used anywhere (layers,
  "sequence 0") are merged indices. For `ct_urban.mdl`: 0 = `ragdoll`, then
  the 723 shared sequences at 1–723 (`Idle_lower` = 1, `Run_lower` = 3), then
  the 33 AK sequences at 724–756 (`Run_Aim_AK` = 724); 757 in all.
- Animations: same rule (first name wins). Six AK-model animations share a
  name with shared ones (`Stand_Shoot_Rifle_layer`, `Crouch_Shoot_Rifle_layer`,
  four `*_WeapPos_neutral`) and resolve to the shared model's data. 1445
  merged animations for `ct_urban.mdl`.
- Pose parameters: same rule; a duplicate widens the existing range to the
  union of both ranges. Merged order for CS:S player models: `move_yaw`,
  `body_pitch`, `body_yaw` (from the player model), `move_y`, `move_x`.
- Bones: each included model's bone maps to the target model's bone of the
  same name; the target model's bones not present in an include are never
  written by that include's animations (`ct_urban.mdl` bone 49
  `ValveBiped.forward` keeps its default).
- When sampling an included animation, defaults, position/rotation scales
  and Euler bases come from the bones of the model that owns the animation;
  bones the sequence touches but the animation lacks get the defaults of the
  model that owns the sequence. Consequence: every animated bone of
  `ct_urban.mdl` takes the shared skeleton's lengths, not its own (its own
  thigh offset is 4.0461, the shared one 4.1628; calf 18.5621 vs 19.0976).
  The mesh is still skinned with `ct_urban.mdl`'s own bind matrices.

### 9. Playing the main sequence

- Cycle rate (cycles/s) of a sequence at the current pose parameters: the
  four grid cells around (i0, i1) with bilinear weights (1−s0)(1−s1),
  s0(1−s1), (1−s0)s1, s0·s1 (cells clamped into the grid), summing
  weight·fps/(N − 1) over cells with weight > 0 and N > 1. (Bilinear even
  though the pose uses the triangle blend.)
- Each frame: cycle ← cycle + dt · rate · playback_rate. Outside [0, 1):
  looping → keep the fraction; non-looping → clamp to [0, 1] and mark
  finished.
- Starting a sequence (new sequence chosen for a new activity): if the
  outgoing sequence does not loop, cycle ← 0; otherwise the cycle carries
  over unchanged (idle → run keeps its phase). Playback rate ← 1.
- Cross-fade (client): when the main sequence changes and the new one does
  not have the snap flag, the outgoing sequence is kept with fade time
  F = min(outgoing fade-out, incoming fade-in) (0.2 s for all lower-body
  sequences) measured from the moment it stopped being current. Its weight
  is u = 1 − (now − stop time)/F, shaped as 3u² − 2u³, and it is removed once
  u ≤ 0. Its cycle keeps advancing at its own rate × the playback rate it had.
  Fading sequences are accumulated after the current one, the most recently
  replaced first and the oldest last, each with its fading weight. With the snap flag
  the queue is cleared and the new sequence pops in.

### 10. Overlay layers

The entity keeps a small array of overlay layers, each (sequence, cycle,
weight, order). Before bone setup the layers are sorted by order (layers
with "no order" are skipped; two layers claiming the same order: the later
one is dropped). Each layer with weight > 0 is accumulated (§6) with its
cycle wrapped (looping) or clamped to [0, 0.999] (non-looping) and its
weight capped at 1. Layers whose sequence index is out of range are skipped.

### 11. Other per-sequence data (informational)

- Movement segments of an animation (44 bytes): +0 end frame, +4 motion
  flags, +8 start speed, +12 end speed (units/s), +16 yaw at end, +20
  direction vector, +32 position. CS:S move animations author: run ≈ 150
  units/s (`a_RunN` 149.80), walk ≈ 107 (`a_WalkN` 107.00), crouch-walk 44–83
  depending on direction (`a_Crouch_walkN` 66.92). Directions: N = +X,
  W = +Y (left), consistent with the move grid. Zero-direction cells
  (`a_RunZero` etc.) have no movement and 61 frames.
- Events (80 bytes): +0 cycle, +4 event number, +8 type, +12 options string.
  The lower-body sequences carry footstep events: `Run_lower` at cycles 0.35
  ("rfoot") and 0.8 ("lfoot"), numbers 7001 and 4001; `walk_lower` at 0.3125
  and 0.7812 (7002/4002); `Crouch_walk_lower` at 0.2083 and 0.7083;
  `Jump` at 0.92 (7001, both feet).

### 12. Player animation state (SDK template, CS-derived reference)

This decides, per frame, the main sequence, the pose parameters, the overlay
layers and the render yaw from: eye yaw and pitch (normalized to
(−180, 180]), horizontal velocity (server: simulation velocity; client: the
smoothed velocity of the interpolated origin; local player: its own
velocity), ducked flag, on-ground flag, active weapon's animation suffix,
and events (fire, jump, reload, grenade throw). Speed = |velocity_xy|;
moving = speed > 0.5.

Configuration of the CS-derived template: 9-way legs (`move_x`/`move_y`),
aim sequences on, max body yaw 90°.

12.1 Main activity (lower body):
- Jumping (set by a jump event) → `ACT_HOP` (`Jump`, 26 frames,
  non-looping). On the first update after the event the main sequence
  restarts (cycle 0). Once more than 0.2 s have passed since the jump and the
  player is on the ground, jumping ends and the main sequence restarts again.
- Else ducked: moving → `ACT_RUN_CROUCH` (`Crouch_walk_lower`), else
  `ACT_CROUCHIDLE` (`Crouch_Idle_Lower`).
- Else moving: speed > 175 → `ACT_RUN` (`Run_lower`), else `ACT_WALK`
  (`walk_lower`). Not moving → `ACT_IDLE` (`Idle_lower`).
- The sequence for an activity is chosen at random among sequences with
  that activity weighted by |activity weight| (one each in CS:S). The main
  sequence is replaced only when the new sequence's activity differs from
  the current one's (§9 for cycle and cross-fade).

12.2 Ground speed and playback rate:
- Ground speed for the current main sequence's activity: walk or idle 100,
  run 250, crouch-walk 85, anything else 0.
- rate = 1 if not moving; else if ground speed < 0.001, 0.01; else
  clamp(speed / ground speed, 0.01, 10).
- With 9-way legs the main sequence's playback rate stays 1; speed is
  expressed through the pose parameters instead.

12.3 `move_x` / `move_y`:
- Gait yaw = atan2(v_y, v_x) in degrees, updated only while moving (keeps
  its last value otherwise).
- d = −(eye yaw − gait yaw), reduced to [−180, 180] (truncating division by
  360, then ±360).
- Moving: move_x = cos d · rate, move_y = −sin d · rate. Not moving: both 0.
- Stored through §5's encoding, so each is clamped to [−1, 1] separately.
- Meaning on the grid (`Run_lower`, `walk_lower`, `Crouch_walk_lower`; X axis
  = `move_y` with keys −1, 0, 1; Y axis = `move_x` with keys 1, 0, −1):

  | | move_y −1 (left) | 0 | +1 (right) |
  |---|---|---|---|
  | move_x +1 (forward) | NW | N | NE |
  | 0 | W | Zero | E |
  | −1 (back) | SW | S | SE |

12.4 Feet yaw, `body_yaw`, render yaw (per update, dt = frame time):
- First update after a reset (spawn, entering view): feet yaw = goal = eye
  yaw, last-turn time = 0.
- Else if moving: goal = eye yaw.
- Else if now − last-turn time > 3 s: goal = eye yaw.
- Else: e = normalize(goal − eye yaw); if |e| > 90, goal moves 90° toward the
  eye yaw (goal −= 90 if e > 0, += 90 otherwise).
- goal normalized to (−180, 180]. If feet ≠ goal: converge (below) and set
  last-turn time = now.
- Converge: diff = normalize(goal − feet); a = |diff|; scale = clamp(a/45,
  0.01, 1) if a ≤ 45 else 1; step = 720·dt·scale; if a > 90 (max body yaw),
  step = a − 90 instead (snap so exactly 90° remain); if a < step, feet =
  goal, else feet moves by step toward goal; normalize.
- Torso yaw = normalize(eye yaw − feet yaw) → `body_yaw` (range ±90, so it
  clamps). Render angles = (0, feet yaw, 0). The server also turns the
  entity (and hitboxes) to the feet yaw.

12.5 `body_pitch` = eye pitch (if > 180 subtract 360), clamped to ±90.

12.6 Upper-body weapon layers. Suffix = the active weapon's animation
extension (no weapon → `Pistol`); sequence names are looked up
case-insensitively; a failed lookup yields merged sequence 0 (`ragdoll` in
CS:S player models).
- Layer order 1–2, "idle aim": sequence = `Crouch_Idle_Upper_<suffix>` if the
  main activity is crouch-idle, else `Idle_Upper_<suffix>`.
- Layer order 3–4, "moving aim", only if the main activity is run, walk or
  crouch-walk and the player is moving: `Run_Upper_<suffix>` (run),
  `Walk_Upper_<suffix>` (walk), `Crouch_Walk_Upper_<suffix>` (crouch-walk);
  weights scaled by rate (§12.2).
- Each pair is driven by its own transition queue (same mechanism as §9:
  checks for a change of the chosen sequence, fade time = min(old fade-out,
  new fade-in)). One entry → first layer weight 1, second 0. Two or more →
  first layer = the oldest entry with its fading weight u, second layer = the
  next entry with weight 1 − u (a third, newer entry waits until the oldest
  is gone). Both weights are then multiplied by the scale (1 for idle, rate
  for moving) and clamped to [0, 1]. Both layers' cycle = the main sequence's
  cycle (upper body is phase-locked to the legs).
- Then: idle layer 1's weight = max(0, 1 − sum of the weights of layers 2–4
  that are in use); and every layer before the last one whose weight
  exceeds 0.99 is switched off (an optimization, but it changes nothing
  visible only because that layer covers the same bones).
- Layer order 5, fire: on a fire event, cycle 0 and a sequence by the current
  main activity: run → `Run_Shoot_<suffix>`, walk → `Walk_Shoot_<suffix>`,
  crouch-idle → `Crouch_Idle_Shoot_<suffix>`, crouch-walk →
  `Crouch_Walk_Shoot_<suffix>`, else `Idle_Shoot_<suffix>`. Each update the
  cycle advances by its cycle rate × dt; past 1 the layer turns off. Weight 1.
- Layer order 6, reload: on a reload event, `reload_<suffix>` if it exists
  else `reload_m4`; same advance/stop rule. (CS:S data has
  `<Move>_Reload_<suffix>` sequences instead, see Open questions.)
- Layer order 7, grenade: while a grenade's pin is pulled (or a throw is
  pending) play `idle_shoot_gren1` from cycle 0 (from cycle 1 if the state
  was reset less than 0.4 s ago), holding at cycle 1; when a throw is pending
  and the prime has reached 1, switch to `idle_shoot_gren2` from 0 and play
  it out (layer off past 1).
- Layer order 0 (an idle blended under 8-way legs) is unused with 9-way legs.

### 13. Multiplayer animation state (later games; alternative reference)

Only the differences from §12 that would matter for CS:S data:
- `move_x`/`move_y`: direction (cos d, −sin d) is pushed out to the unit
  square (divide by max(|x|, |y|)); the authored ground speed G of the
  current sequence at that pose is read from its movement data; if G >
  speed, both are scaled by speed/G (never scaled up). Playback rate 1.
- Feet: moving (|v| > 1, 3D) → goal = eye yaw; standing → if
  |normalize(goal − eye)| > 45, goal moves 45° toward the eye. Converge at
  720°/s, scale = clamp(|diff|/60, 0.01, 1), no snap rule, no 3 s face-front.
- Aim pose parameters are set to the negated eye pitch and torso yaw (that
  game family's rigs use the opposite sign; CS:S rigs match §12, see §14).
- Gestures: layers whose cycle advances by rate × dt × gesture rate and that
  either stop at 1 or are removed past 1.

### 14. CS:S model inventory

Pose parameters of the shared model (name, start, end, loop):
`move_yaw` −180 180 360; `body_pitch` −90 90 360; `body_yaw` −90 90 360;
`move_y` −1 1 0; `move_x` −1 1 0. `move_yaw` is not used by any sequence.

Lower body (flags looping; fade 0.2/0.2; all bone weights 1):

| Sequence | Activity | Grid | Axes (X, Y) | Animations (row-major) | Frames |
|---|---|---|---|---|---|
| `Idle_lower` | ACT_IDLE | 1×1 | – | `@Idle_lower` | 61 |
| `Crouch_Idle_Lower` | ACT_CROUCHIDLE | 1×1 | – | `@Crouch_Idle_Lower` | 61 |
| `Run_lower` | ACT_RUN | 3×3 | `move_y` keys −1,0,1; `move_x` keys 1,0,−1 | `a_RunNW a_RunN a_RunNE a_RunW a_RunZero a_RunE a_RunSW a_RunS a_RunSE` | 21 (Zero 61) |
| `walk_lower` | ACT_WALK | 3×3 | same | `a_Walk*` with `a_walkZero` | 33 (Zero 61) |
| `Crouch_walk_lower` | ACT_RUN_CROUCH | 3×3 | same | `a_Crouch_walk*` | N/NE/NW/S/SE/SW 25, E/W 17, Zero 61 |
| `Jump` | ACT_HOP | 1×1 | – | `@Jump` (not looping) | 26 |

Death: `Death1` (101 frames) and `deathpose_{front,back,right,left}`,
`deathpose_crouch_{front,back,right,left}` (activities
`ACT_DIE_[CROUCH_]{FRONT,BACK,RIGHT,LEFT}SIDE`, 7 frames).

Per weapon suffix X (shared model): PISTOL, ELITES, FAMAS, SCOUT, M4, AUG,
SG550, AWP, GALIL, SG552, G3, MP5, P90, TMP, UMP45, MAC10, M3S90, XM1014,
M249, GREN, KNIFE, C4; and AK from `ak_anims_ct.mdl` / `ak_anims_t.mdl`
(CT player models include the first, T models the second; they differ only
in that the CT one has `Crouch_HandPos_AK`).

| Sequence pattern | Flags | Content |
|---|---|---|
| `Idle_Upper_X`, `Walk_Upper_X`, `Run_Upper_X`, `Crouch_Idle_Upper_X`, `Crouch_Walk_Upper_X` | 0 | 1×1 base pose of arms/neck/head (bone weight 0 on pelvis, legs, spine); 61/33/21 frames for idle/walk/run; crouch ones 1 frame for 16 of the 22 suffixes, 61 (crouch-idle) or 25/61 (crouch-walk) for the rest. Autolayers below. |
| `Run_Upper_X_staticlayer` | 0 | autolayer of `Run_Upper_X`: fixed arm pose, bone weights 0.8 on arms/hands (0.5 for ELITES, UMP45; KNIFE: 0.5 left arm, 0.8 neck/head) |
| `{Run,Walk,Idle,Crouch}_BodyOffset_X` | delta+post | 1-frame additive body offset |
| `{Run,Walk,Idle,Crouch}_Aim_X` | delta+post | 3×3, X axis `body_yaw` keys −60, 0, 60; Y axis `body_pitch` keys −70, 0, 70 (Crouch_Aim_FAMAS: −50, 0, 50); cells `*_aim_{up,mid,down}_{right,center,left}` row-major (up row first, right column first); all 1-frame raw deltas |
| `{Run,Walk,Idle,Crouch}_HandPos_X` | snap | 1-frame hand/finger/weapon-bone pose; weights 0 on body, 1 on hands |
| `{Idle,Walk,Run,Crouch_Idle,Crouch_Walk}_Shoot_X` | delta+post | recoil layer (`Stand_Shoot_*_layer`, `Crouch_Shoot_*_layer`), 11 frames for pistol |
| `{Run,Walk,Idle,Crouch_Idle,Crouch_Walk}_Reload_X` | 0 | reload base (61 frames for pistol) with autolayer `..._Reload_X_seq` ramped 0 → 0.133 → 0.867 → 1 (values vary by weapon) |

Autolayer sets: `Run_Upper_X` → staticlayer, BodyOffset, Aim, HandPos (in
that order; C4 has no Aim); `Walk_Upper_X`, `Idle_Upper_X` → BodyOffset, Aim,
HandPos (KNIFE order BodyOffset, HandPos, Aim); `Crouch_Idle_Upper_X` and
`Crouch_Walk_Upper_X` → `Crouch_Aim_X` with the NaN ramp (§7), sometimes plus
`Crouch_HandPos_X`; FAMAS and a few others have BodyOffset, Aim, HandPos.

Aim cell orientation check: `body_yaw` = eye − feet, positive = eyes turned
left → grid column 2 (`*_left` cells); `body_pitch` positive = looking down →
grid row 2 (`*_down` cells). So CS:S rigs take `body_yaw` and `body_pitch` without
negation (unlike §13).

Specials: ELITES has `{..}_Shoot_ELITES_R/_L` (alternating hands) and no
plain shoot; M3S90 and XM1014 reload as `{move}_reload_X_start/_loop/_end`
plus `Reload_X_End_seq`; ELITES and M249 also have a single `Reload_X`;
grenades have `{Move}_Shoot_GREN1` (prime, fade-in 0.3) and `_GREN2`
(throw, with a ramped `_seq` autolayer) and `Idle/Crouch_Idle_Shoot_GREN2_seq`;
KNIFE and C4 have no reload. `speaker_testpose` is unused.

Bone weights summary (shared skeleton): lower-body sequences 1 everywhere;
upper-body base 0 on pelvis, legs and the four spine bones (and the three
weapon-hand helper bones for some weapons); aim/body-offset deltas cover the
whole body including pelvis and legs (the aim twists the pelvis and
counter-rotates the thighs); hand-pose sequences touch only hands, fingers
and weapon bones. Implementations should read the weights from the file.

Weapon suffix: the active weapon script's animation-extension key
(`PlayerAnimationExtension` in CS:S scripts), matched case-insensitively.
Expected mapping (by name, unverified): ak47 → AK; aug → AUG; awp → AWP;
c4 → C4; deagle, glock, usp, p228, fiveseven → PISTOL; elite → ELITES;
famas → FAMAS; g3sg1 → G3; galil → GALIL; m249 → M249; m3 → M3S90;
m4a1 → M4; mac10 → MAC10; mp5navy → MP5; p90 → P90; scout → SCOUT;
sg550 → SG550; sg552 → SG552; tmp → TMP; ump45 → UMP45; xm1014 → XM1014;
knife → KNIFE; hegrenade, flashbang, smokegrenade → GREN.

## Per-frame order

Animation-state update (once per frame per player, client; the server runs
the same logic per tick for hitboxes):
1. Mark all overlay layers unused.
2. If the player is dead: reset the state (§12.4 first-update rule next
   time) and stop.
3. Normalize eye yaw and pitch.
4. Main activity → main sequence (§12.1), possibly restarting it.
5. Ground speed for the (new) main sequence's activity (§12.2).
6. Upper-body layers 1–4 (§12.6), then the idle-weight fix-up.
7. Fire, reload and grenade layers advance (§12.6).
8. `body_pitch` (§12.5).
9. Feet yaw, `body_yaw`, render yaw (§12.4).
10. `move_x`, `move_y` (§12.3).

Bone setup (each rendered frame, after the main cycle advanced by dt):
1. Default pose of the target model.
2. Main sequence at its cycle, weight 1, with its autolayers.
3. Fading previous main sequences (§9).
4. Overlay layers by order (§10), each with autolayers.
5. (IK, out of scope.)
6. Local → world: root bones under the entity transform (origin, yaw = feet
   yaw), children under their parents.

## Edge cases

- Cycle exactly 1 on a non-looping sequence samples the last frame (f = N −
  1, t = 0). Overlay layers clamp non-looping cycles to 0.999 instead.
- A ramped autolayer is skipped at parent cycle ≥ end (so at cycle 1.0
  exactly when end = 1).
- Pose values beyond the grid's keys clamp to the edge cell (aim keys ±60
  inside a ±90 parameter; pitch keys ±70 inside ±90).
- `move_x`/`move_y` are clamped per axis, so diagonal over-speed distorts the
  direction toward the diagonal.
- One-frame animations: f = 0 for every cycle.
- A missing upper-body sequence name selects merged sequence 0 (`ragdoll`),
  which would pose the whole upper body as the ragdoll reference. Treat a
  failed lookup as "no layer" unless reproducing the bug deliberately.
- Bones of the target model with no counterpart in the included model
  (`ValveBiped.forward`) stay at the target model's default.
- The shared skeleton's reference pose faces −Y; never draw the reference
  pose of included bones under the entity yaw and expect it to face forward.

## Quirks

- Bone lengths come from the shared skeleton, not the player model (§8).
  Keep: that is how every CS:S player looks in game.
- Raw rotations/positions ignore the bone default even in non-delta
  animations (they are absolute). Keep.
- Frame interpolation of animated rotations blends quaternions linearly
  (nlerp), not slerp; grid blending is nlerp too; only layer merging uses
  slerp. Keep for fidelity (differences are tiny).
- Cycle rate uses bilinear cell weights while the pose uses the triangle
  blend. Keep.
- At a full-speed 45° diagonal, `move` = (0.707, −0.707) lies on the cell
  diagonal: the pose is 70.7 % `a_RunNW` + 29.3 % `a_RunZero`, never pure
  `a_RunNW`.
- Feet snap rule: when the feet are more than 90° behind their goal they
  jump so that exactly 90° remain, ignoring the rate for that frame.
- Standing turn rule moves the goal by 90° (max body yaw), leaving only
  |eye − goal| − 90 of twist, not "keep twist at 90".
- Upper body is phase-locked to the legs: switching idle → run keeps the
  cycle, so the upper idle (61 frames) and run (21 frames) are both read at
  the leg cycle.
- NaN autolayer ramps (§7). Harmless; treat as full weight.
- Transition queue with three entries ignores the newest until the oldest
  fades (§12.6).

## Test cases

Values decoded from the user's CS:S install (`cs_player_shared.mdl` unless
stated). Positions in units, quaternions (x, y, z, w); compare rotations up
to sign (q ≡ −q) with tolerance 1e-3, positions 1e-3.

Format and decoding:

| Setup | Input | After | Expected |
|---|---|---|---|
| shared model header | – | – | version 44; 49 bones; 1382 animations; 723 sequences; 5 pose params; 0 includes; 0 animation blocks |
| `ct_urban.mdl` header | – | – | version 44; 50 bones (bone 49 `ValveBiped.forward`, parent 14); 1 animation `@ragdoll`; 1 sequence `ragdoll`; pose params `move_yaw`, `body_pitch`, `body_yaw`; includes `models/player/cs_player_shared.mdl`, `models/player/ak_anims_ct.mdl` |
| merged `ct_urban.mdl` | – | – | 757 sequences; 1445 animations; `Idle_lower` = 1, `Run_lower` = 3, `Run_Aim_AK` = 724; pose order `move_yaw, body_pitch, body_yaw, move_y, move_x` |
| 48-bit quaternion bytes `00 80 3a 96 00 40` | decode | – | (0, 0.173645, 0, 0.984808) |
| 48-bit vector bytes `4e 3e 96 c1 00 00` | decode | – | (1.576172, −2.792969, 0) |
| Euler (0.1, 0.2, 0.3) rad | to quaternion | – | (0.034271, 0.106021, 0.143572, 0.983347) |
| `@Idle_lower` bone 2 `L_Calf` record | read | – | flags 0x08; rotation offsets (0, 0, 6); z stream first span V = 61, T = 61, values 0, −4, −1, 11, 33, 65, …; rotation scale z 5.818989e-5; default Euler z 0.766488 |
| `@Idle_lower` (61 frames, looping) | `L_Calf` at frame 3 | – | q (0, 0, 0.374228, 0.927337), p (19.0976, 0, 0) |
| same | `L_Calf` at frame 4.5 (cycle 4.5/60) | – | q (0, 0, 0.375253, 0.926922) |
| same | `L_Calf` at frame 60 and at cycle 1.0 | – | q (0, 0, 0.373931, 0.927456) |
| same | `Pelvis` at frame 3 | – | p (−0.410149, 0.002761, 37.420749) |
| `@Idle_lower` frame 0 | Pelvis | – | p (−0.4101, 0.0028, 37.4364), q (0.6258, 0.3292, 0.3773, 0.5980) |
| same | L_Thigh | – | p (4.1628, 0, 0), q (−0.7435, 0.4042, −0.4644, 0.2611) |
| same | Spine4 | – | p (9.5460, 0, 0), q (−0.0208, 0.0068, 0.0765, 0.9968) |
| same | Head1 | – | p (3.8520, 0, 0), q (0.3161, −0.0216, 0.2915, 0.9025) |
| same | R_Hand | – | p (12.2854, 0, 0), q (−0.4850, −0.1127, 0.3484, 0.7941) |
| same | weapon_bone | – | p (18.2926, 24.1148, −1.2414), q (−0.4368, −0.2736, −0.5558, 0.6522) |
| `@Idle_lower` frame 30 | Pelvis | – | p (−0.4101, 0.0028, 36.8621), q (0.6252, 0.3298, 0.3778, 0.5980) |
| same | L_Calf | – | q (0, 0, 0.4097, 0.9122) |
| same | weapon_bone | – | p (18.3004, 24.2163, −1.5578), q (−0.4515, −0.2620, −0.5615, 0.6421) |
| `a_RunN` (21 frames) frame 0 | Pelvis | – | p (−0.4101, 0.0028, 33.1589), q (0.5087, 0.5086, 0.4912, 0.4912) |
| same | L_Thigh | – | q (−0.5869, 0.4104, −0.5906, 0.3719) |
| same | L_Calf | – | q (0, 0, 0.6019, 0.7986) |
| `a_RunN` frame 10 | Pelvis | – | p (−0.4101, −2.4856, 33.1589) |
| same | L_Thigh | – | q (−0.6660, 0.2435, −0.6567, 0.2567) |
| same | L_Calf | – | q (0, 0, 0.8302, 0.5575) |
| `a_RunN` frame 2.5 (cycle 0.125) | Pelvis | – | p (0.3877, −0.3000, 33.9363), q (0.4431, 0.5638, 0.5156, 0.4690) |
| same | L_Thigh | – | q (−0.3920, 0.5228, −0.4234, 0.6275) |
| `Run_Pistol_aim_up_left` (1 frame, delta) | Pelvis (record flags 0x13) | – | p (1.5762, −2.7930, 0), q (0, 0.1736, 0, 0.9848) |
| same | Spine4 | – | p (0, 0, 0), q (0.0679, −0.0302, −0.0728, 0.9946) |
| same | a bone with no record, e.g. L_Toe0 | – | p (0, 0, 0), q (0, 0, 0, 1) |
| `Stand_Shoot_Pistol_layer` (11 frames, delta) frame 5 | weapon_bone (flags 0x1C) | – | p (0.5391, −2.4649, −0.2930), q (−0.0672, 0, 0, 0.9977) |
| every looping lower-body animation | frame 0 vs frame N−1 | – | identical rotations |
| `ct_urban.mdl` merged, `Idle_lower` cycle 0 | L_Thigh, `ValveBiped.forward` | – | L_Thigh p (4.1628, 0, 0) (shared length, not 4.0461); forward p (2.0, −3.0, 0), q (−0.5572, 0.4353, −0.4353, 0.5572) (its default) |

Pose parameters and grids:

| Setup | Input | After | Expected |
|---|---|---|---|
| `body_yaw` (−90..90, loop 360) | set 30 | – | stored 0.666667 |
| `body_yaw` | set 120 | – | stored 1.0 (reads back 90) |
| `move_yaw` (−180..180, loop 360) | set 270 | – | wraps to −90, stored 0.25 |
| `body_pitch` | set −100 | – | stored 0.0 |
| defaults | nothing set | – | all five 0.5 |
| `Idle_Aim_Pistol` | `body_yaw` 20, `body_pitch` −35 | – | X: i0 = 1, s0 = 0.3333; Y: i1 = 0, s1 = 0.5 |
| same | `body_yaw` 75 | – | X: i0 = 1, s0 = 1.0 (clamped from 1.25) |
| `Run_lower` | `move_x` 0.8660, `move_y` −0.5 | – | X (`move_y`): i0 = 0, s0 = 0.5; Y (`move_x`): i1 = 0, s1 = 0.1340 |
| triangle blend | (i0, i1, s0, s1) = (0, 0, 0.5, 0.133975) | – | cells (0,0), (1,0), (1,1); weights 0.5, 0.366025, 0.133975 (`a_RunNW`, `a_RunN`, `a_RunZero` on `Run_lower`) |
| triangle blend | (0, 0, 0.2, 0.6) | – | cells (1,1), (0,1), (0,0); weights 0.2, 0.4, 0.4 |
| triangle blend | (1, 0, 1/3, 0.5) | – | cells (1,1), (1,0), (2,0); weights 0.5, 0.166667, 0.333333 (`mid_center`, `up_center`, `up_left` on an aim grid) |
| triangle blend | (1, 0, 0.8, 0.6) | – | cells (2,0), (2,1), (1,1); weights 0.4, 0.4, 0.2 |
| `Run_lower` | `move_x` 0, `move_y` −1 | – | exactly `a_RunW` |
| `Crouch_walk_lower` | `move_x` 0.5, `move_y` 0 | – | 50 % `a_Crouch_walkN` + 50 % `a_Crouch_walkZero`; cycle rate 0.875 /s |
| `Run_lower` at (`move_x`, `move_y`) = (0.866, −0.5), cycle 0 | Pelvis | – | p (−0.4101, −0.0011, 33.7320), q (0.5146, 0.4902, 0.4904, 0.5043) |
| same | L_Thigh | – | q (−0.6194, 0.4318, −0.5525, 0.3531) |
| same | L_Calf | – | q (0, 0, 0.5696, 0.8219) |
| same | Spine | – | p (0, 3.5793, −3.1906), q (0.5639, 0.4015, 0.5074, 0.5132) |
| same, cycle 0.25 | Pelvis | – | p (0.1756, −0.4573, 36.7036), q (0.4139, 0.5670, 0.5228, 0.4836) |
| same, cycle 0.25 | L_Calf | – | q (0, 0, 0.4473, 0.8944) |
| `Run_lower` at (`move_x`, `move_y`) = (0.7071, −0.7071) | cycle rate | – | 1.414214 /s (bilinear: NW 0.5, N 0.2071, W 0.2071, Zero 0.0858) |
| cycle rates, (`move_x`, `move_y`) | `Idle_lower`; `Run_lower` at (1, 0); `walk_lower` at (1, 0); `Run_lower` at (0, 0) | – | 0.5; 1.5; 0.9375; 0.5 /s |

Layering (pose: `body_yaw` 20, `body_pitch` −35, `move_*` 0; base = shared
defaults, then `Idle_lower` at cycle 0, weight 1):

| Setup | Input | After | Expected |
|---|---|---|---|
| base + `Idle_Aim_Pistol` (delta+post) | weight 1 | – | Pelvis p (−0.6500, −0.9282, 36.6908), q (0.5426, 0.4493, 0.4738, 0.5284); Spine4 q (0.0029, 0.0012, 0.0443, 0.9990); Head1 q (0.2748, 0.0130, 0.4177, 0.8659) |
| same | weight 0.5 | – | Pelvis p (−0.5301, −0.4627, 37.0636), q (0.5868, 0.3910, 0.4275, 0.5657); Spine4 q (−0.0090, 0.0040, 0.0604, 0.9981) |
| base + `Idle_Upper_PISTOL` (with its autolayers) | weight 1, cycle 0 | – | Pelvis as the weight-1 aim row; L_Thigh q (−0.6673, 0.2997, −0.6039, 0.3166); Spine4 q (0.0029, 0.0012, 0.0443, 0.9990); Head1 q (0.2741, 0.0127, 0.4176, 0.8662); R_Hand q (−0.3304, −0.0804, 0.2841, 0.8965); weapon_bone p (31.6520, 23.5236, 1.0931), q (−0.5305, −0.1653, −0.6531, 0.5145); weapon_bone_RHand p (−1.5703, −4.0742, −5.9219), q (0.4021, −0.5860, 0.4637, 0.5291) |
| base + `Run_Upper_PISTOL_staticlayer` (bone weight 0.8 on arms) | weight 1, cycle 0 | – | L_UpperArm: base (−0.8297, −0.0747, −0.4851, 0.2659), layer (−0.8141, −0.2071, −0.4545, 0.2964), result slerp 0.8 = (−0.8185, −0.1809, −0.4614, 0.2907); R_Hand result (−0.5164, −0.0840, 0.4447, 0.7270) |

Sequence cross-fade and animation state (pure math):

| Setup | Input | After | Expected |
|---|---|---|---|
| main sequence switch, fade 0.2 s | 0.05 s after switch | – | outgoing weight 0.84375 |
| same | 0.1 s | – | 0.5 |
| same | 0.2 s | – | removed |
| first update | eye yaw 30 | – | feet 30, `body_yaw` 0 |
| standing, feet = goal = 0, recent turn | eye 30, dt 0.01 | – | feet 0, `body_yaw` 30 |
| standing, feet = goal = 0, recent turn | eye 120, dt 0.01 | – | goal 90; feet 7.2; `body_yaw` stored as 90 (112.8 clamped) |
| moving, feet 0 | eye 100, dt 0.01 | 1 update | feet 10 (snap rule); `body_yaw` 90 |
| same | – | 2nd update | feet 17.2 |
| moving, feet 0 | eye 10, dt 0.01 | – | feet 1.6 (scale 10/45) |
| standing, feet 0, last turn 3.5 s ago | eye 50, dt 0.01 | – | goal 50, feet 7.2 |
| eye yaw 0, velocity (0, 250), not ducked | – | – | activity run; rate 1; `move_x` 0, `move_y` −1 → `a_RunW` |
| eye yaw 0, velocity (−100, 0) | – | – | walk; rate 1; `move_x` −1, `move_y` 0 → `a_WalkS` |
| eye yaw 0, velocity (130, 0) | – | – | walk (130 ≤ 175); rate 1.3; `move_x` stored as 1.0 |
| ducked, velocity (42.5, 0) | – | – | crouch-walk; rate 0.5; `move_x` 0.5 |
| eye pitch 350 | – | – | `body_pitch` −10 |
| standing still with a weapon (suffix PISTOL) | – | – | layer 1 `Idle_Upper_PISTOL` weight 1, cycle = main cycle; layers 3–4 unused |
| running at 250 with PISTOL, steady | – | – | layer 3 `Run_Upper_PISTOL` weight 1; layer 1 weight 0 and switched off |
| walking at 50 (ground speed 100) | – | – | layer 3 `Walk_Upper_X` weight 0.5; layer 1 weight 0.5 |

## Open questions

- CS:S's own player animation state is not public. The data strongly
  suggests it follows the template: 9-way legs on `move_x`/`move_y` (the
  only grids that use them; `move_yaw` is vestigial), `body_yaw`/
  `body_pitch` unnegated, upper layers named exactly as the template builds
  them (`Idle_Upper_`, `Run_Shoot_`, …). Unknown: the ground speeds (template
  100/250/85; the animations are authored at ≈107/150/≈67, so the template
  numbers make feet slide), the walk/run threshold (175), max body yaw (90),
  whether CS:S scales the main playback rate. Check in game: `cl_showanimstate`
  style debug is not available in retail; instead record a demo of a bot
  walking/running/strafing, and compare leg phase per second (run cycle 1.5/s
  at full pose if the playback rate is 1) and torso twist when turning in
  place (feet should start turning when twist exceeds the max body yaw and
  face front 3 s after the last turn).
- Reload: the template looks for `reload_<suffix>` (exists only for ELITES
  and M249) and falls back to `reload_m4` (missing). CS:S clearly picks
  `{Run,Walk,Idle,Crouch_Idle,Crouch_Walk}_Reload_<suffix>` by movement, and
  start/loop/end sequences for the pump shotguns. The selection and timing
  (cycle rate vs. the weapon's reload duration) are unknown.
- Grenades: CS:S has movement variants `{Move}_Shoot_GREN1/2`; the template
  uses only `idle_shoot_gren1/2`. ELITES alternate `_R`/`_L` shoot layers; the
  rule (probably by shots fired parity) is unknown.
- Jump: whether CS:S uses `Jump` exactly as §12.1 (restart on land, 0.2 s
  ignore) and whether the upper body blends differently in the air.
- Death: which of `Death1`/`deathpose_*` play, and when the ragdoll takes
  over (likely immediately; the death poses look like ragdoll seeds).
- Weapon suffixes: confirm the `PlayerAnimationExtension` value for each
  weapon from the decrypted scripts (mapping in §14 is inferred from names).
- Footstep events (7001/4001, 7002/4002) in the lower-body sequences: CS:S
  plays footsteps from movement code; whether these events do anything in
  CS:S is unknown (probably not).
- IK: lower-body animations carry IK rules and some sequences IK locks
  (feet planting, hand locks). Out of scope here; without them feet may
  float slightly on slopes and hands may drift from the weapon during
  blends.
- The server runs the anim state per tick for hitbox placement; whether CS:S
  lag compensation uses the same layers (it probably records pose
  parameters, sequence and cycle) belongs in a hitbox/lag-compensation spec.
