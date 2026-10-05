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

## Progress

- [x] 1 sky by leaf visibility
- [ ] 2 `.phy` collision
- [ ] 3 prop shadows
- [ ] 4 ladders and water
- [ ] 5 physics simulation
