# Performance

How to measure, what the numbers were, and what the map visibility culling
does. Tools are listed in [OBSERVABILITY.md](OBSERVABILITY.md) ("3c.
Performance").

## Measuring

- `mashup_perf 1` in the console: frame times, main-world CPU, GPU time
  per render pass (Vulkan/DX12 timestamps), entities, meshes and
  triangles drawn, the camera's visibility cluster. `mashup_perf_log 1`
  logs it as text once a second; `bugreport` saves it in report.txt.
- `refcmp bench --views tools/refcmp/<map>.toml`: 200 frames per view
  after settling, vsync off, offscreen at 1280x720; prints avg/p95/max
  frame ms, drawn meshes/total, triangles drawn, map parts potentially
  visible. Build the profile you want first (`cargo build --features dev`,
  or `cargo build --profile playtest` and run `target/playtest/refcmp`).
- A/B: `-- +r_novis 1` (no PVS culling) or `MASHUP_MERGED_WORLD=1` (the
  world as one mesh per material, nothing culled: the code before
  chunking).
- Per-system CPU time: a `--features profile` build writes a Chrome
  trace (`TRACE_CHROME=<file>`, else `trace-*.json` in the working
  directory); `tracesum <file> --skip 200` prints the main and render
  worlds' time per frame and the top spans by self time (OBSERVABILITY.md
  3c has the commands); https://ui.perfetto.dev shows the timeline.
- `REFCMP_OUT=<dir>` keeps refcmp's captures and reports in your own
  folder (the CS:S references stay shared).

Frame times move with machine load (other builds on the dev box): compare
runs taken back to back, and trust ratios more than absolute numbers.

## Visibility culling (`map::vis`)

Source maps carry precomputed visibility: the BSP tree's leaves belong to
clusters, and the visibility lump lists, per cluster, every cluster
potentially visible from anywhere inside it (PVS, run-length encoded).
`games/cs_source/bsp.rs::visibility` reads it into the game-independent
`MapVisibility` (tree, leaf clusters, per-cluster bit sets).

- World meshes (merged per material by the importer) are cut into chunks
  on a 512-unit grid (`vis::CHUNK_SIZE`, by triangle centroid); each
  chunk knows the clusters its triangles' boxes touch. Chunks have tight
  bounds, so Bevy's frustum culling works on them too.
- Static props, prop entities that don't move, sprites (glows included:
  no occlusion rays while culled), ropes, dust volumes and static props'
  dynamic shadows are tagged by the clusters their boxes touch.
- Each frame `vis::cull` finds the main camera's cluster (tree descent);
  when it changes it shows the tagged parts whose clusters are potentially
  visible and hides the rest. In solid or outside the map (no cluster),
  or with `r_novis 1`, everything is drawn.
- The water's reflection camera (`map::water`) draws only for a
  reflecting volume the main camera can see (in its frustum and in its
  cluster's PVS; the spec renders the views of the volume the camera
  sees). While it draws, parts potentially visible from the clusters
  around its surface are drawn too (`water::ReflectionClusters`: what is
  reflected is seen from the surface). Its mirrored eye is under the
  surface, often in solid: looking it up there turned culling off for the
  whole map whenever any reflecting water lay below the eye. The
  view-model and sky cameras draw only their own layers and are ignored.
- Meshes of entities with their own node (movers, breakables) stay whole;
  the node is tagged by the clusters its meshes' bounds touch, found again
  whenever it moves (`map::tag_moved_brush_entities`, from its
  `BrushEntityBounds` and `GlobalTransform`), so doors and breakables are
  culled like the world around them (props riding them with them). The
  logic hides removed ones with `LogicHidden`, as for props.
- Not culled: the 3D skybox (drawn by the sky camera only where the
  camera's leaf sees sky), physics props and their shadows (they move),
  characters, runtime decals, particles.
- Areaportals: the BSP's areas (leaf lump), areas lump (20), areaportals
  lump (21) and clip portal vertices (41) give each leaf an area and each
  func_areaportal/func_areaportalwindow an opening between two areas
  (`vis::MapAreas`). Logic closes a portal while its linked door
  (`target`) is fully closed, or by Open/Close/Toggle when unlinked
  (`vis::AreaPortalStates`); a portal with glass or a grate in its
  opening is never closed (`see_through`). Windows (func_areaportalwindow):
  their brush (`target`) draws with an alpha rising from
  `TranslucencyLimit` at `FadeStartDist` to opaque at `FadeDist` from the
  opening (`vis::fade_windows`; hidden at alpha 0, blended in between),
  and past `FadeDist` the portal closes behind the opaque brush. Windows
  without a brush stay open.
  Each frame `vis::cull` floods from the camera's area through open
  portals, each portal narrowing a screen rectangle (its opening's
  projected bounds, intersected with the rectangle it was reached by;
  an opening crossing the eye's plane covers the screen), and draws only
  clusters in the PVS that have a leaf in a reached area. The water
  reflection camera floods whole areas (no rectangle). `r_portalsopenall 1`
  goes back to PVS only.
- Occluders (func_occluder): the occlusion lump (9) gives each occluder
  its polygons (indices into the vertex lump); the entity's
  `occludernumber` names it, `StartActive` and Activate/Deactivate/Toggle
  switch it (`vis::OccluderStates`). Each frame, while the main view has
  active occluder polygons in sight (wholly in front of the eye), static
  props and prop entities that stay put (`vis::Occludee`, their bounds;
  not world chunks: occluders are documented as hiding models, and
  stock occluders stick out of walls over windows, so hiding chunks
  showed holes) are hidden when
  the bounds lie wholly behind a polygon's plane and their screen
  rectangle lies inside its projected outline (conservative: no merging
  of neighbouring occluders). Not while the water reflection draws.
  `r_occlusion 0` turns them off; `mashup_perf 1` shows how many parts
  they hide. de_aztec's occluders were compiled without polygons, so they
  do nothing there (nor in the game, presumably).
- Not done: LOD models, per-leaf frustum culling. See tech-debt.

Prop fade distances: static props' `fademaxdist` (static prop lump) and
prop entities' `fademaxdist` key hide the prop beyond that distance from
its origin. Between `fademindist` and `fademaxdist` the prop's meshes are
dithered out (`vis::fade_band`: a Bevy `VisibilityRange` per mesh, a 4x4
screen-door pattern in `prop.wgsl`/`prop_prepass.wgsl`, and for opaque
props a dither-only fragment stage added to Bevy's depth-only prepass).
Dithering keeps props in the opaque passes (no blending, no sorting) at
the cost of a stipple instead of a smooth alpha. A negative `fademindist`
(or one not below the far distance) pops at `fademaxdist` as before.
Fades apply with `r_novis 1` too.

### Checks

- `cargo test --test map_vis`: from every spawn and ~250 nav area centres
  per map (de_dust2, de_nuke, cs_office), 400 rays from eye height; every
  world surface a ray reaches (translucent ones on the way, then the
  first opaque one) must be in a potentially visible chunk, unless the ray
  passed through solid first (sky brushes, which the game draws as sky and
  we don't draw). `MASHUP_VIS_FULL=1`: every nav area, 2048 rays.
- `refcmp vischeck`: renders with culling off and on must match. Results
  (de_dust2: 316 views, de_nuke: 311): a handful of views differ by a few
  to a few hundred pixels, all inspected: geometry visible through sky
  brushes and sub-pixel cracks (culling hides it, as the game would), and
  one de_nuke view that also differs between two runs without culling.
  Two real bugs it found are fixed: physics props settling differently
  between runs (`host_timescale 0` now freezes physics from the first
  frame) and the first view rendering before shader pipelines were ready.

## Baseline and results

RTX 3080, Linux, 1280x720 offscreen, dev box under other agents' build
load. Frame ms averaged over the views of each map's refcmp file.

Playtest build, de_dust2 (32 views), two interleaved runs each at low
machine load. Frame ms is wall time; CPU ms is process CPU per frame over
all threads (Bevy's worker threads included); GPU ms from timestamps.

| de_dust2 | frame ms (avg) | CPU ms | GPU ms | meshes drawn | triangles drawn |
|---|---|---|---|---|---|
| before (merged per material) | 5.8 / 6.2 | 12.0 / 12.7 | 0.27 | 552 | 108k |
| chunks 512 units + PVS | 6.9 / 4.5 | 13.3 / 10.0 | 0.26-0.31 | 718 | 66k |
| chunks 1024 units + PVS | 9.2 (loaded) / 4.7 | 10.2 | 0.26 | 575 | 67k |
| chunks 4096 units + PVS | 5.2 / 4.3 | 11.3 / 9.6 | 0.25 | 427 | 72k |

After merging main (water with its reflection camera, movers), playtest
build, each map's refcmp views (de_nuke 27, cs_office 23), runs taken
under heavy machine load (load 8-27): read the ratios, not the absolute
times.

| map | build | frame ms avg | CPU ms | GPU ms | meshes drawn | triangles drawn |
|---|---|---|---|---|---|---|
| de_nuke | before (merged) | 58 / 101 | 77 / 107 | 3.0 / 6.9 | 1623 | 595k |
| de_nuke | 512 + PVS | 32 / 42 | 49 / 63 | 1.9 / 3.4 | 1027 | 157k |
| de_nuke | 4096 + PVS | 27 / 52 | 46 / 64 | 1.6 / 3.7 | 652 | 162k |
| cs_office | before (merged) | 45 / 57 | 65 / 67 | 2.1 / 3.5 | 1075 | 303k |
| cs_office | 512 + PVS | 28 / 47 | 45 / 55 | 1.1 / 2.4 | 835 | 150k |
| cs_office | 4096 + PVS | 34 / 62 | 46 / 57 | 1.5 / 2.0 | 634 | 154k |

The GPU does little (a 3080 draws dust2 in a quarter of a millisecond):
frames are CPU bound, by per-entity work, so culling pays by drawing fewer
entities as well as fewer triangles; chunks that are too small add
entities. 512 and 4096 units measure about the same; 512 stays the
default (finer culling, and what vischeck and the ray tests checked).
`MASHUP_CHUNK_SIZE=<units>` overrides it for measurements.

Areaportals (playtest build, `refcmp bench`, load 14-20 on 12 cores, so
frame times swing 2x between runs; back-to-back runs with and without
`+r_portalsopenall 1`):

| map (views) | areaportals | frame ms avg (runs) | meshes drawn | map parts drawn (mean) |
|---|---|---|---|---|
| de_nuke (27) | on | 7.6 / 7.1 | 548 | 723 |
| de_nuke (27) | open all | 16.2 / 8.8 | 548 | 726 |
| cs_italy (6) | on | 8.2 | 412 | 539 |
| cs_italy (6) | open all | 7.6 | 412 | 545 |
| cs_office (11) | (has none) | 21.2 | 380 | 583 |

At these views the gain is in the noise: their doors are open or out of
view, the PVS already culls most of what lies beyond, and what remains
beyond an opening but outside its screen rectangle is mostly outside the
frustum anyway (meshes drawn don't change). The win is behind shut doors
(cs_militia: 54 map parts behind its front door, cs_assault: 15; see
`tests/map_areaportals.rs`). `refcmp vischeck` on de_nuke lists the same
15 views, pixel for pixel, with and without `+r_portalsopenall 1`, so none
come from areaportals; one of them, nav1627_90 (16k pixels inside the
glass of the door beside the A site door), is over vischeck's 0.5% limit.
It isn't culling: rays from that eye through the glass (a scratch test
like `map_vis`) reach no culled chunk or prop, and capturing the view
alone gives the same image with and without culling (equal to the
culled run's). Capturing nav1627_0 then nav1627_90 without culling
reproduces the 15997 pixels, also with game time running: something the
previous view drew changes how the glass draws next (backlog).

Occluders (playtest build, `refcmp bench` over each map's 24 spawn views
in `tools/refcmp/`, two back-to-back pairs with `+r_occlusion 1` and `0`;
load 8-15 on 12 cores, so frame times are noise; the counts are exact):

| map | occluders | frame ms avg (runs) | meshes drawn | triangles drawn | map parts drawn (mean) |
|---|---|---|---|---|---|
| cs_assault | on | 9.9 / 8.2 | 364 | 37k | 778 |
| cs_assault | off | 6.9 / 8.8 | 369 | 39k | 784 |
| cs_compound | on | 32.0 / 19.2 | 439 | 29k | 1149 |
| cs_compound | off | 14.7 / 21.1 | 458 | 40k | 1168 |
| de_port | on | 14.1 / 19.6 | 1162 | 69k | 3192 |
| de_port | off | 14.9 / 15.2 | 1162 | 69k | 3192 |

Props only (see above). cs_compound's spawn views look along occluded
walls (27% fewer triangles: the hidden props are detailed); de_port's
occluders guard views its spawns don't have; de_aztec's have no
polygons. A first version also hid world chunks (cs_compound: 327
meshes, 28k triangles) but showed holes through windows in walls whose
occluder brushes stick out of them (`refcmp vischeck` on cs_assault and
cs_compound, up to 73% of a view). With props only, `refcmp vischeck`
(unculled run: `r_novis 1`, `r_occlusion 0`) passes on cs_compound (19
of 320 views differ, at most a few hundred pixels) and de_port (2 of
312); cs_assault differs with occluders off too (backlog).

## Many brush entities (mg_lego_multigames_v2)

A community minigame map (BSP v20, 578 func_breakable, 170 func_door, 937
brush models, 32k triangles, reflecting water). Looking into its room of
breakable blocks (`tools/refcmp/mg_lego_multigames_v2.toml`, view
`blocks_room`) it ran at 11-17 fps. Text readouts (`mashup_perf_log 1`,
window 1920x1080, playtest build):

| | fps | frame ms | main world ms | GPU ms | meshes drawn | vis |
|---|---|---|---|---|---|---|
| before | 14 | 70-72 | 12.2-14.2 | 4.6 (bin_unpacking 4.2) | 1964/4737 | off (2539 parts all drawn) |
| after | 60 (vsync) | 16.7 | 4.2-6.1 | 0.5-0.9 | 1767/4737 | cluster 1088, 626/3306 parts |

Root causes, from a trace of that view (`tracesum`, 200 frames):

- Every brush entity's mesh had a material asset of its own (a mesh per
  material per entity: 2209 WorldMaterials for 14 distinct materials), so
  nothing batched: each mesh its own bin, bind group and draw. Render
  world 59 ms a frame: `write_binned_instance_buffers` 13.4 ms (opaque)
  + 5.9 ms (prepass), `prepare_preprocess_bind_groups` 6.5, `unpack_bins`
  6.3, `queue_submit` 6.9. Now meshes with the same material description
  share one asset (`spawn_map` keys them by the material's full
  description and the skybox flag): 14 assets, render world about 5 ms
  plus waiting for vsync.
- Visibility culling was off: the map has visibility, and the camera's
  cluster was found, but the water reflection camera (active whenever a
  reflecting volume lay anywhere below the eye) had its mirrored eye in
  solid, which drew everything. Fixed as above (reflection only for water
  in sight; its culling from its surface's clusters).
- Brush entities were never culled; now by their bounds' clusters.
- The main world paid for the slow frames: at 15 fps each frame ran four
  fixed ticks (logic, movement, physics). In the logic bridge,
  `sync_movers` looked each mover's node up in a list (quadratic in
  movers) and rebuilt and re-inserted every mover's `MovingSolid` brushes
  each call (twice a tick); now a map lookup, and the brushes only when
  the mover moved or changed (`logic: sync_movers` span 0.19 ms a call).

`refcmp bench` (offscreen 1280x720, vsync off, playtest build), mean of
the file's 4 views, back-to-back runs at load 8-14 on 12 cores:

| build | blocks_room ms | mean frame ms | main world ms | GPU ms | meshes drawn (mean) |
|---|---|---|---|---|---|
| before | 59.4 / 59.5 | 25.8 / 25.7 | 5.9 / 6.0 | 1.97 / 1.81 | 1471 |
| after | 4.2 (one loaded run: 16) | 3.1 | 1.8 | 0.22 | 512 |

So about 240 fps at blocks_room and 320 fps on average at 720p here
(the GPU does under 0.5 ms; frames are CPU bound). No regression on the
stock maps (same runs, mean over each file's views; noise between runs
is larger than the difference):

| map | before frame ms | after frame ms | meshes drawn before -> after |
|---|---|---|---|
| de_dust2 (32 views) | 4.07 / 4.80 | 5.32 / 3.78 | 248 -> 248 |
| de_nuke (27 views) | 8.35 / 10.58 | 7.90 / 14.03 (loaded) | 548 -> 489 (its doors culled) |

Pictures are unchanged: refcmp report mean abs diff de_dust2 0.0308
(32 views), de_nuke 0.0250 (27), de_aztec 0.0409 (13), each view within
0.0005 of the shared captures; de_port's 24 views (reflecting water)
differ from the old build by at most 28 pixels. `refcmp vischeck` on the
lego map: 48 views, two differ by 1732 and 1965 pixels (0.2%): a roof
seen through a grate, which the map's PVS doesn't list from that
cluster (with or without areaportals; backlog section 9, like de_nuke's
nav1627_90).

What is left at that view: about 1770 meshes drawn (the wall of blocks is
~580 breakables, a mesh per material each); main world ~2-5 ms of mostly
Bevy's per-entity work (visibility checks, transform and collider
propagation over the map's tree). Merging unbroken breakables into
shared meshes would cut entities further.

## Cheap wins found

- Glow sprites: the occlusion test borrowed the sprite material mutably
  every frame, which re-prepares it for the GPU even when nothing changed;
  now only when the colour changes. Culled glows skip their five rays.
- Particles and dust: each particle material's mesh and each dust volume's
  mesh was rewritten every frame, even with nothing in it (13 mesh assets
  modified per frame in a dust2 bot match). Any modified mesh makes every
  mesh entity re-check its pipeline (`AssetChanged<Mesh3d>` in each
  material's `check_entities_needing_specialization`) and the mesh is
  uploaded again. Now an empty mesh is written once, and hidden dust
  volumes wait (2-5 per frame left, from live particles).
- Contacts nobody uses: avian computed contacts every step for each
  character capsule and mover against the world and static props, though
  two non-dynamic bodies never get constraints. The collision hooks now
  drop pairs without a dynamic body unless a side reports contacts
  (`map::contact_filter`).
- Redundant writes: the sky and view-model cameras got their target,
  tonemapping and projection re-inserted every frame, the radar its
  material; now only on change.

Measuring here: the dev box runs other agents' builds (load 4-16 on 12
cores), which moves frame times by 2-4x between runs, more than these
changes. `mashup_perf 3` counts (above) are exact. Before/after frame
times on a quiet machine are still to be taken.

Tried and dropped: one physics substep while no dynamic body is awake
(characters and movers are kinematic with zero velocity). It made
sleeping ragdolls wake again every step (`bullets_push_a_dead_body`
failed), so substeps stay at avian's 6. Props, movers and the map's collision
as hierarchies of their own instead of children of the map's root (avian
walks every tree holding a collider each tick in
`propagate_collider_transforms`, about 0.3 ms per tick on dust2 under
load): a dust2 physics prop then settled differently
(`physics_props_settle_and_get_pushed`), so it is left for later.
