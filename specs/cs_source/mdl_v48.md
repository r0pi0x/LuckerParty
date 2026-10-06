# Counter-Strike: Source: studio model version 48 (differences from 44)

Source basis: Valve's public Source SDK 2013 only: the studio model header
(model header and its second header block, bones, linear bone table,
attachments, animation descriptions, per-bone animation records, sections,
animation blocks, sequences, events), the version-conversion helper in that
header, the animation-data lookup code (sections, blocks, stalls), the shared
bone-setup code where it falls back to zero-frame data and its Hermite
curve helper, the model byte-swapping code (which walks every structure,
including the zero-frame data), and the vertex (.vvd)
and strip (.vtx) file headers. No leaked or decompiled code was used.

Data was measured from the user's install with a throwaway decoder outside
the repo: `models/weapons/v_snip_awp.mdl` (version 48) against
`models/weapons/v_snip_scout.mdl` (version 44, same rig family), plus a scan
of every `.mdl` in `cstrike_pak_dir.vpk` and the HL2 `hl2_*_dir.vpk` files
CS:S mounts, and the external animation files (`.ani`) of the HL2 version 48
models. All test values below come from that decoder.

Extends [animation.md](animation.md) (§1–§3 there define the version 44
layout, value formats and sampling this spec refers to) and
[view_models.md](view_models.md).

Status: draft

## Summary

- Version 48 is a superset of version 44 with the same byte layout for
  everything a loader reads: header, bones, attachments, animation
  descriptions, per-bone animation records and value streams, sequences,
  events, include models, body parts/models/meshes. No offset moves, no
  record size changes, no new per-bone animation encoding.
- What differs in practice: the version number; a second header block
  that every version 48 file has (it pushes the bone array from 408 to 664,
  so offsets must be followed, never assumed); 64-bit raw quaternions are
  the common way to store constant bone rotations (version 44 CS:S player
  files used the 48-bit form); and fields used only for streamed animation
  (a "zero-frame" cache and two bone flags) that the AWP does not use.
- The AWP view model needs only: accept version 48 and handle the record
  flags 0x01, 0x08, 0x0C, 0x20 and 0x21. Our v44 sampling rules then
  decode it correctly. The companion `.vvd` (version 4) and `.vtx`
  (version 7) are identical in format to the version 44 ones.

## Units and conventions

As in animation.md: little-endian; every offset is relative to the start
of the structure that holds it unless stated; Hammer units, Z up;
quaternions (x, y, z, w); Euler angles in radians; frames at the
animation's own frames per second. Names here are ours.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| model_magic | `IDST` | 4 bytes | bytes 0–3 of every `.mdl` |
| version_offset | 4 | byte | i32 version |
| header_size | 408 | bytes | main header, same in 44 and 48 |
| header2_size | 256 | bytes | second header block (when present) |
| bone_size | 216 | bytes | unchanged |
| attachment_size | 92 | bytes | unchanged |
| animdesc_size | 100 | bytes | unchanged |
| sequence_size | 212 | bytes | unchanged |
| event_size | 80 | bytes | unchanged |
| bodypart / model / mesh size | 16 / 148 / 116 | bytes | unchanged |
| bone_flag_saveframe_pos | 0x00200000 | – | bone has zero-frame positions (version 48 only) |
| bone_flag_saveframe_rot | 0x00400000 | – | bone has zero-frame rotations (version 48 only) |
| section_frames_seen | 30 | frames | frames per section in every sectioned v48 animation found |
| ani_magic | `IDAG` | 4 bytes | external animation file |
| vvd_version | 4 | – | `.vvd`, both model versions |
| vtx_version | 7 | – | `.vtx` (dx90, dx80, sw), both model versions |

## Behavior

### 1. Detecting the version

Read the i32 at byte 4 after checking the `IDST` magic. Versions found in
the install:

| Package | 44 | 45 | 46 | 47 | 48 |
|---|---|---|---|---|---|
| `cstrike_pak` (CS:S's own) | 1406 | 0 | 0 | 0 | 5 |
| `hl2_misc` (HL2 content CS:S mounts) | 2069 | 6 | 9 | 2 | 65 |

A loader for CS:S content needs 44 and 48. Versions 45–47 appear only in
HL2 content (lists under "Version 48 models in the install"); the rules that make them loadable are in §9.

### 2. Header

Bytes 0–407 have the same layout in both versions (animation.md §1 gives
the fields we read; all of them keep their offsets). Fields whose
content differs:

| Offset | Type | Version 44 (CS:S files) | Version 48 |
|---|---|---|---|
| 380 | i32 | must be treated as junk if version < 47 (older tools stored a cache there); 0 in all CS:S files | 0 (reserved) |
| 384, 388 | i32, i32 | 0, 0 | count and offset of flex-controller UI entries (0 entries in CS:S files; offset may be nonzero with count 0) |
| 392 | f32 | 0 | fixed-point scale for vertex animation, meaningful only if header flag 0x00200000 is set (never in CS:S) |
| 400 | i32 | 0 in every CS:S v44 file (53 HL2 v44 files have one) | offset of the second header block from file start; 408 in all 70 v48 files |

Because the second header block sits right after the main header, the bone
array of a version 48 file starts at 664 (AWP) instead of 408 (scout). A
reader that follows the bone offset at header 160 needs no change.

Second header block (256 bytes; offsets relative to its own start):

| Offset | Type | Meaning |
|---|---|---|
| +0, +4 | i32, i32 | count and offset of source-bone transforms (0 in CS:S files) |
| +8 | i32 | attachment that replaces the header's illumination position (0 in the AWP, which then uses the header's position; how nonzero values are numbered is not visible in the public SDK) |
| +12 | f32 | maximum eye deflection, as a cosine (0 = default cos 30° = 0.866) |
| +16 | i32 | offset of the linear bone table (0 = none) |
| +20 | i32 | offset of a name string (0 = none; use the 64-byte name at header 12) |
| +24, +28 | i32, i32 | count and offset of bone-driven flex drivers (0 in CS:S files) |
| +32 | 56 × i32 | reserved, all 0 |

### 3. Bones and the linear bone table

- Bone records are unchanged (animation.md §1). Version 48 adds two flag
  bits, 0x00200000 and 0x00400000, marking which bones have entries in the
  zero-frame data (§6). They are set only on HL2 models with streamed
  animation; no CS:S v48 bone has them.
- The linear bone table (second header +16) is a copy of the bone array
  stored as parallel arrays, for cache-friendly bone setup. Layout
  (offsets relative to the table start): +0 i32 bone count; then nine i32
  array offsets, in order: flags (u32 per bone), parent (i32), default
  position (3 f32), default quaternion (4 f32), default Euler angles (3 f32),
  bind-to-bone matrix (12 f32), position scale (3 f32), rotation scale
  (3 f32), alignment quaternion (4 f32); then 6 reserved i32. Every value
  equals the bone array's. A loader may ignore the table.
- The bone-by-name table (header 364, one byte per bone, bone indices
  sorted by case-insensitive name) is the same in both versions.

### 4. Attachments, events, include models

Unchanged.

- Attachment (92 bytes): +0 name offset, +4 flags, +8 bone, +12 a 3×4
  matrix (rows of 4 f32: rotation columns plus translation in the 4th
  column), +60 8 reserved i32.
- Events (80 bytes): +0 cycle (f32), +4 event number, +8 type, +12 64-byte
  options string, +76 offset of an event name (0 = none). Both versions
  already use named events (type bit 1024 with a name such as
  `AE_CLIENT_EFFECT_ATTACH`, event number 0).
- Include models: 8-byte entries at header 336/340 as in animation.md §8.

### 5. Animation descriptions

Same 100-byte record; the fields in animation.md §1 keep their offsets and
meaning. Additional fields:

| Offset | Type | Meaning |
|---|---|---|
| +28 … +51 | 6 × i32 | version 48: 0. Version 44 files may hold leftover values here (the scout's look like a bounding box: −11.49, −3.34, −10.71, 26.96, 11.64, −1.91). Ignore in both. |
| +60, +64 | i32, i32 | IK rule count and offset (in this file) |
| +68 | i32 | IK rule offset inside the animation's external block |
| +72, +76 | i32, i32 | local-hierarchy count and offset |
| +88 | i16 | zero-frame span: frames between stored zero frames |
| +90 | i16 | zero-frame count |
| +92 | i32 | zero-frame data offset, relative to this description (0 = none) |
| +96 | f32 | runtime scratch; ignore |

The fields at +88…+99 are only valid in version 48 files (§9).

### 6. Zero-frame data (version 48, streamed models only)

Purpose: when a model's animations live in an external `.ani` file that is
loaded on demand, the `.mdl` keeps a coarse copy of each animation's first
frames so playback can start before the block arrives.

Layout, starting at description + (zero-frame offset), with C = zero-frame
count: for each bone in bone order, if the bone has flag 0x00200000, C
48-bit half-float position vectors (6 bytes each); then, if the bone has
flag 0x00400000, C 64-bit quaternions (8 bytes each). Formats as in
animation.md §2. Entry j (0 ≤ j < C) is the bone's local transform as the
animation decodes it (animation.md §3) at frame min(j · span, N − 1).

Check: on `models/crow.mdl` (all 14 animations with zero frames) every
entry matched the decoded animation at frame j · span within 0.032° of
rotation and 0.024 units of position.

An implementation that loads all animation data up front can ignore zero
frames. For reference, the stock fallback when an animation's records are
not available (block not loaded yet, or missing):

1. Every bone used by the sequence starts at its default (identity / zero
   for delta animations), as in animation.md §3 step 3.
2. If the animation has zero-frame data, bones with save-frame flags are
   overwritten from it at the fractional frame F = cycle · (N − 1):
   - C = 1: the single stored value.
   - C ≥ 2: i = ⌊F / span⌋; if i ≥ C − 1 then i = C − 2 and s = 1, else
     s = clamp((F − i · span) / span, 0, 1). Take p0 = entry max(i − 1, 0),
     p1 = entry i, p2 = entry min(i + 1, C − 1) and evaluate a three-point
     Hermite curve from p1 to p2 with tangents d1 = p1 − p0, d2 = p2 − p1:
     result = h1·p1 + h2·p2 + h3·d1 + h4·d2, h1 = 2s³ − 3s² + 1,
     h2 = 1 − h1, h3 = s³ − 2s² + s, h4 = s³ − s². Positions use this per
     axis. Quaternions first align p0 and p1 to p2 (animation.md §4 Align),
     apply it per component, then normalize.
   - So the cache only covers frames 0 … (C − 1) · span; later frames hold
     the last stored entry.
3. When a block arrives after a stall, the real pose is cross-faded with the
   zero-frame pose for 0.2 s (zero-frame weight = smoothstep of
   clamp((0.2 − elapsed) · 5, 0, 1)). A sectioned animation whose current
   section is missing but an earlier one is loaded plays the last frame of
   the latest loaded earlier section instead.

### 7. Per-bone animation records and value streams

Unchanged from animation.md §1–§3: same 4-byte record header, same flags,
same payload order, same run-length value streams, same half-float,
48-bit and 64-bit quaternion formats, same "raw data holds for every frame"
rule, same default/delta handling.

What the AWP uses (distinct records per animation, counted once):

| Animation | Frames | Records | 0x20 | 0x21 | 0x01 | 0x08 | 0x0C |
|---|---|---|---|---|---|---|---|
| `@awm_idle` | 11 | 35 | 31 | 3 | 1 | 0 | 0 |
| `@awm_fire` | 42 | 39 | 6 | 1 | 0 | 27 | 5 |
| `@awm_draw` | 31 | 38 | 18 | 2 | 0 | 15 | 3 |
| `@awm_reload` | 111 | 40 | 4 | 1 | 0 | 29 | 6 |

- 0x20: constant 64-bit quaternion, no position data (position = bone
  default). 0x21: constant 64-bit quaternion followed by a constant
  half-float position; the position starts 8 bytes into the payload.
- 0x01 alone: constant position, rotation = bone default.
- Raw values replace the default (they are not added to it): the AWP's
  idle stores rotations up to 93° away from the bone defaults.
- `@awm_idle` is entirely constant records: every frame decodes to the
  same pose (the scout's idle is also static).
- Bones without a record keep their default (non-delta animations; the AWP
  has no delta animations). 21–26 of the 61 bones have no record per
  animation.
- HL2 v48 models also use 0x11, 0x30 (64-bit raw quaternion, delta) and
  0x31 in their in-file animations (their `.ani` data was not surveyed). No
  record seen mixes raw and animated data.
- An animation can have an empty record list (first record's bone byte is
  255); `hay_bails.mdl` and `hay_bail_stack.mdl` have one. The pose is
  then all bone defaults.

### 8. Sections and external animation blocks

The lookup rule of animation.md §1 ("Where the per-bone records start")
holds unchanged for version 48. Verified on HL2 v48 models: decoding across
section boundaries (frame 29 → 30, 59 → 60, …, and the separately stored
last frame) gives steps no larger than inside a section.

- Sectioned animations have ⌊N / S⌋ + 2 section entries (8 bytes: block,
  offset); the last entry holds the final frame alone. All 365 sectioned
  v48 animations found use S = 30. Sections of one animation may point at
  different blocks.
- External file (`.ani`, named by the string at header 348, relative to the
  game folder; backslashes may appear): bytes 0–3 `IDAG`, i32 version at 4
  (48), i32 file length at 76; the rest of the leading header is 0. Block
  table (header 352/356; 8-byte entries: start, end, absolute byte offsets
  in the `.ani`): entry 0 is a placeholder (0, 0) standing for "this
  `.mdl`"; real blocks start at entry 1 (the crow's first block starts at
  byte 416). Records of block b start at (block start) + (description or
  section offset).
- Block −1 means the data is missing (the model must be recompiled): there
  are no records, so the animation decodes as bone defaults (plus zero
  frames if present, §6).
- No CS:S-pak model of either version uses sections or external blocks:
  the five CS:S v48 models keep everything in block 0, unsectioned.

### 9. Loading versions 45–47

Only HL2 content; included for completeness.

- Versions below 46 (in practice 45, the only version that wrote them): a
  sectioned animation (frames per section ≠ 0) has unusable data. Treat it
  as one frame, block −1 (missing). No CS:S v44 animation is sectioned, so
  this does not affect animation.md.
- Versions below 47: header 380 and the zero-frame fields (+88…+99) are
  junk; treat as no zero-frame data.
- Version 47: zero-frame data was in an incompatible layout; ignore it.
- Everything else reads as version 48.

### 10. Sequences

Same 212-byte record; every field in animation.md and view_models.md keeps
its offset. In version 48, +184 (i32 offset) and +188 (i32 count) describe
activity modifiers (4-byte entries, each the offset of a name
string); 0 in every CS:S file of both versions. The other 5 i32 after them are reserved (0).

### 11. Mesh: body parts, `.vvd`, `.vtx`

Identical in both versions:

- Body part (16), model (148: +64 type, +68 radius, +72/+76 mesh count and
  offset, +80 vertex count, +84 vertex byte offset into the `.vvd` vertex
  block (48 bytes per vertex), +88 tangent byte offset (16 per tangent)),
  mesh (116: +0 material, +8 vertex count, +12 vertex offset within the
  model, +32 mesh id, +52 per-LOD vertex counts).
- `.vvd`: `IDSV`, version 4, checksum, LOD count, 8 per-LOD vertex counts,
  fixup count and offset, vertex data offset, tangent data offset.
- `.vtx`: version 7, vertex cache size, max bones per strip (u16), max bones
  per triangle (u16), max bones per vertex (3), checksum, LOD count,
  material-replacement offset, body-part count and offset.
- The checksum (`.mdl` byte 8) must equal the `.vvd` and `.vtx` checksums.

## Per-load order

1. Check `IDST`; read the version. Accept 44 and 48 (45–47 with §9).
2. Read the header fields of animation.md §1 at their usual offsets. Never
   assume the bone array, or anything else, starts right after the header.
3. Optionally read the second header block (header 400 ≠ 0); nothing in it
   is needed for the AWP.
4. Read bones, attachments, animation descriptions, sequences, events,
   pose parameters and includes exactly as for version 44.
5. For each animation: if version 48 and zero-frame data is present, it
   may be skipped. Locate records (block, section) as in animation.md §1.
   Decode records with flags 0x01, 0x02, 0x04, 0x08, 0x10, 0x20 and their
   combinations.
6. Load the mesh from `.vvd`/`.dx90.vtx` as for version 44.

## Edge cases

- The flex-controller UI offset (header 388) can be nonzero with a zero
  count (AWP: 42352, count 0). Do not use it as evidence of content.
- The animation-block name offset (header 348) points at an empty string
  when there are no blocks (AWP and scout both).
- The scout and AWP attachments `1` and `2` sit on different bone indices
  (scout: bone 1 `v_weapon.scout_Parent`; AWP: bone 0
  `v_weapon.awm_parent`). Look attachments up by name and resolve the bone
  index per model.

## Quirks

- The AWP's bones and animations are named for the "AWM" (`v_weapon.awm_*`,
  `@awm_fire`); the model and its sounds are `v_snip_awp` / `Weapon_AWP.*`.
- The AWP's first bone is the gun (`v_weapon.awm_parent`, a root); the
  hands hang under a second root `v_weapon` (bone 3). The scout has one
  root.
- The `.mdl` internal name (header 12) uses a backslash:
  `weapons\v_snip_awp.mdl`.

## Test cases

All from the user's install (`cstrike_pak`), decoded with the rules above
and animation.md §1–§3.

| Setup | Input | After | Expected |
|---|---|---|---|
| `v_snip_awp.mdl` header | read | – | magic `IDST`, version 48, checksum 1463800543, file length 52756 (byte 76) = file size; flags 1 |
| same | counts | – | 61 bones at 664; 2 attachments at 13840; 4 animations at 15324; 4 sequences at 40276; 0 includes; 0 pose params; 0 animation blocks; 2 body parts; 3 textures |
| same | second header | – | at 408; source-bone transforms 0; illumination attachment 0; eye deflection 0; linear bone table at 408 + 42500 = 42908, 61 bones, equal to the bone array field by field |
| `v_snip_scout.mdl` header | read | – | version 44; second header offset 0; bones at 408 (62 bones) |
| AWP bones | read | – | bone 0 `v_weapon.awm_parent` (root), 1 `…awm_clip` (parent 0), 2 `…awm_bolt_action` (parent 0), 3 `v_weapon` (root), 6 `v_weapon.Left_Arm`, 34 `v_weapon.Right_Arm`, 37 `v_weapon.Right_Hand`; no bone has flags 0x00600000 |
| AWP bone 0 defaults | read | – | position (5.0353, −9.6803, −1.0282); Euler (1.5708, 0, −0.0392); quaternion (0.7070, −0.0139, −0.0139, 0.7070); position scale 0.0039064 on all axes |
| AWP attachments | read | – | `1` on bone 0, translation (0, 3.5, 28), identity rotation; `2` on bone 0, translation (0, 3.5, 4.2), rotation rows (−0.940, 0, −0.342), (0, −1, 0), (−0.342, 0, 0.940) |
| AWP animations | read | – | `@awm_idle` 11 frames, `@awm_fire` 42, `@awm_draw` 31, `@awm_reload` 111; all 30 fps, flags 0, block 0, unsectioned, no zero frames; record offsets (from file start) 15728, 16176, 22656, 25520 |
| AWP sequences | read | – | `awm_idle` ACT_VM_IDLE flags 0; `awm_fire` ACT_VM_PRIMARYATTACK flags 2; `awm_draw` ACT_VM_DRAW flags 2; `awm_reload` ACT_VM_RELOAD flags 0; all 1×1 grids on animations 0–3, fade 0.2/0.2, activity weight 1 |
| `awm_fire` events | read | – | cycle 0: 5001 `1`; 0.3659, 0.4634, 0.7805: 5004 `Weapon_AWP.Bolt`; 0.6341: event 0, type 1024, name `AE_CLIENT_EFFECT_ATTACH`, options `EjectBrass_338Mag 2 70` |
| `awm_reload` events | read | – | 0.2364 5004 `Weapon_AWP.Clipout`; 0.4545 5004 `Weapon_AWP.Clipin`; 0.8091 5004 `Weapon_AWP.Bolt` |
| `@awm_fire` record flags | read | – | per §7 table: 39 records, 0x20 ×6, 0x21 ×1, 0x08 ×27, 0x0C ×5 |
| `@awm_fire` frame 0 | decode | – | bone 0 (0x0C): q (0.5000, 0.5000, 0.5000, 0.5000), p (9.1839, 5.5584, −7.0011); bone 2 (0x0C): q (0, 0, 0, 1), p (0, 3.4670, 1.1766); bone 3 (0x21): q (0.5, 0.5, 0.5, 0.5), p (−7.0000, 2.9980, −8.6953); bone 6: q (0.1233, 0.5834, 0.5825, 0.5524); bone 34: q (0.1122, 0.7034, −0.6770, −0.1851); bone 37: q (0.0133, −0.2681, −0.0376, 0.9626) |
| `@awm_fire` frame 10 | decode | – | bone 0: q (0.5181, 0.4958, 0.5081, 0.4771), p (11.2582, 5.8709, −7.3214); bone 2: q (0, 0, 0.0893, 0.9960); bone 3 unchanged from frame 0; bone 34: q (0.2438, 0.7463, −0.5630, −0.2581); bone 37: q (−0.0409, −0.4096, 0.0667, 0.9089) |
| `@awm_fire` frame 11 | decode | – | bone 0: q (0.5259, 0.4916, 0.5088, 0.4721), p (11.6566, 5.8358, −7.3448); bone 2: q (0, 0, 0.0865, 0.9963) |
| `@awm_fire` cycle 0.25 | sample | – | frame 10.25: bone 0 q (0.5200, 0.4947, 0.5083, 0.4759), p (11.3578, 5.8621, −7.3273); bone 2 q (0, 0, 0.0886, 0.9961); bone 37 q (−0.0384, −0.3946, 0.0802, 0.9145) |
| `@awm_fire` frame 41 | decode | – | equals frame 0 for bones 0, 2, 3, 6, 34, 37 (the loop closes) |
| `@awm_reload` frame 55 | decode | – | bone 0: q (0.0546, 0.6349, −0.2108, 0.7413), p (9.6370, 3.4021, −3.6807); bone 6: q (−0.2091, −0.0461, 0.7534, 0.6217); bone 34: q (0.3191, 0.6812, 0.6587, 0.0150) |
| `@awm_idle` | decode all frames | – | identical pose every frame; every decoded quaternion has length 1 ± 3·10⁻⁸ |
| AWP `.vvd` | read | – | `IDSV`, version 4, checksum 1463800543, 1 LOD, 2811 vertices, vertex data at 64, tangents at 134992 |
| AWP `.dx90.vtx` | read | – | version 7, cache 24, 53 bones per strip, 9 per triangle, 3 per vertex, checksum 1463800543, 1 LOD, 2 body parts |
| AWP meshes | read | – | body part `studio`: model `awm_reference.smd`, 1525 vertices, meshes of 13 (material 0) and 1512 (material 1); body part `hands`: 1286 vertices at vertex byte offset 73200, material 2 |
| `models/items/cs_gift.mdl` | read | – | version 48, 1 bone `root`, 1 animation `@spin` 31 frames at 10 fps, looping, a single animated-rotation (0x08) record |
| `hay_bails.mdl` | decode `@idle` | – | empty record list; pose = bone default |
| `models/crow.mdl` (HL2, v48) | zero frames of `@Idle01` | – | span 9, count 3; 30 frames per section, 9 section entries all in block 1; zero frame j matches frame 9j within 0.032° / 0.024 units |

## Version 48 models in the install

CS:S's own package (`cstrike_pak`), 5 files:
`models/weapons/v_snip_awp.mdl`, `models/items/cs_gift.mdl`,
`models/props/de_inferno/hay_bails.mdl`,
`models/props/de_inferno/hay_bail_stack.mdl`,
`models/props_wasteland/rockcliff_cluster01b.mdl`. Only the AWP has more
than one bone; the hay bales and the rock cliff are single-bone static props
with one 1-frame animation (their meshes already load through the mesh
path, which does not check the version).

HL2 content CS:S mounts (`hl2_misc`), 65 files, all with external `.ani`
blocks and zero-frame data: advisor; alyx_animations, alyx_gestures,
alyx_postures; antlion, antlion_guard; barnacle; barney_animations,
barney_gestures, barney_postures; breen_anims, breen_gestures,
breen_postures; combine_dropship_animations; combine_soldier_anims;
combine_strider; crow; dog_animations, dog_gestures, dog_postures;
effects/teleporttrail_alyx; eli_anims, eli_gestures, eli_postures; gman,
gman_gestures; headcrab, headcrabblack, headcrabclassic;
humans/female_gestures, female_postures, female_shared, female_ss,
humans/male_ss; ichthyosaur; kleiner_animations, kleiner_gestures,
kleiner_postures; lamarr; leech; monk_animations; mortarsynth;
mossman_anims, mossman_gestures, mossman_postures; pigeon;
police_animations, police_ss; props_combine/breen_arm, citadel_pods,
combine_citadel_animated, pod_extractor; props_lab/scanner1_scrapyard,
scanner2_scrapyard; seagull; shield_scanner; stalker; synth;
vortigaunt_anims, vortigaunt_gestures, vortigaunt_postures;
zombie/classic, classic_torso, fast, poison (all under `models/`).

Versions 45–47 (HL2 content only): 45: alyx, humans/group03/male_08,
male_09, props_c17/furnituretable003a, props_combine/health_charger001,
weapons/w_smg1; 46: combine_apc_destroyed_gib01, combine_soldier,
combine_super_soldier, gunship, items/combine_rifle_ammo01,
props_c17/furnitureshelf002a, vortigaunt, vortigaunt_slave,
weapons/v_pistol; 47: airboat, buggy.

## Open questions

- The "entry j = frame j · span" meaning of zero frames is measured (the
  public code only shows how entries are interpolated). Irrelevant if all
  data is loaded up front.
- The meaning of the leftover values at animation description +28…+51 in
  version 44 files is not documented in the public SDK (they look like a
  bounding box). Nothing reads them.
- Why the `.ani` header is 416 bytes before the first block (only the
  magic, version and length are set) is unknown; the block table gives
  the real offsets, so it does not matter.
- Not checked against the running game: whether the decoded AWP poses look
  right in CS:S (e.g. the hands grip the gun in `awm_idle`). A screenshot
  comparison of our view model against CS:S's would confirm it.
