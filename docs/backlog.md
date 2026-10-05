# Backlog

Things to build, roughly in priority order within each section. Shortcuts
already in the code live in [tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a plan
when work starts; delete them when done.

## Tools

- **Reference comparison against real CS:S** (in progress): script the real
  game (console `setpos`/`setang`/`jpeg`) and our engine to capture the same
  views, then produce side-by-side and difference images with brightness
  stats. Runs on the Linux box with the Steam client.

## dust2 fidelity

Counts are from de_dust2's entity lump and static prop lump.

- **Baked per-vertex prop lighting (`.vhv`)** for maps that ship it (dust2
  doesn't; its props use the per-prop light probe, as in the game).
- **Remaining decals**: 6 of dust2's 135 sit on props or brush entities
  rather than world faces; decals on displacements (none on dust2).
- **Verify inferred Source rules** with the comparison tool, using a local
  copy of a map with test entities added where dust2 has no example: floor
  and ceiling decal orientation, decal reach, overall brightness/tonemapping.
- **Dust motes** (`func_dustmotes`, 2): the drifting particles in T house
  and inside long doors. Brush volumes that spawn slow-moving sprites;
  parameters (count, size, speed, color) come from the entity keys.
- **Sky**: 2D skybox from the `skyname` world key, and the 3D skybox
  (`sky_camera`) drawn scaled around the camera instead of at 1:1 off to the
  side.
- **Physics props** (`prop_physics_multiplayer`, 75): barrels and similar,
  loaded as props first, physically simulated later.
- **Brush entities**: `func_brush` (1); doors if any map needs them.
- **Prop collision from `.phy`** for physics-solid props (now the visible
  mesh).
- **Ropes** (`move_rope`, 25) and **sprites** (`env_sprite`, 18; lamp glows).
- **Fire** (`env_fire`, 16) and other effects, if they show in normal play.
- **Materials**: bump maps, WorldVertexTransition blending on displacements,
  `$basetexturetransform`, the VTF's own mip levels.
- **Lightmap styles** (switchable lights) and bumped lightmaps.
- **Fog and tonemap** (`env_fog_controller`, `env_tonemap_controller`).
- **Sound**: `ambient_generic` and soundscapes, once there's an audio slot.
