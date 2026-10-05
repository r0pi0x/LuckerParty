# Counter-Strike: Source: keyframe ropes (move_rope / keyframe_rope)

Source basis: Valve's public Source SDK 2013. I read the server rope entity, the client rope entity with its simulation helper and render batching, the shared rope constants, the generic fixed-step Verlet integrator, the rope "hang distance" helper, the client beam drawing code (for width conventions only) and the cable material shader. CS:S's own game DLL source is not public. This spec assumes CS:S uses the same shared rope code, which ships in every Source multiplayer game of that era. The engine's point-lighting call and the strip builder that turns rope points into camera-facing quads are engine/tier2 code. They are not in the SDK, so the parts that depend on them are marked as inferred.
Status: draft

## Summary

- A rope keyframe draws one rope from itself to the entity named in its NextKey, using its own keys. A chain of N keyframes draws N-1 ropes.
- Each rope is a short chain of point masses (2, 4, 5 or 10 nodes, chosen by the Type key, not by Subdiv). It is simulated on the client at a fixed 50 Hz with damped Verlet integration, gravity 1500 units/s², and 3 Gauss-Seidel sweeps of "max length" link constraints per step.
- When a rope is created, it is pre-simulated for about 5 s in one go, so the player never sees it fall. The constraint solver never fully converges, so the rope behaves as if slightly elastic. The settled shape is therefore the fixed point of that exact step, not a catenary.
- Slack is not "extra length": the solver's total length is straight distance + Slack - 100. Ropes with Slack ≤ 100 are still sagging, but only because gravity beats 3 solver sweeps.
- For drawing, extra points are added between nodes on a Catmull-Rom spline (Subdiv points per gap). The rope is drawn as one camera-facing strip, Width world units wide, textured along its length and vertex-lit with one light sample per node.

## Units and conventions

- Distances are in Hammer units (1 unit = 1 inch). Z is up. Gravity points toward -Z.
- Simulation time is seconds. The simulation step is fixed at 1/50 s, independent of server tick rate and frame rate. Each frame runs as many whole steps as needed to catch up, rounding up, and the drawn positions are interpolated (see Per-frame order).
- Node 0 is the keyframe that owns the rope (start). The last node is the NextKey entity (end).
- An entity's position is its origin. If a rope end is attached to a model attachment, the attachment's position is used instead. For an end entity that is not a rope keyframe and has no attachment, the client uses the centre of that entity's bounds.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| step | 1/50 (as 32-bit float 0.0199999996) | s | fixed simulation step |
| gravity | (0, 0, -1500) | units/s² | per-node acceleration |
| damping | 0.98 | - | multiplier on the previous step's displacement |
| accel_factor | step²/2 = 0.0002 | s² | acceleration is multiplied by this, not by step² (see Quirks) |
| gravity_per_step | (0, 0, -0.3) | units | gravity's displacement contribution per step |
| solver_sweeps | 3 | - | constraint sweeps per step |
| slack_offset | -100 | units | added to every rope's length (the "fudge") |
| initial_hang_time | 5 | s | pre-simulation on creation = 251 steps (see Edge cases) |
| max_nodes | 10 | - | |
| subdiv_max | 7 | - | Subdiv values ≥ 8 are clamped to 7 |
| rest_threshold | 0.03 | units² | a node "moved" if its squared displacement in the last step exceeds this |
| wind_cull_dist | 1000 (cvar rope_wind_dist) | units | wind is only simulated when the camera is within this distance of the endpoint line |
| gust_base | 50 | units/s² | maximum gust strength before the time envelope |
| wind_vel_factor | 10 | 1/s | env_wind velocity → acceleration |
| collide_half_extent | 2 | units | half-size of the box swept for world collision |
| collide_pushout | 2.2 | units | distance a node is placed in front of a hit plane |
| collide_friction | 0.3 | - | fraction of this step's motion removed on contact |
| texels_per_unit | 4 / TextureScale | texels/unit | texture density along the rope |

## Entity keys → rope parameters

Read from the keyframe entity (move_rope and keyframe_rope behave identically). The defaults below are what the entity uses when a key is absent. Hammer normally writes every key, and its own defaults may differ (see Open questions).

| Key | Meaning | Default if absent |
|---|---|---|
| NextKey | targetname of the end entity | none |
| Slack | integer, units | 0 |
| Type | node count: 0 → 10, 1 → 4, any other value → 2 | 5 nodes (no Type key) |
| Subdiv | spline points inserted per gap, render only | 2 |
| Width | strip width, world units (see Rendering) | 2 |
| TextureScale | see Texture coordinates | 4 |
| RopeMaterial | material path. ".vmt" is appended if missing | cable/cable.vmt |
| RopeShader (legacy) | 0 → cable/cable, 1 → cable/rope, other values → cable/chain | |
| Collide = 1 | collide with world brushes | off |
| Dangling = 1 | end node is not pinned | pinned |
| Barbed = 1 | barbed-wire zigzag spline (see Rendering) | off |
| NoWind = 1 | no wind | wind on |
| Breakable = 1 | can be broken by damage | off |
| spawnflags bit 0 (value 1) | "auto resize": recompute length when endpoints move | off |

Other notes on the keys:
- Type decides how many simulated nodes the rope has. Subdiv does not change the simulation.
- Breakable and auto resize do nothing on a static map. Ignore them.

Values as they reach the client: they are networked, so the client sees them quantized.
- Slack: 12-bit signed integer.
- Rope length: 15-bit signed integer.
- Subdiv: 4-bit unsigned (0..15, then clamped to 7).
- Node count: 4-bit.
- TextureScale: 10-bit fixed-point over [0.1, 10], a step of about 0.0097.
- Width: sent exact.

## Behavior

### Which ropes exist

- At map activation, each keyframe looks up the first entity whose name equals its NextKey.
  - If one is found, the rope is start = this keyframe, end = that entity.
  - If none is found and Dangling is not set, the keyframe draws nothing (it is only the end of someone else's rope).
  - If none is found and Dangling is set, the rope has only a start, and the end node is free.
    - The client code intends to place the end at the start position when there is no end entity.
    - But its end-position lookup always reports success, so the free end's straight-line starting point comes from a cached end position that is never written for a missing entity. In effect it is undefined.
    - Recommended: place it at the start. See Open questions.
- Rope length L is the straight distance between the two entities' origins, truncated toward zero to an integer. If there is no end entity, L = 0.
- The client draws nothing while the cvar r_ropetranslucent = 0 (default 1). Despite its name, this cvar turns rope drawing on and off; it has nothing to do with translucency. The cheat cvar r_drawropes = 0 also disables drawing and simulation.

### Link length

```math
n = \text{node count},\qquad
\ell = \max\!\left(0,\ \frac{L + \text{Slack} - 100}{n - 1}\right)
```

Every one of the n-1 links has the same maximum length ℓ.

### Initial state

- Node i (i = 0..n-1) is placed on the straight segment at t = i/(n-1), with zero velocity (previous position = position).
- Then, before the first frame is drawn, the rope is simulated for 5 s (251 steps; see Edge cases).
- Then the per-node light values are sampled once (see Lighting). They are never re-sampled on a static map.

### One simulation step (fixed 1/50 s)

State per node: current position p and previous position q.

1. **Integrate every node, including the pinned ends:**
   ```math
   p' = p + 0.98\,(p - q) + a\cdot\frac{\Delta t^2}{2},\qquad q' = p
   ```
   Here a = gravity (unless no-gravity is set, which a map rope never has), plus wind (below).
2. **Repeat 3 times:**
   1. **Link pass.** For links k = 0..n-2 in order, using positions already updated earlier in this pass (Gauss-Seidel):
      - Let d = p_k - p_{k+1} and D = |d|.
      - If D² > ℓ², then p_k -= ½(1 - ℓ/D)·d and p_{k+1} += ½(1 - ℓ/D)·d.
      - Links are "max length" only: a link shorter than ℓ is left alone.
      - The ends are moved by this pass too, and are re-pinned next.
   2. **World collision**, only if Collide = 1 and rope_collide ≥ 1, or for every rope if rope_collide = 2. See Collision.
   3. **Re-pin the ends.** Node 0 is set to the start attachment position. Node n-1 is set to the end position, unless the rope is Dangling.
      - Direction locking (bending the first or last two links toward an attachment's forward vector) is never enabled for map ropes.

### Settled shape (the deterministic rule)

With no wind, the shape the player sees is the fixed point of the step: the positions p* that one step maps back to themselves when q = p*.

```math
p^* = \Pi\!\left(p^* + (0,0,-0.3)\right)
```

Π is "three link sweeps, each followed by re-pinning", exactly as above.

How to compute it:
- Start from the straight line with zero velocity.
- Run the step until the largest squared per-step displacement is ≤ 0.03, or for a fixed 251 steps. The game shows the 251-step state first, then keeps simulating in real time until rest.
- For Slack up to about 200 over 256 units, 251 steps is already within about 0.2 units of the fixed point. For Slack ≤ 100 it is already exact to 0.001.

Properties of the fixed point:
- Every link is stretched slightly past ℓ, because 3 sweeps cannot cancel the 0.3 units of gravity per step. So even Slack ≤ 100 ropes sag a few units.
- The shape is not symmetric. The sweep always runs from start to end, so the start side ends up slightly lower and the lowest node shifts toward the start (see Test cases).
- Node spacing along the rope is not uniform.

### Wind (makes the rope sway; skip it for a still render)

Wind applies only when all of these hold:
- NoWind is not set.
- Fewer than half the nodes touched the world in the last step.
- The camera is less than rope_wind_dist (1000) from the line segment between the two end nodes.

While wind applies, the rope never counts as resting and simulates every frame.

- **With an env_wind in the map:** each node gets an extra acceleration of 10 × the current wind velocity. With several env_wind entities, the first one registered is used.
- **Without env_wind:** gusts.
  - Gust timing:
    - The first gust starts 1-3 s after creation (uniform random).
    - Each gust lasts T = uniform 2-3 s.
    - The next gust starts uniform 3-4 s after the previous one started.
  - Gust direction: a random unit vector u, built by normalizing a vector with each component uniform in [-1, 1].
  - Gust strength: the vector is 50·s·u, where s is uniform in [-1, 1].
  - Envelope: at time τ into the gust, the acceleration on every free node is
    ```math
    \bigl(1 - \cos(\pi\,\tau/T)\bigr)\cdot 50\,s\,u
    ```
    This peaks at 100 units/s², about 7 % of gravity, and is the same for all nodes. For τ ≥ T it is 0.
  - The gust timers advance by frame time only on frames where the rope simulates.

### Collision (Collide = 1)

For each node, up to 10 attempts:
1. Sweep a ±2-unit box from q to p against world brushes only.
2. If nothing is hit, stop.
3. If the sweep starts or ends inside solid, or hits at fraction 0: set p = q, mark the node as touching, and stop.
4. Otherwise:
   - Remove 30 % of this step's motion: p -= 0.3(p - q).
   - Move p along the hit plane's normal N so that it sits 2.2 units in front of the plane: p += N·(2.2 - (N·p - dist)).
   - Mark the node as touching, and try again.

If all 10 attempts are used up, p = q.

### Rest detection (per frame)

The rope simulates this frame if any of these is true:
- Network data changed this frame (always true on the first frames).
- A pinned endpoint moved more than 0.1 units on any axis.
- Wind applies.
- An impulse changed.
- Any node's squared step displacement exceeds 0.03.

Otherwise it is resting and costs nothing.

### Rendering: points

Build the rendered point list from the interpolated node positions. With s = Subdiv clamped to 0..7:

- Emit node 0.
- For each gap i → i+1, emit s spline points, then emit node i+1.
- The spline is uniform Catmull-Rom through P0 = node max(i-1, 0), P1 = node i, P2 = node i+1, P3 = node min(i+2, n-1). In the first gap P0 = P1, and in the last gap P3 = P2.
- Evaluate it at t_k = k/(s+1) for k = 1..s:
  ```math
  C(t) = P_1 + \tfrac{t}{2}(P_2 - P_0) + \tfrac{t^2}{2}(2P_0 - 5P_1 + 4P_2 - P_3) + \tfrac{t^3}{2}(-P_0 + 3P_1 - 3P_2 + P_3)
  ```

The total is n + (n-1)·s points (28 for n = 10, s = 2).

Barbed = 1 replaces this with exactly 3 points per gap at t = 1.5, -0.5 and 0.5, in that order. They overshoot past the next node and back behind the current one, which makes the zigzag.

### Rendering: strip, width

The points form one triangle strip with 2 vertices per point and (points-1)·2 triangles. Each vertex pair is offset sideways, perpendicular to the rope and facing the camera.

- **Width is the full width of the strip in world units, i.e. a diameter, not a radius.** This is inferred:
  - The client passes Width unchanged as the per-point strip width.
  - Other client beam code passes 2 × its radius-style widths into the same strip builder.
  - The anti-aliasing path below treats Width × (half screen width) / depth as the rope's on-screen size in pixels.
- **Expected side vector** (inferred, the strip builder is not public): the normalized cross product of the rope tangent at that point and the direction from the point to the camera. Vertices sit at point ± side·Width/2.
- The strip is drawn in the opaque pass, after the opaque entities, batched by material. It is two-sided, i.e. not back-face culled (inferred).
- Width is constant along the rope. There is no distance-based LOD: node count and Subdiv never change with distance.
- **Optional fake anti-aliasing** (only if a material named "<RopeMaterial>_back" exists, rope_smooth = 1 (default) and hardware MSAA is off):
  - First the "_back" material is drawn as a translucent strip. Its width is Width, or wider so that it is never under 0.3 px. Its alpha ramps 0.2 → 0.5 as the on-screen width goes from 0.3 px to 1.75 px.
  - Then the main material is drawn narrower, at Width - depth × 1.4 / screen width.
  - The cvars involved are rope_smooth_enlarge 1.4, rope_smooth_minwidth 0.3, rope_smooth_minalpha 0.2, rope_smooth_maxalphawidth 1.75, rope_smooth_maxalpha 0.5, and rope_solid_minwidth / maxwidth / minalpha / maxalpha = 0.3 / 1 / 0 / 1.
  - Without the "_back" material, every point gets alpha 0.3 and width Width.

### Rendering: texture coordinates

- U runs across the strip, 0 on one edge and 1 on the other.
- V runs along the rope. It starts at 0 (ScrollSpeed is stored, but the client never advances the scroll) and grows by a constant step per rendered point:

```math
\Delta V = \frac{(4/\text{TextureScale})\,(L + \text{Slack} - 100)}{(n-1)\,s + 1}\cdot\frac{1}{H}
```

H is the material's mapping height, i.e. the base texture's height in texels.
- The intent is: 4/TextureScale texels per unit of rope length (L + Slack - 100), spread evenly over the points.
- The divisor undercounts the points (see Quirks), so the last point gets V = ΔV·(n + (n-1)s - 1), not the intended total.
- V does not follow actual arc length: points are evenly spaced in V even when they are unevenly spaced in space.

### Lighting

- **When it is sampled:** once per node, right after the initial hang, at the node's interpolated position. It uses the engine's "light at a point" query, the same one used to light models. It is not a lightmap lookup. This is inferred to be the leaf ambient cube plus the map's static world lights, averaged over the six axis directions and clamped to [0, 1]; see Open questions.
- **rope_averagelight 0** (default 1): the colour is rescaled to the strongest of the six directional samples, then divided by its largest channel if that exceeds 1.
- **Vertex colour:**
  - Node points get their node's light.
  - Spline points between nodes i and i+1 step linearly from light_i toward light_{i+1}, by (light_{i+1} - light_i)/(s+1) per point.
  - mat_fullbright 1 forces white.
- **Cable shader** (the material's shader is "Cable"), per pixel:
  ```math
  c = \left(\tfrac{1}{2}(2b - 1) + \tfrac{1}{2}\right)^2 \cdot \text{base.rgb} \cdot \text{vertex.rgb} = b^2\,\text{base.rgb}\,\text{vertex.rgb}
  ```
  - b is the normal map's blue channel in [0, 1]. The light direction is fixed at the strip's own normal, which faces the camera, so this is "half-Lambert, squared".
  - The default normal map is cable/cablenormalmap. Its gradient across U is what makes the flat strip look round.
  - Alpha = base alpha × vertex alpha.
  - The material's $minlight and $maxlight parameters exist but this shader version ignores them.
  - Fog is applied. If the material is $translucent it is alpha-blended without depth writes, otherwise opaque.

## Per-frame order

1. If r_drawropes = 0, skip everything. On first use, set up the rope (Initial state, including the 251-step hang and light sampling).
2. Run rest detection. If not resting:
   - Clear the per-node "touching" marks.
   - Advance simulated time by frame time.
   - Run ceil(t_sim/step) - steps_done whole steps.
   - Advance the gust timers.
3. Interpolate drawn positions: for each node, lerp from q to p by α = (t_sim - (steps_done - 1)·step)/step.
4. Build the points, strip, texture coordinates and colours, then draw.

## Edge cases

- **Initial hang is 251 steps, not 250:** 5/0.0199999996 = 250.0000056, rounded up.
- **L + Slack - 100 ≤ 0** (e.g. a 64-unit rope with Slack 25): ℓ = 0, so every link wants zero length. Each sweep halves the gaps and the rope collapses toward the line between the pinned ends, pulled down by gravity. A Dangling rope with no NextKey and Slack ≤ 100 collapses to a point at its start and is effectively invisible.
- **Type 2 (2 nodes):** both nodes are pinned. The rope is a perfectly straight strip with no sag, whatever the Slack.
- **Subdiv 0:** the points are the nodes only, and ΔV = full intended length / H per node.
- **Dangling with NextKey present:** L is the real distance and the free end falls under gravity from its straight-line start.
- **Endpoint entity is a prop or brush (not a keyframe):** its bounds centre is used on the client, but the length L is computed from its origin on the server.

## Quirks

- **Half-strength gravity term.** Acceleration is scaled by Δt²/2 instead of Δt², so effective gravity is 750 units/s² in usual Verlet terms. Keep this: the sag numbers depend on it.
- **"Slack" is offset by -100.** Slack 0 is a rope 100 units shorter than its span, which only sags a few units because the solver doesn't converge. Keep this.
- **One-directional Gauss-Seidel sweeps make sag asymmetric.** The low point is nearer the start. Keep this (it is subtle, but it is in the fixed point).
- **Texture V divisor undercounts the points:** it uses (n-1)·s + 1 instead of n + (n-1)·s. The texture is stretched about 1.4× more than TextureScale suggests for n = 10, s = 2. Keep this to match how the textures look.
- **r_ropetranslucent hides ropes entirely when 0.** Keep this only if we expose that cvar.
- **ScrollSpeed is ignored on the client.**

## Test cases

Endpoints are at (0,0,0) and (x,0,z), simulated in 32/64-bit float. A tolerance of ±0.05 units covers float32 vs float64. "Sag" is -z of a node.

| Setup | Input | After | Expected |
|---|---|---|---|
| any rope | initial hang | count steps | 251 steps of 0.02 s |
| L 256 horizontal, Slack 25, Type 0 (10 nodes) | ℓ | - | (256+25-100)/9 = 20.111 |
| same | settle (251 steps = fixed point) | node z | 0, -1.819, -2.869, -3.548, -3.859, -3.803, -3.379, -2.585, -1.420, 0 |
| same | | node x | 0, 35.102, 62.707, 90.321, 117.940, 145.562, 173.182, 200.799, 228.405, 256 |
| same, Subdiv 2 | rendered points | count | 28 |
| same, Subdiv 2 | spline points between nodes 4 and 5 | (x, z) | (127.147, -3.881), (136.355, -3.862). Deepest rendered point ≈ 3.88 sag |
| same, Subdiv 2 | spline points between nodes 0 and 1 (P0 = P1) | (x, z) | (9.378, -0.500), (22.657, -1.202) |
| L 256, Slack 25, no Type key (5 nodes) | settle | node z | 0, -0.692, -0.831, -0.561, 0 (midpoint node sag 0.831) |
| L 256, Slack 25, Type 1 (4 nodes) | settle | node z | 0, -0.455, -0.384, 0 |
| L 256, Slack 0, Type 0 | settle | node z | 0, -1.445, -2.211, -2.701, -2.918, -2.861, -2.531, -1.925, -1.042, 0 |
| L 256, Slack 100, Type 0 | 251 steps | node z | 0, -12.091, -21.163, -27.357, -30.482, -30.431, -27.204, -20.900, -11.714, 0 |
| L 256, Slack 200, Type 0 | 251 steps | node z (x) | -35.518 (19.192), -67.809, -94.672, -111.115 (108.079), -111.097 (147.854), -94.597, -67.609, -35.181 |
| L 256, Slack 200, Type 0 | fixed point (≥ 20000 steps) | node 4, node 5 z | -111.255, -111.256 |
| L 256, Slack 200, no Type (5 nodes) | fixed point | node z (x) | -74.798 (48.714), -115.394 (128.002), -74.723 (207.334) |
| L 256, Slack 200, Type 1 | fixed point | node z (x) | -96.988 (68.650), -96.956 (187.383) |
| L 256, Slack 200, Type 2 | any | nodes | straight line, no sag |
| end at (128,0,-64), Slack 125, Type 0 | L | - | int(143.108) = 143. ℓ = 168/9 = 18.667 |
| same | 251 steps | node z (x) | -17.964 (8.566), -34.599 (18.071), -50.134 (29.146), -63.930 (42.225), -74.860 (57.715), -81.189 (75.554), -81.222 (94.517), -74.904 (112.431) |
| end at (0,0,-256) vertical, Slack 25, Type 0 | fixed point | node z | 0, -35.885, -63.738, -91.492, -119.143, -146.693, -174.156, -201.483, -228.741, -256 |
| L 256, Slack 25, Type 0, Subdiv 2, TextureScale 1, texture height 128 | ΔV | - | 4·181/19/128 = 0.29770; last point V = 27·ΔV = 8.038 |
| Subdiv 12 | effective s | - | 7 |
| fixed-point check | after settling | max squared step displacement | ≤ 0.03 (resting, no wind) |
| r_ropetranslucent 0 | any rope | drawn | nothing |

## Open questions

- **Measured in CS:S (refcmp, de_dust2, 2026-10-05): rope gravity is 800, not 1500.**
  - The two overhead A-site cables (r_elec_f_1 → f_2 with 5 nodes, and f_2 → f_1 with 10 nodes, Slack 105, span 542) were compared against CS:S.
  - With gravity 1500 the 5-node cable draws 15 px low across a 1280×720 view. With 800 (the value of sv_gravity) it lands within 1-2 px everywhere.
  - So CS:S's client likely takes rope gravity from sv_gravity, or otherwise differs from the SDK 2013 rope code here.
  - The 10-node cable is still about 11 px low with 800 (it was about 60 px low with 1500). The remaining difference is unexplained: wind, a different sweep count, or something else.
  - Implementation: the test cases above use 1500 (SDK behaviour). The CS:S loader uses 800.

- **Strip builder (not in the public SDK).**
  - Side vector: is the tangent from the previous point, the next point, or the average of both?
  - Which edge gets U = 0?
  - Is per-point alpha written into the vertex colour alpha?
  - Is the strip really drawn without back-face culling?
  - Check: screenshot a rope in CS:S edge-on and with a test texture.
- **Width = full width.** This is inferred, not read from source. Check: in game, compare a Width 8 rope against an 8-unit brush at the same depth.
- **Engine point-lighting function (not public).** What exactly does it return: ambient cube plus which lights, with or without the 0.5 overbright factor or gamma? Check: put a rope in a dark area next to a bright light_spot and compare its brightness with a nearby prop.
- **Hammer's default keys for move_rope in the CS:S FGD** (from memory: Slack 25, Type 0, Subdiv 2, Width 2, TextureScale 1). These matter only when a VMF omits keys; compiled maps always carry them. Check against the user's cstrike.fgd.
- **Does CS:S ship any "<material>_back" rope materials?** That decides whether the fake anti-aliasing path is ever active. Check: list materials/cable/ in the user's install. Also check whether cable/cable.vmt is $translucent or $alphatest.
- **Exact TextureScale network quantization** (rounding mode of the 10-bit encoding). The error is under 0.01. Check the encoded value against the rope keys of a CS:S map if exactness matters.
- **Dangling rope with no NextKey:** where does the free end start? See "Which ropes exist". Check: find such a rope in a CS:S map, if any exists.
- **CS:S-specific code.** If CS:S's client DLL differs from the SDK 2013 shared rope code (e.g. different wind or rest rules), this spec would not show it.
