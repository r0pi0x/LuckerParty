# Counter-Strike: Source: Water (DX9, LDR)

Source basis: Valve's public Source SDK 2013 (current GitHub head). I read the DirectX 9 standard shader library's water shader (C++ parameter set-up, pass selection, constants) with its shader model 2.0/2.0b vertex and pixel shaders and the shared shader include files (normal decode, pixel fog, final output), plus the separate "cheap water" vertex and pixel shaders the same material draws with. On the client side I read the view-rendering code that decides the water mode and sets up the reflection, refraction, intersection and under-water views (clip heights, fudge offsets, clear colours, draw flags), the render-target creation code, the water-LOD entity and material proxy, and the texture-scroll and animated-texture material proxies. The engine (fog-volume state, which surfaces are drawn per view, the bottom-material swap, material patching) and the D3D9 shader API are **not** in the SDK; what they do is described from what the client and shaders ask of them, and marked *derived* or listed under Open questions. Material data: the water materials and their in-map patch files in the user's CS:S install (stock maps and `cstrike_pak`/`hl2_misc` VPKs), the `water_lod_control` and `env_fog_controller` entities of every stock map, the default video settings file (`bin/dxsupport.cfg`), and the headers and texel statistics of the water normal and DuDv textures. Nothing extracted is committed.
Status: draft

## Summary

- A `Water` surface at DX9 is drawn in **one or two passes**:
  1. **Expensive pass** (opaque): samples a screen-sized **refraction** texture (the scene below the surface, rendered this frame with water fog, its alpha holding "how fogged") and, if the material has `$reflecttexture`, a **reflection** texture (the scene rendered from a camera mirrored across the water plane). The two lookups are offset by the normal map. A Schlick-like Fresnel term `(1 − N·V)^5` blends refraction → reflection, and the refraction is pushed toward `$fogcolor` by its fog alpha.
  2. **Cheap pass** (alpha-blended over pass 1), drawn only when there is no reflection texture but there is an `$envmap`: the cubemap reflection, with alpha = Fresnel + a distance ramp from `$cheapwaterstartdistance` to `$cheapwaterenddistance`, faded out at the shore.
- Which stock map gets what (default DX9 settings, mat_hdr_level 0):
  - **Reflection + refraction** (real planar reflection): de_chateau, de_piranesi (`liquids/water_pretty1`, entities reflected), de_port (`nature/water_wasteland002b`, world only by default).
  - **Refraction + cubemap reflection** (cheap pass): de_aztec, de_inferno, cs_militia (`liquids/aztecwater`, `infernowater`, `militiawater`).
  - **Not the Water shader at all**: de_nuke's pool is an additive LightmappedGeneric material (see [shaders.md](shaders.md)); only its fog volume follows this spec.
  - cs_italy, de_cbble, de_dust/dust2, de_train and the rest have no water.
- Several material parameters do **nothing** in the DX9 shader at its default settings. The ones people expect to matter: `$refracttint` (applied only with `$blurrefract 1`), `$dudvmap`/`$bumpmap` (DuDv maps are a DX8 thing), `$scale`, `$time`, `$waterdepth`, `$envmaptint`. `$reflectamount`/`$refractamount` are **distortion strengths**, not blend weights.
- Under water the screen is cleared to the water fog colour and everything is range-fogged with the water's `$fogcolor`/`$fogstart`/`$fogend`. The surface seen from below is drawn with the `$bottommaterial` (a refraction-only Water material with `$abovewater 0`).

## Units and conventions

- World units (inches), Z up, as everywhere in Source. The water plane is horizontal at height `W` (the top of the water volume).
- Colours 0..1. A byte `b` in a `{r g b}` material colour means b/255; `[r g b]` is already 0..1. "Gamma" is stored/display encoding, "linear" is after decode. Material colour parameters go gamma → linear with a pure 2.2 power where stated (as in [shaders.md](shaders.md) section 1).
- Screen/texture coordinates: `u` right, `v` down, 0..1. Clip space: `(x, y, z, w)` after the projection; NDC `x/w`, `y/w` in −1..1 with `y` up; `z` (pre-divide) is the "clip-space depth ≈ view distance" used by Source's pixel fog ([shaders.md](shaders.md) section 7, Open question 12).
- Tangent space: x = texture u, y = texture v, z = surface normal (up, for a water top surface).
- Time: `t` in seconds. Material proxies use the client's game time; the shader's own scroll uses the shader API's current time (Open question 6). Everything is per frame; nothing is tick-based.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| fresnel_power | 5 | - | Fresnel = (1 − N·V)^5, no bias, no scale |
| fog_alpha_bias | 0.05 | refraction alpha | subtracted before the refraction→fog blend and the shore fade |
| shore_fade_gain | 20 | - | shore fade = clamp((a − 0.05)·20, 0, 1): fully faded in at a ≥ 0.10 |
| multitex_weight | 0.33 | - | each of the three scrolled normal samples is weighted 0.33 (not 1/3) |
| multitex_scale_1 | 0.1 | uv multiplier | second normal layer: 0.1 · (u + v, v − u), i.e. rotated 45° and ×0.1414 bigger texels |
| multitex_scale_2 | 0.45 | uv multiplier | third normal layer: 0.45 · (v, u) |
| blur_step | 0.005 | uv | `$blurrefract` 5×5 box step |
| clip_fudge | 2 | units | refraction views clip/fog at W + 2; reflection clips at W − 2 |
| intersect_fudge | 7 | units | near-plane-crosses-water test margin |
| eye_water_epsilon | 10 | units | `r_eyewaterepsilon` (cheat), used only with software clip planes |
| water_rt_size | 1024 × 1024 | texels | `_rt_WaterReflection` / `_rt_WaterRefraction`, shrunk by mat_picmip (Open question 3) |
| refraction_rt_format | RGBA 8:8:8:8 | - | alpha holds the water-fog factor |
| reflection_rt_format | back-buffer format | - | alpha unused |
| cheap_start_default (shader) | 500 | units | `$cheapwaterstartdistance` if the material does not set it |
| cheap_end_default (shader) | 1000 | units | `$cheapwaterenddistance` if the material does not set it |
| view_cheap_start_default | 0 | units | client-side LOD start when the map has no `water_lod_control` |
| view_cheap_end_default | 0.1 | units | client-side LOD end when the map has no `water_lod_control` |
| water_lod_control defaults | 1000 / 2000 | units | entity keyvalues `cheapwaterstartdistance` / `cheapwaterenddistance` when unset |
| missing_fogcolor | (1, 0, 0) | gamma | `$fogcolor` if the material forgets it (red water, with a console warning) |
| missing_abovewater | 1 | - | `$abovewater` if unset (with a console warning) |

## Behavior

### 1. Settings: expensive vs cheap

Console variables (all client-side):

| Convar | Default | Effect |
|---|---|---|
| `r_waterforceexpensive` | 0 in code; **1** from `dxsupport.cfg` at dxlevel 90/95/98, 0 below | allows planar reflection (see below). Archived; the user's config has 1 |
| `r_waterforcereflectentities` | 0 in code; dxsupport: 0 at 90/95, **1 at 98** | reflection view also draws models (players, props, static props, weapons) |
| `r_WaterDrawReflection` | 1 | 0 disables the reflection view |
| `r_WaterDrawRefraction` | 1 | 0 disables the refraction view (water turns opaque/cheap) |
| `mat_drawwater` | 1 (cheat) | 0 hides water surfaces |
| `r_ForceWaterLeaf` | 1 | culling optimisation for views that start in a water leaf |
| `mat_clipz` | 1 | 0 disables the water-height clip in the sub-views |
| `r_cheapwaterstart` / `r_cheapwaterend` | (commands) | print or set the client LOD distances |
| `r_debugcheapwater` | 0 (cheat) | prints the decision |
| `mat_specular` | 1 | 0 zeroes the cheap pass's reflection colour |

The in-game "Water detail" option maps to: Simple reflections = (0, 0), Reflect world = (1, 0), Reflect all = (1, 1) for (`r_waterforceexpensive`, `r_waterforcereflectentities`). The CS:S default on any modern GPU is **dxlevel 95 → Reflect world**; mat_hdr_level is 2 at dxlevel 95 by default but this spec targets mat_hdr_level 0 (the user's config).

Per material:

- `$forcecheap 1` → never expensive (no reflection or refraction view; only the cheap pass, unblended, see section 7).
- `$forceexpensive`. **Quirk:** on PC the shader sets `$forceexpensive 1` on every Water material that does not set it. The client reads this back. So "force expensive" is effectively **on for every DX9 Water material** unless `$forcecheap` is set, whatever `r_waterforceexpensive` says (Open question 1). With the user's settings (`r_waterforceexpensive 1`) the result is the same either way.

Per frame, for the water volume the camera can see (one water volume per view; Source picks the visible fog volume from the camera position):

1. Let `force_expensive` = `r_waterforceexpensive` OR `$forceexpensive`; `force_cheap` = `$forcecheap`; `force_cheap` wins.
2. **Planar reflection** is on when `force_expensive`, `r_WaterDrawReflection` is 1, and the material has `$reflecttexture` set to a texture (stock: `_rt_WaterReflection`, only in water_pretty1/canals_water2's `Water_DX90` block and in water_wasteland002b). An unset `$reflecttexture` does **not** count (Open question 2).
3. If (the camera's distance to the water volume ≥ the client's cheap **end** distance AND there is no planar reflection) OR `force_cheap`: **cheap mode**. No sub-views are rendered.
4. Otherwise **refraction** is on when `r_WaterDrawRefraction` is 1 and `$refracttexture` is a texture (all stock Water materials set `_rt_WaterRefraction`).
5. Entities in the reflection: `r_waterforcereflectentities` OR `$reflectentities`.
6. Cheap mode = neither reflection nor refraction.

The client's cheap distances come from the map's `water_lod_control` (if absent: 0 and 0.1, so any water ≥ 0.1 units away is cheap unless it has a planar reflection). The `WaterLOD` material proxy copies the same two numbers into `$cheapwaterstartdistance`/`$cheapwaterenddistance` every bind, so the material's own values are overridden whenever the proxy is present (all stock top-surface materials have it).

Inside the material, the passes are:

- **Pass 1 (expensive)** if not `force_cheap` and (planar reflection or refraction). It has REFLECT = planar reflection (`force_expensive` AND `$reflecttexture`) and REFRACT = `$refracttexture`.
- **Pass 2 (cheap)** if there is no planar reflection, `$envmap` is a texture, and the material is not a decal. It is alpha-blended (section 7) unless `force_cheap`, in which case it is opaque.
- If neither applies, the surface is drawn with no shader output that matters (tools fallback).

### 2. Parameters

| Parameter | Default | Used by | Effect |
|---|---|---|---|
| `$normalmap` | `dev/water_normal` | both passes | the normal map; read raw (no sRGB). Animated frames via `$bumpframe` |
| `$bumpframe` | 0 | both | frame of `$normalmap` (driven by an AnimatedTexture proxy, section 4) |
| `$bumptransform` | identity | pass 1 only | uv transform of the first normal sample (driven by a TextureScroll proxy). **Pass 2 ignores it** |
| `$scroll1`, `$scroll2` | [0 0 0] | both | if `abs($scroll1.x) > 0`, three-layer scrolling normals (section 4). Only x, y are used |
| `$refracttexture` | (render target) | pass 1 | `_rt_WaterRefraction`; sRGB-decoded on read |
| `$reflecttexture` | (render target) | pass 1 | `_rt_WaterReflection`; sRGB-decoded on read |
| `$refractamount` | 0 | pass 1 | refraction lookup offset per unit of normal xy, in screen-uv units |
| `$reflectamount` | 0.8 | pass 1 | reflection lookup offset per unit of normal xy |
| `$refracttint` | [1 1 1] | pass 1, **only with `$blurrefract 1`** | gamma → linear, multiplies the refraction |
| `$reflecttint` | [1 1 1] | both | pass 1: gamma → linear multiply on the reflection. Pass 2: multiplies the envmap **raw** (no conversion) |
| `$fogcolor` | (1,0,0) + warning | both, and the fog volume | pass 1: gamma → linear. Pass 2: raw |
| `$fogstart`, `$fogend` | 0 | pass 1 under water, and the fog volume | units |
| `$abovewater` | 1 + warning | pass 1 | 1 = top surface (fog from refraction alpha), 0 = bottom surface (fog from depth) |
| `$bottommaterial` | none | engine | material drawn for the surface when seen from below (section 9) |
| `$underwateroverlay` | none | client | full-screen material drawn while the eye is in this water (section 9) |
| `$envmap` | `env_cubemap` | pass 2 | cubemap, read **raw** in LDR |
| `$cheapwaterstartdistance` / `enddistance` | 500 / 1000 | pass 2 | overwritten by the WaterLOD proxy |
| `$forcecheap`, `$forceexpensive` | 0 / 1 (PC) | section 1 | |
| `$reflectentities` | 0 | section 1 | |
| `$nofresnel` | 0 | pass 2 | 1 = use `$reflectblendfactor` instead of Fresnel |
| `$reflectblendfactor` | 1 | pass 2 | Fresnel replacement when `$nofresnel 1` |
| `$blurrefract` | 0 | pass 1 (ps2.0b) | 5×5 box blur of the refraction, enables `$refracttint` |
| `$basetexture` | none | pass 1 | lightmapped base variant (no stock CS:S material uses it; section 6.6) |
| `$nolowendlightmap` | 0 | - | lightmap allocation only |
| `$waterdepth` | (map patch) | - | required to be defined; not used for rendering. vbsp writes `*_depth_N` patch materials with it |
| `$scale`, `$time`, `$bumpmap` (DuDv), `$dudvmap`, `$envmaptint`, `$envmapframe` (pass 2 does use it as the cubemap frame), `$fogenable`, `%compilewater`, `%tooltexture`, `$surfaceprop` | - | - | no effect on DX9 rendering (`$surfaceprop` is physics; `%compilewater` marks the brush as a water volume) |

### 3. Producing the reflection and refraction textures (eye above water)

The views are rendered **before** the main view each frame, in this order: reflection (if on), refraction (if on), main view, then the intersection view (if needed).

**Reflection view** (`_rt_WaterReflection`):
- Camera: the main camera mirrored across the water plane `W`:
  - eye z' = 2W − eye z (x, y unchanged);
  - pitch' = −pitch, roll' = −roll, yaw unchanged;
  - same field of view and the **main view's aspect ratio**, rendered into the full square render target (so the image is squashed horizontally on a widescreen display; it is stretched back when sampled with screen coordinates).
  - Because this is an upright camera rather than a true mirror, the image comes out vertically flipped relative to the mirror image. The water shader's reflection lookup uses `v = (1 + y_ndc)/2` (no y-flip) to undo this (section 5).
- Clip: only geometry **above** the height `W − 2` is drawn (hardware user clip plane; `mat_clipz 1`).
- Drawn: world brushes, translucent world brushes, the **2D skybox** (not the 3D skybox) and, only with reflected entities, all models and other renderables (opaque then translucent). No water surfaces. Without entities there are **no static props, detail props, players or weapons** in the reflection.
- Fog: normal world fog (the map's `env_fog_controller`), as in the main view.
- Clear: depth only (the skybox covers the colour).

**Refraction view** (`_rt_WaterRefraction`), eye above water:
- Camera: identical to the main view (same eye, angles, FOV, main aspect), rendered into the square render target.
- Clip: only geometry **below** `W + 2` is drawn. Only under-water leaves are drawn; entities are drawn.
- Fog: the water's **height fog** (section 8), with the fog plane at `W + 2`. Every opaque surface writes its fog factor `f_h` into the **alpha** channel and blends its colour toward the water fog colour by the same `f_h`.
- Clear: colour = water fog colour, alpha = 1.0; depth cleared. Where nothing is drawn, the texture reads "fully fogged".
- No water surfaces.

**Main view** (eye above water):
- Draws above-water leaves, the water surface and entities with world fog.
- When refraction is on, under-water geometry is **not** drawn in the main view: everything below the surface is only seen through the refraction texture on the water. When refraction is off and the water material is translucent (de_nuke's additive pool), under-water leaves are drawn directly in the main view.
- When refraction is off and there is no 3D skybox, the main view also clears colour.

**Intersection view**: if refraction is on and the camera's near plane straddles the water (some corner of the near-plane rectangle is within 7 units above and some within 7 units below `W`, and the box around them touches the water volume):
- the main view is clipped to "above `W − 2`";
- an extra pass then draws the under-water part (clip "below `W − 2`", entities, water height fog), so the slice of the screen below the waterline shows fogged geometry directly.

### 4. Normal sampling and animation

Vertex texture coordinate `(u, v)` = the face's texinfo mapping divided by the material's texture size. All stock water normal/DuDv maps are 256 × 256, so this is 256 either way.

- **Single layer** (`$scroll1.x` = 0):
  - uv₀ = `$bumptransform` applied to (u, v) (rows · (u, v, 0, 1));
  - texel = sample(`$normalmap` frame `$bumpframe`, uv₀);
  - N = 2·texel.rgb − 1, Nₐ = texel.a (raw). N is **not renormalised**.
- **Three layers** (`abs($scroll1.x) > 0`, de_port), with T = shader time:
  - uv₀ as above;
  - uv₁ = (0.1·(u + v) + T·scroll1.x, 0.1·(v − u) + T·scroll1.y);
  - uv₂ = (0.45·v + T·scroll2.x, 0.45·u + T·scroll2.y) (note u and v swapped);
  - s = 0.33 · (texel(uv₀) + texel(uv₁) + texel(uv₂)) on all four channels;
  - N = 2·s.rgb − 1, Nₐ = s.a (so ≈ 0.99 for opaque alpha).
  - **Quirk:** 0.33 instead of 1/3 biases a flat normal: three (128,128,255) texels give N ≈ (−0.0061, −0.0061, 0.98).
  - The shader cannot have three layers and `$basetexture` together.
- **TextureScroll proxy** (aztec/inferno/militia/pretty1: rate r, angle a): `$bumptransform` = translation (frac(r·t·cos a), frac(r·t·sin a)), scale 1, where frac keeps the result in [0, 1) (negative values wrap up).
- **AnimatedTexture proxy** on `$normalmap`: frame = floor(rate · t) mod frame_count, t from map start. `dev/water_normal` has 29 frames; stock rate 16 fps → one loop every 1.8125 s. Frames are not blended (hard switch).
- Texture statistics, for reference: `dev/water_normal` and `nature/water_coast01_normal` are 256 × 256 BGRA8888 with 9 mips, mean (R, G, B) ≈ (126.8, 126.8, 254.0), alpha 255 everywhere. So Nₐ = 1 and the normals are close to flat.

### 5. Screen-space lookups (pass 1)

In the vertex stage, with clip position (x, y, z, w):

- reflect_base = ((x + w)/2, (y + w)/2), i.e. after dividing by w: u = (1 + x_ndc)/2, **v = (1 + y_ndc)/2** (bottom of the screen = v 0: vertically flipped);
- refract_base = ((x + w)/2, (−y + w)/2), i.e. the usual screen uv, v down.

Both are interpolated, then divided by w per pixel. Then:

- a₀ = alpha of the refraction texture at refract_base (undistorted). For `$abovewater 0`, a₀ = 1.
- Distortion strengths: k_refl = `$reflectamount`, k_refr = `$refractamount`. Unless `$blurrefract 1` (or `$basetexture` set), **both are multiplied by a₀**. Distortion therefore fades to nothing where the water is shallow or close to its edges (small fog factor).
- reflect_uv = reflect_base + k_refl · Nₐ · (N.x, N.y)
- refract_uv = refract_base + k_refr · Nₐ · (N.x, N.y)
- The same tangent-space (N.x, N.y) is added to both. Because the reflection's v axis is flipped, a given normal moves the reflection and the refraction in opposite screen-vertical directions.
- No clamp on the uvs; the render targets use clamp-to-edge addressing.

### 6. Pass 1 per-pixel maths

Inputs: R = sRGB-decode(reflection texture at reflect_uv); P = sRGB-decode(refraction texture at refract_uv), P.a = raw alpha there.

1. **Reflection colour:** R ← R · linear(`$reflecttint`) (pure 2.2 power per channel).
2. **Refraction colour:**
   - default: P as sampled, **no `$refracttint`** (quirk);
   - `$blurrefract 1` (ps2.0b): P = mean of 25 samples at refract_uv + (i·0.005, j·0.005), i, j ∈ {−2..2}; then P ← P · linear(`$refracttint`) and R ← R · overbright (1 in LDR).
3. **Fog depth a:**
   - `$abovewater 1`: a = P.a at the distorted uv (the blurred alpha with `$blurrefract`). Note a₀ (undistorted) scales the distortion, but this distorted a drives fog and the shore fade.
   - `$abovewater 0`: a = 1.
4. **Fresnel:**
   - V = normalize(eye − position), expressed in the surface tangent frame using the vertex tangent, bitangent and normal (interpolated, normalised per pixel).
   - F = (1 − clamp(V · N, 0, 1))^5, with the un-normalised N from section 4.
   - Without `$basetexture`: F ← F · clamp((a − 0.05)·20, 0, 1) (**shore fade**: no reflection where the refraction alpha is below 0.05, full above 0.10).
5. **Refraction fog:**
   - `$abovewater 1`: P ← lerp(P, linear(`$fogcolor`)·L, clamp(a − 0.05, 0, 1)), with L = linear light scale (1 in LDR).
   - `$abovewater 0`: g = clamp((z − `$fogstart`)/(`$fogend` − `$fogstart`), 0, 1), with z the clip-space depth of the **water surface pixel**; P ← lerp(P, linear(`$fogcolor`)·L, g).
   - **Quirk (double fog):** above water, the refraction texture's colour was already blended toward the fog colour by f_h when it was rendered (section 3). Here it is blended again by (a − 0.05). Keep both.
6. **Combine:**
   - reflection and refraction: C = lerp(P, R, F);
   - reflection only: C = R (no Fresnel);
   - refraction only: C = P (every stock CS:S material except water_pretty1/canals_water2/wasteland002b; the cheap pass then adds the reflection);
   - neither: black.
7. **Output:** alpha 1 (opaque, no blending). Then the standard final step ([shaders.md](shaders.md) section 7): range fog with the current fog (world fog above water; water fog when the eye is under water) blended by f², then sRGB encode on write. The surface is not tone-mapped (no ×L at the end).

6.6 `$basetexture` variant (not used by stock CS:S water, for completeness): the base is sampled at uv₀ (sRGB), lit by the three bump lightmaps weighted by the **squared** clamped dot products with the bump basis, normalised by their sum and multiplied by the lightmap scale. With reflection only, C = base·light + R·F·base.alpha. Distortion is not scaled by a₀ and Fresnel has no shore fade.

### 7. Pass 2: cheap water

Vertex: world-space tangent frame (tangent, bitangent, normal), E = eye − position (world space), normal uv = the raw (u, v) (**no `$bumptransform`**), the same three-layer coordinates as section 4 when `$scroll1.x` ≠ 0, and the screen-uv refract_base when blending with refraction.

Pixel:

1. N as in section 4, then Nw = N.x·tangent + N.y·bitangent + N.z·normal (world space, not normalised).
2. d = |E|, Ê = E/d.
3. Reflection vector r = 2·(Nw·Ê)/(Nw·Nw)·Nw − Ê; S = envmap(r) · E_scale · `$reflecttint`. The envmap is read **raw** (no sRGB decode) and `$reflecttint` is not converted. E_scale is the envmap scale, expected 1 in LDR ([shaders.md](shaders.md) Open question 3). With `mat_specular 0`, S = 0.
4. Fresnel: F = (1 − max(0, Ê·Nw))^5, or `$reflectblendfactor` with `$nofresnel 1`.
5. **Blended (the normal case):**
   - ramp = clamp(d/(end − start) − start/(end − start), 0, 1) with start/end = `$cheapwaterstartdistance`/`enddistance` (i.e. the map's `water_lod_control` values via the WaterLOD proxy);
   - α = clamp(F + ramp, 0, 1);
   - if refraction is on: α ← α · clamp((a₀ − 0.05)·20, 0, 1), with a₀ = refraction alpha at the undistorted screen uv (shore fade);
   - output colour S, alpha α, blended "source alpha, one minus source alpha" over pass 1.
6. **Opaque (`$forcecheap`):** α = 1, colour = lerp(`$fogcolor` raw, S, F).
7. Final: × linear light scale, range fog f² with the current fog colour, and **no sRGB write in LDR**. The pass works on gamma values throughout: raw envmap in, raw out, and the blend with pass 1 happens on the gamma-encoded framebuffer. Treat it as "gamma-space shading" (Open question 5).

Net effect at default settings on de_aztec (start 500, end 2000):
- near water shows the fogged refraction with the cubemap added only at grazing angles (F);
- the cubemap fades in linearly from 500 to 2000 units;
- beyond 2000 units it covers the refraction completely.

### 8. Water fog in the refraction texture (height fog)

For an opaque surface point P rendered in the refraction view (eye E above water):

- depth_below = W' − P.z with W' = W + 2 (the fudged fog plane);
- h = clamp(depth_below / (E.z − P.z), 0, 1): the fraction of the eye→P segment that is under water;
- f_h = clamp(h · z · K, 0, 1), with z the clip-space depth of P and K the engine-set reciprocal fog range (Open question 4: expected 1/(`$fogend` − `$fogstart`); `$fogstart` itself does not enter the formula);
- colour ← lerp(colour, fog_linear, f_h) (not squared), and alpha ← f_h. Only fully opaque materials write f_h to alpha. Translucent ones keep their own alpha, but they are blended into what is already there.
- The vertex-shader formula for older hardware clamps depth_below ≥ 0 and is the same otherwise.

So a = 0 at the waterline and rises with the length of the underwater path, reaching 1 at a path length of 1/K. Pixels where nothing was drawn read a = 1 (clear).

### 9. Eye under water

The camera counts as under water when its position is inside the water volume (the engine's fog-volume test).

- **Refraction view (looking up out of the water)**, if refraction is on:
  - renders the **above-water** world (clip "above `W − 2`"), entities and the skybox (2D and 3D, the 3D skybox clipped) with world fog, into the back buffer, cleared to the water fog colour;
  - the viewport rectangle is then copied (stretched) into `_rt_WaterRefraction`;
  - its alpha is not meaningful, which is why `$abovewater 0` forces a = 1.
- **Main view:**
  - clear: depth; also colour = water fog colour when refraction is off;
  - draws under-water leaves, clip "below `W + 2`", entities, and the water surface;
  - when refraction is off and the water is translucent, also the above-water leaves;
  - **fog** = range fog using the water volume's fog: colour `$fogcolor`, start `$fogstart`, end `$fogend`, blended by f² ([shaders.md](shaders.md) section 7);
  - the 2D skybox is not drawn under water.
- **The surface from below:** the engine draws the water's `$bottommaterial` for the under side. All stock bottom materials are refraction-only Water materials with `$abovewater 0`, so the above-water world is seen through them:
  - distorted by `$refractamount` (full strength, a₀ = 1);
  - fogged by depth g (section 6, step 5);
  - then range-fogged again by the scene's water fog (f²).
  - No reflection and no cheap pass (bottom materials have no `$envmap`).
- **Screen tint:** there is no separate tint pass. The "tint" is the fog colour from the clear plus the range fog. If the fog-volume material has `$underwateroverlay` (stock: only `nature/water_coast01_beneath` names `effects/water_warp01`, a Refract-shader screen warp), that material is drawn full-screen after the view models (Open question 7).
- **Waterline split:** when the near plane crosses the surface with the eye just under water, the main view's clip still applies. With software clip planes (not the reference hardware) the water height is pushed 10 units (`r_eyewaterepsilon`) away from the eye.

### 10. Stock materials

All top surfaces are patched per map by vbsp:
- `materials/maps/<map>/...` files `include` the base material and replace `$envmap` with the nearest built cubemap `maps/<map>/c<x>_<y>_<z>`;
- the patch also writes an empty `"Proxies" {}` replace block (Open question 8);
- `*_depth_N` patches only insert `$waterdepth N` (physics).

| Map | Top material | Bottom material | Mode at defaults | `$refractamount` (DX90) | `$reflectamount` | `$fogcolor` | `$fogstart`/`$fogend` | normal anim / scroll | LOD (`water_lod_control`) |
|---|---|---|---|---|---|---|---|---|---|
| de_aztec | `liquids/aztecwater` | `dev/dev_waterbeneath2` | refraction + cheap cubemap | 0.2 | 0.8 (unused distortion, no reflection RT) | [.15 .1 0] → linear (0.01540, 0.00631, 0) | 5 / 50 | 16 fps, scroll 0.01 @45° | 500 / 2000 |
| de_inferno | `liquids/infernowater` | `dev/dev_waterbeneath2` | refraction + cheap | 0.5 | 0.8 | [.1 .12 .05] → (0.00631, 0.00943, 0.00137) | 5 / 50 | 16 fps, 0.01 @45° | 1000 / 2000 |
| cs_militia | `liquids/militiawater` | `liquids/militiawaterbeneath` | refraction + cheap | **0** (no distortion) | 0.8 | [.07 .14 .12] → (0.00288, 0.01323, 0.00943) | 5 / 80 | 16 fps, 0.05 @45° | 512 / 960 |
| de_chateau, de_piranesi | `liquids/water_pretty1` | `dev/dev_waterbeneath2` | reflection (**with entities**) + refraction | 1 | 0.3 | [.05 .05 0] → (0.00137, 0.00137, 0) | 0 / 50 | 16 fps, 0.01 @45° | 1000 / 2000 |
| de_port | `nature/water_wasteland002b` (unpatched: `$envmap env_cubemap`) | `nature/water_coast01_beneath` | reflection (world only; entities at dxlevel 98) + refraction, three-layer normals | **5** | 1 | {7 16 18} = (.0275, .0627, .0706); `srgb?$fogcolor` {21 48 52} = (.0824, .1882, .2039) (Open question 9) | 100 / 500 | no frame anim; `$bumptransform` scroll 0.02 @25°; scroll1 (.01, .01), scroll2 (−.025, .025) | 3000 / 5000 |
| de_nuke | `de_nuke/nukwater_movingplane` (**LightmappedGeneric**, additive, 21 fps base and bump animation) | `de_nuke/nukwater_movingplane_beneath` (UnlitGeneric, additive) | not the Water shader; water volume only | - | - | {150 200 200} | 64 / 512 | - | 1000 / 2000 |
| test_hardware | `nature/water_canals_water2` | `dev/dev_waterbeneath2` | as water_pretty1 | 1 | 0.3 | [.05 .05 0] | 0 / 200 | as pretty1 | 20000 / 30000 |

Bottom materials:

| Material | `$refractamount` | `$fogcolor` | start / end | other |
|---|---|---|---|---|
| `dev/dev_waterbeneath2` | 1.0 | {22 20 10} = (.0863, .0784, .0392) | 1 / 400 | `$reflectamount` 1 (no reflection texture), 30 fps frames, scroll 0.05 @45°, WaterLOD |
| `liquids/militiawaterbeneath` | 1 | [.07 .14 .12] | 5 / 150 | `$reflectamount` 0 |
| `nature/water_coast01_beneath` | 1 | {7 16 18} | **−256** / 512 | `$forceexpensive 1`, three-layer normals (same scrolls as the top), `$underwateroverlay effects/water_warp01` |

Edge notes on the table:
- Every DX90 block value overrides the material's root value at DX9 (`Water_DX90` blocks apply at dxlevel ≥ 90).
- The other `*_dx70`, `*_dx8`, `Water_DX6x/8x` fallbacks are irrelevant at DX9.

## Per-frame order

1. Find the water volume visible from the camera; decide the mode (section 1).
2. Eye above water: reflection view → reflection RT (if on); refraction view → refraction RT with height-fog alpha (if on).
   Eye under water: refraction view of the above-water world → back buffer → copied to the refraction RT (if on).
3. Main view: world, opaque entities, then translucent; the water surface is drawn with the world (pass 1, then pass 2 blended).
4. Intersection view, if the near plane crosses the water (above-water case).
5. View models, then the under-water overlay material (if any), then screen fades.

Within pass 1, per pixel: normal sample → a₀ → distorted uvs → sample R, P, a → tints → Fresnel (× shore fade) → refraction fog → combine → range fog (f²) → sRGB encode.

## Edge cases

- **Shore and thin water:** a → 0 where the refracted geometry is right at the surface. Distortion (∝ a₀), reflection (Fresnel × shore fade) and the cheap pass (× shore fade) all vanish there. This hides the seam where the water meets the shore.
- **Refraction picks up above-water objects:** the distorted lookup can land on a pixel of the refraction texture whose geometry is above the plane. That pixel shows the fog-colour clear (a = 1) unless something under water was drawn there. This shows as fog-coloured fringes at strong distortion (de_port, `$refractamount` 5). The refraction clip at W + 2 lets a 2-unit sliver of shoreline into the texture.
- **Water out of the LOD range:** beyond the cheap end distance, aztec/inferno/militia water renders no sub-views. Pass 1 then samples a refraction texture left from an earlier frame, but pass 2's α = 1 covers it. The shore fade there reads stale alpha. Implementation: treat a₀ as 1 in that case (Open question 10).
- **Several water volumes:** only one water volume's views are rendered per frame (the one picked from the camera). Other water surfaces sample the same render targets made for a different plane height (visible mismatches in the original; e.g. de_aztec has surfaces at −455, −352, −157 and −149).
- **`$fogend` = `$fogstart`:** division by zero in the under-water surface fog; no stock material does this.
- **`$fogstart` < 0** (water_coast01_beneath, −256): legal; g > 0 already at the surface.
- **Grazing angles:** V·N < 0 (only possible with tilted normals) clamps to 0 → F = 1.
- **The refraction RT is square (1024²) but sampled with full-screen uvs.** It is a squashed full-screen image; resolution is lower horizontally on wide screens.
- **Reflection without entities** omits static props. Chateau/piranesi (`$reflectentities 1`) include them.

## Quirks

- `$refracttint` ignored unless `$blurrefract 1`: keep (no stock material sets either).
- Double fog of the refraction (height fog in the texture, then lerp by a − 0.05 in the shader): keep. It is the look.
- 0.33 weights in three-layer normals (slightly darker, tilted normals): keep.
- The cheap pass works in gamma space with a raw envmap and raw tints: keep, for matching LDR screenshots.
- `$forceexpensive` silently defaults to 1 on PC: keep the resulting behaviour (planar reflection whenever `$reflecttexture` is set).
- `$reflectamount` defaults to 0.8 but only matters with a reflection texture.
- The cheap pass ignores `$bumptransform`, so its normals do not scroll with the proxy (they still animate by frame).

## Test cases

Linear values; "encode" = the final sRGB framebuffer write.

| Setup | Input | Expected |
|---|---|---|
| Fresnel, flat N = (0,0,1) | view straight down (V·N = 1) | F = 0 → pass 1 colour = fogged refraction only |
| Fresnel | V·N = 0.5 (60° from vertical) | F = 0.5⁵ = 0.03125 |
| Fresnel | V·N = cos 85° = 0.0871557 | F = 0.9128443⁵ = 0.63384 |
| Shore fade | a = 0.08 | F multiplier = clamp(0.03·20) = 0.6 |
| Shore fade | a = 0.04 | 0 (no reflection) |
| Refraction fog, aztec | P = (0.2, 0.2, 0.2), a = 0.55 | weight 0.5 → (0.10770, 0.10316, 0.1) |
| Distortion, aztec | a₀ = 0.5, `$refractamount` 0.2, N.xy = (0.1, −0.05), Nₐ = 1, base uv (0.4, 0.6) | refract_uv = (0.41, 0.595) |
| Distortion, pretty1 reflection | a₀ = 1, `$reflectamount` 0.3, N.xy = (0.1, −0.05) | reflect uv offset (0.03, −0.015) |
| Screen mapping | pixel at NDC (0.5, 0.5) | reflect uv (0.75, 0.75); refract uv (0.75, 0.25) |
| Reflection camera | eye (0, 0, 100), pitch 30° down, roll 5°, W = 0 | eye' (0, 0, −100), pitch 30° up, roll −5° |
| Reflection camera, aztec | eye z = −300, W = −455 | eye' z = −610; clip draws z > −457 |
| Refraction clip | W = −455 | refraction view draws z < −453, height fog plane −453 |
| Cheap ramp, aztec | d = 1250, F = 0.03125, a₀ ≥ 0.1 | ramp = (1250 − 500)/1500 = 0.5 → α = 0.53125 |
| Cheap ramp, aztec | d = 400 | ramp 0 → α = F |
| Cheap ramp, aztec | d = 2500 | α = 1 (cubemap only) |
| Cheap, militia | d = 736, F = 0 | ramp = (736 − 512)/448 = 0.5 |
| Mode, aztec | camera 1500 units from the water, `r_waterforceexpensive 1` | refraction view rendered, no reflection view; passes 1 (refract only) + 2 |
| Mode, aztec | camera 2100 units away | cheap: no sub-views, pass 2 opaque-equivalent (α = 1) |
| Mode, chateau | any distance | reflection (+ entities) and refraction views; pass 1 only |
| Mode, port | dxlevel 95 | reflection without entities; with `r_waterforcereflectentities 1`: with entities |
| Height fog | W' = 0, eye z = 100, point z = −50 directly below (z_clip = 150) | h = 50/150; f_h = clamp(50·K); with K = 1/45 (aztec, Open question 4) → 1 |
| Height fog | eye z = 100, point z = −5, z_clip = 105 | h = 5/105; f_h = 5K; K = 1/45 → 0.1111 |
| Under-water surface fog, dev_waterbeneath2 | z = 200 | g = (200 − 1)/399 = 0.49875 |
| Under-water surface fog, coast01_beneath | z = 0 | g = 256/768 = 0.33333 |
| Three-layer coords, port | u = v = 1, T = 10 | uv₁ = (0.3, 0.1); uv₂ = (0.2, 0.7) |
| Three-layer flat bias | three texels (128,128,255,255) | N = (−0.00612, −0.00612, 0.98), Nₐ = 0.99 |
| Normal frame | 16 fps, 29 frames, t = 2.0 s | frame 3 |
| Normal frame | t = 1.8125 s | frame 0 (29 mod 29) |
| TextureScroll, aztec | rate 0.01, 45°, t = 100 s | translation (0.70711, 0.70711) |
| TextureScroll, port | rate 0.02, 25°, t = 100 s | (frac 1.81262, frac 0.84524) = (0.81262, 0.84524) |
| Fog colour, aztec | `$fogcolor` [.15 .1 0] | linear (0.015400, 0.0063096, 0); encodes back to ≈ (38, 26, 0) under a pure 2.2 curve (GPU sRGB curve: ≈ (33, 19, 0), see [shaders.md](shaders.md) Edge cases) |
| Pass 1 combine | P' = (0.1, 0.1, 0.1), R = (0.5, 0.6, 0.7), F = 0.25 | C = (0.2, 0.225, 0.25) |

## Open questions

1. **Does `$forceexpensive` really default to 1?** The shader's parameter set-up does so on PC, which makes `r_waterforceexpensive` irrelevant for Water materials. Check: on de_chateau run `r_waterforceexpensive 0` and look for the planar reflection of the castle walls in the moat (a cubemap reflection does not move with the camera).
2. **Unset `$reflecttexture`:** the shader declares a default name. I assume undeclared parameters with defaults are not "textures" for the client's test, so aztec/inferno/militia get no planar reflection. Check: on de_aztec with `r_waterforceexpensive 1`, look at a player standing next to the canal: there should be no player reflection, only the static cubemap.
3. **Render-target size in CS:S:** the generic client default is 1024 × 1024, shrunk by mat_picmip; CS:S's own client is not public. Check: `mat_showwatertextures 1` (shows both targets at `mat_wateroverlaysize`), or a RenderDoc capture of the reference.
4. **Height-fog range K:** engine-set. Expected 1/(`$fogend` − `$fogstart`) (the vertex formula has no start term), but it could be 1/`$fogend`. Check: in de_aztec, look straight down into the canal from a known height at a flat floor of known depth, and compare the pixel to lerp(floor colour, fog colour, f) under both choices.
5. **Cheap pass colour space:** no sRGB write and no sRGB read on the envmap are enabled in LDR. I assume the hardware then blends on gamma values. Check: compare an aztec far-water pixel to the raw cubemap texel colour × 1.
6. **Shader time base** for `$scroll1`/`$scroll2` (shader API current time) vs the proxies' game time: does de_port's water layer scroll stop when the game is paused?
7. **Which material is the "fog volume" material under water:** the top material (aztecwater: fog 5–50, colour [.15 .1 0]) or the `$bottommaterial` (fog 1–400, {22 20 10})? It decides the under-water fog and whether de_port shows the `water_warp01` overlay. Check: dive in de_aztec and measure how far a wall stays visible (≈ 50 vs ≈ 400 units), and sample the colour of the clear.
8. **Patched materials' empty `"Proxies" {}`:** does the engine's patch "replace" drop the base material's proxies (no frame animation, no scroll, no WaterLOD on in-map water) or merge recursively (keep them)? Check: on de_aztec, does the canal normal pattern animate? (Expected: yes, merge.) Our vmt loader must do the same.
9. **`srgb?$fogcolor` on de_port:** which form CS:S uses. Our model-material work found the `srgb?` forms win in CS:S (src/games/cs_source/material.rs); assume the same here. Check: de_port's under-water clear colour should be ≈ {21 48 52}, not {7 16 18}.
10. **Pass 1 when the refraction view was not rendered this frame** (cheap mode): is the refraction target rebound to a black/white texture, or left stale? Check: walk away from de_aztec's canal past 2000 units and look for the shore fade flickering.
11. **Tangent frame of water brushes:** are the vertex tangent and bitangent normalised (engine)? This affects V and so Fresnel only if the texture axes are scaled. Stock water uses scale 0.25–1, so it matters. Check: a Fresnel gradient on de_chateau compared with the prediction at a known angle.
12. **Which water volume gets the sub-views** when several are visible (de_aztec has surfaces at different heights): the engine picks one. Check: stand at the upper canal (−149) looking at the lower one (−455) and see if its reflection/refraction is misaligned.
