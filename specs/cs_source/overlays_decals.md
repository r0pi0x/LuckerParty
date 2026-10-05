# Counter-Strike: Source: overlays (info_overlay) and map decals (infodecal)

Source basis: Valve's public Source SDK 2013. I read:
- the BSP file-format header (overlay, overlay-fade and water-overlay lumps, surface flags);
- the map compiler's overlay handling (reading info_overlay keys, binding overlays to brush sides and their output faces, writing the lump) and its material-flag rules;
- the server's infodecal entity and the server/client decal temp entities;
- the engine-facing decal interfaces (signatures only);
- the decal shaders.

**Important limitation.** Several things this spec was asked to cover are done inside the closed engine, which is not in the public SDK:
- projecting decals onto world surfaces (which faces, orientation, size, clipping);
- clipping overlays to their faces;
- sorting overlays against each other and against decals.

Those sections give everything the public code determines, and list the rest as Open questions with a concrete way to measure each one in game. No engine source was read, and no leaked code was used.
Status: draft

## Summary

- **Overlays** are fully baked by the compiler. Each one is a quad defined in its own (U, V, normal) basis, a texture-coordinate rectangle, a material, a list of up to 63 BSP faces it may draw on, a 2-bit render order and optional fade distances. The engine projects the quad onto those faces at load time.
- **infodecal** entities are not baked. The server finds out which brush entity (or the world) the decal sits on, then asks the engine to project the decal at that point. It gives no direction and no orientation, so the engine chooses them from the surfaces it finds.
- Decal materials usually blend as "modulate 2×", with a depth bias so they sit on top of the surface.

## Units and conventions

- Hammer units (inches). Z is up. All BSP data is little-endian.
- "Face" means a BSP face (lump 7), index into the face lump. "Side" means a brush side in the source VMF; the compiler converts sides to faces.
- Overlay "U, V" means the overlay's own 2D basis on the surface. Texture "u, v" means material texture coordinates in [0, 1] repeat space.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| overlay_lump | 45 | - | overlay records |
| overlay_fade_lump | 60 | - | per-overlay fade distances |
| water_overlay_lump | 50 | - | water-transition overlays (separate format, 256 faces each) |
| overlay_record_size | 352 | bytes | one overlay |
| overlay_max_faces | 64 slots, at most 63 used | - | the compiler errors at 64 or more |
| overlay_max_count | 512 | - | per map |
| render_orders | 4 (0..3) | - | 2 bits |
| infodecal_probe | ±(5,5,5) | units | diagonal segment used to find the target entity |
| no_decal_flag | 0x2000 | - | texinfo surface flag "receives no decals" |
| sf_not_in_deathmatch | 2048 | - | infodecal spawnflag |

## Behavior

### Overlay record (lump 45), byte layout

Each record is 352 bytes:

| Offset | Size | Field |
|---|---|---|
| 0 | i32 | overlay id. It is the overlay's index in compile order, and is also referenced by info_overlay_accessor entities (key OverlayID) |
| 4 | i16 | texinfo index. Only its texdata (and so the material name) matters. Its texture vectors are all 0 with offsets -99999, a marker meaning "not a normal surface mapping" |
| 6 | u16 | low 14 bits: face count. Top 2 bits: render order (0..3) |
| 8 | 64 × i32 | face indices. The first "face count" entries are valid |
| 264 | 2 × f32 | texture u at the start and end of the quad (keys StartU, EndU) |
| 272 | 2 × f32 | texture v at the start and end (keys StartV, EndV) |
| 280 | 4 × vec3 f32 | quad corners uv0..uv3. x, y = corner position in the overlay's (U, V) basis. The z components are reused (below) |
| 328 | vec3 f32 | basis origin (key BasisOrigin), in world space |
| 340 | vec3 f32 | basis normal (key BasisNormal) |

```math
\text{faceCount} = w \,\&\, \text{0x3FFF},\qquad \text{renderOrder} = w \gg 14
```

**Recovering the basis.** The compiler packs BasisU into the unused z of the first three corners, and a V-flip flag into the z of the fourth.

```math
\mathbf U = (uv_0.z,\ uv_1.z,\ uv_2.z),\qquad
\mathbf N = \text{basis normal},\qquad
\mathbf V = \begin{cases} \mathbf N \times \mathbf U & uv_3.z \ne 1 \\ -(\mathbf N \times \mathbf U) & uv_3.z = 1 \end{cases}
```

- The flag is written as 1.0 when, in the editor, (N × U)·V_editor < 0, and is left at 0 otherwise.
- After recovery, ignore the z of all four corners.
- World position of corner k, before projection onto faces:

```math
\mathbf P_k = \text{origin} + uv_k.x\,\mathbf U + uv_k.y\,\mathbf V
```

**Which faces.**
- In the editor, the overlay lists brush sides (key "sides").
- The compiler records every BSP face it emits from those sides. That includes all the fragments a side is split into, and displacement faces built from a listed side.
- Duplicates are removed. Face order is emission order.
- So overlays can sit on displacements, and the face list says exactly which faces may receive the overlay. Props and brush faces that are not listed never receive it.

**Fade (lump 60).**
- One record per overlay, in the same order: two f32 values, min and max fade distance, **already squared**.
- The compiler squares a key value only if it is > 0. A 0 or negative key is stored unchanged, so 0 means "no fade".

**Render order.**
- It comes from the key RenderOrder (0..3). The compiler errors out on any other value.
- Every overlay is otherwise treated the same. How the engine uses this field is not public; see Open questions.
- The Hammer help text for the key says higher values render on top of lower ones. That is a documented intent, not something read from the source.

**info_overlay entity in the entity lump.**
- An info_overlay without a targetname is removed from the entity lump.
- One with a targetname becomes an info_overlay_accessor entity with an OverlayID key, so material proxies can bind to it.
- info_overlay_transition entities are always removed. Water overlays go to lump 50, a separate format that this spec does not cover.

### infodecal: how a map decal gets placed

1. **Material.** The key "texture" names a decal material (a path under materials/, e.g. decals/…), which is precached as a decal. If it can't be found, the entity is removed and nothing is drawn.
2. **Removal on spawn.** The decal is removed at spawn if spawnflag 2048 ("not in deathmatch") is set and the game reports deathmatch mode. See Open questions for whether CS:S does.
3. **Named or unnamed:**
   - **Unnamed** (no targetname): a static decal, placed at map activation, before any player joins.
   - **Named**: nothing happens at load. It is applied when it receives the Activate input, or is used by another entity. Then it is sent as a one-shot BSP decal to all clients, and the entity removes itself 0.1 s later.
4. **Finding the target entity (static decals).**
   - Trace a line from origin - (5,5,5) to origin + (5,5,5). It is a diagonal segment, not a box.
   - The trace hits solids, ignoring any entity that is not drawn and any weapon, item, ragdoll, dynamic/static/physics prop or bullseye.
   - If the first hit is a brush entity: the decal belongs to that entity's model, and the position is converted into that entity's local space. If that entity has model index 0, the decal is dropped with a warning.
   - Otherwise (no hit, or hit the world): the decal belongs to the world (entity 0) at the original position.
   - The trace only picks the target model. It does **not** decide which surfaces receive the decal. A decal placed 6+ units from every surface still goes to the engine with the world as target.
   - Named decals use a similar trace, against brushes only, to pick the entity.
5. **Hand-off to the engine.** The engine receives:
   - the position (in the target's local space);
   - the decal index;
   - the target entity and model;
   - the LowPriority key.
   - LowPriority = 1 means "not permanent": the engine may recycle it like a dynamic decal.

   It does **not** receive a direction, a normal or an orientation axis. So the engine:
   - finds the surfaces near the point by itself;
   - picks the decal's in-plane orientation by itself;
   - takes the decal's size from the material.

   None of this is in the public SDK (see Open questions).

### Surfaces that never take decals

- Faces whose texinfo has flag 0x2000 receive no decals.
- The compiler sets this flag on materials marked %compileWater or %compileSlime.
- Other materials may carry the flag if their texinfo was authored with it. Read the flag from the texinfo; don't re-derive it.

### Decal size

- The engine is not public, so the exact rule is unknown.
- What the public code shows: the editor-side texture interface exposes a per-material decal scale (the material parameter $decalscale) beside the texture's width and height.
- So the decal's world size is expected to be the base texture's size in texels × $decalscale (default 1), in world units. See Open questions.

### Decal blending (from the shaders)

- **"DecalModulate" materials** (the usual dirt and bullet-hole style):
  - The framebuffer colour is multiplied by 2 × the texture colour: dst' = dst·src + src·dst. A texture value of 0.5 grey leaves the surface unchanged.
  - Fog on these is softened to fog_factor^0.4, so that fogged decals don't turn into dark patches.
- **Lightmapped decal materials** (LightmappedGeneric with $decal) are alpha-blended over the surface and lit by the surface's lightmap.
- Both kinds are drawn with the "decal" polygon depth offset, which pulls them toward the camera so they don't z-fight.
- Overlays are drawn with ordinary world materials (typically LightmappedGeneric with $decal or $translucent) and the same decal depth offset. That last part is inferred from the material flags; the engine draw call is not public.

## Per-frame order

Decal placement is a one-time event at map load, not per tick. The load order is:
1. Load the overlay lump.
2. The engine projects and clips overlays onto their faces (engine, not public).
3. Spawn entities.
4. Activate every entity. Each unnamed infodecal traces, hands its static decal to the engine, and removes itself.
5. The engine sends static decals to clients during sign-on.

Draw order within a frame, between world surfaces, overlays and decals, is engine-side and listed in Open questions.

## Edge cases

- **More than 63 faces** on one overlay: compile error. The lump never holds more than 63.
- **RenderOrder outside 0..3:** compile error. The lump never holds one.
- **Overlay whose sides were all removed** (e.g. nodraw sides that produce no faces): face count 0. It draws nothing.
- **infodecal touching a func_detail:** func_detail is world geometry after compile, so the target is the world.
- **infodecal near a prop_static:** props are ignored when picking the target. Whether the engine's projection then also skips nearby static props is an open question.
- **infodecal with a targetname that is never fired:** it never appears.

## Quirks

- The overlay basis U and the V-flip flag are hidden in the z components of the corner points. A reader that treats the corners as 3D points gets garbage.
- Fade distances are stored squared, except when ≤ 0.
- The infodecal target trace is a diagonal 17.3-unit segment, not a sphere or box. For a decal centred on a wall, the segment still crosses the wall plane, because the decal sits within 5 units of it on every axis. That is why it works in practice.

## Test cases

| Setup | Input | After | Expected |
|---|---|---|---|
| overlay record, u16 at offset 6 = 0x8005 | decode | - | face count 5, render order 2 |
| u16 = 0xC03F | decode | - | face count 63, render order 3 |
| u16 = 0x0001 | decode | - | face count 1, render order 0 |
| corner z values 0, 0, 1 (uv0..uv2), uv3.z = 0, normal (1,0,0) | recover basis | - | U = (0,0,1), V = N×U = (0,-1,0) |
| same but uv3.z = 1 | recover basis | - | V = (0,1,0) |
| floor overlay: normal (0,0,1), U = (1,0,0), uv3.z = 0 | recover basis | - | V = (0,1,0) |
| origin (100,200,0), U = (1,0,0), V = (0,1,0), uv0 = (-32,-16) | corner world position | - | (68, 184, 0) |
| key fademindist 512, fademaxdist 1024 | lump 60 record | - | 262144, 1048576 |
| key fademindist 0, fademaxdist -1 | lump 60 record | - | 0, -1 |
| infodecal at (0,0,3), floor plane z = 0 is world | target trace | - | the segment from (-5,-5,-2) to (5,5,8) starts inside the floor brush (it would cross z = 0 at fraction 0.2). Hit = world. Target = world (0), position (0,0,3) |
| infodecal at (0,0,8) above a floor at z = 0, nothing else nearby | target trace | - | no hit. Target = world, position (0,0,8). Still handed to the engine |
| infodecal 2 units in front of a func_brush wall, not drawn (nodraw effect) | target trace | - | the brush is ignored. Target = world |
| infodecal with targetname "x", never fired | map load | - | no decal |
| face whose texinfo flags include 0x2000 | any decal | - | the face receives no decal |
| DecalModulate texel 0.5 grey over surface colour c | blend | - | c (unchanged). Texel 0.25 → 0.5c. Texel 1.0 → 2c, clamped |

## Open questions

All of these are inside the closed engine. Each comes with a way to settle it on the Windows PC with CS:S installed. Use screenshots with sv_cheats 1, mat_fullbright 1 and r_drawdecals / r_drawoverlays toggles.

1. **Orientation of a world decal (the floor/ceiling case).**
   - Not determinable from public source: no orientation axis is passed to the engine for map decals.
   - Measure:
     - Find an asymmetric infodecal (e.g. an arrow or text decal) on a floor in a stock CS:S map.
     - Compare its in-game "up" direction with the world axes, and with the floor's texture axes from the BSP texinfo.
     - Repeat for a wall (normal ±X and ±Y) and a ceiling.
   - Candidate rules to tell apart:
     - (a) The decal's up follows the receiving face's texture "t" axis.
     - (b) A fixed world axis: +X, or +Y, for floors and ceilings, and world down (-Z) projected onto the face for walls.
   - A floor whose texture is rotated 90° tells (a) from (b) in one screenshot.
2. **Which faces a static decal reaches.**
   - Does it cover every face within the decal's half-size of the point, including faces at an angle (e.g. wrapping around an outside corner), or only the face nearest the point?
   - Measure: an infodecal placed exactly on a box edge in a test map, or found in a stock map.
3. **Decal size.**
   - Confirm world size = base texture width/height × $decalscale.
   - Measure: an infodecal whose texture size and $decalscale are known, next to a brush of known size.
4. **Decals on displacements and props.**
   - Do static map decals land on displacement faces?
   - Do they land on static props near the point? Model decals use a separate engine path.
   - Measure: stock maps with decals on terrain. In de_dust2, look for infodecals near displacement ground.
5. **Overlay render order semantics.**
   - The public code shows only that the value is 0..3 per overlay.
   - Expected, but not confirmed: overlays on the same face are drawn in ascending render order, so higher values land on top, with ties in an engine-defined order (probably lump order).
   - Measure: two overlapping overlays on one face in a stock map (dust2 has stacked posters/graffiti). Compare which one is on top with their lump render-order values.
6. **Overlays vs decals.**
   - Are all overlays drawn before all decals on a surface (decals always on top of overlays), or are they interleaved?
   - Measure: shoot a bullet hole onto a poster overlay. If the hole draws over the poster, decals come after overlays. Repeat with a static infodecal that overlaps an overlay.
7. **Overlay texture-coordinate corner mapping.**
   - Which corner (uv0..uv3) gets (StartU, StartV), and in what winding?
   - Also: how is the quad projected onto non-coplanar faces (along N, or by clipping in the basis plane)?
   - Measure: a known asymmetric poster overlay in a stock map against its lump corners.
8. **The deathmatch flag in CS:S.**
   - Is the game's "deathmatch" global set in CS:S? If so, spawnflag 2048 infodecals never appear.
   - Measure: find an infodecal with that flag in a stock map's entity lump and look for it in game.
9. **Overlay fade.**
   - How does the engine apply min/max (linear alpha ramp, or a hard cut at max)?
   - Measured from what: the eye position, or the overlay origin?
