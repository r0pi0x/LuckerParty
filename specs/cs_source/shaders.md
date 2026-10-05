# Counter-Strike: Source: world, prop, decal and sky shading (DX9, LDR)

Source basis: Valve's public Source SDK 2013 (current GitHub head). I read the DirectX 9 standard shader library: the C++ shader definitions and their parameter set-up for the lightmapped world shader, the displacement-blend shader (which reuses it), the vertex-lit/unlit model shader, the two decal shaders, the sky shaders and the Phong "skin" variant; their HLSL vertex and pixel shaders and the shared shader include files (colour-space helpers, bump basis, fog, final output, vertex lighting, ambient cube). For the lightmap encoding I also read the shared math library (gamma tables, compressed-exponent colour decode, bump basis construction), the public displacement builder (displacement tangent frames) and the radiosity compiler's lightmap and bump-basis code, whose own comment says its lightmap conversion "matches the engine's". The engine itself and the D3D9 shader API layer are **not** in the SDK. Values those layers set (the lightmap scale constant, the tone-map scale, the environment-map scale, the default alpha-test reference, the brush tangent stream, blend states) are therefore marked as *derived* or listed under Open questions. CS:S's shipping binaries may come from a slightly different revision of the same library.
Status: draft

## Summary

- At mat_hdr_level 0 every DX9 shader here works in **linear light**:
  - Colour textures (base, base2, envmap, and the lightmap itself) are decoded from sRGB/gamma by the texture sampler.
  - Lighting is multiplied in linear space.
  - The framebuffer write re-encodes to gamma. Output is then clamped per channel to [0, 1].
- World brushes multiply albedo by a lightmap that is stored **gamma-encoded at half intensity** ("2× overbright in gamma space"). After the sampler's linear decode, the shader multiplies it by a constant that is 2^2.2 ≈ 4.595 in linear terms. Under a pure-2.2 model this is exactly "output_gamma = base_gamma × 2 × lightmap_texel" before clamping.
- Bump-mapped brushes use **radiosity normal mapping**: three extra lightmap pages, one per fixed tangent-space basis direction.
  - Each page is weighted by the squared, clamped dot product of the decoded normal with its basis direction.
  - The weighted sum is divided by the sum of the weights.
  - Normal maps are DirectX-style: +X follows texture u (S axis), +Y follows texture v (T axis, i.e. image-down), and green is not flipped.
- Detail textures default to mode 0, "mod2x": albedo × lerp(1, 2·detail, $detailblendfactor), with $detailscale 4 and $detailblendfactor 1.
  - In the world shader the mod2x detail texel is used **raw**, without sRGB decode.
- Static props (vertex-lit) get per-vertex light: an ambient cube weighted by the squared normal components, plus up to 4 local lights with 1/(c + l·d + q·d²) attenuation. Both are in linear space with no overbright factor.

## Units and conventions

- **World space.** Source units (1 unit = 1 inch), Z up, right-handed. Eye and vertex positions are in world units.
- **Colours.** Colours are written as 0..1 floats. An 8-bit value b means b/255.
  - "Gamma" means the stored or display encoding.
  - "Linear" means after decode.
  - The shaders' own conversions use a pure power 2.2: linear = gamma^2.2. The CPU-side conversion of material colour parameters uses the same 2.2 power through a 256-entry table (see Quirks).
  - Hardware sRGB texture decode and framebuffer encode use the GPU's sRGB curve, which is close to but not identical to 2.2 (see Edge cases).
- **Tangent space.** x = tangent (texture S/u direction), y = binormal (texture T/v direction), z = surface normal pointing out of the face.
  - Texture v grows downward in the image (row index), so tangent-space +Y points "down" in image space.
- **Lightmap luxel storage in the BSP.** Each luxel is 4 bytes (r, g, b as unsigned bytes and e as a signed byte). linear = c · 2^e / 255 per channel.
  - At mat_hdr_level 0 the LDR lighting lump is used. HDR has its own lump.
  - A bumped face stores 4 pages per light style, in this order: flat normal, then basis 0, basis 1, basis 2.
- Everything here is per pixel unless marked per vertex. There is no time dependence except animated texture frames and scrolling, which are not covered.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| gamma | 2.2 | - | exponent for every shader-side and material-parameter gamma↔linear conversion |
| overbright | 2.0 | gamma-space factor | lightmaps and static vertex lighting are stored at 1/overbright of their gamma value |
| lightmap_linear_scale | 2^2.2 = 4.594793 | linear factor | multiplier applied after linear decode of a lightmap texel (*derived*, see Lightmap decode) |
| lightmap_encode_range | 0 .. 4095/1024 (≈ 3.999) | linear | linear luxel values are clamped to this range before encoding |
| lightmap_encode_steps | 1/1024 | linear | luxel value is rounded to this grid before the gamma curve |
| max_lightmap_texel | 239 | byte | texel produced by any luxel ≥ 3.999 linear |
| bump_basis_0 | (√(2/3), 0, 1/√3) = (0.816497, 0, 0.577350) | tangent space | basis direction for bump page 1 |
| bump_basis_1 | (-1/√6, 1/√2, 1/√3) = (-0.408248, 0.707107, 0.577350) | tangent space | basis direction for bump page 2 |
| bump_basis_2 | (-1/√6, -1/√2, 1/√3) = (-0.408248, -0.707107, 0.577350) | tangent space | basis direction for bump page 3 |
| detail_scale_default | 4 | uv multiplier | $detailscale |
| detail_blendfactor_default | 1 | - | $detailblendfactor |
| detail_blendmode_default | 0 (mod2x) | - | $detailblendmode |
| detail_tint_default | (1, 1, 1) | - | $detailtint |
| envmaptint_default | (1, 1, 1) | - | $envmaptint |
| envmapcontrast_default | 0 | - | $envmapcontrast |
| envmapsaturation_default | 1 | - | $envmapsaturation |
| fresnelreflection_default | 1 | - | $fresnelreflection (1 means no fresnel) |
| fresnel_power | 5 | - | exponent of (1 - N·V) |
| luma_weights | (0.299, 0.587, 0.114) | - | greyscale used by $envmapsaturation |
| selfillumtint_default | (1, 1, 1) | - | $selfillumtint |
| alphatestreference_default | 0 (meaning "use the engine default") | - | $alphatestreference |
| alpha2_default | 1 | - | $alpha2 (world shader only; values ≤ 0 mean 1) |
| decal_mod2x | 2 | - | DecalModulate result = 2 · src · dst |
| decal_fog_exponent | 0.4 | - | DecalModulate fog factor is raised to this power |
| decal_fog_colour | 0.5 grey | gamma | DecalModulate fogs toward middle grey |
| max_local_lights | 4 | lights | vertex-lit shader |
| halflambert_bias | 0.5 · N·L + 0.5, then squared | - | $halflambert |
| sky_rgbs_scale | 8 | linear factor | HDR compressed (RGB × alpha) sky |
| sky_int16_scale | 16 | linear factor | HDR sky stored as 16-bit integer RGBA |
| hdr_vertex_overbright | 16 | - | maximum overbright of the old HDR alpha-scaled formats (reference only) |
| fog_curve_pixel | f² | - | pixel-shader range fog blends with the square of the fog factor |

## Behavior

### 1. Colour spaces and output (all DX9 shaders here)

- **Inputs read through sRGB decode (LDR):**
  - $basetexture and $basetexture2;
  - the lightmap;
  - $envmap;
  - Sky's base texture, unless it is a 16-bit-per-channel format;
  - in the world shader, $detail only when $detailblendmode = 1;
  - in the model shaders, $detail for every mode except 0.
- **Inputs read raw (treated as linear numbers):**
  - $bumpmap and $bumpmap2;
  - $envmapmask;
  - $blendmodulatetexture;
  - $lightwarptexture;
  - mod2x detail.
- **Output:** the colour after fog is written with an sRGB framebuffer encode, then quantized to 8 bits. There is no tone mapping in LDR. A final multiply by the "linear light scale" happens before fog. That value is the tone-map scale, expected to be 1.0 in LDR (Open questions).
- **Clamping:** no shader clamps colour except where stated below. The 8-bit framebuffer write saturates each channel independently, so there is no hue-preserving clamp in LDR.
- **mat_hdr_level ≥ 2 (HDR) differences:**
  - lightmaps and envmaps are not sRGB-decoded (they are stored linear);
  - the lightmap multiplier is not 2^2.2 (engine-defined);
  - the linear light scale becomes the auto-exposure tone-map scale.

### 2. LightmappedGeneric (world brushes)

**Base texture.**
- base = sRGB-decode(sample($basetexture, uv_base)).
- uv_base = $basetexturetransform applied to the vertex texture coordinates. The vertex texture coordinates come from the face's texinfo S/T vectors; that is engine/BSP data.
- If no $basetexture is set, base = (1,1,1,1). If $envmap is set as well, base = (0,0,0,1) instead (envmap-only surfaces have black albedo).

**Lightmap decode (LDR).**
- Encoding, done by the engine when uploading. The SDK's compiler states it mirrors the engine. Per channel:
  - L = luxel linear value = c · 2^e / 255;
  - i = round(L · 1024), clamped to [0, 4095];
  - t = 0.5 · (i/1024)^(1/2.2);
  - texel = round(255 · t).
  - The resulting colour is then scaled down if its largest channel exceeds 1, and negatives go to 0. Since t ≤ 0.9389 this never triggers.
  - Alpha = 255.
- Decode in the shader:
  - lm_linear = sRGB-decode(bilinear sample of the texel);
  - lighting = lm_linear × lightmap_linear_scale × modulation.rgb.
- lightmap_linear_scale is 2^2.2 ≈ 4.5948 (*derived*): it is the only value that makes the texel round-trip to L under the 2.2 model. The vertex-lighting path in the same library does exactly this round-trip in-shader, as (2 · t)^2.2.
- modulation.rgb is $color, raw. It is **not** gamma-converted in this shader. The default is (1,1,1).
- Where the scale applies: it multiplies the lighting term only, after the bump combination (if any) and before multiplying by albedo. It never touches the base texture, the envmap or self-illumination.
- Bilinear filtering happens on the gamma-encoded texels before decode, as the hardware does. The convar r_lightmap_bicubic (default 0) switches to a 4-tap bicubic B-spline filter; ignore it at the default.

**Bumped lightmaps.** Used when $bumpmap is set, $nodiffusebumplighting is 0, and the convars mat_bumpmap ≠ 0 and mat_reducefillrate = 0.
- Normal decode: n = 2 · rgb − 1, on the raw texel (no sRGB). n.a = texel alpha.
  - There is no renormalisation and no channel flip.
  - x follows +u, y follows +v (image-down), z is out of the surface. This is the "DirectX" green convention.
- Three lightmap samples are taken at uv_lm + k · Δ for k = 1, 2, 3.
  - Δ is a per-vertex offset, equal to one page width in the atlas.
  - Page 0 (k = 0) is the flat-normal lightmap. The bump path does not read it.
- Weights: w_k = clamp(n · basis_k, 0, 1)². The clamp is applied first, then the square.
- lighting = (Σ w_k · lm_k) / (Σ w_k) × lightmap_linear_scale × modulation.rgb, where lm_k are the decoded linear samples.
  - The normalisation by Σw means lighting never darkens just because the normal tilts.
  - For n = (0,0,1) each w_k = 1/3, so lighting = the mean of the three pages.
- **The basis frame used to bake the pages** (radiosity compiler):
  - The frame is built as follows: B = normalize(N_phong × S_axis); T = normalize(B × N_phong); N = N_phong. N_phong is the smoothed vertex normal, and S_axis/T_axis are the texinfo texture axes.
  - If (S_axis × T_axis) · N_face < 0 (mirrored texture), B is negated.
  - Basis k in world space = bx·T + by·B + bz·N.
  - So tangent x is S projected into the face plane, and tangent y is the side of the plane that T points to. A renderer that uses x = dP/du and y = dP/dv (both projected into the plane) matches without flipping anything.
  - For displacements the compiler uses the same construction with the texture axes, after re-projecting them into lightmap-axis space when the two disagree (|dot| < 0.999).
- $ssbump 1 (self-shadowed bump; rare in CS:S content):
  - weights = the raw rgb of the texel (no 2x−1, no square, no division by the sum);
  - lighting = (r·lm_1 + g·lm_2 + b·lm_3) × scale × modulation.
  - For reflection the normal is normalize(Σ rgb_k · basis_k).
- $nodiffusebumplighting 1: the bump map is used only for the envmap normal. Lighting is the single flat page 0.

**Brush tangent frame for envmap reflection.**
- world_normal = n.x · T_vert + n.y · B_vert + n.z · N_vert. It is not renormalised before the reflection or fresnel maths.
- T_vert and B_vert are per-vertex tangent-S and tangent-T vectors supplied by the engine. The public displacement builder sets T = normalized S-axis direction and B = normalized T-axis direction, both orthogonalised against the vertex normal, with S's sign following the S axis. The same is assumed for flat brushes (Open questions).

**$detail.**
- uv_detail = $detailscale × ($basetexturetransform applied to the vertex uv). Default scale 4.
- d = sample($detail).
  - sRGB-decoded only when $detailblendmode = 1.
  - d.rgb is multiplied by $detailtint (raw, not gamma-converted). On shader model 2.0-only hardware the tint is ignored.
- Combine with albedo A (rgba, after the base2 blend), using f = $detailblendfactor:
  - Mode 0 (default): A.rgb ×= lerp(1, 2·d.rgb, f).
  - Mode 1: A.rgb += f · d.rgb.
  - Mode 2: A.rgb = lerp(A.rgb, d.rgb, f · d.a).
  - Mode 3: A = lerp(A, d, f), including alpha.
  - Mode 4: A.rgb = lerp(A.rgb, d.rgb, f · (1 − A.a)), then A.a = d.a.
  - Mode 5 and 6 (post-lighting additive): no effect in the world shader. Its pixel shader never applies the post-lighting step.
  - Mode 7: c = lerp(d.r, d.a, A.a) (scalar); A.rgb ×= lerp(1, 2c, f).
  - Mode 8: A = lerp(A, A·d, f), including alpha.
  - Mode 9: A.a = lerp(A.a, A.a·d.a, f).
  - Modes 10/11 are automatic when the detail texture is flagged as ssbump:
    - mode 10 (surface is bumped): the bump weights w_k are multiplied by 2·d (per component) before combining;
    - mode 11 (not bumped): A.rgb ×= (d.r + d.g + d.b) · 2/3.

**$envmap, its mask and modifiers.**
- If the envmap name is literally "env_cubemap" and the map was not built with cubemaps, the envmap is dropped. The convar mat_specular 0 also drops the envmap (only when a base texture exists).
- The specular mask starts at s = (1,1,1).
  - $normalmapalphaenvmapmask: s ×= normal-map alpha. With $basetexturenoenvmap / $basetexture2noenvmap the corresponding normal map's alpha is forced to 0 first.
  - $envmapmask: s ×= mask.rgb, sampled raw at $envmapmasktransform(uv). It is not allowed together with $bumpmap; a bumped material silently loses its $envmapmask.
  - $basealphaenvmapmask: s ×= (1 − base alpha). This is **inverted**; see Quirks. With $basetexture2, "base alpha" here means the alpha after the base/base2 blend.
- Reflection:
  - V = eye − P (unnormalised);
  - R = 2(N·V)N − (N·N)V, where N is the world normal above;
  - spec = sRGB-decode(cube(R)) × envmap_scale (expected 1 in LDR) × s × $envmaptint. $envmaptint is raw, not gamma-converted.
- Contrast: spec = lerp(spec, spec², $envmapcontrast).
- Saturation: g = dot(spec, luma_weights); spec = lerp(g, spec, $envmapsaturation).
- Fresnel: F = (1 − N·normalize(V))^5 · (1 − R0) + R0, with R0 = $fresnelreflection (default 1, so F = 1). spec ×= F.
  - This is the only fresnel in these shaders. The vertex-lit shader has none.
- The contrast/saturation steps are skipped when the material has $blendmodulatetexture.
- The steps actually honoured depend on a fast-path rule; see Quirks.

**$selfillum.**
- Only kept if the base texture actually has an alpha channel. It is cleared otherwise, and it also disables $alphatest.
- diffuse = lerp(diffuse, $selfillumtint × A.rgb, base.a). base.a is the base texture's own alpha, before the base2 blend. $selfillumtint is raw.

**Vertex colour.**
- A.rgb ×= vertex.rgb when $vertexcolor is set. The vertex colour is raw: it is **not** linearised in this shader. Otherwise vertex.rgb = 1.
- With $basetexture2 or $bumpmap2, the vertex alpha is the blend factor and not an alpha multiplier (see 3).

**Alpha.**
- α = base.a (only if neither $basealphaenvmapmask nor $selfillum is set; otherwise 1) × vertex.a × $alpha × $alpha2.
- vertex.a is the vertex alpha when $vertexcolor is set without base2/bump2; otherwise it is $alpha.
- **Alpha is never taken from $basetexture2.** On blended displacements only the first texture's alpha counts.
- **$alphatest:** the fixed-function alpha test passes when α ≥ ref.
  - ref = $alphatestreference if it is > 0; otherwise the engine default (Open questions).
  - It is disabled when $selfillum or $basealphaenvmapmask is set.
- **$translucent:** alpha blending, src·α + dst·(1−α), assumed from the shared base class which is not in the SDK. Depth writes are an engine-state question.

**Final.**
- out = A.rgb × lighting + spec. Self-illum, when present, replaces part of the A.rgb × lighting term.
- Then out ×= linear_light_scale, then fog (section 7), then sRGB encode and 8-bit clamp.

**mat_fullbright** (cheat, default 0):
- 2: base, base2 and detail are replaced by flat 50% grey (with alpha 0 when self-illuminated), modulation goes to 0 and envmap tint goes to 0. This is the debug "lighting only" view.
- 1: handled by the engine (Open questions).

### 3. WorldVertexTransition (displacement blending)

- This is the world shader with $basetexture2 (plus optional $bumpmap2, $blendmodulatetexture, $basetexture2noenvmap). Everything in section 2 applies.
- **Blend factor.**
  - b = the per-vertex alpha of the displacement vertex: the BSP's 0..255 displacement alpha divided by 255, interpolated across the triangle.
  - $maskedblending 1: b is not taken from the vertex; see below.
- **$blendmodulatetexture** (raw, sampled at uv_base through $blendmasktransform):
  - lo = clamp(m.g − m.r, 0, 1); hi = clamp(m.g + m.r, 0, 1); b = smoothstep(lo, hi, b), where smoothstep(a, c, x) = t²(3 − 2t) and t = clamp((x−a)/(c−a), 0, 1).
  - So the green channel moves the transition point and the red channel sets its softness: r = 0 gives a hard step at g.
  - Skipped when the material has $selfillum or water height fog is active. mat_disable_fancy_blending 1 removes it.
  - $maskedblending 1 instead uses b = m.g directly.
- **Albedo:** A.rgb = lerp(base1.rgb, base2.rgb, b). Both textures are sRGB-decoded and blended in linear space. The "blended alpha" lerp(base1.a, base2.a, b) feeds only $basealphaenvmapmask.
- **Bumped variant:**
  - $bumpmap alone: both layers share one normal map.
  - With $bumpmap2: n.xyz = lerp(n1.xyz, n2.xyz, b), not renormalised. Alpha stays n1's, unless $blendmodulatetexture and $normalmapalphaenvmapmask are both set, in which case alpha is lerped too.
  - Bump lighting then proceeds as in section 2. The division by Σw absorbs most of the length loss.
- The $bumpmap2 transform is $bumptransform2. If a $detail is present, bump uses uv_base with no bump transform.
- Default textures and params match section 2.

### 4. VertexLitGeneric (static props, no baked per-vertex light)

**Routing.**
- $phong 1 with a $bumpmap (or a $phongwarptexture-type diffuse warp, or a base-alpha phong mask) switches to a separate Phong ("skin") shader. That shader adds a fresnel-ranged Blinn-Phong specular and rim light. Stock CS:S props almost never use it, so it is not specified here (Open questions).
- $bumpmap without phong uses a per-pixel-lit bump variant: the same lighting terms evaluated per pixel with the decoded normal.

**Albedo.**
- A = sRGB-decode(base). Then detail, using the same combine table as 2.
  - Detail tint is gamma→linear converted here.
  - Detail is sRGB-decoded for every mode except 0.
  - Mode 5 adds f·d.rgb after lighting.
  - Mode 6 adds clamp(m·d.rgb + a, 0, 1) after lighting, where:
    - for f ≥ 0.5: m = 1/f and a = 1 − m;
    - otherwise: m = 4f and a = −0.5m.
- Then A.rgb ×= modulation.rgb. Modulation is the material colour ($color2 on models, see Open questions) **converted gamma→linear** per channel. Values > 1 pass through unchanged.

**Per-vertex lighting** (linear light):

```math
\text{light} = \sum_{i<n} C_i \cdot \text{cosine}_i \cdot \text{atten}_i \;+\; \text{ambient}(N)
```

- N is the normalized world normal.
- Up to 4 lights. The engine picks which, and they come with linear colour C_i.
- **Ambient cube:** six linear colours +X, −X, +Y, −Y, +Z, −Z (world axes). With N² = (Nx², Ny², Nz²), ambient = Nx²·(Nx ≥ 0 ? +X : −X) + Ny²·(Ny ≥ 0 ? +Y : −Y) + Nz²·(Nz ≥ 0 ? +Z : −Z).
  - The weights sum to 1 for a unit normal. An axis component of exactly 0 picks the + face with weight 0.
- **cosine:** L = normalize(light_pos − P) for point and spot lights, or −light_dir for directional lights.
  - Normal: max(0, N·L).
  - $halflambert: (0.5·N·L + 0.5)², which is never clamped before squaring (range 0..1).
- **atten:**
  - Point: 1 / (a0 + a1·d + a2·d²), with d = |light_pos − P| and a0/a1/a2 the light's constant/linear/quadratic terms.
  - Spot: multiply that by clamp(max(1e−4, (cosθ − cos φ_outer) / (cos θ_inner − cos φ_outer))^exponent, 0, 1), where cosθ = light_dir · (−L). If the cone spread is ≤ 1e−10, the divisor is 1.
  - Directional: 1.
  - There is no range cut-off in the shader.
- Lighting is computed per vertex and interpolated (Gouraud).
  - The per-pixel bump variant evaluates the same terms per pixel, using a 6-colour ambient cube passed to the pixel shader.
- No overbright constant is applied to dynamic or ambient light. Their scale is whatever the engine supplies.
- **Static per-vertex lighting**, for reference when a prop does have it: light += (2 · v)^2.2, where v is the 0..1 vertex colour. This is the same overbright-2 gamma encoding as lightmaps. Ambient and local lights are added only when the engine flags the mesh as dynamically lit.

**Rest of the pixel.**
- diffuse = A.rgb × light.
  - With $vertexcolor on a lit model, A is additionally × vertex colour.
  - $selfillum: lerp(diffuse, $selfillumtint·A, mask), where mask = base.a, or the $selfillummask texture.
  - $selfillum_envmapmask_alpha: uses 8× the envmap-mask alpha as an over-bright self-illum factor.
- Envmap: as in section 2, but with no fresnel. $envmaptint is gamma→linear converted. The envmap mask is sampled at uv_base, with no own transform in the non-bump shader.
- α = $alpha × base.a (when not used as a mask) × vertex alpha (if $vertexalpha).
- Final output and fog as in 1 and 7.

### 5. Decals

**LightmappedGeneric with $decal 1** (most brush decals) is just section 2 drawn with depth offset. The engine supplies the decal's base uv and the underlying surface's lightmap uv.

**LightmappedGeneric_Decal** (an engine-internal shader for decals on bump-lightmapped surfaces):
- result = Σ_k lm_k · (1/3) over the three bump pages, which are the flat-normal weights (basis_k.z)² = 1/3;
- then × decal rgb × $color modulation × vertex colour; alpha = decal alpha × modulation alpha × vertex alpha;
- normal alpha blending.
- Its set-up does not show sRGB or lightmap-scale state. Whether CS:S uses it is unknown (Open questions).

**DecalModulate:**
- Everything stays in gamma space: no sRGB decode of the decal texture and no sRGB framebuffer encode.
- Blend: dst_new = src·dst + dst·src = **2 · src · dst**, per channel, clamped to 1. A texel of 0.5 (128) leaves the surface unchanged; darker darkens, brighter brightens up to 2×.
- Alpha test: passes when the decal alpha > 0. Fully transparent texels are discarded; partial alpha does **not** fade the effect.
- No depth write, depth offset (decal polygon offset), no alpha write.
- Fog: the fog colour is 0.5 grey, so fully fogged decals become neutral. The fog factor is first raised to the power 0.4, i.e. fog = clamp(f, 0, 1)^0.4, and then used per section 7, so decals fade faster than the surface under them.
- No lighting term; the underlying surface already carries it.

### 6. UnlitGeneric and Sky

**UnlitGeneric** is the same model shader as section 4, without lighting:
- out = A.rgb (after detail and modulation, where $color is gamma→linear converted) [× vertex colour, gamma→linear converted, if $vertexcolor] + envmap term;
- α as in 4;
- fogged unless $nofog.
- Many CS:S skybox face materials use it.

**Sky (LDR, i.e. when HDR is off):**
- out = sRGB-decode(base) × $color. $color defaults to (1,1,1) and is raw.
- Never fogged, no depth test, sRGB encode on write. So an 8-bit sky texel appears on screen unchanged: no overbright and no scale.
- The texture coordinates are the engine-supplied face coordinates through $basetexturetransform. The shader adds no scaling.
- 16-bit-per-channel textures are read linear (no sRGB). Integer-16-bit textures, and float-16 on integer-HDR hardware, are multiplied by 16.

**HDR sky (reference only):**
- An "RGBS" compressed texture (8-bit rgb plus a scale in alpha, read linear) decodes to rgb · a · 8 · $color.
- Decoding happens **before** filtering. Four point samples, offset by ±(0.5/size − 0.01/max(w, h)) texels, are each decoded and then bilinearly blended by hand.
- The three-texture "exposure" path is a stub in this SDK revision that outputs solid red.
- The convar mat_use_compressed_hdr_textures (default 1) selects the compressed texture when present.

### 7. Fog (range fog, ps2.0b and later, used by all of the above except Sky)

- z = the clip-space depth of the pixel (≈ view distance).
- f = clamp(min(max_density, (z − start)/(end − start)), 0, 1).
- Shader-side blend: colour = lerp(colour, fog_colour_linear, f²).
  - The squaring is deliberate, to approximate fixed-function fog in mid-range.
  - Fog colour is supplied in linear space.
- Radial fog uses the true eye-to-point distance in place of z.
- Water height fog uses: lerp(colour, fog_colour, clamp(f_water, 0, 1)) with
  - f_water = clamp(((waterZ − Pz)/(eyeZ − Pz)) · z / range, 0, 1).
  - It is not squared.

## Per-pixel order

LightmappedGeneric / WorldVertexTransition, in order:

1. Sample the base texture (and base2) and decode them from sRGB. Sample the normal map(s), raw.
2. Decode the normal: 2·rgb − 1.
3. Sample the lightmap page(s) at the bilinear-filtered gamma texel and decode them to linear.
4. Sample the detail texture (decoded only for mode 1) and multiply it by $detailtint.
5. Blend base/base2 by the vertex alpha, after $blendmodulatetexture shaping. Do the same for normal1/normal2.
6. Build the specular mask: normal alpha, then $envmapmask, then (1 − blended alpha).
7. Albedo = blended base. α = base1.a unless it is used as a mask.
8. Detail combine (modes 0–4, 7–11).
9. Albedo.rgb ×= vertex rgb. α ×= vertex alpha × $alpha × $alpha2.
10. Lighting = bump-weighted, sum-normalised page mix (or the single flat page) × 2^2.2 × $color.
11. diffuse = albedo × lighting.
12. Self-illum lerp by base1 alpha.
13. Envmap: reflect, cube sample (decoded), × mask × tint, contrast, saturation, × fresnel.
14. result = diffuse + specular.
15. × linear light scale (1 in LDR).
16. Fog blend (f²).
17. sRGB encode and 8-bit clamp. Alpha test (α ≥ ref) and blend states act on the output alpha.

VertexLitGeneric: per vertex, ambient + Σ lights. Per pixel: base, detail, × modulation, × light, post-lighting detail, self-illum, envmap (no fresnel), then steps 14–17.

## Edge cases

- **sRGB curve vs 2.2.** The texel encoding assumes a 2.2 power, but DX9 hardware decodes with the piecewise sRGB curve.
  - A lightmap texel of 128 decodes to 0.21586 (sRGB) instead of 0.21953 (2.2). The lightmap therefore lands at 0.9918 instead of 1.0087.
  - After sRGB re-encode the visible difference is about 1/255 at mid-tones, larger in deep shadows: small texels are darker under the real curve's linear toe.
  - Implementations should use the GPU's sRGB decode/encode to match the reference, not pow 2.2.
- **Lightmap saturation.**
  - Per channel, linear luxels above 3.999 all encode to texel 239 (≈ 4.0 after decode), and the clamp is per channel (hue shift).
  - On screen, albedo × light saturates at 1.0 per channel: anything with lighting > 1/albedo clips to white.
- **Bump sum = 0.** If the decoded normal points away from all three basis directions (n·basis_k ≤ 0 for all k, e.g. a z ≤ 0 normal), the divisor is 0. The reference GPU result is undefined (NaN/∞). Content never does this on purpose; treat as black or guard with a small epsilon (Open questions).
- **Un-normalised normals.** Normal-map texels are not renormalised. For example, (128,128,255) gives (0.0039, 0.0039, 1.0). Weights still sum to ≈ 1 because of the division.
- **Blend-modulate with m.r = 0.** The two smoothstep edges coincide, so the step is hard. A blend factor exactly equal to m.g divides 0 by 0 (undefined on GPU). Any other value gives 0 or 1.
- **Texel quantisation of the normal.** Byte 128 maps to +0.0039, not 0. There is no exact zero.
- **No base texture + envmap:** black albedo, so only the reflection shows.
- **Self-illum on a texture without alpha:** silently ignored (flag cleared at load).
- **$envmapmask on a bumped world material:** dropped at load.
- **Alpha test with self-illum or $basealphaenvmapmask:** alpha test is turned off.
- **Decals on bumped surfaces:** see section 5 and Open questions.
- **Shader model 2.0-only GPUs** lose pixel range fog, detail tint and the in-shader sRGB step. The reference is shader model 2.0b/3.0, so ignore them.

## Quirks

- **$basealphaenvmapmask is inverted:** the mask is (1 − alpha), so opaque (alpha 1) base texels get *no* reflection. This is what the shader does, in both the world and the model shaders. Keep it.
- **Fast-path contrast bug (world shader).** Unless one of the following holds, the material is drawn on a fast path where contrast = 0 (or exactly 1 if $envmapcontrast = 1), saturation = 1, fresnel = 1 and self-illum tint = 1:
  - 0 < $envmapcontrast < 1 (or > 1) **and** $envmapsaturation ≠ 1;
  - or $fresnelreflection ≠ 1;
  - or a non-white $selfillumtint is in use.
  - Consequence: $envmapcontrast 0.5 with default saturation is **ignored**, and so is $envmapsaturation 0.5 with default contrast.
  - Keep it to match the look.
- **Colour parameters differ by shader.** The world shader uses $color, $envmaptint, $detailtint and $selfillumtint raw as linear multipliers. The model shaders convert $color/$color2, $envmaptint and $detailtint from gamma to linear first.
- **The parameter gamma→linear conversion is a table**, applied only to components ≤ 1:
  - the value is rounded to the nearest 1/255;
  - any value ≥ 0.95 becomes exactly 1.0;
  - values > 1 pass through unchanged.
  - For example, a tint of 0.96 becomes 1.0, and 1.5 stays 1.5.
- **World-shader vertex colour is not linearised**: it multiplies linear albedo directly. Unlit and vertex-lit models linearise vertex colours with pow 2.2.
- **Displacement alpha** never comes from $basetexture2's alpha.
- **Detail modes 5/6 do nothing on world brushes.**
- **Half-lambert is squared** in the shaders. The CPU-side software lighting in the math library does not square it, so baked tools may differ.
- **DecalModulate** ignores partial alpha (binary test at > 0) and ignores sRGB entirely. Its 2× multiply happens in gamma space.

## Test cases

For colour results the model is pure 2.2 power for both decode and encode, unless marked "sRGB". Expected 8-bit outputs are ±1. Linear values are given to 4–5 significant digits.

| Setup | Input | After | Expected |
|---|---|---|---|
| Luxel decode | luxel (128, 64, 255, e = −1) | decode | linear (0.25098, 0.12549, 0.5) |
| Lightmap encode | L = 0, 0.01, 0.25, 1.0, 2.0, 4.0, 8.0 | texel byte | 0, 16, 68, 128, 175, 239, 239 |
| Lightmap decode | texel 128 | × 2^2.2 | 1.0086 (2.2 model); 0.99183 (sRGB) |
| Lightmap decode | texel 64 | × 2^2.2 | 0.21952 (2.2); 0.23557 (sRGB) |
| Unbumped brush | base byte 200, lightmap texel 128, $color 1 | output byte | 200.8 → 201 (2.2); 199.3 → 199 (sRGB) |
| Unbumped brush | base byte 200, texel 64 | output byte | 100.4 → 100 (2.2: exactly half of base in gamma); 103.2 → 103 (sRGB) |
| Unbumped brush, overbright clip | base 200, texel 255 | output | linear 2.692 → clamped → 255 |
| Bump weights, flat normal | n = (0,0,1); pages (0.9, 0.3, 0.0) | lighting / scale | w = (1/3, 1/3, 1/3); mix = 0.4 |
| Bump, byte-flat normal | n texel (128,128,255) → (0.00392, 0.00392, 1.0); same pages | mix | w ≈ (0.33704, 0.33469, 0.32830), Σ ≈ 1.00003; mix ≈ 0.40373 |
| Bump, normal = basis 0 | n = (0.816497, 0, 0.577350); same pages | mix | w = (1, 0, 0); mix = 0.9 |
| Bump, grazing normal | n = (1, 0, 0); same pages | mix | w = (0.66667, 0, 0), Σ = 0.66667; mix = 0.9 |
| Bump, tilted | n = (0.5, 0.5, 0.707107); same pages | mix | w ≈ (0.66667, 0.31100, 0); mix ≈ 0.70914 |
| Bump, real texel | n texel (200, 90, 220) → (0.56863, −0.29412, 0.72549); pages (1.0, 0.5, 0.25) | mix | w ≈ (0.77994, 0, 0.15578); mix ≈ 0.87514 |
| Detail mod2x | A = 0.5 (linear), d raw 0.5, f = 1 | A | 0.5 |
| Detail mod2x | A = 0.5, d = 1.0, f = 1 | A | 1.0 |
| Detail mod2x | A = 0.5, d = 1.0, f = 0.5 | A | 0.75 (×1.5) |
| Detail mode 2 | A = 0.2, d = (0.8, a = 0.5), f = 1 | A | 0.5 |
| Envmap fresnel default | R0 = 1, any N·V | F | 1 |
| Envmap fresnel | R0 = 0, N·V̂ = 0.5 | F | 0.03125 |
| Envmap contrast/sat (slow path) | spec (0.8, 0.4, 0.2), contrast 1, saturation 0 | spec | (0.28984, 0.28984, 0.28984) |
| Fast-path quirk | spec (0.8, 0.4, 0.2), contrast 0.5, saturation 1, R0 = 1 | spec | (0.8, 0.4, 0.2), contrast ignored |
| $basealphaenvmapmask | base alpha 1.0, spec 0.6 | spec | 0 |
| $basealphaenvmapmask | base alpha 0.25, spec 0.6 | spec | 0.45 |
| Self-illum | A = 0.5, lighting 0.2, base.a = 1, tint 1 | diffuse | 0.5 (lighting ignored) |
| WVT blend | b = 0.25, base1 (1,0,0), base2 (0,0,1) | A (linear) | (0.75, 0, 0.25) |
| WVT blendmodulate | m.r = 0.1, m.g = 0.5, b = 0.5 | b' | 0.5 |
| WVT blendmodulate | m.r = 0.1, m.g = 0.5, b = 0.45 | b' | 0.15625 |
| WVT blendmodulate hard | m.r = 0, m.g = 0.5, b = 0.49 / 0.51 | b' | 0 / 1 |
| Ambient cube | N = (0.6, 0, −0.8); +X = (1,0,0), −Z = (0,0,1), others 0 | ambient | (0.36, 0, 0.64) |
| Point light | C = 10000, a = (0,0,1), d = 100, N·L = 1 | contribution | 1.0 |
| Point light | C = 1, a = (1, 0.5, 0), d = 2, N·L = 0.5 | contribution | 0.25 |
| Half-lambert | N·L = 0 / −1 / 1 | cosine | 0.25 / 0 / 1 |
| Static vertex light | vertex byte 128 | linear light | (2·128/255)^2.2 = 1.0086 |
| Model $color2 | 0.5 gamma | modulation | 0.2195 (table: round to 1/255 then ^2.2; 0.5 → 128/255 → 0.21953) |
| Model tint ≥ 0.95 | 0.96 | modulation | 1.0 |
| DecalModulate | dst 0.4, decal 0.5 (gamma) | dst | 0.4 |
| DecalModulate | dst 0.4, decal 0.25 / 1.0 | dst | 0.2 / 0.8 |
| DecalModulate | dst 0.7, decal 1.0 | dst | 1.0 (clamped) |
| DecalModulate alpha | decal alpha 0 / 0.01 | drawn? | no / yes (full strength) |
| Sky LDR | base byte 173, $color 1 | output byte | 173 |
| HDR RGBS sky | rgb (0.5, 0.25, 1.0), a = 0.5, $color 1 | linear | (2, 1, 4) |
| Pixel range fog | f = 0.5, colour 1.0, fog colour 0 | colour | 0.75 |
| Decal fog | f = 0.5 | effective fog factor | 0.5^0.4 = 0.75786, then squared for range fog → 0.57435 |

## Mapping to our renderer

Quantities a renderer needs as inputs, named neutrally:

- **Per world vertex:**
  - position;
  - base texture uv (from the texinfo S/T vectors and texture size);
  - lightmap atlas uv of page 0;
  - per-vertex page offset Δ (atlas width of one lightmap page) for bumped faces;
  - face/vertex normal;
  - tangent (S-axis direction) and binormal (T-axis direction), projected into the face plane;
  - for displacements, the 0..1 blend alpha.
- **Lightmap atlas:**
  - per face and style, 1 page (unbumped) or 4 pages (bumped: flat, b0, b1, b2) side by side at a constant uv step;
  - texels encoded as in section 2 (8-bit, gamma, half intensity), sampled bilinearly with sRGB decode;
  - a scalar lightmap scale of 2^2.2.
- **Per material:**
  - textures, with whether each is sRGB-decoded (section 1);
  - every $parameter used above, with the defaults from Constants;
  - flags: $alphatest, $translucent, $selfillum, $basealphaenvmapmask, $normalmapalphaenvmapmask, $vertexcolor, $vertexalpha, $halflambert, $nodiffusebumplighting, $ssbump, $decal, $nofog;
  - which fast-path rule applies (Quirks).
- **Per view:**
  - eye position;
  - fog start, end, max density, colour (linear) and mode;
  - linear light scale (1).
- **Per model instance (vertex-lit):**
  - 6 ambient cube colours;
  - up to 4 lights, each with type, linear colour, position, direction, attenuation (constant, linear, quadratic), spot inner/outer half-angle cosines and exponent;
  - optional per-vertex static light colours.
- **Per cubemap:** the cube texture (sRGB in LDR) that the engine assigns to each brush or prop.
- **Output:** sRGB-encoded 8-bit framebuffer; DecalModulate's 2·src·dst blend done in gamma space.

**Convars that change behaviour** (default in brackets):

- mat_hdr_level [2 on capable hardware; reference uses 0]
- mat_fullbright [0]
- mat_specular [1]
- mat_bumpmap [1]
- mat_reducefillrate [0]
- mat_fastnobump [0]
- mat_disable_fancy_blending [0]
- mat_disable_lightwarp [0]
- r_lightmap_bicubic [0]
- mat_use_compressed_hdr_textures [1]

The defaults for mat_specular, mat_bumpmap and mat_reducefillrate live in the engine config and are not in the SDK; the values given are the usual ones.

## Open questions

- **Measured in CS:S (refcmp, de_dust2, 2026-10-05, Linux build, mat_hdr_level 0):**
  - The game is 1.22× brighter, in linear light, than albedo × lighting with the LDR lightmap encoding and the 2^2.2 scale above (mean absolute error 0.071 over 5.4M pixels in 9 views).
  - The scale constant or the tone-map scale may differ from the derived values, or the Linux renderer's sRGB handling may differ.
  - Bump pages with the basis order and the unflipped normal decode above match the game visually on dust2's A-site walls; a green flip visibly does not.

1. **The exact lightmap scale constant.** The engine/shader-API value of the lightmap scale is not in the SDK. 2^2.2 = 4.5948 is derived from the published encoding (0.5 · L^(1/2.2)) and confirmed by the in-shader vertex path (2v)^2.2.
   - To check: in CS:S with mat_hdr_level 0, make a test map with a white base texture (255) and a luxel of exactly linear 1.0 (texel 128). The surface should read 255 (or 254 because of the sRGB curve); a scale of 4 would read about 243.
   - Also confirm that the engine encodes LDR lightmaps with the curve above (the compiler says so) and does not use the hardware sRGB curve.
2. **Bumped page encoding.** The compiler contains an unused routine that rescales the three bump pages so their mean equals the gamma-encoded flat value, instead of gamma-encoding each page independently. This spec assumes each page is encoded independently with the same curve, which is consistent with the DX9 shader's linear, sum-normalised mix.
   - To check: compare a bumped wall in game against both encodings.
3. **Linear light scale and envmap scale in LDR.** These are expected to be 1.0. They are set by the shader API, which is not public.
4. **Default alpha-test reference** when $alphatestreference is 0 or unset. It is set by the shader API; commonly cited values are 0.5 and 0.7. To check: fence or foliage textures with known alpha ramps.
5. **Brush tangent streams.** It is assumed that the engine's per-vertex tangent S/T for flat brushes match the displacement builder (S/T axes orthogonalised against the normal). This affects only envmap reflection on bumped brushes; diffuse bump is fixed by the baked basis.
6. **mat_fullbright 1** is engine-side. Presumably it binds a white or neutral lightmap; whether the 2^2.2 scale still applies (which would make everything ~4.6× too bright in linear) is unknown.
7. **Which colour parameter is the model modulation.** $color for brushes/unlit and $color2 for models is the usual rule, but the code that picks it is not in the SDK. The world shader's $color behaviour also comes from that code.
8. **Translucent and additive blend states and depth writes** come from a base class that is not in the SDK. Assumed: $translucent = standard alpha blend with no depth write; $additive = src·α + dst (or src + dst).
9. **Decals on bump-lightmapped brushes.** Does CS:S draw them with LightmappedGeneric_Decal (an average of the three bump pages, with no visible lightmap-scale or sRGB set-up) or with the decal material's own shader sampling page 0? Check in game by comparing the brightness of a decal on a bumped and an unbumped wall.
10. **Bump weight sum of 0:** what the reference hardware shows (black, white or garbage).
11. **Vertex-lit inputs from the engine:**
    - how CS:S static props are actually lit at mat_hdr_level 0: baked per-vertex (computed by the engine at load or by the compiler), or ambient cube + local lights;
    - how the engine picks and scales the 4 lights;
    - how it fills spot parameters (inner/outer cone stored as half-angles?);
    - whether ambient cube values are scaled.
12. **Pixel-fog depth.** It is assumed that z is the clip-space depth (≈ view distance), as written by the vertex shader. The exact projection terms are engine-set.
13. **Revision skew.** This was read from the current public SDK (which includes 2025-era additions such as bicubic lightmaps). CS:S's shipping shader DLL may predate some of them; the behaviour above at default convars is expected to be the same.
14. **$phong on props** (the skin shader) is not specified. Specify it if any CS:S content we load uses it.
