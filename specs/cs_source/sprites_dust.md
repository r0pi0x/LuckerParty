# Counter-Strike: Source: sprites (env_sprite) and dust motes (func_dustmotes)

Source basis: Valve's public Source SDK 2013 (current GitHub head). I read:
- the shared sprite entity (server spawn, keyvalues, network fields and their quantisation, animation) and its client drawing code;
- the client's sprite-model loader (how a sprite model becomes materials, size and quad extents);
- the client pixel-visibility (occlusion query) system and the glow sight-distance helper;
- the client entity render-effect blend code (renderfx) and the translucent-entity draw loop;
- the DX9 Sprite shader (C++ set-up plus its vertex and pixel shaders) and the shared final-output include;
- the server and client dust entity code, the client particle manager (allocation cap, sort, time step), the particle quad helper and the wind helper.

CS:S's own game DLL source is not public. This spec assumes CS:S uses the same shared sprite and dust code, which ships in every Source multiplayer game of that era. Several layers are **not in the SDK**: the engine's model loader (how a `model` keyvalue path reaches the sprite loader), the engine's network float encoder (rounding of quantised fields), the material system (shader-parameter defaults, the pixel-diameter helper, occlusion-query hardware details) and the occlusion-proxy materials. Rules that depend on them are marked *derived* or listed under Open questions.

I checked the map data directly from the user's CS:S install (de_dust2 entity lump, `materials/sprites/glow.vmt`, `materials/particle/sparkles.vmt`, texture headers). No game files are committed.
Status: draft

## Summary

- An env_sprite is one textured quad, two-sided. Its size is the base texture's pixel size times `scale`, in world units: a 128×128 texture at scale 1 is 128×128 units. Its orientation comes from the **material** (`$spriteorientation`), not the entity. `vp_parallel` (used by all of de_dust2's glows) means the quad is parallel to the view plane, using the camera's own right/up vectors. It does not turn toward the camera position.
- Render mode picks the blend:
  - 5 (Additive) is "src·α + dst", depth-tested.
  - 3 (Glow) and 9 (World Space Glow) are also additive but **ignore the depth buffer**. They draw after all other translucents and are hidden instead by a hardware occlusion query on a small proxy quad (GlowProxySize units). Visibility fades at 16 per second (full fade in 1/16 s).
  - Glow modes also fade with distance (1200²/d²), unless renderfx is 14 (Constant Glow).
  - Mode 3 additionally grows with distance, so it keeps a fixed size on screen.
- renderamt is applied **more than once** in the glow modes. With de_dust2's renderfx 14 it effectively scales the light by (renderamt/255)³.
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
| renderamt | "Brightness": vertex alpha, and the entity blend for every mode except 0. |
| GlowProxySize | Size of the occlusion proxy, units; networked as integer 1..64 (1b). Default 2. |
| framerate | Animation frames per second (server side). |
| HDRColorScale | Multiplies RGB **only when HDR is enabled**. No effect in LDR. Default 1. |
| spawnflags 1 (Start on) | Only matters for a sprite with a targetname. A named sprite without this flag starts hidden. Unnamed sprites always start visible. |
| spawnflags 2 (Play once) | When the animation passes its last frame, the sprite hides itself. |
| angles | **Hammer quirk:** if yaw ≠ 0 and roll = 0, yaw is moved into roll (roll = yaw, yaw = 0). Roll matters for `vp_parallel` materials (section 2). |
| inputs | ShowSprite / HideSprite / ToggleSprite, SetScale, ColorRedValue/Green/Blue (0–255, clamped). |

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
- C = the material's `$color`/`$alpha` constant (1 unless the vmt sets them).
- The sprite shader by default does **no** sRGB conversion (`$nosrgb` defaults to on). All maths is in gamma space, on raw texture values, as LDR Source always did for sprites.
- Output is clamped to [0, 1] per channel. There is no overbright factor; the tone-map scale is 1 in LDR.
- Vertex values for one sprite in one frame:
  - V.a = renderamt (the brightness; see section 4 for transitions);
  - V.rgb = rendercolor, except in modes 3 and 9 (section 5).

| Mode (Hammer name) | Shader colour | Blend | Depth test / write | Fog colour | Draw pass |
|---|---|---|---|---|---|
| 0 Normal | T (vertex colour ignored) | none (opaque square, alpha ignored) | test on / write on | fog colour | translucent entity pass |
| 1 Color, 2 Texture, 4 Solid | T·V | src·α + dst·(1−α) | on / off | fog colour | translucent pass |
| 3 Glow, 9 World Space Glow | T·V | src·α + dst (additive) | **off** / off | black | after all other translucents ("no-Z" pass) |
| 5 Additive | T·C, ×V only if the vmt's `$ignorevertexcolors` is 0 (see note) | src·α + dst | on / off | black | translucent pass |
| 7 Additive Fractional Frame | two additive passes: frame ⌊f⌋ weighted (1 − frac f)·$alpha, then frame ⌊f⌋+1 weighted frac f·$alpha | src·α + dst | on / off | black | translucent pass |
| 8 Alpha Add | pass 1: alpha blend; pass 2: src·(1−α) + dst | as stated | on / off | fog colour, then black | translucent pass |
| 6 Environmental, 10 Don't Render | not drawn | | | | |

Mode 5 and vertex colours:
- `$ignorevertexcolors` is declared with a default of 1. The shader's own set-up code only fills in defaults for other parameters. *Derived (likely):* an undefined `$ignorevertexcolors` reads as 0. Under that reading:
  - mode 5 output = T.rgb·V.rgb·C.rgb, weighted by T.a·V.a·C.a;
  - so rendercolor and renderamt **do** apply, once each, with no distance fade or occlusion fade.
- If instead the default 1 applies, mode 5 ignores rendercolor and renderamt entirely. See Open question 3. Test case S6 distinguishes the two.

Other mode notes:
- Mode 7: the frame is passed to the material as an integer, so the fractional blend effectively always shows a whole frame (Quirk 4).
- The entity blend (section 4) is not applied to vertex alpha in modes 1, 2, 4, 5, 7 and 8. It only decides whether the sprite is drawn at all (blend ≤ 0 → skipped).
- Within a pass, translucent sprites are drawn back-to-front with the other translucent entities.

### 4. Entity blend (renderamt and renderfx)

- Each frame the client computes an entity blend B ∈ 0..255:
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
- If B/255 ≤ 0 the sprite is not drawn.
- Sprites have a separate "brightness" (renderamt) and "scale". Each can be set to change over a networked time. The client then interpolates linearly from the old to the new value over that time. Map-placed sprites never use this, so V.a = renderamt.

### 5. Glow modes (3 and 9): visibility, distance fade, colour

Inputs:
- P = sprite position;
- E = eye position;
- d = \|P − E\| × zoom_factor. zoom_factor = current FOV / default FOV, so it is 1 unless zoomed; for example a 40° scope at default 90° gives 0.444.

Steps:
1. vis = visible fraction from the occlusion system (5a). If vis ≤ 0, the sprite is not drawn.
2. If d ≤ 0, it is not drawn.
3. Glow factor G:
   - renderfx 14 (Constant Glow): G = (renderamt/255) · vis. There is no distance fade.
   - otherwise: G = min(1, 1200² / d²) · vis. For mode 3 the scale is also set to s·d/200 (s = 1 if 0).
4. blend = (B/255) · G.
5. V.rgb = ⌊rendercolor · blend⌋, truncated to an integer per channel. V.a stays renderamt.
6. If blend ≤ 0, the sprite is not drawn.
7. Draw additive, no depth test: dst += T.rgb · V.rgb · T.a · V.a.

Consequences:
- With renderfx 0, renderamt appears twice: once in V.rgb via B and once in V.a. The intensity scales with (renderamt/255)².
- With renderfx 14 it appears three times: (renderamt/255)³.
- The distance fade is 1 out to 1200 units and then falls as 1/d². At 2400 units it is 0.25.

#### 5a. Occlusion proxy and fading

- **When the visible fraction is undefined.** If the hardware cannot do occlusion queries, r_dopixelvisibility is 0 or cubemaps are being built:
  - vis = 1 if a line from the eye to P − view_forward·4 (only pulled back when d > 4) hits nothing opaque, else 0. The line is tested against world, brush entities and physics props, and also monsters/debris contents.
  - There is no fade in this case.
- **Proxy shape.** Per sprite and per view, a flat square proxy is drawn after the scene for that view:
  - It is parallel to the view plane, with half-diagonal p = GlowProxySize. That is half-side p·0.7071 along view right and p·0.7071 / aspect along view up, where aspect = sprite width/height (1 for square textures).
  - It is centred at P moved **toward the eye by p units** (the unenlarged p), so a sprite sitting on a wall is not hidden by that wall.
  - If the proxy's projected size is under 5 pixels, p is enlarged so that p × (pixel diameter of a 1-unit sphere at P) = 5. *Derived:* this helper lives in the material system, not the SDK.
- **The two queries.** For each proxy:
  - "possible" draws the proxy with the depth range squeezed to [0, 0.01], so it effectively passes the depth test everywhere on screen;
  - "visible" draws it with the normal depth test against the finished opaque and translucent scene.
  - Neither query writes colour (*derived*: the proxy materials `engine/occlusionproxy*` are not in the SDK).
- **Clip fraction.** The fraction of the proxy's screen rectangle that lies inside the viewport, clamped to [0, 1]. It is 0 if any proxy corner is behind the eye.
- **Reading results.** Results are read the **next** frame (one frame of latency). Each frame:
  1. target = visible/possible; if target ≥ 0.95 it becomes 1. If possible = 0, the faded value snaps to 0.
  2. faded = move faded toward target by at most 16 × frame_time. Starting from 0, a full fade-in takes 1/16 s ≈ 62.5 ms.
  3. vis = faded × clip_fraction.
- **Edge cases.**
  - Before the first result arrives, vis = 0.
  - A sprite not drawn for more than one frame (left the PVS or frustum) loses its query state. It fades in again from 0 when it returns.
  - With r_pixelvisibility_partial 0 the target is 1 if any pixel is visible, else 0. The fade-in is then half as fast (8/s) and the fade-out 16/s.
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
2. **renderfx pulses and strobes do not change non-glow sprites' alpha.** They only switch them fully off when B hits 0. This is because vertex alpha uses renderamt, not B. Keep.
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
| S6 | de_dust2 mode 5: rendercolor 254 248 211, renderamt 25, scale 3 | draw, texel T = (1, 1, 1, 1) | 384 × 384 units, depth tested. Expected dst increase (vertex colours used): (254/255·25/255, 248/255·25/255, 211/255·25/255) = (0.0977, 0.0954, 0.0811). If $ignorevertexcolors defaults to 1: (1, 1, 1). Live check decides (Open question 3). |
| S7 | de_dust2 mode 9, renderfx 14, renderamt 254, rendercolor 254 248 211, scale 0.8, vis = 1 | draw | B = 254. G = 254/255 = 0.996078. blend = 0.992172. V.rgb = (252, 246, 209), V.a = 254. Size 96 units at any distance. Added colour for T = (1, 1, 1, 1): (252/255·254/255, 246/255·254/255, 209/255·254/255) = (0.9844, 0.9609, 0.8164). No depth test. |
| S8 | same as S7 but renderamt 200, scale 1 | draw, vis = 1 | blend = (200/255)² = 0.615148. V.rgb = (156, 152, 129), V.a = 200. Added for T = 1: (0.4798, 0.4675, 0.3967). Size 128 units. |
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
2. **Undefined shader parameters.** Does an undefined `$spriteorigin` or `$ignorevertexcolors` read as the shader's declared default ([0 0 0] and 1) or as zero/undefined?
   - The shader's own init code fills in defaults for `$alpha`, `$hdrcolorscale` and `$nosrgb` by hand, which suggests declared defaults are *not* applied automatically.
   - Under that reading, sprites without `$spriteorigin` are centred and mode 5 uses vertex colour.
   - Check: screenshot de_dust2's mode 5 halo at (−1406, 1148, 168) in CS:S and compare with S6. A full-white 384-unit square-ish glow means the default is applied; a faint warm halo means it is not.
3. Same as 2, for mode 5 specifically. It matters a lot for de_dust2's four mode 5 halos.
4. **Model path with `materials/` prefix.** We assume the engine strips it. Confirm by checking that de_dust2's glows render (they do in game).
5. **Pixel diameter helper** for the 5-pixel minimum proxy: assumed to be the projected diameter, in pixels, of a 1-unit-radius sphere at P, i.e. ≈ 2·H / (2·depth·tan(vfov/2)) = H/(depth·tan(vfov/2)). This is material-system code, not in the SDK.
6. **Network bit masking** of out-of-range dust values (Lifetime > 15, SpawnRate > 4095): the engine's integer encoder is not in the SDK.
7. **Random number range on Linux.** The dust spawn position, life and size use the C library random function scaled by 1/32767. On Windows (MSVC) that function's maximum is 32767, which gives the uniform [0, 1] behaviour described. On a Linux client with a larger maximum the positions would mostly fall outside the brush and the lives would be enormous, unless the engine replaces that function. Check: compare a mote count or screenshot of de_dust2's B-site dust between the Windows and Linux clients. Implement the Windows behaviour.
8. **Proxy material state** (colour writes off, depth writes off, depth test on) is assumed. Those materials are engine content.
9. **Brush-model contents for dust.** The spawn test needs the point to be "solid" in the brush model. We assume any point inside one of the model's brushes counts, whatever its tool texture.
10. **Fog on additive sprites:** modes 3, 5, 7 and 9 fog toward black, which fades them out in fog. Not checked against de_dust2's fog controller settings.
