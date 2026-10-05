# Known technical debt

Small, current, honest. Remove entries when fixed.

| Area | Debt | Why it's OK for now |
|---|---|---|
| movement | Placeholder movement has no step-up (stairs block it) and doesn't shrink the collider when crouching | It gets replaced by spec-based movement |
| movement | Ground check is one downward shape cast | Good enough until a spec defines grounding |
| client | Mouse capture fights the inspector (F1 releases it; click recaptures) | Debug-only |
| tests | `no_game_assets_tracked` blocks all .wav/.ogg/.mp3, including our own future sounds | Revisit when we have original audio |
| client | Local input overwrites the local player's `Intent` every frame, so remote tools can't drive it | Scripted intent playback is planned (docs/OBSERVABILITY.md) |
| map | Brushes carrying displacements are found by matching plane and centre (compiled maps don't record it); brush entities (func_brush, doors) aren't loaded | Matches dust2; check on other maps |
| map | Materials use only `$basetexture`: no bump maps, no WorldVertexTransition blending (displacements show their first texture), no `$basetexturetransform` | Looks right at a glance; refine with lighting |
| map | Lightmaps use style 0 only (no switchable lights), the unbumped sample only, and LDR data; brightness depends on Bevy's default camera exposure and tonemapping | Looks right on dust2; revisit if colors look off against the real game |
| map | Mipmaps are box-filtered at load instead of using the VTF's own levels; textures decode on every load (no cache yet) | dust2 still loads in well under a second |
| map | Physics-solid props collide as their visible mesh; the game uses the `.phy` collision model (not parsed yet). Box-solid props (most dust2 windows) are exact | Windows block correctly; meshes are close for crates and domes |
| map | Prop light probes: shadow rays ignore other props; samples in a leaf are blended by inverse distance (the engine's exact weighting is unknown); props are unlit materials, so dynamic lights (muzzle flashes) won't affect them | Matches the map's lightmaps at a median ratio of 1.05 (test) |
| map | Decal reach (plane within 4 units or half the decal's smaller side, in front of the face) and floor/ceiling orientation (aligned to world X) are inferred, not from a spec; overlapping decals have no defined draw order | All dust2 wall decals read correctly; floors unverified |
| map | Sky: the band of sky near the horizon differs slightly from CS:S on side views (possibly 3D-skybox fog); the down face's orientation is assumed | Sun and clouds line up; mean difference ~0.05-0.08 on sky views |
| map | Maps load synchronously before the app starts | dust2 converts in well under a second |
| deps | `vbsp` pulls `binrw` 0.14, which rustc flags as future-incompatible | Upstream; watch on Rust upgrades |
| build | Windows build not yet verified | Needs the Windows PC set up (see plans) |
