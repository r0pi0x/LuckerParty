# Source engine: particle systems and smoke entities (info_particle_system, env_smokestack, env_particlelight, env_smoketrail)

Source basis: Valve's public Source SDK 2013 only (the current GitHub
release, which also carries TF2 code; TF2-only parts are ignored). I read
the server and client code of info_particle_system, the shared code that
loads particle manifests (global and per-map) and dispatches particle
effects, the client-side "particle property" that binds a running effect's
control points to entities, env_smokestack (server and client), the
env_particlelight helper it reads, the client sphere-particle lighting
helper and the quad drawers it uses, env_smoketrail (server and client)
and the simple-emitter base it runs on. The particle library itself
(reading `.pcf` files, operators, emitters, renderers, "skip to time") is
a binary library, not in the SDK: only its public header is. So this spec
describes the **entity side** of info_particle_system and what the engine
must provide; what an effect looks like is defined by data in the `.pcf`
file (pointers below). The SDK has two builds of the old-style smoke code:
HL2 episodic and non-episodic. CS:S is not episodic, so the non-episodic
behaviour is given (episodic differences under Quirks). CS:S's own DLLs
are not public: CS:S could override the game-rules hooks named below
(Open questions).
Status: draft

## Summary

info_particle_system is a point entity that names one effect from the
`.pcf` particle files and turns it on and off. While on, every client runs
that effect with control point 0 at the entity (following it, with its
orientation) and control points 1–63 following named entities. Turning it
off stops emission; particles already alive finish their lives. A client
that starts the effect late fast-forwards it to the time the server
started it. env_smokestack is an older hard-coded client effect: a column
of camera-facing smoke puffs that rise from a ring, spread sideways, drift
with a constant "wind" acceleration, can spiral ("twist") about the
column and fade in then out over a lifetime of JetLength ÷ Speed seconds.
env_smoketrail is a similar older emitter of grey puffs that darken to
black, normally attached to a moving object.

## Units and conventions

Source units (inches), Z up; angles (pitch, yaw, roll) in degrees;
forward/right/up(angles) are the usual Source basis vectors (angles 0 0 0:
forward +X, right −Y, up +Z). Server tick dt = 0.015 s. Particle effects
are client-side and advance once per rendered frame by the frame time Δt.
Colours: "rendercolor" is "R G B" 0–255, "renderamt" 0–255. U(a, b) is a
uniform random float, I(a, b) a uniform random integer, ends included.
"Size" of an old-style particle is its **half-width**: the quad spans ±size
around the centre in screen-aligned axes. Keyvalue, input and output names
are what Hammer shows.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| max_control_points | 64 | — | control points per effect (0–63) |
| entity_cpoints | 63 | — | info_particle_system keys "cpoint1" … "cpoint63" |
| entity_cpoint_parents | 7 | — | keys "cpoint1_parent" … "cpoint7_parent" only |
| particle_name_bits | 13 | bits | effect names are sent as an index into a string table of ≤ 8192 names |
| skip_threshold | 0.01 | s | a late client fast-forwards the effect if it is more than this behind |
| global_manifest | particles/particles_manifest.txt | — | list of `.pcf` files loaded at startup |
| map_manifest | maps/<mapname>_particles.txt | — | per-map list (also "particles.txt" inside the BSP pak, see 1.1) |
| map_manifest_max | 64 | entries | entries past this are ignored (warning) |
| smokestack_default_material | particle/SmokeStack.vmt | — | used when "SmokeMaterial" is absent |
| smokestack_full_sim | 5 | s | after creation the smokestack simulates and emits even unseen |
| smokestack_min_alpha | 0.5 | of 255 | puffs with alpha below this are not drawn |
| smoketrail_materials | particle/particle_smokegrenade, particle/particle_noisesphere | — | picked 50/50 per puff |
| smoketrail_near_fade | 64 → 128 | units (view depth) | smoketrail puffs are invisible nearer than 64, fully visible from 128 |
| smoketrail_roll_decay | 8 | 1/s | roll speed decay rate |
| smoketrail_min_roll | 0.5 | rad/s | roll speed floor |
| particlelight_default_intensity | 5000 | — | env_particlelight "Intensity" when absent |

Client-side defaults of env_smokestack (only seen if the server never sends
a value; the server sends every field, and its own defaults are 0 for the
keys below, so a map that omits a key gets 0 — see Edge cases): BaseSpread
20, SpreadSpeed 15, Speed 30, StartSize 10, EndSize 15, Rate 80, JetLength
180. env_smoketrail defaults (server): spawnrate 10, startcolor 0.5 grey,
endcolor black, lifetime 5, minspeed 2, maxspeed 4, mindirectedspeed and
maxdirectedspeed 0, startsize 35, endsize 55, spawnradius 2, opacity 0.5,
emitting.

## Behavior

### 1. Particle data the engine needs (`.pcf`)

**1.1 Which files are loaded.** At startup every `file` entry of
`particles/particles_manifest.txt` (a KeyValues text file: a block whose
entries are all named "file", value = path of a `.pcf` relative to the
game root) is read; a path may start with `!`, a precache marker (strip it
before opening; files with it are precached at once, the rest when used).
At map load a per-map manifest is looked for, first match wins:
1. `particles.txt` inside the map's embedded pak (BSP lump 40),
2. `maps/<mapname>_particles.txt` inside the pak,
3. `maps/<mapname>_particles.txt` on the game's search paths.

Map name is lower-cased, without folder and extension. Same format; only
the first 64 entries count; an entry whose folder (after stripping `!`) is
not exactly `particles` is refused with a warning. Map files are loaded
**after** the global ones, and a system name defined again replaces the
earlier definition (*inferred* from the "override" argument; Open
questions). Community maps pack their `.pcf` files under `particles/` in
the BSP pak with such a manifest. Whether the CS:S build reads the
BSP-internal names (1 and 2, a later addition) or only (3): Open questions.

**1.2 The `.pcf` format** is Valve's DMX ("Data Model eXchange") element
format, binary encoding: a header line naming the encoding and version, a
string table, then elements (each a type name, a name and a GUID) with
typed attributes; element-reference attributes link them into a tree. A
particle file's root holds a list of particle system definitions, each with
its name, base properties (max particles, initial radius, colour, material,
bounding box, control-point configuration, child systems with delays) and
four ordered lists of operator elements — renderers, operators, initializers,
emitters — each named by its function name (e.g. "Render animated sprites",
"Movement Basic", "Lifetime Decay", "Emit continuously") with its
parameters as attributes. Public references: the Valve Developer
Community pages "DMX", "DMX/Binary format" (byte layout of encodings 2–5)
and "Particle System Overview" / "List of Particle System Operators" (what
each named operator does, with its parameters). Do not take the operator
semantics from this spec; a separate spec per operator family is needed
before mashup draws `.pcf` effects (Open questions).

**1.3 Names on the wire.** The server registers every effect name an
entity will use (at spawn) in a string table; the entity sends the index.
Mashup has no network split, so the name can be resolved directly; keep
"unknown name" behaviour: the client warns and draws nothing.

### 2. info_particle_system

Keyvalues: "effect_name" (system name), "start_active" (0/1),
"flag_as_weather" (0/1), "cpoint1" … "cpoint63" (entity names),
"cpoint1_parent" … "cpoint7_parent" (control point indices, 0 = none),
plus base keys ("targetname", "parentname", "angles", "origin"). No
spawnflags. Inputs: **Start**, **Stop**, and the base inputs (**Kill**,
**SetParent**, **ClearParent**, **AddOutput**, ...). No outputs other than
the base OnUser outputs. There is **no DestroyImmediately** input in this
code base (that input, and "StopPlayEndCap", come from later engine
branches); in CS:S a map sending it gets the usual unknown-input log line
and nothing happens (Open questions).

The entity is a plain point entity: it has no model, is never solid, and
is always sent to every client (not PVS-culled), so its effect runs
wherever the viewer is.

**2.1 Spawn**: an empty "effect_name" logs a warning (the entity remains,
doing nothing visible). The name is registered (1.3).

**2.2 Activate** (after all map entities spawned): resolve the effect
index; if start_active: start (2.3) at once, so the effect's start time is
the activation time.

**2.3 Start** (input or start_active): if already on, nothing. Else: on :=
true, start time := now, and resolve control-point entities **now**: for
each cpointN with a non-empty name, the first entity whose **name**
matches (the usual name search, including `*` wildcards and `!`-names
relative to this entity), or failing that the first entity whose
**class name** matches (so "cpoint1" "player" binds the first player),
becomes the entity of control point N; not found: warning,
that slot keeps whatever entity it held before (initially none). Names are
resolved on every start, not continuously: an entity spawned after the
start is not picked up until the next Stop+Start.

**2.4 Stop**: on := false. Nothing else on the server.

**2.5 What each viewer does** (client, every time it sees the on/off
state change, and when it first receives the entity):
- **Turned on** (or first received while on): on the next client frame,
  create a new instance of the named effect, unless the game rules refuse
  map particle effects of that name or weather is disabled and
  "flag_as_weather" is 1 (base game rules allow both; CS:S: Open
  questions). Then:
  - control point 0 := this entity's position and orientation (forward,
    right, up from its angles), **following** it every frame (so a
    parented info_particle_system carries its effect along);
  - for N = 1…63 with a resolved entity: control point N := that entity's
    position and orientation, following it every frame;
  - for N = 1…7 with cpointN_parent = p ≠ 0: control point N's parent :=
    control point p (how operators use a control point's parent is defined
    by the particle library, *inferred*: offsets given relative to that
    control point; Open questions);
  - fast-forward: δ = now − start time; if δ > 0.01 s the effect is
    simulated ahead by δ before it is first drawn (a viewer joining 30 s
    after a one-shot effect started sees nothing left of it; a looping
    effect is in steady state).
- **Turned off**: stop emission of **every** effect this entity is
  running: no new particles; live particles finish their lifetimes and
  then the instance is freed.
- Start and Stop within the same server tick (or Stop+Start): the client
  sees no change of the on flag, so nothing happens on clients (the effect
  keeps running, it is not restarted; Quirks).
- Start, Stop and Start again in separate ticks: the old instance keeps
  fading out (emission stopped) while a new instance starts: two
  instances overlap for the old particles' remaining life.
- **Kill** (entity removed): every effect it runs stops emitting; live
  particles finish (*inferred*: the client destroys effects that are asleep
  at once and lets the others finish; Open questions).
- If a control point's entity is removed, that control point stops
  following and keeps its last value (*inferred*).

**2.6 Following and orientation.** Each frame before simulating, the
effect's control points are refreshed from their entities: position = the
entity's absolute origin, orientation = its forward/right/up. (Effects
started "at" a point without following are set once; info_particle_system
always uses the following mode.)

### 3. env_smokestack

Keyvalues: "InitialState" (0 off / 1 on), "BaseSpread" (units),
"SpreadSpeed" (units/s), "Speed" (units/s), "StartSize", "EndSize"
(units, half-width), "Rate" (puffs/s), "JetLength" (units), "WindAngle"
(degrees, integer), "WindSpeed" (units/s², integer), "Wind" (raw "x y z"
acceleration, overrides the two above if it comes later in the entity
text), "SmokeMaterial" (material path, ".vmt" appended if missing),
"Twist" (degrees/s), "Roll" (degrees/s), "rendercolor" (base colour,
default 0 0 0), "renderamt" (alpha scale, default 255), "angles" (the
column rises along up(angles)). Inputs: **TurnOn**, **TurnOff**,
**Toggle**, and float inputs **JetLength**, **SpreadSpeed**, **Speed**,
**Rate** (each sets that key; see Quirks for Rate), plus the base inputs
(**Color**, **Alpha** change rendercolor/renderamt). No outputs.

**3.1 Server.** Spawn: emitting := InitialState ≠ 0. Activate: find every
env_particlelight whose "PSName" equals this entity's name (exact,
case-sensitive): a directional one ("Directional" 1) fills the
directional light slot, else the ambient slot; for several lights of the
same kind, the last one in entity order wins. Each slot takes the light's
"Intensity", "Color" (R G B 0–255, divided by 255) and position. A slot
with no light has intensity 0 (no effect). Wind vector from the
keyvalues:

```math
\vec a_{wind} = (\cos\theta \cdot W,\ \sin\theta \cdot W,\ 0), \quad \theta = \text{WindAngle (deg)},\ W = \text{WindSpeed}
```

(both read as integers: "12.5" → 12). Visibility to clients: only while
the entity is in the viewer's PVS, or always if the entity is inside the
3D skybox; never if the entity has the "no draw" effect flag.

**3.2 Client: creation.** When the viewer first receives the entity:
load the one material (CS:S, non-episodic: only the named material; see
Quirks), set up the emission clock with the **current** Rate (first puff
at the first update, then one every 1/Rate s), compute lifetime T =
JetLength / Speed, and simulate/emit unconditionally for the first 5 s.
Afterwards (5 s after creation) it only emits and moves while it was
drawn in the previous frame (3.5).

**3.3 Emission**, each frame of length Δt, if emitting and (drawn last
frame or within the first 5 s): with (F, R, U) = forward, right, up of
the entity's current angles and O its current origin, for each puff due
this frame (count from the clock: carries the remainder between frames;
all puffs of a frame are born at age 0, there is no sub-frame aging):

```math
\phi = U(0, 2\pi),\quad \vec p_0 = O + \text{BaseSpread}\,(\cos\phi\,\vec R + \sin\phi\,\vec F)
```

```math
\vec v_0 = U(-s, s)\,\vec R + U(-s, s)\,\vec F + \text{Speed}\,\vec U,\quad s = \text{SpreadSpeed}
```

acceleration a = wind vector (3.1), age 0, roll angle 0, roll rate
U(−Roll, Roll) deg/s. Puffs start **on a circle** of radius BaseSpread
(not inside a disc).

**3.4 Simulation**, each frame (Δt), for each puff in order:
1. age += Δt; t = age / T (= age × Speed / JetLength). If t ≥ 1: remove,
   next puff.
2. Twist (if Twist ≠ 0): rotate the puff's horizontal offset from the
   entity origin by −Twist·Δt degrees about the world Z axis through the
   origin (positive Twist turns the column **clockwise seen from above**);
   z unchanged; velocity not rotated.
3. Move: p += v·Δt + ½·a·Δt²; then v += a·Δt.
4. Roll angle += roll rate × Δt.

While emitting, the simulation is **frozen** (no ageing, no movement) on
frames when the effect was not drawn in the previous frame and the first
5 s are over. When not emitting (TurnOff), simulation always runs, so the
column dies out even unseen.

**3.5 Drawing.** Each puff is a camera-facing quad of the smoke material,
sorted back to front, at half-width

```math
r(t) = \text{StartSize} + (\text{EndSize} - \text{StartSize})\,t
```

rotated in screen space by its roll angle, with alpha (0–255 scale)

```math
\alpha_0(t) = \tfrac12 - \tfrac12\cos(2\pi t),\qquad \alpha(t) = \text{renderamt}\cdot\begin{cases}\alpha_0 & t \le 0.5\\ \alpha_0^2 & t > 0.5\end{cases}
```

(0 at birth, full at half life, then fading faster). Not drawn when
α < 0.5. Colour (0–255 per channel), with c_base = rendercolor/255 and
d_X the distance from the puff to light slot X:

```math
\vec c = \vec c_{base} + \frac{I_{amb}}{d_{amb}^2}\vec c_{amb} + \frac{I_{dir}}{d_{dir}^2}\vec c_{dir}
```

(a term with intensity 0 is skipped; d² ≤ 0.0001 → factor 1000 instead);
the directional term is added here only if the material does **not**
declare `$USINGPIXELSHADER` (with it, the material's shader lights the
puff from the directional light: Open questions). Then if the largest
channel m > 1: c × 255/m (keeps the hue, saturates), else c × 255. With no
env_particlelight the colour is simply rendercolor. The material decides
blending (SmokeStack.vmt is a translucent sprite-card material).

**3.6 Inputs at run time.** TurnOn/TurnOff/Toggle switch emission (live
puffs carry on). JetLength and Speed change T for **all live puffs** at
once (their t jumps). SpreadSpeed affects new puffs. Rate: see Quirks.

### 4. env_particlelight

A server-only point entity read by env_smokestack (3.1); keys
"PSName" (target smokestack name), "Color" (R G B 0–255, default red
"1 0 0" when absent — effectively black-ish; Hammer writes a value),
"Intensity" (default 5000), "Directional" (0/1). Position = its origin at
the smokestack's activation. Not networked itself; no inputs.

### 5. env_smoketrail

Keyvalues: "opacity" (0–1), "spawnrate" (puffs/s), "lifetime" (s),
"startcolor", "endcolor" ("R G B"), "emittime" (s; stop emitting this long
after the map loads; absent = forever), "minspeed", "maxspeed",
"mindirectedspeed", "maxdirectedspeed" (units/s), "startsize", "endsize"
(units, half-width), "spawnradius" (units). No inputs besides the base
ones, no outputs. Usually parented to a moving object ("parentname"). Sent
to viewers in PVS like env_smokestack.

Client, each frame Δt (puffs born only if emitting and before the stop
time), with O the entity origin, V its absolute velocity, F its forward
vector; for the k-th puff due this frame at elapsed time τ_k into the
frame:
- position = O − V·Δt + V·τ_k + (U(−r, r), U(−r, r), U(−r, r)), r =
  spawnradius (spreads a frame's puffs along the path travelled);
- velocity = (U(−1,1), U(−1,1), U(−1,1)) × U(minspeed, maxspeed) + V +
  U(mindirectedspeed, maxdirectedspeed) × F;
- colour c = startcolor + (startcolor / max(startcolor)) × U(−0.2, 0.2),
  each channel clamped to 0–1, stored as bytes;
- start alpha = clamp(U(0.75, 1.25) × opacity, 0, 1) × 255 (byte);
- material: either of the two smoke materials (Constants);
- roll angle I(0, 360) used directly as **radians**; roll rate U(−1, 1)
  rad/s.

Simulation per frame: p += v·Δt (no drag, no gravity); age += Δt; roll
angle += rate·Δt; rate += rate × (−8·Δt), then if |rate| < 0.5: rate :=
±0.5 (sign kept; 0 → −0.5); remove when age ≥ lifetime. Drawing at life
fraction f = age/lifetime: half-width = startsize + (endsize −
startsize)·f; colour = c × (1 − f) (fades to black; **endcolor is
ignored**, Quirks); alpha = start alpha/255 × sin(π f) × near fade, where
near fade = 0 for view depth ≤ 64, (depth − 64)/64 up to 128, 1 beyond;
not drawn when alpha < 0.001.

## Per-tick order

Server (each tick): 1. event queue delivers inputs (Start/Stop,
TurnOn/TurnOff, Rate/Speed/...) in firing order (specs/source/entity_io.md);
2. entity state is sent to clients at the end of the tick.

Client (each rendered frame): 1. apply received entity changes (on/off
transitions schedule a create for this frame's think or stop emission at
once; smokestack data changes recompute T); 2. client thinks: pending
info_particle_system creations run (create, bind control points,
fast-forward); 3. control points of following effects are refreshed from
their entities; 4. every effect is updated (emission, then simulation;
for env_smokestack: emission in its update step, then the simulate pass
over all puffs including those just emitted, so a new puff ages by Δt in
its first frame, *inferred*: the particle manager's call order is in a
library not fully read); 5. drawing, sorted.

## Edge cases

- **info_particle_system with an unknown effect name**: nothing is drawn;
  warning per start. Entity otherwise works (inputs accepted).
- **cpointN naming a missing entity**: warning, slot empty; the effect
  still runs; the particle library then uses that control point's default
  (origin, *inferred*).
- **cpoint parents beyond 7** are not keyvalues (cpoint8_parent is
  ignored as an unknown key).
- **Parented info_particle_system**: follows its parent through control
  point 0 (and, as a point entity, moves with it on the server).
- **env_smokestack with a key omitted**: the server's value is 0, sent to
  clients: Speed 0 → T = JetLength/0 → t = 0 forever: puffs never die and
  are never visible (α = 0) but accumulate (Quirks: cap them); JetLength 0
  → t = ∞ → every puff dies on its first simulate; Rate 0 → interval
  1/0: one puff at the first update, then never again; BaseSpread 0 → all
  puffs start at the origin.
- **"SmokeMaterial" given without ".vmt"**: the extension is appended.
- **env_smokestack outside the viewer's PVS**: not sent; when the viewer
  first sees it, it is created then (empty column building up for the
  first JetLength/Speed seconds, emitting even unseen for 5 s).
- **env_smoketrail "emittime" 0 or absent**: absent = never stops; an
  explicit 0 sets the stop time to the load time, which stops emission at
  once if the load time is > 0 (Open questions).
- **startcolor black** for env_smoketrail: the jitter divides by 0 → NaN →
  clamped; treat as black (*inferred*).

## Quirks

- **Start+Stop in one tick is invisible to clients** (no restart). Keep:
  maps that "restart" an effect with Stop then Start in the same tick
  see no restart in CS:S. Mashup, being single-process, must compare the
  on flag at end of tick, not react per input.
- **Stop does not kill particles**; only emission stops. Keep.
- **Late joiners fast-forward** the effect. Keep (cheap: simulate δ
  seconds in a few large steps or skip one-shot effects whose lifetime has
  passed).
- **env_smokestack Rate input does not change the emission rate** on a
  client that already has the entity (the clock interval is set once at
  creation; later Rate values only matter for clients that create the
  entity afterwards). Keep for fidelity; low impact.
- **Speed/JetLength inputs rescale every live puff's age fraction** at
  once (visible pop). Keep.
- **Smokestack freezes when not looked at** (after the first 5 s): no
  emission, no motion. Mashup may simulate always (cheaper to reason
  about, visually the same while looking); note the difference.
- **Twist rotates positions, not velocities**, so a twisting column also
  spreads outward; positive Twist is clockwise from above.
- **WindAngle/WindSpeed are integers**; "Wind" can override them.
- **env_smoketrail ignores "endcolor"**: puffs darken from their start
  colour to black. Keep.
- **env_smoketrail roll uses degrees-range values as radians**: random
  start angle in effect. Keep.
- Episodic HL2 builds (not CS:S) differ: smokestack up to 8 materials
  (material name with trailing digit replaced by 1, 2, ...), random start
  roll, simpler integration (p += v·Δt + a·Δt), quarter rate with
  "mat_reduceparticles". The server still precaches the numbered
  materials in all builds; CS:S's client draws only the first.

## Test cases

| Setup | Input | After | Expected |
|---|---|---|---|
| info_particle_system start_active 1, effect "x" | map load | activation | on; start time = activation time; control points resolved |
| same, start_active 0 | Start at t = 10 s | — | on, start time 10 |
| on | Start | — | nothing (start time unchanged) |
| on, cpoint1 = "a" (exists), cpoint2 = "b" (missing) | Start | — | CP1 follows a; CP2 empty, one warning |
| cpoint1 = "info_target", no entity named so, one info_target exists | Start | — | CP1 follows that info_target (class-name fallback) |
| on | Stop then Start in the same tick | client | no change on the client: the instance keeps running |
| on since t = 0 | viewer creates the entity at t = 12 | first frame | effect simulated ahead by 12 s before drawing |
| on since t = 0 | viewer gets it at t = 0.005 | — | no fast-forward (0.005 ≤ 0.01) |
| on | Stop | client | emission stops; live particles continue until their lifetimes end |
| cpoint1_parent 2 | Start | — | CP1's parent = CP2 |
| map with manifest particles.txt in pak listing "particles/a.pcf" and "models/b.pcf" | load | — | a.pcf loaded; b.pcf refused (not under particles/) |
| manifest with 70 file entries | load | — | first 64 loaded, warning |
| smokestack Speed 30, JetLength 180 | — | — | T = 6 s |
| same, StartSize 20, EndSize 30 | puff age 3 s | — | t = 0.5, half-width 25, α = renderamt (255) |
| same | age 1.5 s | — | t = 0.25, α₀ = 0.5 → α = 127.5 |
| same | age 4.5 s | — | t = 0.75, α₀ = 0.5 → squared 0.25 → α = 63.75 |
| same | age 0.0001 s | — | α ≈ 0 < 0.5 → not drawn |
| smokestack Rate 20, T = 6 s, emitting, always visible | steady state | — | ≈ 120 puffs alive |
| Rate 20, first frame Δt = 0.2 s | first update | — | 1 puff at once, then 4 more due (0.2 s / 0.05 s; clock remainders carry) → 5 puffs, all age 0 |
| WindAngle 90, WindSpeed 10 | — | — | a = (≈0, 10, 0) units/s² |
| puff born at rest relative to wind only (SpreadSpeed 0, Speed 0 ignored for y) | 2 s | — | y displacement ½·10·2² = 20 units (exact for constant Δt) |
| Twist 90, puff at offset (r, 0) from origin, no velocity sideways | 1 s of frames | — | offset ≈ (0, −r) (clockwise from above) |
| WindSpeed "12.5" | load | — | treated as 12 |
| rendercolor 128 128 128, no particlelight | draw | — | colour (128, 128, 128) |
| rendercolor 0 0 0, ambient light Color "255 0 0" Intensity 10000, puff 100 units away | draw | — | c = (0 + 10000/100² × 1, 0, 0) = (1, 0, 0) → (255, 0, 0) |
| same, puff 50 units away | draw | — | c.r = 4 → max 4 > 1 → scaled to (255, 0, 0) |
| rendercolor 200 100 0, ambient (255 255 255) at intensity giving +0.5 | draw | — | (0.784+0.5, 0.392+0.5, 0.5) max 1.284 → ×255/1.284 → (255, 177, 99) |
| smokestack Speed 0 (key omitted) | 10 s | — | puffs never die, never visible |
| smoketrail startcolor 128 128 128, opacity 0.5 | puff | — | colour channels U-jittered within [0.302, 0.702]; start alpha byte in [95, 159] |
| smoketrail lifetime 5 | puff age 2.5 s | — | f = 0.5: colour × 0.5, alpha = start × sin(π/2) = start; half-width (35+55)/2 = 45 with defaults |
| smoketrail puff 96 units in front of the camera | draw | — | near fade (96−64)/64 = 0.5 |
| smoketrail roll rate 1 rad/s, Δt = 0.01 | one step | — | rate = 1 − 0.08 = 0.92 |
| roll rate 0.52, Δt = 0.01 | one step | — | 0.52 × 0.92 = 0.478 < 0.5 → 0.5 |

## Open questions

1. **CS:S game-rules filters**: does CS:S refuse any map particle effect
   or weather-flagged effect (the base allows all)? Place an
   info_particle_system with flag_as_weather 1 on a test map and look.
2. **Per-map manifest locations in CS:S**: does the current CS:S build
   read `particles.txt` / `maps/<map>_particles.txt` from the BSP pak, or
   only from loose files? Pack a test effect both ways and check the
   console ("Successfully loaded particle effects manifest" is printed in
   developer mode).
3. **Redefinition**: when a map `.pcf` defines a system with the same name
   as a stock one, which wins? Pack a recoloured copy of a stock effect
   under its stock name and look.
4. **Operator semantics** of `.pcf` effects (emitters, initializers,
   operators, renderers, children, control-point parents): needs its own
   spec from public documentation plus measurement (record a stock effect
   frame by frame with `host_timescale` 0.1 and compare).
5. **Kill while running**: do live particles finish or vanish at once?
   Kill an info_particle_system running a long-lived effect and watch.
6. **DestroyImmediately** and other later-branch inputs: confirm CS:S logs
   them as unknown (developer 2 console) and nothing happens.
7. **env_smokestack directional light with a pixel-shader material**: how
   does the material's shader use the directional light (SmokeStack.vmt
   declares the shader)? Compare a smokestack with a directional
   env_particlelight under and over it.
8. **Hammer defaults** of env_smokestack keys (the FGD is not in the SDK):
   decompile nothing; read the keyvalues Hammer writes for a fresh
   env_smokestack, or check a community map's entity lump.
9. **Smokestack in the 3D skybox**: drawn scaled with the skybox like
   other skybox entities? Look at a map with one in its skybox.
10. **emittime 0** on env_smoketrail: emits or not? Place one with
    emittime 0 and look.
11. **Network quantization** of env_smoketrail spawnrate (8 bits over
    1–1024) and lifetime (16 bits over 0.1–100): the effective rate is
    coarser than typed (e.g. 10/s may arrive as ≈9 or ≈13/s). Count puffs
    per second in game with a known spawnrate.
