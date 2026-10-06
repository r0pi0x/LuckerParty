# Counter-Strike: Source: WorldTwoTextureBlend (DX9, LDR)

Source basis: Valve's public Source SDK 2013 (current GitHub head), DirectX 9 standard shader library only. I read the C++ definition of the two-texture world shader (parameters, defaults, texture set-up, sampler colour-space state, the static and dynamic shader options it picks), its shader model 2.0/2.0b pixel shader, the lightmapped world vertex shader it shares with LightmappedGeneric, the shared shader include files (bump basis, lightmap coordinates, fog, final output), and the base-shader helper that builds the scaled detail texture transform. For comparison I re-read the LightmappedGeneric pixel shader's bump weighting and detail combine. The engine and the D3D9 shader API are not in the SDK; values they set are taken from [shaders.md](shaders.md) (which has RenderDoc measurements) or listed under Open questions. Material data: the 47 WorldTwoTextureBlend materials in the user's CS:S install under `materials/de_aztec/`, and three of their textures, decoded in a scratch folder (not committed).
Status: draft

## Summary

- WorldTwoTextureBlend is a lightmapped world shader with **two colour textures**: `$basetexture` and `$detail`. It has no envmap, no `$basetexture2`, no detail blend modes and no `$color` tint.
- It has two blend behaviours, picked by `$detail_alpha_mask_base_texture`:
  - **0 (default): "detail over base".** albedo = lerp(base, detail, detail alpha).
  - **1: "base is a 2× grime mask".** albedo = clamp(clamp(2·base)·detail.a + (1 − detail.a)) × detail.rgb. Where detail alpha is 0 you see the detail texture alone. Where it is 1 the detail is multiplied by twice the base.
  - **Every stock de_aztec material uses mode 1.** The base texture is a dark, large-scale grime map (mean byte ≈ 100) and the detail is the stone pattern, tiled 1–16× over it. The detail texture is the wall you see; the base only darkens it where the detail alpha says so.
- Three colour-space details make it darker than LightmappedGeneric, and they explain most of what refcmp shows:
  1. The base texture is sRGB-decoded to linear before the "×2". A grime texel of byte 100 becomes 2 × 0.127 = 0.254, not 2 × 0.39 = 0.78.
  2. The detail texture is **not** sRGB-decoded. Its stored bytes are used directly as linear numbers.
  3. The lightmap is multiplied by a **fixed 2.0** in linear space, not by the 2^2.2 ≈ 4.595 that LightmappedGeneric uses. A wall under the same lightmap is lit 2/4.595 = 0.435× as brightly (in linear light).
- Bumped variants (6 de_aztec materials) weight the three bump lightmap pages by the clamped dot product, **not squared** (LightmappedGeneric squares it), and read the normal map at the **detail** texture coordinates.

## Units and conventions

Same as [shaders.md](shaders.md) ("Units and conventions"): colours 0..1, a byte b means b/255; "gamma" is stored/display encoding, "linear" is after decode; tangent space x = texture u, y = texture v (image-down), z = out of the surface. All maths here is per pixel at mat_hdr_level 0 on shader model 2.0b or later hardware (the CS:S reference). The vertex stage is the lightmapped world vertex shader described in shaders.md section 2.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| wttb_lightmap_scale | 2.0 | linear factor | multiplies the decoded lightmap; fixed in the pixel shader, not engine-set |
| lightmap_linear_scale (LightmappedGeneric, for comparison) | 2^2.2 = 4.594793 | linear factor | from shaders.md; **not** used by this shader |
| wttb_detail_scale_default | 4 | uv multiplier | `$detailscale` when the material does not set it |
| wttb_mask_mode_default | 0 | - | `$detail_alpha_mask_base_texture` |
| wttb_mask_base_gain | 2 | - | the "×2" applied to the decoded base in mask mode |
| selfillumtint_default | (1, 1, 1) | gamma colour | `$selfillumtint` |
| bump_basis_0..2 | as in shaders.md | tangent space | same three directions as LightmappedGeneric |

## Behavior

### 1. Parameters this shader reads

Only these change the result. Anything else in the .vmt (`$detailblendmode`, `$detailblendfactor`, `$detailtint`, `$detailtexturetransform`, `$envmap*`, `$basetexture2`, `$color`, `$alpha2`, `%keywords`, `%tooltexture`, `$surfaceprop`) is ignored by the renderer.

| Parameter | Default | Effect |
|---|---|---|
| `$basetexture` | none (white) | first colour texture, sRGB-decoded. If absent, base = (1,1,1,1) and `$selfillum` is cleared |
| `$basetexturetransform` | identity | uv transform for the base, and (scaled) for the detail |
| `$frame` | 0 | base texture frame |
| `$detail` | none | second colour texture, read **raw** (no sRGB decode). If absent, detail = (1,1,1,1) |
| `$detailscale` | **4** (when not set) | scalar, or a 2-vector [su sv], see section 2 |
| `$detailframe` | 0 | detail texture frame |
| `$detail_alpha_mask_base_texture` | 0 | 0 = detail over base, 1 = 2× grime mask (section 3) |
| `$bumpmap` | none | normal map; used only if mat_bumpmap is on |
| `$bumpframe` | 0 | normal map frame |
| `$bumptransform` | identity | normal map uv transform, only when there is no `$detail` |
| `$nodiffusebumplighting` | 0 | 1 = normal map does not affect lighting (flat lightmap) |
| `$albedo` | none | replaces `$basetexture` when `$bumpmap` is set, `$nodiffusebumplighting` is 0 and mat_bumpmap is on |
| `$selfillum` (flag) | off | self-illumination by base alpha (section 6) |
| `$selfillumtint` | [1 1 1] | self-illum colour, gamma→linear converted (section 6) |
| `$vertexcolor` (flag) | off | multiplies albedo by the raw vertex colour |
| `$alpha` | 1 | output alpha multiplier |
| `$alphatest`, `$translucent`, `$additive`, `$nocull`, `$decal`, `$nofog` (flags) | off | standard; blending is decided by whether the base texture has alpha, as for LightmappedGeneric |
| `$seamless_scale` | 0 | non-zero switches to world-space mapping; broken in this shader (Quirks) |

Unknown flags handled by the shared base class (e.g. `$basealphaenvmapmask`) have no visible effect: there is no envmap. `$basealphaenvmapmask` and `$selfillum` are both cleared when the base texture has no alpha channel.

### 2. Texture coordinates

Let (u, v) be the vertex base texture coordinate (from the face's texinfo; engine data, as for LightmappedGeneric). Let M be `$basetexturetransform` as a 2×4 matrix (rows r0, r1; only columns 0, 1 and 3 matter).

- uv_base = (r0[0]·u + r0[1]·v + r0[3], r1[0]·u + r1[1]·v + r1[3]).
- **uv_detail** uses the *base* transform scaled by `$detailscale` = (su, sv) (a scalar s means su = sv = s; unset means 4):
  - u_d = su·r0[0]·u + sv·r0[1]·v + su·r0[3]
  - v_d = su·r1[0]·u + sv·r1[1]·v + sv·r1[3]
  - With an identity transform this is simply (su·u, sv·v). `$detailtexturetransform` is not used.
- **uv_bump**:
  - with `$detail`: uv_bump = uv_detail (the normal map tiles with the detail, not with the base);
  - without `$detail`: uv_bump = `$bumptransform` applied to (u, v) the same way as uv_base.
- uv_lightmap: as for LightmappedGeneric (page 0, plus k·Δ for bump pages k = 1..3).

### 3. Albedo

Samples (all bilinear/trilinear with the texture's own filtering):

- B = sample(`$basetexture`, uv_base), rgb **sRGB-decoded** to linear by the sampler; B.a raw.
- D = sample(`$detail`, uv_detail), rgba **raw**: the stored 0..1 values, no decode.

Then, per rgb channel:

- **Mode 0** (`$detail_alpha_mask_base_texture` 0 or unset):
  - A.rgb = B.rgb + (D.rgb − B.rgb)·D.a, i.e. lerp(B, D, D.a). Linear base and raw detail are mixed directly.
- **Mode 1** (`$detail_alpha_mask_base_texture` 1):
  - k = clamp(2·B.rgb, 0, 1)
  - m = clamp(k·D.a + (1 − D.a), 0, 1)
  - A.rgb = m · D.rgb
  - D.a = 0 gives A = D.rgb (detail alone). D.a = 1 gives A = min(2B, 1)·D.rgb. Any base texel with linear value ≥ 0.5 (byte ≥ 188 under the sRGB curve) leaves the detail unchanged.
- In both modes **A.a = B.a**: the detail never changes alpha.
- Then, only if `$vertexcolor` is set: A.rgb ×= vertex.rgb (raw, not linearised). `$color` is never applied (see Quirks).

### 4. Lighting

- **Unbumped** (no `$bumpmap`, or `$nodiffusebumplighting` 1, or mat_bumpmap 0):
  - lm = sRGB-decode(bilinear sample of lightmap page 0 at uv_lightmap).
  - One tap only. The bicubic lightmap filter (r_lightmap_bicubic, which CS:S turns on for LightmappedGeneric) does **not** apply here.
- **Bumped** (`$bumpmap`, `$nodiffusebumplighting` 0, mat_bumpmap on):
  - n = 2·sample(`$bumpmap`, uv_bump).rgb − 1 (raw texel, not renormalised, no channel flip). mat_fastnobump 1 substitutes a flat normal texture.
  - lm_k = sRGB-decode(sample of bump page k), k = 1, 2, 3 (page 0 is not read).
  - w_k = clamp(n · basis_k, 0, 1). **Not squared.**
  - lm = (w_1·lm_1 + w_2·lm_2 + w_3·lm_3) / (w_1 + w_2 + w_3).
- light = 2.0 · lm (wttb_lightmap_scale). There is no `$color` multiply and no engine lightmap scale.
- The lightmap texel encoding and the bump page encoding are the engine's, the same as for LightmappedGeneric (shaders.md section 2).

### 5. Combine and output

- diffuse = A.rgb · light.
- Self-illum, if enabled (section 6), replaces part of diffuse.
- result = diffuse. There is no specular or envmap term.
- result ×= linear light scale (1.0 in LDR, as in shaders.md).
- Fog: the same pixel fog as LightmappedGeneric (shaders.md section 7: range fog with f², or water height fog). A flashlight pass, if any, fogs toward black.
- sRGB framebuffer encode, 8-bit, per-channel clamp.
- Output alpha α = (B.a unless `$selfillum`, else 1) × vertex.a, where vertex.a is `$alpha` (times the vertex alpha when `$vertexcolor` is set). `$alpha2` is not applied. When the material is opaque and water height fog is active, the fog factor is written to alpha instead (destination-alpha bookkeeping only; not visible).

### 6. Self-illumination

- Kept only if the base texture has an alpha channel; otherwise the flag is cleared at load. It also disables `$alphatest`.
- T = gamma→linear(`$selfillumtint`) using the material-parameter table from shaders.md (Quirks: rounded to 1/255, ≥ 0.95 → 1). This differs from LightmappedGeneric, which uses the tint raw.
- diffuse = lerp(A.rgb·light, T·A.rgb, B.a).
- None of the de_aztec materials set it.

### 7. `$detailblendmode` in LightmappedGeneric

Re-checked against the same source: the mode table, the sRGB rule (detail decoded only for mode 1) and the defaults in [shaders.md](shaders.md) section 2 are correct. Nothing to change. `$detailblendmode` has no meaning in WorldTwoTextureBlend; do not map WorldTwoTextureBlend onto LightmappedGeneric mode 2 (it differs in detail colour space, lightmap scale, and, for de_aztec, in the blend itself).

## Per-pixel order

1. uv_base, uv_detail (base transform × `$detailscale`), uv_bump, lightmap uv (vertex stage).
2. Sample the lightmap: page 0 (unbumped) or pages 1–3 (bumped); sRGB-decode.
3. Sample `$detail`, raw.
4. Sample `$basetexture`, sRGB-decoded.
5. Albedo: mode 0 lerp or mode 1 grime mask (section 3). Alpha stays the base's.
6. Sample and decode the normal (bumped only).
7. A.rgb ×= vertex rgb (only with `$vertexcolor`); α = B.a (unless self-illum) × vertex alpha.
8. light = unclamped-weight mix of pages (bumped) or page 0, × 2.0.
9. diffuse = A.rgb × light; self-illum lerp by B.a.
10. × linear light scale (1), fog, sRGB encode, 8-bit clamp; alpha test/blend on α.

## Edge cases

- **No `$detail`:** D = (1,1,1,1). Mode 0 gives A = white (the base is fully replaced by white); mode 1 gives A = min(2B, 1). Neither looks like LightmappedGeneric. No stock CS:S material does this.
- **`$detail` together with `$bumpmap` and mode 0:** the shader has no variant for this combination; what the engine draws is unknown (Open questions). de_aztec's bumped materials all use mode 1, which is supported.
- **`$bumpmap` with mode 1 and no `$detail`:** uv_bump comes from a transform register that this shader never sets in that case (it holds whatever an earlier draw left). Treat as undefined; not used by stock content.
- **Base byte ≥ 188 in mode 1:** k saturates to 1 and the base has no effect at all.
- **Bump weight sum 0** (normal pointing away from all three basis vectors): division by zero, as in LightmappedGeneric. Guard it.
- **Unnormalised normal map texels:** weights are not renormalised; the division by the sum absorbs the length.
- **Lightmap texel 255:** light = 2.0 exactly (LightmappedGeneric would give 4.595), so a mode-1 wall can never reach more than twice its albedo.
- **Lower DirectX levels** (mat_dxlevel 80/81 and 70) use different fallbacks with gamma-space maths. The reference runs at DX level 90+, so they are out of scope.

## Quirks

Keep all of these; they are the look of de_aztec.

- **Lightmap ×2, not ×2^2.2.** Walls are about 0.435× (linear) or about 0.69× (gamma) as bright as a LightmappedGeneric surface with the same albedo and lightmap.
- **Detail is used raw.** A detail byte of 190 acts as linear 0.745, which displays as about 224 at light 1.0: the stone pattern is lighter and lower in contrast than its texture. Combined with the ×2 lighting an ungrimed stone texel (detail alpha 0) of byte 190 at lightmap texel 128 shows as byte ≈ 154 (see Test cases).
- **The ×2 grime runs in linear space.** The grime maps were evidently authored for a gamma-space ×2 (a neutral byte 128 means "no change" there). In this shader byte 128 becomes 2 × 0.216 = 0.43, so even "neutral" grime darkens the stone by more than half in linear light. The stock materials' pre-composited LightmappedGeneric versions (the `_s1a`-style materials pointing at `stn01grm01_s1` etc.) show the authored gamma-space look; do not use them as the reference for the DX9 look.
- **`$color` is ignored.** The vertex stage only passes `$alpha` through; the rgb modulation never reaches this shader.
- **`$selfillumtint` is gamma-converted** here but raw in LightmappedGeneric.
- **No bicubic lightmap filtering** on these surfaces even when the game turns it on for LightmappedGeneric, so their lighting looks slightly blockier.
- **`$seamless_scale` is broken** in this shader: the vertex stage switches to world-space coordinates but the pixel stage still reads a 2D base coordinate (it gets world x, y × scale, i.e. a top-down planar projection) and the detail coordinate is left at 0. Not used by stock content; reproduce only if a map needs it.
- **Mode 1's "×2 then multiply" can brighten** only up to the detail itself (m ≤ 1). Mode 0 can lighten or darken.

## de_aztec materials

All 47 WorldTwoTextureBlend materials in `materials/de_aztec/` (read from the install, 2026-10-06) use only these keys:

| Key | Values seen | Count |
|---|---|---|
| `$basetexture` | de_aztec/stn01grm01 (24), stn01grm02 (8), stn01grm03 (6), stn01grm04 (5), stn01grm05 (2), stn01grm06 (2) | 47 |
| `$detail` | stonework01 (16), watline01 (5), stonework01g (4), carving01/03b (3 each), carving03/carving04/stair01sd/stair01top/stonework01mossc (2 each), others 1 each | 47 |
| `$detailscale` | 1 (2), 2 (15), 4 (19), 8 (8), 16 (3) | 47 |
| `$detail_alpha_mask_base_texture` | 1 | 47 |
| `$bumpmap` | de_aztec/stonework01_normal (5: the watline01grm* materials), stonework01g_normal (1: stn01g1grm01_s1); commented out in all others | 6 |
| `$surfaceprop` | concrete, rock | 47 |

- The material name's `_sN` suffix tracks `$detailscale` (_s1 → 4, _s2 → 8, _s4 → 16, _s25 → 1, _s50 → 2).
- Textures (512×512, decoded from the install): grime bases `stn01grm01`/`stn01grm02` are DXT1 without alpha, mean bytes ≈ (100, 92, 76) and (98, 88, 72). Detail `stonework01` is DXT5, mean rgb ≈ (191, 183, 161), mean alpha ≈ 217/255; 41% of its texels have alpha > 250, 8% below 128. `carving03` has mean alpha ≈ 116 (more of the base shows).
- So on a typical de_aztec wall the stone (detail) dominates, and the grime darkens it heavily wherever the detail alpha is high, which is most of the stone face.

### Worked example: `de_aztec/stn01grm01_s1`

`$basetexture` de_aztec/stn01grm01, `$detail` de_aztec/stonework01, `$detailscale` 4, `$detail_alpha_mask_base_texture` 1, unbumped.

- uv_detail = 4 · uv_base (identity transform).
- One pixel with base texel bytes (100, 91, 76), detail texel bytes (190, 183, 161, alpha 255), flat lightmap texel 128 grey. Using the GPU sRGB curve:
  - B = sRGB-decode → (0.12744, 0.10461, 0.07227); k = 2B = (0.25488, 0.20923, 0.14453).
  - D.a = 1, so m = k; D.rgb raw = (0.74510, 0.71765, 0.63137).
  - A = m·D = (0.18991, 0.15016, 0.09126).
  - lm = sRGB-decode(128/255) = 0.21586; light = 0.43172.
  - result = A·light = (0.08199, 0.06483, 0.03940) → sRGB encode → bytes (80.9, 72.0, 55.9) → **(81, 72, 56)**.
  - LightmappedGeneric with the detail texel as its decoded base (190, 183, 161) and the same lightmap would show (189, 182, 160). A LightmappedGeneric-style reading of the material lands near that: about 2.3× brighter in display bytes than the real shader.
- Averaged over the whole 512² base (detail tiled 4×, same flat lightmap texel 128): mean bytes ≈ **(95, 88, 74)** for WorldTwoTextureBlend versus ≈ (179, 171, 150) for "lerp(base, decoded detail, detail alpha) × 2^2.2 lightmap". That ≈ 0.53× gamma ratio matches the darker walls in refcmp's de_aztec zone0_270 view.

## Test cases

Expected bytes are given for the GPU sRGB curve ("sRGB") and for pure 2.2 ("2.2"), ±1. Unless stated: unbumped, no `$vertexcolor`, no fog, linear light scale 1.

| Setup | Input | After | Expected |
|---|---|---|---|
| Lightmap scale | lightmap texel 128 | light | 0.43172 (sRGB); 0.43904 (2.2) |
| Lightmap scale | lightmap texel 255 | light | 2.0 |
| Mode 1, opaque detail | base (100,91,76), detail (190,183,161,a 255), texel 128 | A (linear) | (0.18991, 0.15016, 0.09126) sRGB; (0.19004, 0.14875, 0.08805) 2.2 |
| same | | output byte | (81, 72, 56) sRGB; (82, 74, 58) 2.2 |
| Mode 1, transparent detail | same, detail alpha 0 | A / output | A = (0.74510, 0.71765, 0.63137) (raw detail); bytes (154, 151, 142) sRGB; (153, 151, 142) 2.2 |
| Mode 1, half alpha | same, detail alpha 128 | A / output | A = (0.46641, 0.43279, 0.36026); bytes (124, 120, 110) sRGB |
| Mode 1, bright base | base (200,200,200), detail (190,183,161,255), texel 128 | A | (0.74510, 0.71765, 0.63137): 2B saturates, base has no effect |
| Mode 1, full lightmap | case "Mode 1, opaque detail" with texel 255 | output byte | (166, 149, 118) sRGB; (164, 147, 116) 2.2 |
| Mode 0 | base (100,91,76), detail (190,183,161,a 128), texel 128 | A / output | A = (0.43748, 0.41233, 0.35292); bytes (120, 117, 109) sRGB |
| Alpha | base alpha 0.5, mode 0 or 1, detail alpha 0 | α | 0.5 (detail alpha never reaches α) |
| No `$detailscale` | identity transform, uv (0.1, 0.2) | uv_detail | (0.4, 0.8) |
| Vector scale with translation | `$detailscale` [2 3], `$basetexturetransform` with linear part identity and translation column (0.5, 0.25), uv (1, 1) | uv_detail | (2·1 + 2·0.5, 3·1 + 3·0.25) = (3.0, 3.75) |
| Bump weights, flat normal | n = (0,0,1); pages (0.9, 0.3, 0.0) | w, mix | w = (0.57735 ×3), mix = 0.4 |
| Bump, normal = +x | n = (1,0,0); same pages | w, mix | w = (0.81650, 0, 0), mix = 0.9 |
| Bump, tilted | n = (0.5, 0.5, 0.707107); same pages | w, mix | w = (0.81650, 0.55768, 0), Σ = 1.37417, mix = 0.65650 (LightmappedGeneric's squared weights give 0.70914) |
| Bump, real texel | n texel (200, 90, 220); pages (1.0, 0.5, 0.25) | w, mix | w = (0.88314, 0, 0.39469), mix = 0.76834 (LightmappedGeneric: 0.87514) |
| Bumped light | flat normal, pages decoded (0.2, 0.2, 0.2) | light | 0.4 |
| `$color` | `$color` [0.5 0.5 0.5] on mode 1 example | output | unchanged (81, 72, 56) |
| `$selfillumtint` | tint 0.5, B.a = 1, A = 0.4 | diffuse | 0.4 × 0.21953 = 0.08781 (tint table-converted) |

## Open questions

1. **The ×2 lightmap factor in CS:S's shipped shader.** It is a literal in the public SDK's pixel shader. CS:S's shipping shader DLL may come from a different revision. Check: RenderDoc capture of a de_aztec wall draw at mat_hdr_level 0 (as was done for de_dust2 in shaders.md) and look for a pixel-shader constant of 4.5948 on that draw; its absence, plus the light ≈ 2·lm in the shader disassembly, confirms 2.0. Also confirm the detail sampler is a UNORM (non-sRGB) view and the base and lightmap are sRGB views.
2. **Default sampler sRGB state.** The detail sampler is "raw" because the shader never turns sRGB decode on for it; that relies on the shader API defaulting every sampler to non-sRGB at the start of each material's state. The capture in question 1 settles it.
3. **Bump pages on de_aztec.** Whether the compiler baked bumped lightmaps (4 pages) for faces using the 6 materials with `$bumpmap`. If a face has only 1 page, the bumped variant would read the wrong atlas texels. Check the BSP face's lightmap page count for a watline01grm* face.
4. **Mode 0 with both `$detail` and `$bumpmap`** has no shader variant in this revision; what the engine does (falls back, draws nothing, or draws with a stale shader) is unknown. Not used by stock CS:S content.
5. **refcmp check, not run here** (shared-machine rule: the reference game was not touched). After implementing, compare de_aztec zone0_270 and t_spawn: expected mean wall luma roughly halves versus the current stand-in. A single-pixel check: aim at a stonework01 wall with a known lightmap value and compare against the worked example.
6. **Whether `$detailscale` as a 2-vector is accepted** from a .vmt for this shader (the parameter is declared as a float). The transform maths handles a vector, but the material parser may only read the first number. No de_aztec material uses a vector.
