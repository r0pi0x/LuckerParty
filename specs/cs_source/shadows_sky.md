# Counter-Strike: Source: dynamic shadows and sky drawing

Source basis: Valve's public Source SDK 2013 (current GitHub head). I read the client shadow manager (shadow texture atlas, caster bounds and projection set-up, receiver culling, LOD), the shadow_control entity on both server and client, the per-entity shadow-cast rules (base entity, animating, physbox, weapon), the shadow-related material proxies, the four shadow shaders (silhouette build, world receiver, model receiver, blob), the client view renderer (clear flags, 3D skybox pass, sky visibility) and the map compiler's sky and leaf-flag code. CS:S's own game DLL is not public. The engine's shadow manager (it clips shadows onto world surfaces and computes per-vertex fade) and its 2D-sky drawing are not in the SDK; parts that depend on them are marked *inferred*.

I also read, from the local install, the shadow materials in the HL2 shared VPK, the dxlevel config that ships with the game, and the de_dust2 entity lump. These were used for values only; nothing was copied into the repo.
Status: draft

## Summary

- **Shadow casters.** At dxlevel ≥ 80, CS:S draws "render-to-texture" shadows for every studio-model entity: players, prop_physics(_multiplayer), dynamic props, dropped weapons and physboxes.
  - The caster's silhouette is drawn into a small square texture, seen orthographically along one global shadow direction.
  - That texture is projected onto world brushes and brush entities below the caster.
  - The receiving pixels are multiplied toward the shadow colour, with a 5-tap blur.
  - The shadow fades with distance along the projection direction and ends after `distance` units.
- **Global shadow parameters.** Direction, colour and distance come from the map's shadow_control. de_dust2 has one: angles (60, 43, 0), colour (159, 168, 181), distance 75. Without one, the defaults are: direction normalize(0.1, 0.1, −1), distance 50, and a colour taken from the map's ambient light.
- **Sky.** Sky-textured faces (`%compileSky` materials such as tools/toolsskybox) are never drawn. The engine draws the 2D sky box only when the camera's leaf can see sky.
  - If the map has a sky_camera and the camera can see 3D-sky leaves, the 3D skybox is drawn first. The 2D sky is drawn inside that pass, then depth is cleared, and the world is drawn on top.
  - The main colour buffer is **not cleared** by default. Anything not covered by sky or geometry (holes through nodraw, the void outside the map) shows whatever was there before: the sky if it was drawn this frame, otherwise the previous frame ("hall of mirrors").

## Units and conventions

- Source units (1 unit = 1 inch), Z up.
- Angles are (pitch, yaw, roll) in degrees. The forward vector of (p, y) is (cos p · cos y, cos p · sin y, −sin p): positive pitch points **down**.
- Colours are 0..255 bytes unless stated.
- "Gamma→linear" is the same 2.2 table conversion described in shaders.md (values ≥ 0.95 become 1).
- Shadow space for one caster:
  - z runs along the shadow direction d (unit vector, pointing from caster to receiver).
  - x and y span the plane perpendicular to d.
  - The origin is at the "back" of the caster's box.
  - Texture coordinates are u = x / size.x + 0.5 and v = y / size.y + 0.5.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| default_shadow_dir | normalize(0.1, 0.1, −1) = (0.09901, 0.09901, −0.99015) | unit vector | when no shadow_control |
| default_shadow_dist | 50 | units | shadow cast distance when no shadow_control |
| shadow_control_default_dir_angles | (80, 30, 0) | degrees | used if the entity's angles key is all zero |
| shadow_control_default_color | (64, 64, 64) | byte RGB | entity default if no `color` key |
| shadow_control_default_distance | 50 | units | entity default if no `distance` key |
| texels_per_unit | 2 | texels/unit | shadow texture resolution per caster size |
| min_shadow_tex | 16 | texels | |
| max_shadow_tex | 256 | texels | |
| atlas_size | 1024 × 1024, 8-bit RGBA, no depth | texels | all shadow textures share one atlas |
| atlas_layout | 16 blocks of 256²: 1 of 16-px cells, 1 of 32-px, 2 of 64-px, 6 of 128-px, 6 of 256-px | - | cell sizes available |
| shadow_bloat_rtt | 2 · texels_per_unit = 4 | units | added to each projected width |
| max_rendered | 32 (convar r_shadowmaxrendered) | shadows/frame | beyond this, blob shadows |
| falloff_amount | 240 / 255 = 0.9412 (*inferred* scale) | - | maximum lightening at the far end |
| blur_taps | 5 (centre + 4 diagonals at ±1 atlas texel) | - | |
| blur_offset | 1/1024 in u and v | uv | one atlas texel |
| fog_shadow_exponent | 4 | - | shadow strength × (1 − fog)^4 |
| blob_bloat | 10 | units | blob-shadow size padding, also its minimum size |
| blob_origin_snap | 0.5 | units | blob origin is snapped (truncated) to this grid |
| ambient_to_shadow | 3 · ambient + 0.3, clamped to 1 | - | default colour when no shadow_control |

## Behavior

### A. Which entities cast shadows

- **Casts a render-to-texture shadow:**
  - any client entity with a studio model (.mdl), unless it has the "no shadow" effect flag or is not drawn;
  - physboxes (brush models), which are a special case.
- **Animating models:**
  - those with pose parameters, bone controllers or IK chains re-render their shadow texture every frame;
  - others re-render only when they move, rotate or change.
  - Players in CS:S are animating models, so they re-render every frame (*inferred*: CS's own player class is not public).
- **Disabling a shadow:**
  - the entity key `disableshadows 1`;
  - `EF_NOSHADOW` set by code (sprites, beams, areaportals and many effects set it).
- **Weapons:**
  - dropped weapons cast shadows;
  - a weapon carried by the local player in first person does not;
  - weapons carried by others are drawn as part of the player's shadow hierarchy.
- **Child entities** attached to a parent with a render-to-texture shadow are drawn into the parent's texture instead of casting their own.
- **Not casters:**
  - world brushes;
  - static props. Their shadows are baked into lightmaps by the compiler unless the prop has the "no shadow" flag. *Inferred* runtime behaviour; the engine's static-prop code is not public.
- **Per-entity overrides:**
  - the key `shadowcastdist` (if non-zero) replaces the global distance for that entity;
  - render alpha/fade sets a "falloff bias" = 255 − blend. A fully faded entity (bias 255) has its shadow disabled; partial fades lighten it (*inferred*, see D).
- **Convars:**
  - r_shadows (1);
  - r_shadowrendertotexture. Its code default is 0, but the game's dx-level config sets it to 1 for dxlevel 80–98. With 0, every shadow is a blob.
  - r_shadowmaxrendered (32).
  - Cheat commands r_shadowdir, r_shadowangles, r_shadowcolor and r_shadowdist override the globals.

### B. shadow_control and defaults

- **Keys:**
  - `angles` "p y r": the direction is the forward vector of (p, y). If all three are 0, (80, 30, 0) is used instead. A legacy `direction` vector key is used only if angles was not set.
  - `color` "r g b": the shadow colour (bytes).
  - `distance`: the cast distance in units.
  - `disableallshadows`: 1 turns off render-to-texture shadow drawing; blobs remain.
- **Entity defaults** if keys are missing: direction (0.2, 0.2, −2) normalized; colour (64, 64, 64); distance 50; shadows enabled.
- **No shadow_control in the map:**
  - direction normalize(0.1, 0.1, −1);
  - distance 50;
  - colour = per channel round-down(255 · min(1, 3·A + 0.3)), where A is the engine's ambient light colour for the level (engine-defined units, *inferred* 0..1 linear).
- **de_dust2** has exactly one shadow_control:
  - angles (60, 43, 0), giving direction (0.36568, 0.34100, −0.86603);
  - colour (159, 168, 181);
  - distance 75;
  - disableallshadows not set.
  - The sun (light_environment) has the same yaw 43 and pitch −60. Its pitch key uses the opposite sign convention, so the shadow points away from the sun.
- The shadow colour is applied as the shadow material's colour modulation. The receiver shader converts it gamma→linear.

### C. Building one shadow (render-to-texture)

1. **Caster bounds.** Take the caster's shadow-render bounds (local mins/maxs, merged with its shadow children) and its local axes f (forward), l (left, which is negated so the frame is right-handed) and u (up).
2. **Projected frame.**
   - For each local axis a_i, scaled by box size s_i, project it into the plane perpendicular to d: t_i = s_i · (a_i − d·(d·a_i)).
   - y = normalize of the longest t_i.
   - x = y × d.
3. **Projected size.**
   - size.x = Σ_i s_i · |a_i·x| + 4;
   - size.y = Σ_i s_i · |a_i·y| + 4.
4. **Origin and falloff start.**
   - c = box centre;
   - r = half the box diagonal;
   - m = −(c·d_local) + Σ_i d_local,i · (min_i if d_local,i > 0 else max_i). This is the signed distance from the centre back to the box corner furthest against d.
   - origin = c + m · d (in local space, then moved to world space by the caster's origin and axes).
   - falloff_start = r − m.
   - max_dist = cast_distance + falloff_start.
5. **World-to-texture.** shadow-space coordinates are (x·(P−o), y·(P−o), d·(P−o)); u = x/size.x + 0.5 and v = y/size.y + 0.5.
6. **Receivers.**
   - The engine collects world leaves along a swept box from origin to origin + max_dist·d. The box half-extent is ½·√(size.x² + size.y²).
   - It clips the shadow onto world surfaces in those leaves, like a decal, and gives each vertex a fade value (*inferred*, see D).
   - Three extra clip planes (the faces of the caster box that face the light) stop the shadow appearing on surfaces in front of the caster.
7. **Texture size.**
   - texels = 2 · max(box size x, y, z); allocation size = the next power of two ≥ texels, clamped to 16..256.
   - Each frame the actual cell used depends on the shadow's screen size: s = the pixel diameter of the caster's bounding sphere, area = s². The cell is the smallest of 16, 32, 64, 128, 256 with cell² ≥ area, capped at the allocation size.
   - If the atlas has no free cell of that size this frame, a smaller cell is used. An existing cell is kept if it is within one size step and the texture isn't dirty.
   - The 32 largest on-screen shadows (by area) get textures. The rest fall back to blob shadows that frame.
8. **Silhouette render.**
   - The cell (viewport) is cleared to RGBA (255, 255, 255, 0).
   - The caster and its children are drawn orthographically, mapping shadow space to the cell. y is flipped, and depth is unbounded behind (−9999..0).
   - Shading: colour = white; alpha = base-texture alpha × vertex/modulation alpha, if the caster's material is translucent or alpha-tested. Otherwise alpha = 1.
   - Additive blending (ONE, ONE), no depth test or write, both faces drawn. So alpha saturates at 1 where parts overlap.
   - **Result: a binary-ish coverage mask in alpha.** Translucent parts give partial alpha.
9. **Texture coordinates of the cell in the atlas:**
   - u0 = (x + 0.5)/1024, v0 = (y + 0.5)/1024;
   - du = (w − 1)/1024, dv = (h − 1)/1024.
   - The half-texel inset avoids bleeding from neighbouring cells.

### D. How a receiver is darkened

**World and brush-model surfaces** use the shader "Shadow", drawn after the surface with:

```math
\text{dst} \leftarrow \text{dst} \times \text{result}
```

- The blend is (zero, src colour), with sRGB write enabled.
- The shadow texture is read with sRGB decode, but only its alpha is used, and alpha is not affected by sRGB.
- cov = clamp(mean of 5 alpha samples − fade, 0, 1).
  - The 5 samples are at uv, uv ± (1/1024, 1/1024) and uv ± (1/1024, −1/1024).
  - fade is the per-vertex value from the engine's clipper, interpolated.
- result = 1 + cov · (C − 1) = lerp(1, C, cov), where C = gamma→linear(shadow colour / 255).
- Fog: result = 1 − (1 − result) · (1 − f)^4, where f is the pixel fog factor (0 = no fog). The shadow material fogs toward white, so shadows vanish in fog.
- Alpha output = 1. There is no clamp beyond the framebuffer's.
- **Fade (*inferred*).** The engine sets fade = falloff_amount · clamp((z − falloff_start) / (max_dist − falloff_start), 0, 1) + bias/255.
  - z is the receiver point's distance along d from the shadow origin. bias is the caster's falloff bias.
  - Points beyond max_dist (z > max_dist) or behind the origin (z < 0) receive no shadow.
  - The SDK's unused model-receiver shader computes the same ramp ((z − offset)·scale, times amount, with a hard cut at both ends). The shadow manager passes falloff_amount = 240 (read as 240/255).
  - So at the far end the shadow is at most 0.94 lighter. It does not vanish completely before the hard cut.
- **Receiver rules:**
  - The caster never receives its own shadow.
  - Entities with `disablereceiveshadows 1` (or the "no receive shadow" effect) are skipped.
  - **Studio models (players, props) never receive** render-to-texture shadows.
  - **Static props** receive them only from players and NPCs.
  - **World brushes and brush entities** receive them from everything.
  - A receiver is also culled if:
    - its bounding sphere misses the shadow box (|x| ≤ size.x/2, |y| ≤ size.y/2, 0 ≤ z ≤ max_dist);
    - or it is on the light side of the caster. That test uses a separating plane between the two bounding spheres (or boxes), checked against d.
- Displacements and other world surfaces: receive like brushes (*inferred*, engine clipper).

**Blob shadows** (r_shadowrendertotexture 0, or beyond the 32-per-frame limit):
- Same blend and shader with a fixed round texture.
- Projected **straight down** (0, 0, −1) whenever render-to-texture is off. When a blob is only a per-frame fallback, the global direction is still used.
- Size: two of the caster box's axes most perpendicular to d, projected, + 10 units each, minimum 10.
- Origin backed up 2× further than for render-to-texture shadows, and snapped to 0.5 units.

### E. Sky drawing

- **Compile time** (from the map compiler):
  - Faces whose material has `%compileSky` (e.g. tools/toolsskybox) get the "sky" surface flag. Faces with `%compile2DSky` (tools/toolsskybox2d) get "sky + sky2D".
  - All sky faces of a kind share one texinfo. They are kept in the BSP for visibility and lighting, but they are **never drawn and never receive decals or overlays**.
  - Nodraw faces (tools/toolsnodraw) are not drawn either. Unlike sky faces, they give no hint to draw anything behind them.
- **Leaf sky flags** (computed by the radiosity compiler):
  - A non-solid leaf containing a sky face is flagged "3D sky" (sky) or "2D sky" (sky2D).
  - Any other non-solid leaf whose PVS contains a flagged leaf inherits: "3D sky" wins over "2D sky".
- **Per frame** (client view code; the engine part is *inferred*):
  1. The colour clear depends on the case:
     - **Default:** the main view clears depth and stencil only, **not colour**. gl_clear 1 adds a colour clear.
     - **The eye is inside solid:** colour is cleared to black.
     - **The eye is in a water/fog volume:** colour is cleared to the fog colour.
     - **Radial-distance vis is in use with no visible sky:** colour is cleared to the fog colour. dust2 doesn't use radial vis.
  2. The engine reports whether the camera's leaf sees 3D sky, 2D sky or none (the leaf flags above).
  3. **3D skybox.** It is drawn if the leaf sees 3D sky, r_3dsky is 1, and the map has a sky_camera (its area ≠ 255).
     - The scene is rendered from eye/scale + sky_camera origin, with near plane 2 and the sky_camera's own fog. This uses the incoming clear flags, so it does not clear colour.
     - The 2D sky box is drawn inside this pass when r_skybox is 1.
     - Then depth (and stencil) is cleared before the main world.
  4. **Main world.** The 2D sky box is drawn here instead only if the 3D skybox wasn't drawn **and** the leaf sees sky (2D or 3D). If the leaf sees no sky, no sky is drawn at all.
  5. World, props and translucents are drawn over whatever the sky left.
- **Consequences:**
  - On dust2, which has a sky_camera with scale 4, the visible sky is the 3D skybox with the 2D sky_dust box behind it.
  - From a leaf that sees no sky, the sky is not drawn. Holes show stale pixels.
  - Looking out of the map (noclip in the void): the camera is in solid, so black.
  - Looking through a nodraw face: stale pixels or the sky drawn this frame.
- **The 2D sky box itself** (engine, *inferred* from content): six faces using materials skybox/⟨skyname⟩ + rt, bk, lf, ft, up, dn. skyname comes from worldspawn ("sky_dust" on dust2). The faces are drawn around the eye at "infinite" distance, with the Sky shader (see shaders.md, section 6). The exact draw order relative to world and depth is engine-side; see Open questions.

## Per-frame order (shadows)

1. Every shadow whose caster moved, rotated or is animating gets its frame, size, origin and clip planes recomputed (C1–C6). The engine re-clips its world decal.
2. List visible shadows in the view's leaves. Drop those with bias 255 and those outside the frustum. Sort by screen area, largest first.
3. For the first 32: pick an atlas cell (C7). Redraw the silhouette if it is dirty or was moved to a new cell (C8). The rest use blob shadows.
4. World and brush surfaces are drawn. Shadow decals are drawn on them with the multiply blend (D), after the surface and in the same pass as decals.
5. Studio models are drawn (they receive nothing).

## Edge cases

- **Overlapping shadows** multiply: two full-coverage shadows of colour C give dst·C².
- **Shadow over a dark surface:** the multiply blend keeps black black. There is no "shadow brighter than the surface" case.
- **sRGB write with blending.** The result is computed in linear space. Whether the multiply happens on linear or gamma-encoded framebuffer values depends on the GPU/driver (DX9 era: usually gamma; DXVK: linear).
  - Under a pure-power transfer curve, a full-coverage shadow gives the same result either way, because power curves are multiplicative.
  - Partial coverage differs slightly.
- **Very thin or flat casters** still get a 4-unit-wide texture margin, and their texture size is at least 16.
- **A caster taller than about 128 units** hits the 256-texel cap, so its resolution drops below 2 texels/unit.
- **Directional cut-off:** a shadow never extends further than cast_distance beyond the caster's bounding sphere along d. For dust2 that is 75 units, so shadows on distant floors simply end.
- **The clip planes** use the caster's box faces facing toward the light. Thin walls behind the light side of a caster don't get shadowed.

## Quirks

- **Shadows ignore actual lights.** One global direction for the whole map, even indoors and in dark corners.
- **The shadow doesn't fully fade before the hard end.** It reaches at most 94% lightening, then stops abruptly at max_dist (*inferred* from the ramp and amount).
- **Players don't receive** other players' shadows, and props don't receive prop shadows.
- **A colour clear is skipped every frame**, so out-of-map views smear.
- **The model-receiver shader is marked unused** in the SDK. Static-prop receivers use the engine's own path.

## Test cases

| Setup | Input | After | Expected |
|---|---|---|---|
| shadow_control angles | (60, 43, 0) | direction | (0.36568, 0.34100, −0.86603) |
| shadow_control angles all zero | (0, 0, 0) | direction | forward of (80, 30): (0.15038, 0.08682, −0.98481) |
| No shadow_control | - | direction / distance | (0.09901, 0.09901, −0.99015) / 50 |
| Default colour from ambient | ambient A = (0.1, 0.2, 0.3) | colour bytes | (153, 229, 255) |
| Texture size | caster box 30 × 30 × 10 | allocation | 64 |
| Texture size | oil drum ≈ 28 × 28 × 46 | allocation | 128 (2·46 = 92 → 128) |
| Texture size | player ≈ 32 × 32 × 72 | allocation | 256 (2·72 = 144 → 256) |
| Texture size | can 6 × 6 × 8 | allocation | 16 (minimum) |
| Cell from screen size | sphere 30 px across → area 900; allocation 128 | cell | 32 (16² = 256 < 900 ≤ 32² = 1024) |
| Cell from screen size | area 50 000; allocation 128 | cell | 128 (capped) |
| Origin and falloff, d straight down | axis-aligned box 32 × 32 × 48 | m, origin, falloff_start, max_dist (dist 50) | m = −24; origin = box top centre; r = 32.985; falloff_start = 56.985; max_dist = 106.985 |
| Same box, dust2 distance | dist 75 | max_dist | 131.985 |
| Projected size, straight down, axis-aligned | box 32 × 32 × 48 | size | (36, 36) |
| Receiver, full coverage | dst linear 0.5, colour 159 (C = (159/255)^2.2 = 0.35373) | dst | 0.17686 (gamma ≈ 0.4547, i.e. 116) |
| Receiver, half coverage | cov 0.5, same C | result | 0.67686 |
| Receiver, fog | cov 1, f = 0.5 | result | 1 − 0.64627 · 0.0625 = 0.95961 |
| Blur | alpha samples (1, 1, 1, 0, 0), fade 0 | cov | 0.6 |
| Fade (*inferred*) | z = falloff_start + 0.5·(max_dist − falloff_start), bias 0 | fade / cov from full alpha | 0.47059 / 0.52941 |
| Fade beyond end | z > max_dist | shadow | none |
| Overlap | two shadows, cov 1, C = 0.35373, dst 1 | dst | 0.12513 |
| Sky, camera in 3D-sky leaf with sky_camera | r_3dsky 1 | order | 3D sky (with 2D box) → clear depth → world; no colour clear |
| Sky, leaf sees no sky | - | sky drawn? | no; background = previous frame |
| Camera in solid | - | colour clear | black |

## Open questions

1. **Engine shadow clipper.** The exact per-vertex fade formula, how the bias adds in, whether displacements, translucent or `$nodecal` surfaces receive, and the depth bias. To check: RenderDoc a crate on dust2 and read the shadow decal's vertex colours and alpha.
2. **Static props as casters.** Believed not to cast at runtime; confirm in game (a static barrel next to a physics barrel).
3. **CS:S player shadows.** Any player-specific overrides (shadow for dead or ragdoll players, the local player's own shadow in first person, weapon world-model children).
4. **The ambient colour** used for the no-shadow_control default. Which engine value it is (light_environment ambient? scaled how?). Not needed for dust2.
5. **2D sky box placement and order.** Whether it is drawn before or after opaque world, with depth test/write; its distance and face orientation. Our renderer already measured the cube orientation (see the sky fix commit). Confirm the order in RenderDoc.
6. **Whether the user's video settings change r_shadowrendertotexture** (the "Shadow detail" option) on the reference machine. Assumed 1.
7. **The blending colour space** on the reference (DXVK) is linear. A Windows D3D9 driver may blend in gamma space; this only matters for partial coverage.
