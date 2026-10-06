# Performance

How to measure, what the numbers were, and what the map visibility culling
does. Tools are listed in [OBSERVABILITY.md](OBSERVABILITY.md) ("3c.
Performance").

## Measuring

- `mashup_perf 1` in the console: frame times, main-world CPU, GPU time
  per render pass (Vulkan/DX12 timestamps), entities, meshes and
  triangles drawn, the camera's visibility cluster.
- `refcmp bench --views tools/refcmp/<map>.toml`: 200 frames per view
  after settling, vsync off, offscreen at 1280x720; prints avg/p95/max
  frame ms, drawn meshes/total, triangles drawn, map parts potentially
  visible. Build the profile you want first (`cargo build --features dev`,
  or `cargo build --profile playtest` and run `target/playtest/refcmp`).
- A/B: `-- +r_novis 1` (no PVS culling) or `MASHUP_MERGED_WORLD=1` (the
  world as one mesh per material, nothing culled: the code before
  chunking).
- Per-system CPU time: `cargo run --profile playtest --features profile`
  writes `trace-*.json` (Bevy's Chrome tracing) to the working directory;
  open it in https://ui.perfetto.dev.

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
- Not culled: the 3D skybox (drawn by the sky camera only where the
  camera's leaf sees sky), physics props and their shadows (they move),
  characters, runtime decals, particles.
- Not done: areaportals (closed doors/windows hiding what's behind them),
  `func_occluder`, LOD models. See tech-debt.

Prop fade distances: static props' `fademaxdist` (static prop lump) and
prop entities' `fademaxdist` key hide the prop beyond that distance from
its origin. The fade band between `fademindist` and `fademaxdist` is drawn
fully opaque (no per-prop alpha yet). Fades apply with `r_novis 1` too.

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

The GPU does little (a 3080 draws dust2 in a quarter of a millisecond):
frames are CPU bound, by per-entity work, so culling pays mostly by
drawing fewer entities, and chunks that are too small add entities.

## Cheap wins found

- Glow sprites: the occlusion test borrowed the sprite material mutably
  every frame, which re-prepares it for the GPU even when nothing changed;
  now only when the colour changes. Culled glows skip their five rays.


