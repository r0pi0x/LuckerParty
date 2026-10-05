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
| map | 3D skybox content is found by the world's world_mins/world_maxs (computed by the map compiler without the skybox), with a 64-unit margin, not by BSP areas (the sky camera sits in area 0 with solid leaves) | Correct on dust2 |
| map | Sky: the down face's orientation is assumed (never visible in the fitted views) | Rarely visible on dust2 |
| map | Ropes (from specs/cs_source/ropes.md) are drawn at their settled shape: no wind sway. Rope gravity 800 is measured, not specced; dust2's 10-node cable still hangs ~11 px low in the sky_n view. Node light uses our prop probe averaged over six directions (the engine's point-light query is not public) | The 5-node cables match CS:S to 1-2 px |
| movement | Source movement (specs/cs_source/movement.md) lacks water, ladders (KZ maps need them), base velocity, view punch and fall damage (landing speed is recorded). CS:S values, hulls, ducking, stamina and the bunny-hop cap are measured with movecmp/the probe. Not modelled: the bots' automatic jump-duck (bots only). An exactly 18-unit drop is at the edge of the stick-to-ground reach (unmeasured). Max speed fixed at the knife's 250 until a weapon slot exists; console variables only at launch (`--cvar`, `--exec`), no in-game console yet | Walking, stopping, jumping and crouch-walking match CS:S tick for tick |
| movement | Source movement sweeps displacements and concave or rounded props (182 of dust2's 295) with avian shape casts; world brushes and convex props (hull within 1 cm of 75% of the surface, from the visible mesh, not `.phy`) are swept exactly | Exact on crates and boxes; terrain matches CS:S in movecmp |
| map | Maps load synchronously before the app starts | dust2 converts in well under a second |
| deps | `vbsp` pulls `binrw` 0.14, which rustc flags as future-incompatible | Upstream; watch on Rust upgrades |
| build | Windows build not yet verified | Needs the Windows PC set up (see plans) |
