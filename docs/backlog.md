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

Counts are from de_dust2's entity lump and static prop lump.

- **Baked per-vertex prop lighting (`.vhv`)** for maps that ship it (dust2
  doesn't; its props use the per-prop light probe, as in the game).
- **Overlay render order** (the 2-bit order field).
- **Remaining decals**: 6 of dust2's 135 sit on props or brush entities
  rather than world faces; decals on displacements (none on dust2).
- **Verify inferred Source rules** with the comparison tool, using a local
  copy of a map with test entities added where dust2 has no example: floor
  and ceiling decal orientation, decal reach, overall brightness/tonemapping.
- **Dust motes** (`func_dustmotes`, 2): the drifting particles in T house
  and inside long doors. Brush volumes that spawn slow-moving sprites;
  parameters (count, size, speed, color) come from the entity keys.
- **Physics simulation** for `prop_physics_multiplayer` (75 on dust2,
  placed as static props now): pushable, shootable.
- **Brush entities**: doors and visible `func_brush` if a map needs them
  (dust2's one `func_brush` is render mode 10, never drawn).
- **Prop collision from `.phy`** for physics-solid props (now the visible
  mesh).
- **Sprites** (`env_sprite`, 18; lamp glows).
- **Fire** (`env_fire`, 16) and other effects, if they show in normal play.
- **Materials**: env maps (2 dust2 materials, plus map-patched ones with
  `env_cubemap`); detail blend modes other than 0 and 1; `$basetexturetransform`
  (unused on dust2).
- **Lightmap styles** (switchable lights) and bumped lightmaps.
- **Fog and tonemap** (`env_fog_controller`, `env_tonemap_controller`).
- **Sound**: `ambient_generic` and soundscapes, once there's an audio slot.
