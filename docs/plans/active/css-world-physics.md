# CS:S world: sky surfaces, prop collision and physics, shadows, ladders, water

Started 2026-10-05. Five backlog items, in this order (each its own commit,
with tests):

1. **Sky by leaf visibility.** The spec showed the premise was wrong: CS:S
   draws the sky full-screen behind the world whenever the camera's BSP
   leaf sees sky, and clears to black only inside solid. Implemented as a
   per-frame leaf lookup (`MapSkyVis`) that switches the sky camera and
   its 3D layer. Done; refcmp `in_solid` matches.
   - Spec: `specs/cs_source/shadows_sky.md`.
   - Found on the way: vbsp re-sorts leaves by cluster, so prop ambient
     samples were placed with the wrong leaf bounds. Leaves are now read
     raw (`ambient::raw_leaves`).
2. **Prop collision from `.phy`.** A loader for the collision models next to
   `.mdl` files (convex pieces), used for physics-solid static props,
   `prop_physics*` and `prop_dynamic`. Convex pieces become exact brushes
   for movement, as convex props do now. Replaces the visible-mesh and
   hull fallbacks.
   - Spec: `specs/cs_source/physics_props.md` (format and behaviour).
   - Check: every dust2 prop with a `.phy` gets pieces; the crate and
     barrel cases from `props_cannot_be_climbed_by_walking`.
3. **Prop shadows.** Dynamic shadows under physics props (and dynamic
   props, if the spec says so), projected along the map's shadow
   direction.
   - Spec: the shadow section from the shaders agent.
   - Check: refcmp `crates_b`.
4. **Ladders and water** in Source movement.
   - Spec: `specs/cs_source/movement.md` (ladder and water sections).
   - Check: the spec's test cases as scenario tests, plus a ladder and a
     pool in a built test map.
5. **Physics simulation** of `prop_physics_multiplayer`: avian dynamic
   bodies from the `.phy` pieces, with mass, damping and friction from
   the spec, pushed by players and by damage.
   - Spec: `specs/cs_source/physics_props.md`.
   - Check: scenario tests (a prop falls and settles, a player push moves
     it, it sleeps).

## Also done along the way

- `movecmp fuzz` (ladders on de_nuke, water) against a real CS:S server:
  water now matches tick-exact; CS:S's duck rule, 260 swim lift and
  walking (+speed) measured and matched. Walking added.

## Progress

- [x] 1 sky by leaf visibility
- [x] 2 `.phy` collision (props collide by their convex pieces; spec crate cases pass)
- [x] 3 prop shadows (entity props cast; built at load; refcmp crates_b matches; recomputed per frame once props move)
- [x] 4 ladders and water (spec cases in tests/it/source_movement_ladder_water.rs; de_nuke climb test waits on step 2: its ladder models' rungs block until props collide by `.phy`)
- [x] 5 physics simulation (bodies from .phy mass + surfaceprops, sv_gravity, speed clamps; multiplayer push-away: players pass through, shove props, solid ones push back; shadows follow)
