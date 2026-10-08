# Source engine: visual map entities (spotlights, beams, lasers, trails, glows, steam, sparks)

Source basis: Valve's public Source SDK 2013 only (the current GitHub
release, which also carries TF2 and Portal code; those parts are ignored).
I read the shared and server code for point_spotlight and its hidden end
entity, the beam entity base (keyvalues, inputs, damage, sparks, network
fields and their ranges), env_laser, env_beam, env_spritetrail (shared
server/client), env_lightglow (server and client) and the client glow
overlay it uses, env_steam (server and client particle code), env_spark
and the client electric-spark effect it triggers, the temp-entity beam and
glow-sprite messages, and the client beam renderer (segment building,
noise, shading, halo, per-frame update of temporary beams). The low-level
segment strip drawer and the engine's dynamic-light falloff live in
engine/tier2 libraries that are not in the SDK; where a rule depends on
them it is marked *inferred* and listed under Open questions. CS:S's own
client and server DLLs are not public; everything here is shared code that
CS:S compiles in, but CS:S could override parts (Open questions).
Status: draft

## Summary

These entities are pure visuals except for the damage of env_laser and
env_beam. They share one drawing primitive, the **beam**: a camera-facing
strip of quads along a line from a sprite material, drawn additively, with
optional width taper, shading toward one end, fractal or sine noise that is
regenerated every frame, and texture scrolling. point_spotlight is a beam
that fades to nothing along its length plus a glow halo at its source and
(only for named or parented spotlights) a dynamic light where it hits.
env_laser is a beam re-aimed every tick at a target that hurts what it
hits. env_beam is either a permanent beam between two entities or a
"lightning" generator that strikes short-lived temporary beams. The rest
are client effects: env_spritetrail (a ribbon following a moving entity),
env_lightglow (a screen-space glow that fades with distance and
occlusion), env_steam (a particle jet) and env_spark (bursts of spark
streaks at random intervals).

## Units and conventions

Source units (inches), Z up; angles (pitch, yaw, roll) in degrees, pitch
positive looking down; forward(angles) is the usual Source forward vector
(yaw 0 = +X, pitch −90 = +Z). Server tick dt = 0.015 s (66.67 ticks/s);
"next think at now" means the next tick. Client effects run per rendered
frame with frame time Δt. Colours: keyvalue "rendercolor" is "R G B" in
0–255, "renderamt" 0–255. U(a, b) is a uniform random float, I(a, b) a
uniform random integer (both ends included). Keyvalue, input and output
names are exactly what Hammer shows; flag values are spawnflags bits.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| beam_max_width | 102.3 | units | width and end width are clamped to this (also the network range) |
| beam_max_noise | 64 | — | noise amplitude clamp (network range 0–64, 8 bits) |
| beam_max_scroll | 100 | — | entity-beam scroll speed clamp (env_beam) |
| noise_divisions | 128 | — | noise samples along a beam; also the segment cap |
| beam_min_draw_length | 0.1 | units | shorter beams are not drawn |
| beam_damage_period | 0.1 | s | env_laser/env_beam continuous damage needs this much time since the last hit (→ 7 ticks) |
| spot_default_length | 500 | units | SpotlightLength ≤ 0 becomes this |
| spot_default_width | 10 | units | SpotlightWidth ≤ 0 becomes this |
| spot_brightness | 64 | 0–255 | spotlight beam brightness (fixed) |
| spot_halo_scale | 60 | — | spotlight halo size factor |
| spot_light_scale | 1.8 | — | dynamic light scale per unit of beam end width |
| spot_dlight_radius_mul | 3 | — | dlight radius = 3 × light scale |
| spot_dlight_z | 5 | units | dlight sits this far above the beam end |
| spot_think | 0.1 | s | think period of a non-parented, non-static spotlight (parented: every tick) |
| trail_max_points | 256 | — | env_spritetrail history length |
| trail_min_step | 2 | units | a new trail point needs the entity to have moved > 2 units (squared distance > 4) |
| glow_overlay_dist | 100 | units | lightglow quad distance in front of the eye |
| glow_material | sprites/light_glow02_add_noz | — | env_lightglow material |
| glow_pvs_recheck | U(1, 3) | s | env_lightglow re-tests "in PVS" this often (first test after U(0, 3)) |
| steam_ramps | 5 | — | steam lighting samples along the jet |
| spark_think | 0.1 + U(0, MaxDelay) | s | env_spark period |
| spark_glow | sprites/glow01, life 0.2, scale 1.5, brightness 25 | — | env_spark "Glow" flag temp sprite |

Defaults the client uses for env_steam (and Hammer's defaults): SpreadSpeed
15, Speed 120, StartSize 10, EndSize 25, Rate 26, JetLength 80, RollSpeed
8 (server default for RollSpeed is also 8; the other server defaults are 0,
so a map that omits a key gets 0, Edge cases).

## Behavior

### 1. The beam primitive (shared by all beams below)

State of one beam: start point S, end point E (each either a fixed point or
an entity's origin, re-read every frame), sprite material (a `.vmt` sprite;
a name without extension gets `.vmt`), start frame and frame rate, colour
(r, g, b) 0–255, brightness 0–255, width w, end width w_e, noise amplitude
A (0–64), scroll speed s, fade length F (0 = whole length), flags (shade
in, shade out, sine noise, no tile, solid), optional halo sprite and halo
scale.

Per rendered frame:

1. Delta D = E − S, length L = |D|. If L < 0.1: draw nothing.
2. **Render mode**: additive (src + dst) unless the "solid" flag is set
   (no map entity sets it). Colour per vertex =
   (r, g, b)/255 × brightness/255 × shade. The entity's rendermode keyvalue
   does not change this.
3. **Animation frame** = int(start frame + now × frame rate) mod frame
   count.
4. **Segments**: N = int(L × 0.25 + 3) if A ≥ 0.5, else int(L × 0.075 + 3).
   Then, with h = max(w, w_e)/2: if L/(N−1) < 1.414·h, N =
   int(L/(1.414·h)) + 1 (at least 2). Then N = min(N, 128). With sine
   noise N is raised to at least 16.
5. **Noise** (A ≠ 0): an array of 129 samples, first and last 0.
   - Fractal (default): midpoint displacement: the middle sample of a span
     is the mean of its ends plus U(−1, 1)·k, where k starts at 1 for the
     whole span and halves at each subdivision. Regenerated **every frame**
     (the beam crackles at the frame rate).
   - Sine (flag): sample i = sin(π·i/128).
   Segment j (fraction f = j/(N−1)) reads sample int(127·f) (fixed-point
   step). Displacement magnitude = sample × A × L/100 for fractal noise,
   along the direction perpendicular to both the beam and the view
   direction (normalize(view_forward × D)). For sine noise the displacement
   is sample × 100·A, split as sin(f·π·N/10 + q) along the view up axis
   and cos(...) along the view right axis, where q is the beam's
   "frequency" accumulator below.
6. **Shading** (b = 1 by default), with ff = clamp(F/L, 10⁻⁶, 1) (F = 0
   means F = L):
   - shade out only: b = 1 − f/ff;
   - shade in only: b = f/ff;
   - both: b = 2·f/ff for f < 0.5, else 2·(1 − f/ff);
   clamped to [0, 1].
7. **Width** at f: w + f·(w_e − w). The strip's full width on screen is
   **2 × that value** (*inferred*: the renderer passes twice the width to
   the strip drawer, which is not public; Open questions). The strip faces
   the camera (each segment widened perpendicular to the view).
8. **Texture**: v coordinate starts at v₀ = frac(q·s) and advances per
   segment by 1/(N−1) with "no tile", else by (L/100)/(N−1) (one texture
   repeat per 100 units). u spans the width.
   q for map-entity beams = now·s + Δt (rebuilt every frame), so the
   offset is frac((now·s + Δt)·s): the effective scroll rate is s² texture
   lengths per second (a quirk, kept). For temporary beams q starts at
   creation_time·s' and grows by Δt per frame, with s' = 0.1 × the sent
   speed.

Temporary ("temp entity") beams sent by the server (env_beam strikes):
fields are quantised on the way (life 0–25.6 s in 0.1 s steps, width and
end width 0–128 in 10 bits, amplitude 0–64 in 8 bits, speed and frame rate
integers, the client multiplies both by 0.1). They are drawn until
creation + life, at constant brightness (no fade unless a fade flag is
sent; env_beam sends none). Life 0 means "forever" (Edge cases). A temp
beam whose end is an entity with no model on the client is not created
when life ≠ 0; a temp beam attached to an entity that disappears is
dropped.

### 2. point_spotlight

Keyvalues: "SpotlightLength" L (≤ 0 → 500), "SpotlightWidth" W (≤ 0 → 10,
> 102.3 → 102.3), "rendercolor", "IgnoreSolid" (0/1), "HDRColorScale"
(default 1), "mindxlevel" (ignore). Spawnflags: 1 "Start On", 2 "No
Dynamic Light". Inputs **LightOn**, **LightOff**. Outputs **OnLightOn**,
**OnLightOff** (activator = caller = the spotlight).

**Static vs dynamic.** At activation (map load) a spotlight with no parent
is *static*: if it is on (flag 1) it builds its beam once and never updates
it. If it is static **and has no targetname**, the spotlight entity and its
end marker are deleted right after building, leaving only the beam: such a
spotlight can never be turned off and has **no dynamic light**. A
parented spotlight is *dynamic* and re-aims itself every tick (Behavior
2.4). Most map spotlights are static.

**2.1 Building (static).** Let O = origin, d = forward(angles).
1. Beam end B1 = the first hit of a line trace O → O + d·L against world
   brushes and solid brush entities only (no props, players or models);
   with IgnoreSolid, B1 = O + d·L.
2. End marker position B2 = same trace but to O + d·2L.
3. Beam: points O → B1, sprite `sprites/glow_test02.vmt`, colour =
   rendercolor, brightness 64, width W, flags shade-out + no-tile, noise 0,
   halo sprite `sprites/light_glow03.vmt`, halo scale 60.
4. Current length C = |B2 − O|, then the render values:
   - if C > 2L: marker alpha 0, fade length F = L;
   - else if C > L: marker alpha 1 − (C − L)/L **stored as a byte**, which
     truncates to 0 for every C in (L, 2L) (quirk), F = L;
   - else marker alpha 1 (the byte 1), F = C.
   - End width w_e = clamp(W·C/L, 0, 102.3) (not used by the drawing,
     2.3).
   - Light scale = 0 with flag 2, else 1.8·w_e.
5. Fire OnLightOn.

**2.2 Dynamic light** (named or parented spotlights, flag 2 clear, marker
alpha byte > 0, rendercolor not black): every client frame a dynamic light
at B2 + (0, 0, 5), radius 3 × light scale = 5.4·w_e, colour = rendercolor
× marker alpha byte, i.e. rendercolor/255 as a linear intensity (alpha byte
1), refreshed with a 0.05 s lifetime. In practice: a light appears only
when the 2L trace hits a brush within L of the spotlight. How the engine
falls the light off over its radius is not in the SDK (Open questions).

**2.3 Drawing the spotlight beam** (beam with a halo, client): the halo
path overrides the generic beam (1):
- The strip is drawn with **constant width W** (the end width is ignored)
  and exactly 2 segments, with shade-out over fade length F: brightness
  falls linearly from 1 at O to 1 − |B1 − O|/F at B1. Because F ≥ |B1 −
  O| always holds for a static spotlight, the beam reaches 0 at B1 or
  later (it fades to nothing exactly at B1 when F = |B1 − O|).
- **Near-axis fade**: e = distance from the camera to the infinite line
  through O along the beam. If e < 4W, the beam colour is multiplied by
  clamp((e − W)/(3W), 0, 1) (1 at 4W, 0 at W). Standing in the shaft hides
  it.
- Colour = rendercolor/255 × 64/255 × near-axis factor × shade, additive.
- **Halo** at O, a camera-facing square of half-size h·60 where h =
  clamp(1 + (4W − e)/(3.5W), 1, 2) (1 at e ≥ 4W, 2 at e ≤ W/2), sprite
  `sprites/light_glow03.vmt` in glow render mode. k = dot(beam direction,
  normalize(camera − O)); if k ≤ 0 (camera behind the spotlight) no halo;
  else halo colour = rendercolor/255 × clamp((2k)², 0, 1) × the visible
  fraction of a small occlusion proxy at O (pixel-visibility query with
  radius clamp(1 + 60·W/w_e, 1, 8)). Note the halo colour is **not**
  scaled by the brightness 64.
- HDRColorScale multiplies the material's HDR colour scale (beam and
  halo); in LDR it has no effect.

**2.4 Dynamic (parented) spotlights.** The beam is attached to the moving
spotlight; its end follows an end marker that chases the current aim
point:
- Think every tick (parented) or every 0.1 s.
- If off and a beam exists: destroy it (fires OnLightOff). If on and no
  beam: build it (2.1 steps 1–3 with the beam attached from the spotlight
  to the marker).
- Aim point P = trace O → O + d·2L as in 2.1 (B2).
- v = P − marker; if |v| < 1: marker velocity 0, done. Else marker
  velocity = normalize(v)·10|v|; if that exceeds 200 units/s, the marker
  **jumps to P** and its velocity is set to 200 along v.
- C ← 0.6·C + 0.4·|marker − O| (smoothed), then the render values of 2.1
  step 4.
LightOn/LightOff on a dynamic spotlight only flip the state; the next think
builds or destroys.

**2.5 Inputs on static spotlights.** LightOn when off: build (2.1), fires
OnLightOn. LightOff when on: delete beam and marker, fires OnLightOff.
Repeated LightOn/LightOff in the same state do nothing.

### 3. env_laser

Keyvalues: "LaserTarget" (target name), "texture" (beam sprite), "width"
(float), "NoiseAmplitude" (integer), "TextureScroll" (integer),
"framestart", "EndSprite" (optional sprite at the hit point),
"rendercolor", "renderamt" (beam brightness), "renderfx", "damage"
(per second), "dissolvetype" (−1 none default; ≥ 0 HL2 dissolve), and
"HDRColorScale". Spawnflags: 1 "Start On", 16 "Start Sparks", 32 "End
Sparks", 64 "Decal End". Inputs **TurnOn**, **TurnOff**, **Toggle**, plus
the beam inputs **Width** (sets width and end width), **Noise**,
**ColorRedValue**, **ColorGreenValue**, **ColorBlueValue** (each clamped
0–255), **ScrollSpeed**. No outputs.

- No "texture": the entity deletes itself.
- End width = width (no taper). Beam type: two points, start = the laser's
  origin.
- End sprite: an env_sprite at the hit point, glow render mode, colour and
  alpha = the laser's rendercolor/renderamt (specs/cs_source/sprites_dust.md
  for its drawing), shown and hidden with the laser.
- Starts **on** if it has no targetname or flag 1 is set; else off.
- **On** (every tick while on, starting at TurnOn):
  1. Target T = one entity named LaserTarget, chosen uniformly at random
     among all matches, re-chosen every tick (several targets → the laser
     jumps between them). None found: aim at the previous end point.
  2. Trace laser origin → T's origin against everything solid (world,
     brush entities, props, players; nothing ignored). End point = hit
     point (or T's origin).
  3. Beam end and end sprite move there.
  4. If at least 0.1 s passed since the last damage time (7 ticks):
     damage the hit entity (if the trace hit anything, the world included)
     by damage × (now − last damage time), type energy beam (or dissolve
     with dissolvetype ≥ 0), force by the melee rule along the beam; with
     flag 64 put the "BigShot" decal on brush surfaces it hit; emit sparks
     (env_spark effect, magnitude 1, trail 1) at the start (16) and/or end
     (32); last damage time = now. The first damage happens 7 ticks after
     TurnOn.
- **Off**: beam and end sprite hidden, no thinking.
- Toggle: on ↔ off. TurnOn when on and TurnOff when off do nothing.

So a laser with "damage" 100 deals 10.5 every 7 ticks (100 × 0.105) to the
first solid thing on its line, i.e. 100 per second, and its sparks pop
every 0.105 s.

### 4. env_beam

Keyvalues: "LightningStart", "LightningEnd" (entity names), "texture"
(sprite; `.vmt` appended if missing), "BoltWidth", "NoiseAmplitude" (≤ 64),
"TextureScroll" (0–100), "framerate", "framestart", "life" (s),
"StrikeTime" (s), "Radius", "damage", "rendercolor", "renderamt",
"HDRColorScale", "decalname", "TouchType" (0 none, 1 player only, 2 NPC
only, 3 player or NPC, 4 player, NPC or prop_physics), "filtername".
Spawnflags: 1 "Start On", 2 "Toggle", 4 "Random Strike", 8 "Ring", 16
"Start Sparks", 32 "End Sparks", 64 "Decal End", 128 "Shade Start", 256
"Shade End", 512 "Taper Out". Inputs **TurnOn**, **TurnOff**, **Toggle**,
**StrikeOnce**, plus the beam inputs listed under env_laser. Output
**OnTouchedByEntity** (activator = the touching entity).

No texture: the entity deletes itself. Width = BoltWidth; end width = 0
with Taper Out, else BoltWidth.

There are two kinds, chosen at spawn:

**4.1 Persistent beam (life = 0 and not Ring).** A map entity beam from
LightningStart to LightningEnd (first entity of each name, resolved at
activation; missing either: no beam). An endpoint that is a "static point"
(an entity with no parent and either no model or of class info_target,
info_landmark or path_corner) is frozen at its position at activation;
otherwise the beam follows that entity's origin every frame. Noise
amplitude, scroll, start frame as keyed; shade-in flag (128) wins over
shade-out (256) if both are set; colour/brightness from rendercolor and
renderamt.
- Starts on unless it is named and lacks flag 1. Off = hidden.
- While on it thinks **every tick**:
  1. If damage > 0 and ≥ 0.1 s since the last damage time: trace start →
     end against everything solid and damage the first thing hit by
     damage × elapsed (as env_laser 3 step 4, decal "decalname" with flag
     64; no sparks here).
  2. If TouchType ≠ 0: trace start → end against shot-solid **players**
     (and NPCs; and prop_physics with type 4) only. If something is hit,
     passes the type (1: players; 2: NPCs; 3: either; 4: either or a
     prop_physics) and the filter (if "filtername" names a filter): fire
     **OnTouchedByEntity** and **stop thinking** (no more damage or touch
     tests until the next TurnOn; quirk, keep).
- TurnOn (when off): show, sparks at the endpoints per flags 16/32, damage
  clock reset to now, think from the next tick. TurnOff: hide, stop.

**4.2 Strike generator (life ≠ 0, or Ring).** The server sends temporary
beams:
- First strike 1 s after spawn if unnamed or flag 1; named without flag 1:
  idle until TurnOn (TurnOn strikes on the next tick).
- Each strike, then the next one is scheduled at now + life + StrikeTime,
  or now + life + U(0, StrikeTime) with Random Strike (4).
- **With LightningEnd**: pick a random entity of each name (uniform among
  matches; both must exist). If either endpoint is a static point: an
  entity-to-point beam (Ring flag: nothing at all); else an entity-to-
  entity beam, or a ring (Ring flag: a circle whose diameter is the line
  between the two entities). Then sparks per flags, and if damage > 0:
  trace start → end against everything solid and deal **damage × 1** to
  the first thing hit, once per strike (energy beam type).
- **Without LightningEnd, with LightningStart**: up to 10 tries: a random
  direction (each component U(−1, 1), normalized), trace from the start
  entity's origin out to Radius against world brushes; accept if it hits
  at a distance ≥ Radius/10; draw a beam from the start origin to the hit
  point. No damage.
- **Neither**: up to 10 tries: two random directions from the beam's own
  origin whose dot product is ≤ 0; both traces (length Radius, world
  brushes) must hit, the hit points must be ≥ Radius/10 apart and see each
  other; draw a beam between them. No damage.
- Strike beams carry: width = end width = BoltWidth, no halo, no fade
  length, noise amplitude, colour and alpha = rendercolor and renderamt
  (alpha is the brightness), scroll = TextureScroll (×0.1 on the client),
  frame rate = framerate (×0.1), life = life.
- StrikeOnce: one strike now (4.2 rules with endpoints; needs
  LightningEnd), regardless of on/off and of kind.
- TurnOff stops the strikes (beams already sent live out their life).

### 5. env_spritetrail

Keyvalues: "spritename", "lifetime" (s), "startwidth", "endwidth"
(missing → −1 = "no taper"), "rendercolor", "renderamt", "rendermode"
(Hammer default 5 additive), "animate" (unused here). Typical use: parented
to a player or a moving prop (SetParent input) to leave a ribbon.

Client, per frame:
1. **Record**: at most once per lifetime/256 seconds: if there is no point
   yet or the entity's render origin (its attachment if attached) moved
   more than 2 units since the last point, append a point (position, death
   time = now + lifetime, texture coordinate = previous coordinate +
   distance × texture resolution). Over 256 points: the oldest is dropped.
2. **Draw** (if at least one point): a camera-facing strip through all
   points, oldest first, plus a final point at the current origin. For each
   point, l = clamp((death − now)/lifetime, 0, 1):
   - width = endwidth + l·(startwidth − endwidth) if endwidth ≥ 0, else
     startwidth; plus the point's width variance (0 for map trails); ≥ 0.
     This value is the strip's full width (*inferred*, Open questions).
   - colour = rendercolor/255; alpha = renderamt/255 × l (newest point
     l = 1, oldest about 0).
   - points whose death time passed are dropped after being drawn.
   The material's render mode is the entity's rendermode.
3. Map trails have a **texture resolution of 0** (nothing sets it from
   keyvalues): every point has v = 0, so the texture is sampled along a
   single row and does not repeat along the trail (Open questions).
4. Transmission: always sent; a trail is drawn by every client, including
   the player it is parented to (seen in third person / by others).

### 6. env_lightglow

Keyvalues: "rendercolor", "VerticalGlowSize" V, "HorizontalGlowSize" H
(integers), "MinDist", "MaxDist", "OuterMaxDist" (integers; Max and Outer
clamped to 65535), "GlowProxySize" (default 2, network range 0–64),
"HDRColorScale". Spawnflag 1 "Visible only from front". Input **Color**
(sets rendercolor; the client updates the glow colour).

Client:
- **Activation**: the first check happens U(0, 3) s after the entity
  arrives, then every U(1, 3) s: the glow is active only if its origin is in
  the potentially visible set. So a glow can take up to 3 s to appear
  after entering a new area.
- Each frame while active, toward the glow: u = normalize(glow − eye).
  - Occlusion factor o = the visible fraction of a sphere of radius
    GlowProxySize at the glow (pixel-visibility query; if that is
    unavailable, o moves toward 1 or 0 by Δt/0.05 per frame). o = 0: not
    drawn.
  - One-sided (flag 1): if dot(normalize(eye − glow), forward(angles)) <
    0: colour 0.
  - Distance fade, dist = |eye − glow|: if OuterMaxDist > MaxDist and dist
    > MaxDist: f = remap(dist, MaxDist → 1, OuterMaxDist → 0); else f =
    remap(dist, MinDist → 0, MaxDist → 1); both clamped to [0, 1]. With
    MinDist = MaxDist the remap is a step: f = 1 when dist ≥ MaxDist, else
    0.
  - Colour = rendercolor/255 × f × o (renderamt is ignored).
  - Quad: centred at eye + 100·u, half-extents H along right = normalize(u
    × Z) and V along up = right × u; so its on-screen size does **not**
    shrink with distance (it subtends atan(H/100) either side). Culled if
    off screen. Material `sprites/light_glow02_add_noz` (additive, no
    depth test: walls only hide it through o).

### 7. env_steam (and legacy env_steamjet)

Keyvalues: "InitialState" (0 off, 1 on), "Type" (0 normal, 1 heat wave),
"SpreadSpeed", "Speed", "StartSize", "EndSize", "Rate" (particles/s),
"JetLength", "RollSpeed", "rendercolor", "renderamt". Spawnflag 1
"Emissive". Inputs **TurnOn**, **TurnOff**, **Toggle**, and the value
inputs **JetLength**, **SpreadSpeed**, **Speed**, **Rate**.
env_steamjet is the same entity facing its −right direction instead of
forward.

Client, per frame:
- Particle life T = JetLength/Speed (recomputed on every update from the
  server).
- **Emission** (while on): a timer emits one particle immediately on
  creation and then one every 1/Rate s (several per frame if due). Rate is
  read **only when the effect is created**: the Rate input has no visible
  effect afterwards (quirk). Particles are only emitted if the effect was
  drawn last frame (in view); off-screen jets make no particles.
- Each particle: position = jet origin; velocity = Speed·forward +
  U(−SpreadSpeed, SpreadSpeed)·right + U(−SpreadSpeed, SpreadSpeed)·up
  (constant: no gravity or drag); roll U(0, 360), roll rate
  U(−RollSpeed, RollSpeed).
- **Simulation**: age += Δt; removed when age > T; position += velocity·Δt;
  roll += rate·Δt. (Skipped while the effect is off screen and still
  emitting.)
- **Size** = StartSize + (EndSize − StartSize) × **age in seconds** (not
  age/T: a jet whose particles live 2 s grows past EndSize; quirk). For
  type 0 the start and end sizes are first truncated to integers 0–255.
- **Alpha** = sin(π·age/T) × renderamt/255 (fades in and out).
- **Colour** (normal type): five lighting samples at evenly spaced points
  from the origin to origin + forward·Speed·T, each = world light at that
  point × rendercolor/255 (Emissive: + rendercolor/255, clamped to 1),
  rescaled so the largest channel is ≤ 1; the particle's colour is the
  linear interpolation of the samples at age/T. Re-sampled only when the
  jet moves or turns by more than 0.1.
- Material: type 0 `particle/particle_smokegrenade` (camera-facing,
  rolled; size is the half-size); type 1 `sprites/heatwave` (a refracting
  quad with a random normal each frame, not rolled, in the view plane).

### 8. env_spark

Keyvalues: "MaxDelay" (s, < 0 → 0), "Magnitude" (1, 2, 5, 8 in Hammer; sent
in 4 bits so 0–15), "TrailLength" (1–3; 4 bits). Spawnflags: 64 "Start
ON", 128 "Glow", 256 "Silent", 512 "Directional". Inputs **StartSpark**,
**StopSpark**, **ToggleSpark**, **SparkOnce**. Output **OnSpark**
(activator = caller = the spark).

- Spawn: if flag 64, the first spark comes after 0.1 + U(0, 1.5) s.
- Each spark (server think), then the next at now + 0.1 + U(0, MaxDelay):
  1. Spark effect at the entity's centre (client, below), direction
     forward(angles) with flag 512, else (0, 0, 0).
  2. Sound "DoSpark" from the entity unless flag 256.
  3. Fire **OnSpark**.
  4. Flag 128: a temporary glow sprite `sprites/glow01.vmt` at the
     origin: scale 1.5, additive, alpha 25/255, life 0.2 s (animating).
- StartSpark: spark on the next tick, then periodic. StopSpark: stop (the
  already scheduled think time stays recorded until it passes, see
  Quirks). ToggleSpark: start if no think is scheduled, else stop.
  SparkOnce: one spark now **and stop** periodic sparking.

**Client spark effect** for magnitude m, trail length t, direction n (the
same effect as an electric bullet impact, specs/cs_source/impact_effects.md
section 8, with these differences): because the message always carries a
direction (zero when not directional), every spark direction below is
**normalized** after adding the direction term.
- Big sparks: count int(m²·U(2, 4)); life m·U(1, 2); direction (U(−1,1),
  U(−1,1), U(0.5,1)) + 2n, normalized; width U(2, 5); length
  t·U(0.02, 0.05); speed as in the impact effect.
- Small sparks: count m·I(16, 32); direction U3(−1,1) + n, normalized;
  width U(2, 4); length t·U(0.02, 0.03); life m·U(0.1, 0.2); speed
  U(128, 256).
- Glows: inner flare size m·I(4, 8), outer flare size m·I(32, 64); smoke
  as in the impact effect. Sizes are bytes (m·64 > 255 wraps; Edge cases).

## Per-tick order

Server, one tick (thinks in entity order, then the event queue,
specs/source/entity_io.md):
1. env_laser (on): pick target, trace, move end, maybe damage + sparks.
2. env_beam persistent (on): maybe damage, touch test (may fire
   OnTouchedByEntity and stop).
3. env_beam strike generator: strike when due (temp beam, sparks, damage).
4. point_spotlight dynamic: re-aim; static spotlights do nothing.
5. env_spark: spark when due (effect, sound, OnSpark).
6. Event queue: inputs (TurnOn, LightOn, ...) take effect; a TurnOn on
   env_laser traces at once, so its beam end is correct this tick.

Client, one frame: update temp beams (drop expired), draw entity beams
(rebuilding noise and scroll), trails record then draw, steam emits then
simulates then draws, glows (overlays) draw last over the scene.

## Edge cases

- Beam shorter than 0.1 units (env_laser whose target is missing at
  spawn, aiming at its own origin): invisible.
- env_beam with life 0 that receives StrikeOnce: the temp beam it sends
  has life 0, which the client treats as **permanent** (it never expires;
  it is dropped only if an attached entity disappears). Keep: it is what
  the game does; mappers rarely hit it.
- env_beam endpoints that are players or props: the persistent beam
  follows them; strike beams to an entity without a client model are not
  created (life ≠ 0).
- env_steam keys missing from the map: the server sends 0 for Speed
  (particle life infinite/undefined) and Rate (no emission). Treat Speed 0
  or Rate 0 as "no particles".
- env_spark Magnitude above 3: flare sizes m·64 exceed 255 and wrap in the
  byte (e.g. m = 5 → outer flare I(160, 320) wraps for values ≥ 256).
  Rare; reproduce or clamp (decide at implementation, Open questions).
- point_spotlight width > 102.3 is clamped; length ≤ 0 → 500.
- point_spotlight aimed into open sky within 2L: no dynamic light even
  when named (marker alpha 0).
- Two env_lightglows at the same point add (additive material).

## Quirks

- **Unnamed, unparented spotlights are deleted after building their beam**:
  they cannot be toggled and never light anything (keep; matches every
  stock map's look).
- **Spotlight end width is ignored when drawing** (the halo path draws a
  constant-width shaft that fades to nothing): keep.
- **Spotlight dlight only within L, not in (L, 2L]** (byte truncation):
  keep.
- **Entity-beam scroll rate is speed²** (keep, but mostly invisible since
  scroll is usually 0–35 and looks like noise).
- **env_beam stops thinking after OnTouchedByEntity** (keep: maps use the
  output for one-shot laser tripwires and re-arm with TurnOn).
- **env_laser re-picks a random target among same-named targets every
  tick** (keep).
- **Steam particle size uses seconds, not life fraction** (keep).
- **Steam Rate input does nothing after creation** (keep).
- **env_spark StopSpark then ToggleSpark within the pending think time**:
  the old think time is still in the future, so Toggle sees "running" and
  stops again (nothing happens). Keep.
- **SparkOnce also stops a running spark** (keep).
- **Lightglow takes up to 3 s to appear** after it enters the PVS (keep).

## Test cases

dt = 0.015. Colours in 0–1 after /255.

| Setup | Input | After | Expected |
|---|---|---|---|
| point_spotlight, no name, flag 1, L 500, W 50, wall 300 units ahead | map load | activation | beam O→O+300d, width 50 constant, brightness 1 at O falling linearly to 0 at 300; entity removed; no dlight; OnLightOn fired once |
| same, named "s1" | map load | activation | as above plus dlight at wall point + (0,0,5), radius 5.4 × 30 = 162 (w_e = 50·300/500 = 30), colour = rendercolor/255 |
| named spotlight, L 500, nearest wall 700 ahead | map load | — | beam to O+500d; F = 500; marker alpha byte 0 → no dlight |
| named spotlight, nothing within 1000 | map load | — | beam to O+500d, no dlight |
| spotlight W 0, L −5 | spawn | — | W 10, L 500 |
| spotlight W 200 | spawn | — | W 102.3 |
| named spotlight on | LightOff, LightOff | — | OnLightOff once; beam gone |
| named spotlight off | LightOn | — | beam built, OnLightOn |
| spotlight W 50, camera 100 units from the beam axis | draw | — | beam colour × 1/3 ((100 − 50)/150) |
| same, camera 25 from axis, in front (k = 0.5) | draw | — | beam colour × 0; halo half-size 120, halo colour = rendercolor/255 × 1 × visibility |
| spotlight, camera behind (k < 0) | draw | — | no halo |
| camera in front, k = 0.25 | draw | — | halo colour × 0.25 |
| env_laser damage 100, unnamed, LaserTarget at 500 units, player in between at 200 | 1 s | — | first hit at tick 7, then every 7 ticks, 10.5 each (×9 by tick 63 = 94.5; generic accounting) |
| env_laser flags 16+32, damage 0 | 1 s | — | sparks at both ends every 7 ticks (9 times in 66 ticks) |
| env_laser named, no flag 1 | spawn | — | off, hidden, no damage until TurnOn |
| env_laser two entities named "t" | on | — | each tick end point is one of the two at random |
| env_beam life 0, start/end info_targets, damage 50, player crosses | per tick | — | damage 50 × 0.105 = 5.25 every 7 ticks while blocking |
| env_beam life 0, TouchType 1, player crosses beam | — | tick of contact | OnTouchedByEntity (activator = player) once; no further damage or outputs until TurnOn |
| env_beam life 0.5, StrikeTime 1, unnamed, damage 20, ends at two info_targets, player in line | — | — | strikes at t = 1, 2.5, 4, ...; each a beam lasting 0.5 s; 20 damage per strike |
| same with Random Strike | — | — | strike interval 0.5 + U(0, 1) |
| env_beam, no LightningEnd, LightningStart at room centre, Radius 256 | strike | — | beam from start to a wall point ≥ 25.6 away, or none after 10 failed tries |
| env_beam BoltWidth 10, Taper Out | draw | — | width 10 at start, 0 at end |
| beam L = 400, A = 0, w = w_e = 2 | segments | — | N = int(400·0.075 + 3) = 33 |
| beam L = 400, A = 10 | segments | — | N = min(int(103), 128) = 103 |
| beam L = 40, w = 20, A = 0 | segments | — | N = 6 → L/5 = 8 < 1.414·10 → N = int(40/14.14)+1 = 3 |
| beam fade length 0, shade out, f = 0.5 | — | — | brightness 0.5 |
| beam shade in + out, f = 0.25 / 0.75 (F = 0) | — | — | 0.5 / 0.5 |
| env_spritetrail lifetime 1, startwidth 8, endwidth 0, entity moving 100 u/s, client at 60 fps | 1 s | — | a point every 2nd frame (1.67 units per frame is not > 2), ≈ 30 points; newest width 8, alpha renderamt/255; a 0.5 s old point width 4, alpha × 0.5 |
| env_spritetrail endwidth key missing | — | — | constant width startwidth |
| env_lightglow MinDist 100, MaxDist 500, OuterMaxDist 0, eye at 300, unoccluded | — | — | f = 0.5, colour = rendercolor/255 × 0.5 |
| same, Outer 1000, eye at 750 | — | — | f = 0.5 |
| same, eye at 1200 | — | — | 0 |
| env_lightglow H 30, V 15 | — | — | quad half-extents 30 × 15 at 100 units ahead (≈ ±16.7° × ±8.5°) |
| env_lightglow flag 1, eye behind | — | — | not drawn |
| env_steam Speed 120, JetLength 80, StartSize 10, EndSize 25, renderamt 255 | particle age 1/3 s | — | T = 0.667; size 10 + 15/3 = 15; alpha sin(π/2) = 1 |
| same, age 0.6 s | — | — | size 19; alpha sin(0.9π) = 0.309 |
| env_steam Speed 60, JetLength 120 (T = 2) | age 1.5 | — | size 10 + 15·1.5 = 32.5 (past EndSize) |
| env_steam Rate 26 | 1 s on, in view | — | 26 or 27 particles (one at creation) |
| env_spark MaxDelay 0, flag 64 | — | — | first spark at 0.1–1.6 s, then every 0.1 s (7 ticks) with OnSpark |
| env_spark running | SparkOnce | — | one spark, then none |
| env_spark Magnitude 2, TrailLength 1 | spark | — | big sparks int(4·U(2,4)) = 8–15, small 32–64, life ×2 |

## Open questions

1. **Strip width convention**: is a beam's on-screen full width 2 × the
   width keyvalue (and a sprite trail's 1 × startwidth)? Measure: env_beam
   BoltWidth 10 between two info_targets in front of a 64-unit grid
   texture, screenshot at a known distance; same for env_spritetrail
   startwidth 10. (The strip drawer is engine code.)
2. **Dynamic light falloff**: the engine's dlight intensity versus distance
   for a given radius and colour (affects named spotlights). Measure the
   lit wall brightness around a named spotlight's hit point at several
   distances against the same wall unlit.
3. **CS:S damage path for energy-beam damage**: does CS:S armour absorb
   it, does the generic fractional accumulator apply, and does dissolve
   damage (dissolvetype ≥ 0) do anything in CS:S? Probe: env_laser damage
   100 through a player with and without kevlar, read health and armour per
   hit.
4. **CS:S overrides**: CS:S could override any of these client effects
   (the SDK shows the shared/HL2 versions). Compare in-game screenshots of
   a point_spotlight, env_steam and env_lightglow with renders of this
   spec (mg_ maps that use them: see docs/plans/active/community-maps.md).
5. **Pixel-visibility fade**: does the occlusion fraction for halos and
   lightglows change instantly or fade over a short time? Walk behind a
   pillar while recording a demo and step through frames.
6. **Trail texture resolution 0**: confirm map-placed env_spritetrail
   textures do not repeat along the trail (screenshot a trail whose
   texture has a visible pattern).
7. **Steam roll units**: are roll and RollSpeed radians or degrees in the
   particle renderer? Watch a steam puff with RollSpeed 100.
8. **env_spark byte wrap** for Magnitude 5/8: compare flare sizes in-game.
9. **Older builds**: the SDK's point_spotlight passes its parent to the
   beam (newer code); older CS:S builds may not. Parented spotlights are
   rare; check one on a map that has them.
10. **Entity-beam scroll quirk**: confirm that an env_beam with
    TextureScroll 35 scrolls erratically (speed²) rather than at 35 units
    per second; record a slow-motion demo (host_timescale).
