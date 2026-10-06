# Other stock maps (backlog 1E)

Goal: the stock CS:S maps besides de_dust2 load cleanly and look like the
game. Priorities (user, 2026-10-06): de_aztec's missing textures, cs_office
z-fighting and things not loading, lighting on those maps; then de_nuke and
the rest.

How to check:

- Load warnings for every stock map: `cargo test --features dev --test
  map_stock -- --ignored --nocapture all_stock_maps_warnings`.
- Views: `tools/refcmp/{de_aztec,cs_office,de_nuke}.toml` (the maps'
  intermission cameras, first T/CT spawns, bomb/rescue zones four ways;
  cs_office adds hostage rooms and a window). Ours: `cargo run --features
  dev --bin refcmp -- capture-ours --views tools/refcmp/<map>.toml`. The
  reference game is shared: ask the coordinating session before capturing
  it (`capture-ref`).
- `tests/map_stock.rs` holds a test per fix.

## Catalog

| # | Bug | Maps | Where | Cause | Status |
|---|---|---|---|---|---|
| 1 | Stone walls untextured (debug colours), 59 "Unknown variant worldtwotextureblend" errors | de_aztec | everywhere | Shader WorldTwoTextureBlend unknown to the VMT parser | Fixed: parsed as LightmappedGeneric, `$detail` blended over the base by its alpha (new detail mode 2) |
| 2 | Props black in shade (crates, barrels, bars) | all BSP v19 maps (aztec, office, assault, compound, havana, italy, cbble, chateau, piranesi, port, prodigy, tides), de_nuke, de_train | everywhere | Ambient cubes were read only from the v20 sample lumps; v19 stores one cube per leaf inside the leaf, early v20 one per leaf in lump 56 without an index | Fixed |
| 3 | Brush entities missing: glass windows, doors, func_brush/illusionary/breakable geometry; walk-through | all (office: 14 windows, door; nuke: 55 func_breakable, doors, rotating fans) | | Only the world model was converted | Fixed: drawn (rendermode 10 and disabled func_brush skipped) and solid where they spawn (doors closed, breakables whole; illusionary never) |
| 4 | World brush collection read the wrong leaves | all | | vbsp sorts its leaf list by cluster; the tree walk indexed it as stored | Fixed: walks the raw leaf lump |
| 5 | Black triangles and bow ties around lights | de_nuke (reactor, warehouse) | cam10, cam11, cam15 | `$additive` materials (light glows, beams) drawn opaque | Fixed: `MapAlpha::Add` (world and prop shaders) |
| 6 | HDR-capable skies missing ("sky ..._hdrrt: no texture") | de_nuke, de_train, de_dust, cs_militia | sky | `Sky` shader's `$basetexture` not read | Fixed |
| 7 | Static props not found (`./models/...`) | de_nuke (generator), de_train (4), cs_assault | | Paths with a `./` component | Fixed in path normalization |
| 8 | Self-lit porthole decals failed to parse | de_nuke | | DecalBaseTimesLightmapAlphaBlendSelfIllum unknown to the parser | Fixed as a translucent lit decal; self-illumination not modelled |
| 9 | "Decoding Uv88 images is not supported" | aztec, inferno, militia, chateau, piranesi, port | water | Water's `$bumpmap` is a DuDv map | Fixed: water uses `$normalmap` |
| 10 | Water surfaces black | de_aztec canals, others | aztec cam1 | No Water shader (no base texture; refraction/reflection/fog missing) | Open: backlog 7 "Water surfaces" |
| 11 | Decals that find no surface (assault 46, nuke 21, train 21, others ≤ 8) and overlays without geometry (train 8, assault 5) | many | | Probably on brush entities (now drawn) or displacements, which decals and overlays don't project onto | Open |
| 12 | A dark, unlit-looking building block behind the street | cs_office | cam0, left | Unknown: needs the reference view | Open: reference capture needed |
| 13 | View model drawn in `--views` captures | all | | No `r_drawviewmodel` equivalent; capture hides nothing | Open (client) |
| 14 | `--views` crashed (wgpu: attachments of different sizes) | all | | The view-model camera stayed on the window when `--views` retargeted the main camera | Fixed: follows the anchor camera's target |

Not seen in the captured views so far: z-fighting on cs_office (the
office, garage and street views are clean). The most likely candidates
before these fixes were the black props (2) and missing windows (3);
check again once the reference views are available.

## Next

- Reference captures for de_aztec, cs_office, de_nuke at the views above
  (needs the shared reference game), to confirm WorldTwoTextureBlend's
  look and item 12.
- Other stock maps' views (de_train, de_inferno, cs_italy, de_dust, ...).
- Brush entity render modes other than normal and 10 (translucent
  func_brush via renderamt).
