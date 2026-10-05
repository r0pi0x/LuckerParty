# Backlog

Things to build, roughly in priority order within each section. Shortcuts
already in the code live in [tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a plan
when work starts; delete them when done.

## dust2 fidelity

Counts are from de_dust2's entity lump and static prop lump.

- **Static prop lighting** (in progress): dust2 ships no per-vertex prop
  lighting (`.vhv`), so light each prop like the game does: the leaf ambient
  cube at the prop's lighting origin plus direct light from the map's lights
  (sun with a shadow test), baked per vertex at load.
- **Decals** (`infodecal`, 135): stains, posters, graffiti projected onto
  world surfaces. Needs decal material loading and projection onto the
  faces they touch.
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
