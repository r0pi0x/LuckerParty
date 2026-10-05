# Backlog

Things to build, roughly in priority order within each section. Shortcuts
already in the code live in [tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a plan
when work starts; delete them when done.

## Tools

- **HDR parity**: CS:S defaults to mat_hdr_level 2 on dust2 (HDR lightmaps,
  tonemapping, bloom); the reference install runs LDR. Compare and match
  both if players use HDR.
- **More refcmp views** across dust2 (mid, long, B, spawns) and other maps.

## dust2 fidelity

- **Sky only through sky surfaces**: CS:S draws the sky where sky-textured
  faces are, and black elsewhere (e.g. above nodraw ceilings in tunnels);
  we draw the sky cubemap behind everything. Seen in refcmp at
  (-1406, 900, 150), a spot inside solid where CS:S shows black.

Counts are from de_dust2's entity lump and static prop lump.

- **Baked per-vertex prop lighting (`.vhv`)** for maps that ship it (dust2
  doesn't; its props use the per-prop light probe, as in the game).
- **Remaining decals**: 6 of dust2's 135 sit on props or brush entities
  rather than world faces; decals on displacements (none on dust2).
- **Verify inferred Source rules** with the comparison tool, using a local
  copy of a map with test entities added where dust2 has no example: floor
  and ceiling decal orientation, decal reach, overall brightness/tonemapping.
- **Physics simulation** for `prop_physics_multiplayer` (75 on dust2,
  placed as static props now): pushable, shootable.
- **Brush entities**: doors and visible `func_brush` if a map needs them
  (dust2's one `func_brush` is render mode 10, never drawn).
- **Prop collision from `.phy`** for physics-solid props (now the visible
  mesh).
- **Fire** (`env_fire`, 16) and other effects, if they show in normal play.
- **Materials**: env maps (2 dust2 materials, plus map-patched ones with
  `env_cubemap`); detail blend modes other than 0 and 1; `$basetexturetransform`
  (unused on dust2).
- **Lightmap styles** (switchable lights) and bumped lightmaps.
- **Tonemap** (`env_tonemap_controller`; HDR only). Fog on ropes, and
  Source's fog curve (f²) on props (Bevy's linear fog for now).
- **Prop shadows**: CS:S draws soft dynamic shadows under physics props
  (visible in refcmp `crates_b`).
- **Sound**: `ambient_generic` and soundscapes, once there's an audio slot.
