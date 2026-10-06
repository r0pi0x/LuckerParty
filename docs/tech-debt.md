# Known technical debt

Small, current, honest. Remove entries when fixed.

| Area | Debt | Why it's OK for now |
|---|---|---|
| map | Soundscape loops play from their loop point only (no intro), and positioned loops get distance falloff but no panning | dust2's positioned loops are distant music |
| map | World reflections take the bumped normal through a screen-space tangent frame (no per-vertex tangents) and sample the cubemap's top mip only; CS:S's highlight is slightly broader (refcmp tunnel_floor_e, abs diff 0.021) | Visually matches; per-vertex tangents if it matters |
| client | Shot effects are placeholders: tracer streaks (speed and frequency guessed) and dark discs for bullet marks, on static surfaces only | Until decals by surface and the games' sprites load |
| weapon | Characters' bodies and hitboxes stay in the model's reference pose (no animation, no crouch); maps without character models fall back to height bands on the collider | Static targets match CS:S's boxes; animation is a later slice |
| weapon | CS:S's spread uses our own RNG, not Source's seeded generator (spec Q1); the bullet impulse is unmeasured (Q8); no penetration, water or lag compensation; armour running out mid-hit is clamped (unmeasured); landing inaccuracy is approximated (+InaccuracyLand) and the airborne rest value assumed | The rest follows the spec's measured values |
| bot | Bot awareness is ours, not CS:S's (not in the SDK): sight, hearing by each sound's falloff gain, 15 s memory, random roaming; route following is a simple point follower (no gap jumps, ladders untested, no obstacle avoidance); maps without a `.nav` leave bots standing | Plays fairly for deathmatch practice |
| movement | Placeholder movement has no step-up (stairs block it) and doesn't shrink the collider when crouching | It gets replaced by spec-based movement |
| movement | Ground check is one downward shape cast | Good enough until a spec defines grounding |
| client | Mouse capture fights the inspector (F1 releases it; click recaptures) | Debug-only |
| tests | `no_game_assets_tracked` blocks all .wav/.ogg/.mp3, including our own future sounds | Revisit when we have original audio |
| client | Local input overwrites the local player's `Intent` every frame, so remote tools can't drive it | Scripted intent playback is planned (docs/OBSERVABILITY.md) |
| map | Brushes carrying displacements are found by matching plane and centre (compiled maps don't record it); brush entities (func_brush, doors) aren't loaded | Matches dust2; check on other maps |
| map | Materials use only `$basetexture`: no bump maps, no WorldVertexTransition blending (displacements show their first texture), no `$basetexturetransform` | Looks right at a glance; refine with lighting |
| map | Lightmaps use style 0 only (no switchable lights) and LDR data; CS:S's default HDR (mat_hdr_level 2) isn't matched | LDR matches the game within ~1% luma on dust2 (refcmp); revisit for switchable lights or HDR |
| map | Mipmaps are box-filtered at load instead of using the VTF's own levels; textures decode on every load (no cache yet) | dust2 still loads in well under a second |
| map | Physics-solid props collide as their visible mesh; the game uses the `.phy` collision model (not parsed yet). Box-solid props (most dust2 windows) are exact | Windows block correctly; meshes are close for crates and domes |
| map | Prop light probes: shadow rays ignore other props; samples in a leaf are blended by inverse distance (the engine's exact weighting is unknown); props are unlit materials, so dynamic lights (muzzle flashes) won't affect them | Matches the map's lightmaps at a median ratio of 1.05 (test) |
| map | Decal reach (plane within 4 units or half the decal's smaller side, in front of the face) and floor/ceiling orientation (aligned to world X) are inferred, not from a spec; overlapping decals have no defined draw order | All dust2 wall decals read correctly; floors unverified |
| map | 3D skybox content is found by the world's world_mins/world_maxs (computed by the map compiler without the skybox), with a 64-unit margin, not by BSP areas (the sky camera sits in area 0 with solid leaves) | Correct on dust2 |
| map | Sky: the down face's orientation is assumed (never visible in the fitted views) | Rarely visible on dust2 |
| map | Ropes (from specs/cs_source/ropes.md) are drawn at their settled shape: no wind sway. Rope gravity 800 is measured, not specced; dust2's 10-node cable still hangs ~11 px low in the sky_n view. Node light uses our prop probe averaged over six directions (the engine's point-light query is not public) | The 5-node cables match CS:S to 1-2 px |
| movement | Source movement (specs/cs_source/movement.md) lacks water, ladders (KZ maps need them), base velocity, view punch and fall damage (landing speed is recorded). CS:S values, hulls, ducking, stamina and the bunny-hop cap are measured with movecmp/the probe. Not modelled: the bots' automatic jump-duck (bots only). An exactly 18-unit drop is at the edge of the stick-to-ground reach (unmeasured). Max speed comes from the held weapon (`MaxSpeed`) | Walking, stopping, jumping and crouch-walking match CS:S tick for tick |
| movement | Source movement sweeps displacements and concave or rounded props (182 of dust2's 295) with avian shape casts; world brushes and convex props (hull within 1 cm of 75% of the surface, from the visible mesh, not `.phy`) are swept exactly | Exact on crates and boxes; terrain matches CS:S in movecmp |
| map | Sprites: glow visibility is 5 line tests to the occlusion proxy (the game uses GPU occlusion queries) with no fade-in; sprites add in gamma space by reading the scene behind them, so they draw in Bevy's transmissive phase (before translucent surfaces, unlike the game's glows) and overlapping sprites each add their own difference; render modes other than 3, 5 and 9 aren't drawn | dust2's lamp glows match CS:S closely (refcmp glow_lamp) |
| map | Dust motes spawn in the brush model's bounds (the game keeps only points inside its brushes; dust2's volumes are boxes); no wind entities yet (dust2 has none) | Matches the spec on dust2 |
| sound | A prop surface name the surface scripts don't define (dust2's 19 "stone" rock props) steps as "default"; the sound spec doesn't say what the engine does with unknown names | Assumed from "default" being every surface's base; check by ear in CS:S |
| map | Maps load synchronously before the app starts | dust2 converts in well under a second |
| deps | `vbsp` pulls `binrw` 0.14, which rustc flags as future-incompatible | Upstream; watch on Rust upgrades |
| build | Windows build not yet verified | Needs the Windows PC set up (see plans) |
