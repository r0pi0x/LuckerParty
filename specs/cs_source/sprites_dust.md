# Counter-Strike: Source: sprites (env_sprite) and dust motes (func_dustmotes)

Source basis: Valve's public Source SDK 2013 (current GitHub head). I read:
- the shared sprite entity (server spawn, keyvalues, network fields and their quantisation, animation) and its client drawing code;
- the client's sprite-model loader (how a sprite model becomes materials, size and quad extents);
- the client pixel-visibility (occlusion query) system and the glow sight-distance helper;
- the client entity render-effect blend code (renderfx) and the translucent-entity draw loop;
- the DX9 Sprite shader (C++ set-up plus its vertex and pixel shaders) and the shared final-output include;
- the server and client dust entity code, the client particle manager (allocation cap, sort, time step), the particle quad helper and the wind helper.

CS:S's own game DLL source is not public. This spec assumes CS:S uses the same shared sprite and dust code, which ships in every Source multiplayer game of that era. Several layers are **not in the SDK**: the engine's model loader (how a `model` keyvalue path reaches the sprite loader), the engine's network float encoder (rounding of quantised fields), the material system (shader-parameter defaults, the pixel-diameter helper, occlusion-query hardware details) and the occlusion-proxy materials. Rules that depend on them are marked *derived* or listed under Open questions.

I checked the map data directly from the user's CS:S install (de_dust2 entity lump, `materials/sprites/glow.vmt`, `materials/particle/sparkles.vmt`, texture headers). No game files are committed. A second pass re-read the sprite shader, the shader base class's parameter handling, the sprite draw and glow code, and the pixel-visibility helper to settle mode 5 colour, brightness inputs and sRGB; it also checked the full text of `sprites/glow.vmt` (no `$color`, `$alpha`, `$ignorevertexcolors`, `$nosrgb` or proxies) and de_dust2's fog controller.
Status: draft

## Summary

- An env_sprite is one textured quad, two-sided. Its size is the base texture's pixel size times `scale`, in world units: a 128×128 texture at scale 1 is 128×128 units. Its orientation comes from the **material** (`$spriteorientation`), not the entity. `vp_parallel` (used by all of de_dust2's glows) means the quad is parallel to the view plane, using the camera's own right/up vectors. It does not turn toward the camera position.
- Render mode picks the blend:
  - 5 (Additive) is "src·α + dst", depth-tested.
  - 3 (Glow) and 9 (World Space Glow) are also additive but **ignore the depth buffer**. They draw after all other translucents and are hidden instead by a hardware occlusion query on a small proxy quad (GlowProxySize units). Visibility fades at 16 per second (full fade in 1/16 s).
  - Glow modes also fade with distance (1200²/d²), unless renderfx is 14 (Constant Glow).
  - Mode 3 additionally grows with distance, so it keeps a fixed size on screen.
- renderamt is applied **more than once** in the glow modes. With de_dust2's renderfx 14 it effectively scales the light by (renderamt/255)³.
- In mode 5, renderamt (as the sprite's brightness) is applied **once**, as alpha only; rendercolor tints once; renderfx, distance and occlusion do nothing. de_dust2's mode 5 halos (renderamt 25) therefore add at most ≈ +17/255 and are nearly invisible in game.
- The `Alpha` input changes only the entity blend, never the brightness: glow sprites react, mode 5 sprites do not.
- func_dustmotes spawns tiny alpha-blended particles (`particle/sparkles`) at random points **inside the brush**, at SpawnRate per second. Each lives a uniformly random LifetimeMin..LifetimeMax seconds.
  - Alpha follows a raised-cosine bump over its life (0 → 1 at mid-life → 0) and fades linearly to zero at DistMax view depth.
  - Size is constant on screen: Size/10000 of the view depth as half-width.
  - Motes drift vertically at a random constant speed in ±SpeedMax. Their horizontal speed decays to the wind speed, which is 0 on de_dust2, at 50 units/s².

## Units and conventions

- Distances: Hammer units (1 unit = 1 inch). Z is up. Angles: degrees, (pitch, yaw, roll).
- Colours: 0–255 per channel unless stated. Shader maths below is in normalised [0, 1].
- "View right/up/forward" are the camera's basis vectors, including any camera roll. "Depth" means distance along the view forward axis (view-space z), not Euclidean distance.
- Time: seconds.
  - Sprite animation runs on the server every tick and is networked.
  - Glow visibility fades and dust simulation run per **client frame**, using the frame time.
- LDR (mat_hdr_level 0) is assumed, as in shaders.md.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| sprite_scale_max | 64 | - | scale is clamped to [0, 64] at spawn |
| sprite_scale_net_low | 0.25 | - | lowest networked scale (see Quantisation) |
| sprite_scale_net_step | 0.25 | - | networked scale step (8 bits over 0.25..64) |
| glow_proxy_default | 2 | units | GlowProxySize if the key is absent |
| glow_proxy_net | integers 1..64 | units | networked GlowProxySize (6 bits over 1..64) |
| framerate_net_step | 60/256 ≈ 0.234 | frames/s | networked framerate step (8 bits, low 0.234, high 60) |
| glow_full_dist | 1200 | units | glow distance fade: full brightness inside this |
| glow_screen_ref_dist | 200 | units | mode 3 size: scale × dist/200 |
| vis_fade_rate | 16 | 1/s | visible fraction moves toward its target by 16 × frame_time per frame (fade time 0.0625 s) |
| vis_full_threshold | 0.95 | - | visible/possible ≥ 0.95 counts as 1 |
| proxy_min_pixels | 5 | px | the proxy's projected size is at least 5 pixels |
| proxy_half_diag_factor | 1 | - | proxy square has half-diagonal = proxy size (half-side = 0.7071 × size) |
| trace_back_off | 4 | units | fallback line-of-sight trace stops 4 units before the sprite (along view forward) |
| upright_degenerate_cos | 0.999848 | - | cos 1°: upright orientations are skipped when view is within 1° of vertical |
| dust_accel | 50 | units/s² | horizontal velocity approaches wind speed at this rate |
| dust_size_divisor | 10000 | - | func_dustmotes Size → screen-constant factor |
| dust_spawn_tries | 10 | - | random points tested per spawn |
| dust_max_spawn_dt | 0.1 | s | frame time used for spawning is clamped to this |
| particle_max_dt | 0.1 | s | particle simulation step clamp |
| particle_global_cap | 2048 | particles | shared by all classic client particle effects; extra spawns fail silently |
| particle_alpha_cutoff | 0.5 | /255 | particles with vertex alpha < 0.5 are not drawn |
| dust_material | particle/sparkles | - | UnlitGeneric, $translucent, $vertexcolor, $vertexalpha, 32×32 texture |

## Behavior

### 1. env_sprite keyvalues → state

| Key | Effect |
|---|---|
| model | Sprite material path (see 1a). The material decides texture, size in texels, orientation and anchor. |
| scale | Clamp to [0, 64], then quantise for the network (1b). World size multiplier. |
| rendermode | 0–10, see section 3. |
| renderfx | Modulates the entity blend (section 4). 14 (Constant Glow) disables the glow distance fade. |
| rendercolor | Vertex RGB (how it is used depends on mode). |
| renderamt | Read twice at spawn: it becomes both the entity's render alpha (source of the entity blend B, section 4) and the sprite's separate **brightness** (vertex alpha, and the renderfx 14 glow factor). After spawn the two are independent (section 4a). |
| GlowProxySize | Size of the occlusion proxy, units; networked as integer 1..64 (1b). Default 2. |
| framerate | Animation frames per second (server side). |
| HDRColorScale | Multiplies RGB **only when HDR is enabled**. No effect in LDR. Default 1. |
| spawnflags 1 (Start on) | Only matters for a sprite with a targetname. A named sprite without this flag starts hidden. Unnamed sprites always start visible. |
| spawnflags 2 (Play once) | When the animation passes its last frame, the sprite hides itself. |
| angles | **Hammer quirk:** if yaw ≠ 0 and roll = 0, yaw is moved into roll (roll = yaw, yaw = 0). Roll matters for `vp_parallel` materials (section 2). |
| inputs | ShowSprite / HideSprite / ToggleSprite, SetScale, ColorRedValue/Green/Blue (0–255, clamped). The generic entity inputs `Alpha` and `Color` also work: `Alpha` changes only the render alpha (B), never the brightness (section 4a). |

#### 1a. Model path → material

- The sprite loader lower-cases the name, converts back-slashes to '/', strips the extension (so `.spr` and `.vmt` behave the same), prepends `materials/` and appends `.vmt`.
  - `sprites/glow01.spr` → `materials/sprites/glow01.vmt`.
- de_dust2 writes `materials/Sprites/glow.vmt`. That path only resolves if the (non-SDK) engine model loader strips the leading `materials/` first. *Derived*: treat a leading `materials/` as optional.
- From the material's base texture:
  - w, h = the texture's full-resolution pixel size (its mapping size, unaffected by mip/LOD settings);
  - frame count = the texture's animation frame count.
- One material instance exists per render mode: the same vmt with `$spriterendermode` set to that mode. Modes 6 and 10 have no material and never draw.

#### 1b. Quantisation (CS:S, non-HL2 build)

- **scale** is sent in 8 bits over [0.25, 64] with step 63.75/255 = 0.25. The client therefore sees one of 0.25, 0.5, …, 64. Values below 0.25 become 0.25.
  - Rounding to the nearest step versus truncating is engine code (Open question 1). Both give the same result for 0.8 → 0.75, 1 → 1, 2.5 → 2.5 and 3 → 3.
- **GlowProxySize** is sent in 6 bits over [1, 64] with step 1. Integers 1..64.
- **framerate** is sent in 8 bits over [0.234, 60]. It is only used server side, so the quantisation does not matter for drawing.
- **HDRColorScale** is sent unquantised.
- At draw time a scale ≤ 0 would be treated as 1. That cannot happen after quantisation.

### 2. Geometry: size, anchor, orientation

- **Anchor** comes from `$spriteorigin` = [ox oy] (fractions of the texture), read from the vmt.
  - Corner offsets in texels:
    - left = −w·ox, right = w·(1 − ox);
    - up = h·oy, down = h·(oy − 1).
  - `[0.5 0.5]` centres the quad on the entity origin. All of de_dust2's glow vmts say so explicitly.
  - If the vmt has no vector `$spriteorigin`, the loader uses the centred values (ox = oy = 0.5). Open question 2 covers whether a shader default can override this.
- **World quad.** With scale s and orientation axes R (right) and U (up), the four corners are:
  - origin + U·(down·s) + R·(left·s): uv (u0, v1)
  - origin + U·(up·s) + R·(left·s): uv (u0, v0)
  - origin + U·(up·s) + R·(right·s): uv (u1, v0)
  - origin + U·(down·s) + R·(right·s): uv (u1, v1)
  - The UVs are inset by half a texel: u0 = 0.5/w, u1 = 1 − 0.5/w, v0 = 0.5/h, v1 = 1 − 0.5/h. Texture row 0 (v0) is at the top (U side).
  - World width = w·s, height = h·s units.
  - Back faces are not culled.
- **Mode 3 only:** s is first multiplied by (sight_distance / 200), where sight_distance is defined in section 5. A scale of exactly 0 counts as 1 here. Mode 9 does **not** do this; it keeps its world size.
- **Orientation** comes from `$spriteorientation` (a string in the vmt). If it is absent or unrecognised, `parallel_upright` is used.

| Value | R, U |
|---|---|
| `vp_parallel` | R = view right, U = view up. Parallel to the view plane (not toward the eye position), so it includes camera roll. **If the sprite's roll ≠ 0**, it is treated as `vp_parallel_oriented`. |
| `vp_parallel_oriented` | Rotated within the view plane by the sprite's roll r. R = cos r·view_right + sin r·view_up, U = −sin r·view_right + cos r·view_up. |
| `parallel_upright` | U = world +Z. R = normalise(fwd.y, −fwd.x, 0), where fwd is the view forward vector. Not drawn properly if \|fwd.z\| > 0.999848 (see Edge cases). |
| `facing_upright` | U = world +Z. R is horizontal and perpendicular to a direction **t = normalise(−sprite_origin)**, the direction from the sprite toward the world origin, not toward the camera (see Quirks). Skipped if \|t.z\| > 0.999848. |
| `oriented` | R, U from the sprite's own angles (standard Source right/up of (pitch, yaw, roll)). |

- The axes use the entity origin. The quad is placed at the attachment point if the sprite follows an attachment. Map sprites have no attachment.
- Culling bounds: a cube of half-size s·max(w, h)/2 around the origin. Mode 3's distance growth is not included.

### 3. Render modes: blend, depth, colour

Notation:
- T = texture sample, (rgb, a) in [0, 1].
- V = vertex colour/255, (rgb, a).
- C = the material's `$color` (rgb) and `$alpha` (a) constant. `$alpha` is forced to 1 by the shader when the vmt leaves it out. `$color` is not touched by the sprite shader; what an absent `$color` reads as is engine code (Open question 3). It is taken as 1 below. Only modes 5 and 7 use C at all.
- Colour space (all modes, LDR):
  - `$nosrgb` is declared with default 0 but the shader's own set-up forces it to **1** whenever the vmt does not set it. de_dust2's `sprites/glow` does not set it; `light_glow03` and `glow01` set it to 1 explicitly. So every sprite we care about runs the non-sRGB path.
  - Non-sRGB path: the texture is read without sRGB decode, the vertex colour is passed through unconverted, `$color` is used as written, and the render target write has sRGB conversion **off**. All shader maths and the framebuffer blend therefore operate on raw 8-bit gamma-space values.
  - (For completeness: with `$nosrgb 0` the texture is decoded to linear, vertex colour and `$color` are converted gamma→linear in the shader (a `$color` component above 1 is left as is), and the write re-encodes to sRGB, so blending would happen in linear space. No CS:S sprite we use takes this path.)
  - The shader output is multiplied by the tone-map scale constant for gamma-space output. *Derived:* this is 1 in LDR (engine-supplied).
  - On some hardware the engine asks the shader itself to convert its output through a gamma table (a hardware capability switch, engine-side). Not modelled; assume off.
  - A Mac-only adapter path exists; ignore it.
- Output is clamped to [0, 1] per channel. There is no overbright factor.
- Vertex values for one sprite in one frame:
  - V.a = the sprite's **brightness** (renderamt at spawn; section 4a). It is never the entity blend B and never modified by renderfx;
  - V.rgb = rendercolor, except in modes 3 and 9, where it is pre-scaled (section 5).
- The entity blend B (section 4) is handed to the engine as a global blend before every translucent entity draw, and the sprite code sets it again (to B/255 × glow factor for modes 3/9). **Nothing in the SDK passes that global blend into the sprite's material or mesh**: the sprite is drawn as a client-built quad whose only per-entity inputs are the vertex colour and alpha above. Whether the engine applies it anyway is Open question 11.

| Mode (Hammer name) | Shader colour | Blend | Depth test / write | Fog colour | Draw pass |
|---|---|---|---|---|---|
| 0 Normal | T (vertex colour ignored) | none (opaque square, alpha ignored) | test on / write on | fog colour | translucent entity pass |
| 1 Color, 2 Texture, 4 Solid | T·V | src·α + dst·(1−α) | on / off | fog colour | translucent pass |
| 3 Glow, 9 World Space Glow | T·V | src·α + dst (additive) | **off** / off | black | after all other translucents ("no-Z" pass) |
| 5 Additive | T·V·C (vertex colours on, see note; T·C if they were off) | src·α + dst | on / off | black | translucent pass |
| 7 Additive Fractional Frame | two additive passes: frame ⌊f⌋ weighted (1 − frac f)·$alpha, then frame ⌊f⌋+1 weighted frac f·$alpha | src·α + dst | on / off | black | translucent pass |
| 8 Alpha Add | pass 1: alpha blend; pass 2: src·(1−α) + dst | as stated | on / off | fog colour, then black | translucent pass |
| 6 Environmental, 10 Don't Render | not drawn | | | | |

Mode 5 (Additive) exactly:
- Path from the map to the pixel:
  1. Spawn: renderamt → brightness (integer 0..255, networked in 8 bits). rendercolor → render RGB.
  2. Client draw: if B ≤ 0, skip (renderamt 0, or a renderfx strobe at 0). Otherwise B plays no further part. renderfx (including 14) does **nothing else** in mode 5: the glow factor, distance fade, occlusion query and scale growth are glow-mode only.
  3. All four vertices get colour (rendercolor.r, .g, .b, brightness) as bytes; V = that/255.
  4. Pixel shader: s = T; if vertex colours are enabled, s ← s·V (all four channels); then s ← s·C (C.rgb = `$color`, C.a = `$alpha`); fog (toward black) and the light scale apply to s.rgb only.
  5. Blend: dst.rgb ← dst.rgb + s.rgb·s.a (source factor = source alpha, destination factor = 1), depth tested, no depth write. Destination alpha is not used.
- Result with vertex colours enabled:
  - added = T.rgb · rendercolor/255 · C.rgb · T.a · brightness/255 · C.a · (1 − fog).
  - rendercolor appears **once** (colour only). Brightness appears **once** (alpha only). There is no second brightness factor, no distance fade and no occlusion fade.
- Result with vertex colours disabled: added = T.rgb · C.rgb · T.a · C.a · (1 − fog). rendercolor and brightness have no effect at all; only B = 0 can hide the sprite.
- Which one applies depends on how `$ignorevertexcolors` reads when the vmt leaves it out (no CS:S sprite vmt sets it; `sprites/glow.vmt` contains only `$spriteorientation`, `$spriteorigin` and `$basetexture`):
  - The parameter is declared with default 1. The shader's own set-up fills in `$alpha`, `$hdrcolorscale`, `$nosrgb` and the orientation by hand but not `$ignorevertexcolors` or `$color`.
  - Whether declared defaults are applied to absent parameters is material-system code, outside the SDK. The SDK shows the default strings are exposed to that code through the shader interface, but not what it does with them.
  - The live CS:S measurement settles it in practice (see S6): de_dust2's mode 5 glows (renderamt 25) are effectively invisible. With vertex colours disabled they would add ≈ T.rgb, about +176/255 at the centre of `sprites/glow`, which would be glaring. **Implement: vertex colours enabled (absent `$ignorevertexcolors` = 0), `$color` absent = 1.** The predicted +17/255 peak is close to the measured "invisible" but may still be too bright; Open questions 3 and 11 list what could account for the rest.
- Mode 7 follows the same vertex-colour rule, and its constant colour is replaced by (frame weight·`$alpha`) in rgb and 1 in alpha.

Other mode notes:
- Mode 7: the frame is passed to the material as an integer, so the fractional blend effectively always shows a whole frame (Quirk 4).
- The entity blend (section 4) is not applied to vertex alpha in modes 1, 2, 4, 5, 7 and 8. It only decides whether the sprite is drawn at all (blend ≤ 0 → skipped).
- Within a pass, translucent sprites are drawn back-to-front with the other translucent entities.

### 4. Entity blend (renderamt and renderfx)

- Each frame the client computes an entity blend B ∈ 0..255 (in this section "renderamt" means the entity's current render alpha: the keyvalue at spawn, later changed by the `Alpha` input; see 4a):
  - B = 255 for mode 0;
  - B = renderamt otherwise, when renderfx is 0, 14 (Constant Glow), 19 or any value not listed below.
- renderfx variants (t = client time in seconds, k = entity index × 363):
  - 1 Slow Pulse: renderamt + 16·sin(2t + k)
  - 2 Fast Pulse: renderamt + 16·sin(8t + k)
  - 3 Slow Wide Pulse: renderamt + 64·sin(2t + k)
  - 4 Fast Wide Pulse: renderamt + 64·sin(8t + k)
  - 24: 255·\|sin(12t + k)\|
  - 9 / 10 / 11 strobe: 0 when sin(4t + k), sin(16t + k) or sin(36t + k) is negative, else renderamt
  - 12 flicker: 0 when sin(2t) + sin(17t + k) < 0, else renderamt
  - 13 flicker: 0 when sin(16t) + sin(23t + k) < 0, else renderamt
  - 5 / 6 fade: renderamt itself drops by 1 or 4 **per frame** to 0 (frame-rate dependent)
  - 7 / 8 solidify: renderamt itself rises by 1 or 4 **per frame** to 255
  - 15 / 16 hologram/distort: renderamt set to 180, faded by depth past 100 units over 400 units (16 only), plus random −32..31
- In every case the result is converted to an integer and clamped to 0..255.
- Then, if the entity has a client-side distance fade (fademindist/fademaxdist) below full, B = round(B·fade). de_dust2's sprites set none.
- B is computed once per client frame.
- If B/255 ≤ 0 the sprite is not drawn.

#### 4a. Brightness versus renderamt

- A sprite keeps a **brightness** (0..255, networked in 8 bits) separate from the render alpha that drives B.
  - At spawn, brightness = renderamt. That is the only place a map-placed env_sprite gets its brightness.
  - Brightness feeds V.a in every mode and, for renderfx 14, the glow factor (section 5).
  - The render alpha feeds only B.
- Brightness and scale can each be given a change-over time by game code. The client then interpolates linearly from the old to the new value over that time, truncating brightness to an integer. Map-placed sprites never use this.
- **No map-accessible input or keyvalue changes the brightness at runtime.** The generic `Alpha` input (what `ent_fire <sprite> alpha 255` sends) and an `AddOutput renderamt N` both change only the render alpha, so only B changes:
  - modes 3 and 9 brighten or dim, because B multiplies V.rgb (section 5);
  - mode 5 (and 1, 2, 4, 7, 8) does not change at all, except that B = 0 hides it.
  - That is exactly what the live test saw. Only re-spawning the entity re-reads renderamt into brightness.

### 5. Glow modes (3 and 9): visibility, distance fade, colour

Inputs:
- P = sprite position;
- E = eye position;
- d = \|P − E\| × zoom_factor. zoom_factor = current FOV / default FOV, so it is 1 unless zoomed; for example a 40° scope at default 90° gives 0.444.

Steps:
1. vis = visible fraction from the occlusion system (5a). If vis ≤ 0, the sprite is not drawn.
2. If d ≤ 0, it is not drawn.
3. Glow factor G (br = the sprite's brightness, section 4a):
   - renderfx 14 (Constant Glow): G = (br/255) · vis. There is no distance fade and no mode 3 size growth: the function returns before either.
   - otherwise: G = clamp(1200² / d², 0, 1) · vis. For mode 3 the scale is also set to s·d/200 (s = 1 if 0).
4. blend = (B/255) · G, where B is this frame's entity blend (section 4).
5. V.rgb = ⌊rendercolor · blend⌋, truncated to an integer per channel. V.a stays br.
6. If blend ≤ 0, the sprite is not drawn.
7. Draw additive, no depth test: dst += T.rgb · V.rgb · T.a · V.a. Vertex colours are always used in modes 3 and 9; `$color`/`$alpha` are **not** used; fog is toward black.

Where vis enters: only through G, which multiplies **V.rgb only**. V.a is not touched by vis, by the distance fade or by B. So the occlusion and edge fades act once, linearly, on the added colour.

Consequences (with br = renderamt, as for every map-placed sprite):
- With renderfx 0, renderamt appears twice: once in V.rgb via B and once in V.a. The intensity scales with (renderamt/255)².
- With renderfx 14 it appears three times: B and G in V.rgb, br in V.a: (renderamt/255)³.
- After an `Alpha` input changes the render alpha to A: renderfx 14 intensity ∝ (A/255)·(br/255)·(br/255); renderfx 0 ∝ (A/255)·(br/255).
- The distance fade is 1 out to 1200 units and then falls as 1/d². At 2400 units it is 0.25.
- The engine is also told the global blend = blend (step 4). See Open question 11; the SDK does not apply it to the sprite.

#### 5a. Occlusion proxy and fading

- **When the visible fraction is undefined.** If the hardware cannot do occlusion queries, r_dopixelvisibility is 0 or cubemaps are being built:
  - vis = 1 if a line from the eye to P − view_forward·4 (only pulled back when d > 4) hits nothing opaque, else 0. The line is tested against world, brush entities and physics props, and also monsters/debris contents.
  - There is no fade in this case.
- **Proxy shape.** Per sprite and per view, a flat square proxy is drawn after the scene for that view:
  - It is parallel to the view plane, with half-diagonal p = GlowProxySize. That is half-side p·0.7071 along view right and p·0.7071 / aspect along view up, where aspect = sprite width/height (1 for square textures).
  - It is centred at P moved **toward the eye by p units** (the unenlarged p), along the unit direction from the eye to P (not along view forward), so a sprite sitting on a wall is not hidden by that wall. The pull is never enlarged with distance, so it cannot push the proxy through nearby geometry toward the camera.
  - If the proxy's projected size is under 5 pixels, p is enlarged so that p × (pixel diameter of a 1-unit sphere at P) = 5. Only the in-plane size is enlarged. The pixel-diameter value is floored at 0.0001 to avoid division by zero. *Derived:* the pixel-diameter helper lives in the material system, not the SDK (Open question 5).
  - Proxy inputs for an env_sprite: centre P (the draw position: the attachment point if the sprite follows one, else its origin), size p = networked GlowProxySize, aspect from the sprite's texel extents, fade time 0.0625 s.
  - Geometrically the proxy is four triangles meeting at the centre (a pyramid flattened into the plane); it covers the same square as a quad.
- **The two queries.** For each proxy:
  - "possible" draws the proxy with the depth range squeezed to [0, 0.01], so it effectively passes the depth test everywhere on screen;
  - "visible" draws it with the normal depth test against the finished opaque and translucent scene.
  - Neither query writes colour (*derived*: the proxy materials `engine/occlusionproxy*` are not in the SDK).
- **Clip fraction.** Computed when the "visible" query is issued (end of view), from the projected proxy corners in normalised device coordinates [−1, 1]: width from the two top corners, height from the top-left and bottom-right corners, and the same with each edge clamped to the screen. clip = (clamped area)/(full area), clamped to [0, 1]; 0 if the full area is 0.
  - If any proxy corner is behind the eye, no query is issued that frame and clip = 0. The faded value and the "issued" frame are left as they were.
  - The clip fraction from the "possible" query is discarded; only the "visible" query's is kept.
- **Reading results.** Results are read the **next** frame (one frame of latency). Each frame:
  1. target = visible/possible; if target ≥ 0.95 it becomes 1. If possible = 0, the faded value snaps to 0.
  2. faded = move faded toward target by at most 16 × frame_time. Starting from 0, a full fade-in takes 1/16 s ≈ 62.5 ms.
  3. vis = faded × clip_fraction, where clip_fraction is the one computed when last frame's queries were issued.
  - vis is evaluated once per view per frame; later calls in the same view return the same value.
  - If a result is not ready (the query reports "not available"), the previous faded × clip is returned unchanged.
  - visible/possible and clip_fraction multiply. They are independent: both pixel counts are limited to the screen, so the edge only acts through clip_fraction.
- **Edge cases.**
  - Before the first result arrives, vis = 0.
  - A sprite not drawn for more than one frame (left the PVS or frustum) loses its query state. It fades in again from 0 when it returns.
  - With r_pixelvisibility_partial 0 the target is 1 if any pixel is visible, else 0. The fade-in is then half as fast (8/s) and the fade-out 16/s. If the query was not issued on the immediately preceding frame, the faded value snaps to 0.
  - The fade step uses the client frame time (whatever the engine reports; not clamped by this code).
  - Each view (main, reflection, etc.) has its own query state.

### 6. Animation

- Frame count n comes from the texture. max_frame = n − 1.
- The sprite only animates if framerate ≠ 0 **and** max_frame > 1, so it needs at least 3 frames. Play-once sprites also start the animation think.
- Server, every tick: frame += framerate × tick_dt. If frame > max_frame:
  - play-once sprites hide;
  - otherwise frame = fmod(frame, max_frame), when max_frame > 0.
- The frame is networked: 20 bits over [0, 256), rounded down.
- The client draws texture frame ⌊frame⌋.
- Turning the sprite on resets frame to 0.
- de_dust2's `sprites/glow` has 1 frame, so framerate 10 has no effect.

### 7. func_dustmotes keyvalues → state

| Key | Meaning |
|---|---|
| model `*N` | Brush model. It defines the spawn volume. It is never drawn itself. |
| SpawnRate | Particles per second (integer, 12-bit network: 0..4095). |
| SizeMin, SizeMax | Screen-size factor. Half-width = Size/10000 × depth (see 9). Float. |
| SpeedMax | Integer, units/s, 12-bit. |
| LifetimeMin, LifetimeMax | Integers (atoi), seconds, **4-bit network: 0..15**. |
| DistMax | Integer, units of view depth, 16-bit. |
| Color | "r g b". Its alpha is replaced by Alpha. |
| Alpha | 0–255, integer. **If absent, alpha is 0 and the motes are invisible.** |
| Frozen | First character '1' → frozen mode (see 8). |
| StartDisabled | First character '1' → spawning off. Inputs TurnOn / TurnOff toggle it. |
| FallSpeed | Optional, float, units/s subtracted from initial z velocity. Default 0. Not set on de_dust2. |

- func_dustmotes always has the "screen-constant size" behaviour.
- The related func_dustcloud uses the same entity without it; there Size is a world half-width in units.

### 8. Dust spawning and simulation (client)

- **Spawn timing.**
  - An emitter with interval 1/SpawnRate fires once immediately and then every interval.
  - Each client frame it is fed Δ = min(frame_time, 0.1) and fires as many times as fit. Each firing makes one spawn attempt.
  - Time beyond 0.1 s in a slow frame is lost.
  - The emitter is reset (fires immediately again) whenever the entity receives a network update.
- **Spawn position.**
  - Up to 10 tries: pick p = mins + (maxs − mins)·(u1, u2, u3), with each u uniform in [0, 1] (independent per axis), inside the brush model's axis-aligned bounds.
  - Accept the first p that lies inside a brush of this brush model (point-contents solid).
  - If all 10 fail, nothing spawns for this attempt.
  - Motes therefore start **inside the brush volume**, not just its bounding box. For box-shaped brushes the first try always succeeds.
- **The cap.** A spawn also fails silently if the global particle cap (2048, shared with other classic client particle effects) is reached.
- **Initial state** of a spawned mote:
  - velocity v = (uniform[−S, S], uniform[−S, S], uniform[−S, S] − FallSpeed), where S = SpeedMax;
  - age = 0;
  - life T = uniform[LifetimeMin, LifetimeMax], a float;
  - half-size factor k = uniform[SizeMin, SizeMax] / 10000;
  - colour = Color with alpha = Alpha.
- **Per simulation step** (dt = frame time clamped to 0.1 s; skipped while frozen):
  1. age += dt. If age ≥ T, remove the mote.
  2. For x and y only, move v toward the wind velocity by at most 50 × frame_time.
     - The wind comes from the first env_wind on the map, or (0, 0, 0) if there is none. de_dust2 has none.
     - z velocity is never changed.
  3. position += v·dt.
  - There is no collision and no confinement: motes can drift out of the brush.
- On de_dust2 (no wind, SpeedMax 5):
  - horizontal speed drops to 0 within 0.1 s, a horizontal drift of at most 0.25 units;
  - motes then rise or sink at a constant random speed in [−5, 5] units/s, moving at most 25 units over a 5 s life.
- **Frozen mode.**
  - When the entity is created on the client, it makes SpawnRate spawn attempts at once and then never spawns again.
  - Frozen motes never age, never move and never die.
  - Their life alpha is fixed at 1.
- **Steady state** (not frozen, all spawns succeed): count ≈ SpawnRate × mean life.
  - de_dust2 `*6`: 300 × 4 = 1200.
  - de_dust2 `*7`: 50 × 4 = 200.
  - The total of 1400 is close to the 2048 cap.

### 9. Dust rendering

For each mote, with view-space position q (q.z < 0 in front of the camera; depth = −q.z):
1. Life alpha a_life = 0.5 − 0.5·cos(2π·age/T). It is 0 at birth and death, 1 at mid-life, and averages 0.5 over the life. Frozen motes use a_life = 1.
2. If depth > DistMax, the mote is not drawn. Otherwise a = a_life · (1 − depth/DistMax). There is no other edge falloff, so motes near the brush boundary are not faded.
3. Half-size h = k · depth, which makes the size constant on screen.
4. Vertex colour = (round r, round g, round b, round(a · Alpha)). If the vertex alpha is < 0.5 the mote is skipped.
5. Quad: in view space, the corners are (q.x ± h, q.y ± h, q.z). It is parallel to the view plane, side 2h, and textured with the whole `particle/sparkles` texture.

Rendering:
- Material `particle/sparkles`: UnlitGeneric, alpha blended (src·α + dst·(1−α)), not additive.
  - Output = texture × vertex colour, alpha = texture α × vertex α.
  - Colour-space handling follows the UnlitGeneric rules in shaders.md.
  - Depth tested; depth writes off (*derived*, see shaders.md Open question 8).
- Within the effect, motes are kept roughly back-to-front:
  - one bubble-sort pass per frame on depth;
  - plus, about one frame in 9 (random), a 32-bucket sort.
- The whole effect sorts against other translucents as one object, placed at the brush's world-space centre.
- The effect is culled by the brush's bounds grown by SizeMax on every side.

Screen size: the mote's side in pixels = 2·k·H / (2·tan(vfov/2)) = k·H / tan(vfov/2), where H is the viewport height in pixels and vfov the vertical FOV. It does not depend on distance.

## Per-frame order

Sprites:
1. Server tick: advance the animation frame; network frame, scale and brightness.
2. Client frame: compute the entity blend B (section 4) once per frame.
3. Translucent entity pass, back to front: modes 0, 1, 2, 4, 5, 7, 8. Skip if B = 0. Glow modes are skipped here.
4. The no-Z translucent pass: modes 3 and 9, in the same back-to-front order:
   - read vis (from last frame's queries, faded by this frame's time);
   - compute G, then V.rgb, then the scale;
   - draw.
5. End of view: for every sprite whose visibility was asked this frame, issue the "possible" and then the "visible" proxy queries.

Dust:
1. Client think: spawn attempts for this frame's (clamped) time.
2. Particle manager: simulate all motes (age, kill, velocity, move). This is skipped on the effect's first frame.
3. Translucent pass: compute alpha and size per mote, refine the sort, draw.

## Edge cases

- `parallel_upright` / `facing_upright` within 1° of straight up or down: the axes are not computed and the quad is drawn with stale or undefined axes. **Ours:** skip drawing that frame.
- A sprite whose model is not a sprite material: nothing drawn.
- renderamt 0, or a renderfx strobe reaching 0: not drawn in any mode except 0.
- The glow proxy behind the eye (any proxy corner behind the near plane): vis drops to 0 at once. The faded value is kept, so it fades back in quickly once visible again.
- A glow near the screen edge dims as its proxy leaves the screen, through clip_fraction. The two pixel counts are both limited to on-screen pixels, so visible/possible is not affected by the edge. The edge dimming is applied once, via clip_fraction.
- Glow sprites are not occluded by the depth buffer. A glow behind a thin object larger than the proxy is fully hidden; one behind a fence or grate is partly hidden in proportion to the proxy pixels covered.
- Dust with SpawnRate 0: the interval is infinite, so after the first immediate attempt nothing more spawns. *Derived:* we should emit nothing.
- LifetimeMin > LifetimeMax: life is uniform between them either way (the linear remap handles reversed ranges).
- DistMax 0: division by zero; all motes in front fail the depth ≤ 0 test. **Ours:** draw nothing.

## Quirks

1. **renderamt counted two or three times in glow modes** (section 5). Keep: it is what makes de_dust2's lamp glows look the way they do.
2. **renderfx pulses and strobes do not change non-glow sprites' alpha.** They only switch them fully off when B hits 0. This is because vertex alpha uses the brightness, not B. Keep.
3. **The last animation frame is almost never shown.** Wrapping is by fmod(frame, n−1) as soon as frame exceeds n−1, so frame n−1 only shows when frame equals it exactly. 2-frame sprites never animate. Keep for fidelity.
4. **Mode 7 frame blending** receives an integer frame and so never blends. Keep: it is rare.
5. **`facing_upright` faces the world origin**, not the camera: it uses the sprite's absolute position as if it were camera-relative. It is rare in CS:S maps. Recommend implementing it as written, behind a flag, and confirming in game if a map uses it.
6. **Hammer yaw becomes roll**, so a rotated env_sprite with a `vp_parallel` material spins its image in the view plane by that yaw. Keep.
7. **Scale below 0.25 is impossible on the client** (quantisation). Keep.
8. **Dust horizontal velocity uses the frame time, not the clamped simulation step.** This only differs on frames longer than 0.1 s. Keep.

## Test cases

Sprite tests use `sprites/glow` (128×128, 1 frame, `vp_parallel`, origin [0.5 0.5]) unless stated.

| # | Setup | Input | Expected |
|---|---|---|---|
| S1 | scale "1" | spawn | quad 128 × 128 units centred on origin; corners at ±64 along view right/up |
| S2 | scale "0.8" | spawn | client scale 0.75 → 96 × 96 units |
| S3 | scale "0.1" | spawn | client scale 0.25 → 32 × 32 units |
| S4 | scale "100" | spawn | clamped to 64 → 8192 × 8192 units |
| S5 | angles "0 90 0" on a vp_parallel sprite | spawn | roll becomes 90: R = view_up, U = −view_right (image rotated 90° in the view plane) |
| S6 | de_dust2 mode 5: rendercolor 254 248 211, renderamt 25, renderfx 14, scale 3, `sprites/glow` (centre texel ≈ 0.69 grey, alpha 1) | draw, no fog | 384 × 384 units, depth tested. Added at the centre: 0.69 × (254, 248, 211)/255 × 25/255 = (0.0674, 0.0658, 0.0560) ≈ (+17, +17, +14)/255. For a texel T = (1, 1, 1, 1): (0.0977, 0.0954, 0.0811). Unaffected by renderfx (same result with renderfx 0), distance and occlusion. Not (0.69, 0.69, 0.69): that would be the vertex-colours-off reading, ruled out by the live measurement. |
| S7 | de_dust2 mode 9, renderfx 14, renderamt 254, rendercolor 254 248 211, scale 0.8, vis = 1 | draw | B = 254. G = 254/255 = 0.996078. blend = 0.992172. V.rgb = (252, 246, 209), V.a = 254. Size 96 units at any distance. Added colour for T = (1, 1, 1, 1): (252/255·254/255, 246/255·254/255, 209/255·254/255) = (0.9844, 0.9609, 0.8164). No depth test. |
| S8 | same as S7 but renderamt 200, scale 1 | draw, vis = 1 | blend = (200/255)² = 0.615148. V.rgb = (156, 152, 129), V.a = 200. Added for T = 1: (0.4798, 0.4675, 0.3967). Size 128 units. |
| S8a | S7's sprite (renderamt 254 at spawn), then input `Alpha 128` | draw, vis = 1 | B = 128, brightness still 254. G = 254/255. blend = 128·254/255² = 0.499992. V.rgb = (126, 123, 105), V.a = 254. Added for T = 1: (0.4922, 0.4805, 0.4102). |
| S8b | S6's mode 5 sprite, then input `Alpha 255` (or `Alpha 1`) | draw | identical to S6 (B only gates). `Alpha 0`: not drawn. |
| S8c | mode 3, renderfx 14, scale 1, renderamt 255, vis = 1 | eye at 400 and at 2400 units | scale stays 1 (128 units, no growth with distance) and G = 1 at both distances (no distance fade). |
| S8d | mode 5 sprite, `$alpha` absent, `$color` absent | draw | C = (1, 1, 1, 1): no effect on the result. |
| S9 | mode 9, renderfx 0, renderamt 255, colour 255 255 255, vis 1 | eye at 600 / 1200 / 2400 units | G = 1 / 1 / 0.25. V.rgb = 255 / 255 / 63 (⌊63.75⌋). Size unchanged. |
| S10 | mode 3, renderfx 0, scale 1, renderamt 255 | eye at 400 units, not zoomed | scale → 1 × 400/200 = 2 → 256 units. At 800 units, 4 → 512 units, so the projected size is the same. |
| S11 | mode 3 as S10 while zoomed to FOV 40 (default 90) | eye at 400 units | d = 400 × 40/90 = 177.8. scale 0.889 → 113.8 units. Distance fade 1 (177.8 < 1200). |
| S12 | glow, proxy fully visible, faded starts at 0, frame time 1/100 s | 1st, 2nd, 7th frame after the first query result | faded = 0.16, 0.32, 1.0 (reaches 1 on frame 7, since 6 × 0.16 = 0.96). Before the first result, vis = 0. |
| S13 | glow, half of proxy pixels covered (visible/possible = 0.5), faded = 1, frame time 1/60 s | 1 frame | faded = 1 − 16/60 = 0.7333; next frame 0.5 and stays there |
| S14 | glow, visible/possible = 0.96 | steady | target 1, so vis = 1 × clip_fraction |
| S15 | glow, proxy's screen rectangle 40% outside the viewport, all visible pixels unoccluded | steady | vis = 0.6 |
| S16 | GlowProxySize "2.0", eye 100 units in front, sprite 1 unit in front of a wall | proxy | proxy centre 2 units toward the eye from P (3 units off the wall), half-side 1.414 units (if ≥ 5 px). Unoccluded, so vis → 1. |
| S17 | 4-frame animated sprite, framerate 10, server tick 1/64 s | after 0.29 s | frame = 2.9 → frame 2 shown. Next tick: 3.056 > 3 → fmod → 0.056 → frame 0. Frame 3 is never shown. |
| S18 | 2-frame sprite, framerate 10 | any time | frame 0 forever (max_frame = 1, not > 1) |
| D1 | dust: T = 4 s | age 0, 1, 2, 3, 4 | a_life = 0, 0.5, 1, 0.5, (removed) |
| D2 | dust: a_life 0.5, depth 256, DistMax 512, Alpha 255 | render | a = 0.25. Vertex alpha = round(63.75) = 64. |
| D3 | dust: depth 600, DistMax 512 | render | not drawn |
| D4 | dust: Size 15, depth 256 | render | half-size 0.384 units, side 0.768 units. At depth 512: side 1.536 units. |
| D5 | dust: Size 15, viewport height 1080, vertical FOV 73.74° (tan half = 0.75) | render | side = 0.0015 × 1080 / 0.75 = 2.16 pixels, at any depth |
| D6 | dust: wind 0, initial v = (5, −3, 2), frame time 0.01 s | after 5 frames | vx = 5 − 5 × 0.5 = 2.5, vy = −3 + 2.5 = −0.5, vz = 2. After 10 frames: vx = 0, vy = 0 (clamped), vz = 2. |
| D7 | dust: SpawnRate 300, steady 1/60 s frames | 1 s | 300 spawn attempts (first at t = 0). With a single 0.5 s hitch frame, only 0.1 s counts: 30 or 31 attempts that frame, depending on the leftover time to the next event. Never 150. |
| D8 | dust: SpawnRate 300, LifetimeMin 3, LifetimeMax 5, box brush | after 10 s | about 1200 live motes; brightness averages half of peak |
| D9 | dust: Frozen "1", SpawnRate 50 | create entity | exactly 50 spawn attempts at creation; motes static, a_life = 1, never removed. Alpha still fades with depth. |
| D10 | dust: no Alpha key | render | all vertex alphas 0 → nothing drawn |
| D11 | dust: LifetimeMax "20" | network | 4-bit field: 20 mod 16 = 4 (*derived*: the encoder masks to the bit count). Treat values > 15 as 15 or mod 16, see Open question 6. |

## Open questions

1. **Scale quantisation rounding.** The engine's float encoder is not in the SDK, so truncation versus nearest is unknown. It matters for values like 0.9: 0.75 (truncate) or 1.0 (nearest). Check: place an env_sprite at scale 0.9 in a test map and measure its on-screen size against scale 0.75 and 1.
2. **Undefined `$spriteorigin`.** The loader centres the quad when the material's `$spriteorigin` is absent or not a vector. If the material system filled the declared default `[0 0 0]` in as a vector, absent-origin sprites would anchor at the top-left corner instead. All CS:S sprite vmts we use set it, so this does not matter for de_dust2.
3. **Absent `$ignorevertexcolors` and `$color` in mode 5.** The SDK cannot settle this: declared defaults (1 and, for `$color`, presumably white) are exposed to the material system, whose handling is not public. The sprite shader itself fills in `$alpha`, `$hdrcolorscale` and `$nosrgb` but neither of these.
   - Evidence: the live CS:S measurement shows de_dust2's mode 5 halos (renderamt 25) are effectively invisible. "Vertex colours off, `$color` white" would add ≈ +176/255 at the centre: ruled out.
   - Remaining readings: (a) vertex colours on, `$color` white → +17/255 peak (S6), the spec's choice; (b) `$color` absent reads as black → mode 5 sprites without `$color` are never visible. (b) would make every such additive sprite in every Source game invisible, which seems unlikely, but nothing in the SDK excludes it.
   - Check: measure the screen difference at the centre of the halo at (−1406, 1148, 168) from a close, unfogged position with HDR off (`mat_hdr_level 0`), against the same view with `r_drawsprites 0`. About +17/255 (warm tint, less in blue) means (a). 0 means (b), or Open question 11. Alternatively spawn a mode 5 sprite with renderamt 255 and rendercolor 255 255 255: (a) adds ≈ +176/255 at its centre, (b) adds 0.
4. **Model path with `materials/` prefix.** We assume the engine strips it. Confirm by checking that de_dust2's glows render (they do in game).
5. **Pixel diameter helper** for the 5-pixel minimum proxy: assumed to be the projected diameter, in pixels, of a 1-unit-radius sphere at P, i.e. ≈ 2·H / (2·depth·tan(vfov/2)) = H/(depth·tan(vfov/2)). This is material-system code, not in the SDK.
6. **Network bit masking** of out-of-range dust values (Lifetime > 15, SpawnRate > 4095): the engine's integer encoder is not in the SDK.
7. **Random number range on Linux.** The dust spawn position, life and size use the C library random function scaled by 1/32767. On Windows (MSVC) that function's maximum is 32767, which gives the uniform [0, 1] behaviour described. On a Linux client with a larger maximum the positions would mostly fall outside the brush and the lives would be enormous, unless the engine replaces that function. Check: compare a mote count or screenshot of de_dust2's B-site dust between the Windows and Linux clients. Implement the Windows behaviour.
8. **Proxy material state** (colour writes off, depth writes off, depth test on) is assumed. Those materials are engine content.
9. **Brush-model contents for dust.** The spawn test needs the point to be "solid" in the brush model. We assume any point inside one of the model's brushes counts, whatever its tool texture.
10. **Fog on additive sprites:** modes 3, 5, 7 and 9 fog toward black: the added colour is multiplied by (1 − fog factor). de_dust2's env_fog_controller is enabled: fogstart 500, fogend 4000, fogmaxdensity 1, colour 197 196 165. *Derived:* with linear fog the factor is clamp((dist − 500)/3500, 0, 1), so a mode 5 halo seen from 1000 units keeps 86% of S6 (≈ +15/255 peak) and one at 2000 units 57%. The fog formula (range or depth, per-vertex or per-pixel) is engine-side and not confirmed.
11. **Engine global blend.** Before each translucent entity the client hands the engine a global blend (B/255; for glow modes, B/255 × G). The SDK's sprite path never passes it into the material or vertices, so this spec assumes it has no effect on sprites. If the engine applied it anyway (for example as an alpha or `$alpha` modulation of the bound material), every non-glow sprite would gain a factor B/255 (mode 5 at renderamt 25 would peak at ≈ +1.7/255) and glow sprites a further (B/255)·G.
   - The live `ent_fire <sprite> alpha 255` test saw no change in the mode 5 halos. Under reading 3(a) with the engine applying B, they would have brightened about tenfold (to ≈ +17/255), so that combination is unlikely. The observation is consistent with 3(a) without engine blend (≈ +17/255 before and after) and with 3(b) (0 before and after); Open question 3's check separates those.
12. **HDR.** The spec assumes LDR. With HDR on, the shader also multiplies the colour by HDRColorScale (1 on de_dust2) and by the tone-map light scale, which the engine sets from auto-exposure and which can be well below 1 on a bright map. Live measurements of sprite brightness must be taken with `mat_hdr_level 0` or they will read darker than this spec.
