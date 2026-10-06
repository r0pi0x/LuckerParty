# Counter-Strike: Source: first-person view models

Source basis: Valve's public Source SDK 2013 only. I read the shared view
model entity (how its origin and angles are built from the eye each frame,
the generic lag), the "predicted" view model subclass CS:S uses (the
history-based sway), the client view model (handedness flip, attachment
re-projection, draw), the client view set-up and renderer (FOV, aspect
scaling, near plane, the view-model pass and its depth range), the player's
view calculation (eye, punch, shake), the HL2, HL2MP and TF2 weapon bases'
view-model bob, the client animating entity (animation-event firing, the
generic muzzle-flash light, the generic client-effect and brass events), the
legacy temp-entity system (CS shell ejection and its physics), the
interpolated-variable history used by the sway, the studio model header, the
weapon script parser and the vertex-lit model shader set-up (already
specified in shaders.md). CS:S's own client and weapon code (its weapon
base, muzzle flash, brass effect callbacks) and the engine (model lighting)
are **not** public. Everything that depends on them is flagged.

Checked against the user's install and a running CS:S (over RCON, see
docs/OBSERVABILITY.md): console variable names, defaults and flags
(`help <cvar>`), the server class of the view model entity
(`report_entities`), the model files `v_rif_ak47.mdl`, `v_knife_t.mdl`,
`w_rif_ak47.mdl` (attachments, animation events, illumination centre,
parsed with a throwaway reader, not in the repo), the view-model materials,
`particles/muzzleflashes.pcf`, and screenshots of handedness, firing and
running with different bob settings. No leaked CS:S/CS:GO code was used.

Status: draft

## Summary

- The view model is a separate animated model placed every rendered frame
  at the eye with the eye's angles (including view punch), then offset by a
  speed-driven **bob** and a small **sway** that lags 0.1 s behind view
  rotation. It is drawn after the world in its own pass with its own FOV
  (`viewmodel_fov` 54, a 4:3 horizontal value scaled to the screen's
  aspect), near plane 1 unit, and a depth range squeezed into the first 10 %
  of the depth buffer so it never sinks into walls.
- CS:S view models are built **left-handed**. With `cl_righthand 1` (the
  default) every bone, attachment and offset is mirrored across the eye's
  vertical plane, and triangle winding is reversed. The knife flips too.
- Firing restarts a fire sequence whose first frame carries a muzzle-flash
  event on attachment `1` and a brass-ejection effect on attachment `2`.
  The brass is a physical temp model; the muzzle flash itself is CS:S code
  that is not public (data and observations below).
- The model is lit like any vertex-lit prop: an ambient cube plus up to four
  lights, sampled at one point near the eye, through the VertexLitGeneric
  shader (shaders.md section 4). It receives no shadows.

## Units and conventions

- Distances in Source units (inches); 1 unit = 0.0254 m.
- World axes: x forward, y left, z up (right-handed). Angles in degrees as
  (pitch, yaw, roll); pitch positive looks **down**, yaw positive turns
  **left**. Direction vectors of angles (p, y, r):
  forward = (cos p cos y, cos p sin y, −sin p); right and up as usual for
  Source (right of zero angles is (0, −1, 0), up is (0, 0, 1)).
- View-model model space: origin at the eye, x forward, y left, z up (the
  model is placed with the eye's position and angles).
- Time: client render frames, `curtime` in seconds, `frametime` = seconds
  since the last frame. View-model placement runs once per rendered frame,
  outside prediction; it is not tick-based. Server tick 0.015 s is
  irrelevant here except that velocity comes from prediction.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| viewmodel_fov | 54 | deg | `viewmodel_fov` default (client, cheat-protected in CS:S) |
| default_fov | 90 | deg | `default_fov` (client, cheat); the player's normal FOV |
| vm_znear | 1 | unit | near plane of the view-model pass |
| world_znear | 7 | unit | near plane of the world pass (for comparison) |
| vm_depth_max | 0.1 | – | view-model depth range is [0, 0.1] of the buffer |
| cl_righthand | 1 | bool | `cl_righthand` default (client, archived) |
| bob_speed_max | 320 | unit/s | speed is clamped to this; speed/320 is the bob rate |
| bob_amp_per_speed | 0.005 | unit per (unit/s) | bob amplitude = 0.005 · speed |
| cl_bob | 0.002 | – | `cl_bob` (client, cheat); **no visible effect in CS:S** (measured) |
| cl_bobcycle | 0.8 | s of "bob time" | `cl_bobcycle` (client, cheat); vertical bob period |
| cl_bobup | 0.5 | fraction | `cl_bobup` (client, cheat); rising fraction of a cycle |
| bob_clamp | −7 .. 4 | – | clamp on each bob value (never reached at defaults) |
| bob_speed_slew | 320 | (unit/s)/s | max change of the speed fed to the bob |
| bob_fwd | 0.4 | unit per bob | forward offset per vertical bob (TF2 helper; HL2MP uses 0.1) |
| bob_up | 0.1 | unit per bob | world-up offset per vertical bob |
| bob_roll | 0.5 | deg per bob | roll per vertical bob |
| bob_pitch | −0.4 | deg per bob | pitch per vertical bob |
| bob_yaw | −0.3 | deg per bob | yaw per lateral bob |
| bob_right | 0.2 | unit per bob | right offset per lateral bob (TF2 helper; HL2MP uses 0.8) |
| cl_wpn_sway_interp | 0.1 | s | `cl_wpn_sway_interp` (client, **not** cheat); sway lag |
| cl_wpn_sway_scale | 1.0 | unit | `cl_wpn_sway_scale` (client, cheat); sway gain |
| sv_wpn_sway_pred_legacy | 1 | bool | (replicated, cheat) selects the history sway; 1 in CS:S |
| sv_viewmodel_lag_do_angles | 1 | bool | only used by the non-legacy lag; inactive in CS:S |
| vm_shake_scale | 0.1 | – | screen shakes move the view model at 10 % of the view's |
| attach_flash | `1` | attachment name | AK view-model muzzle attachment (index 1) |
| attach_brass | `2` | attachment name | AK view-model ejection attachment (index 2) |
| brass_speed | 150 | unit/s | AK brass base speed (from the model event) |
| brass_life | 10 | s | brass lifetime before fading |
| brass_fade | 2 | s | fade after lifetime (alpha 255 → 0 at 0.5/s of life) |
| mflight_radius | 32..64 (int) | unit | generic SDK muzzle-flash model light (CS:S unverified) |
| mflight_life | 0.05 | s | generic SDK muzzle-flash light lifetime |
| mflight_color | (255, 192, 64), exponent 5 | – | generic SDK muzzle-flash light colour |
| muzzleflash_light | 1 | bool | `muzzleflash_light` (client, archived) |
| cl_ejectbrass | 1 | bool | `cl_ejectbrass` (client) |
| r_drawviewmodel | 1 | bool | `r_drawviewmodel` (client, cheat) |

## Behavior

### 1. Placement relative to the eye

Inputs each rendered frame: the eye position E and eye angles A exactly as
the camera uses them, i.e. after stair smoothing, view roll (`sv_rollangle`
0 in CS:S, so none), **view punch added to the angles**, screen shake and
prediction-error smoothing. So recoil kick rotates the gun together with
the view.

1. Start with origin O = E and angles Q = A.
2. Add the weapon bob (section 3) to O and Q.
3. Add the sway (section 4) to O.
4. Apply any active screen shake again at 10 % amplitude (none in normal
   play; explosions shake).
5. The model is placed with origin O and angles Q. Its bones are then
   mirrored if handedness requires (section 2).

The model's own origin is the eye: in the model files the gun hangs in front
of and below their origin (AK hull −8.6..29.8 forward, −2.3..10.0 left,
−12.7..0 up; knife −1.1..17.9, −16.0..10.6, −14.6..4.9).

The view model is not drawn when: `r_drawviewmodel` is 0; the local player
is drawn in third person; entities are not drawn; or the view is through
another entity that is not a player (e.g. a camera). It is drawn while
observing a player in first person (in-eye).

### 2. Handedness (`cl_righthand`)

- Weapon scripts carry `BuiltRightHanded` (default 1) and `AllowFlipping`
  (default 1). A view model is mirrored when
  `AllowFlipping ≠ 0` and `BuiltRightHanded ≠ cl_righthand`.
- CS:S view models are built left-handed: with `cl_righthand 0` the AK sits
  on the left of the screen, as in the file; with `cl_righthand 1` (default)
  it is on the right. The knife behaves the same: blade in the left hand at
  0, right hand at 1. (Observed in CS:S; implies the scripts say
  `BuiltRightHanded 0` for both.)
- What is mirrored: every bone's final world transform is taken into
  **camera space** (the player's view origin and angles, not the view
  model's), its lateral (left/right) component is negated, rotation and
  translation alike, and it is taken back to world. The mirror plane is the
  camera's vertical forward plane through the eye.
  - Because it acts on the final bones, it mirrors everything: the model
    mesh, the bob's lateral offset and yaw, the sway's lateral offset, and
    the attachments (so the flash and brass come out of the mirrored gun).
  - The mirrored mesh has reversed winding; it is drawn with the opposite
    cull mode.
- Applied every frame; changing the cvar takes effect immediately. It is
  purely client-side and per viewer (in-eye spectating uses the viewer's
  setting).

### 3. Bob

There is **no camera bob**: the eye does not move with walking. Only the
view model bobs. CS:S's bob code is not public. The SDK has two versions;
CS:S measurements fit the cvar-driven one (TF2's helper), described here as
the primary model. The HL2MP version is given as the alternative.

**State** (kept between frames): bob time T_b (starts 0), last time
t_last (starts 0), last speed s_last (starts 0), and the two outputs V
(vertical) and L (lateral).

**Per frame** (frame time `frametime`, current time t):

1. If `frametime` = 0 (paused), keep V and L from the last frame and stop.
2. s = horizontal speed of the local player, √(vx² + vy²) (vertical speed is
   ignored).
3. Slew limit: Δ = max(0, (t − t_last) · 320);
   s ← clamp(s, s_last − Δ, s_last + Δ); then s ← clamp(s, −320, 320);
   s_last ← s.
4. Rate r = s / 320. T_b ← T_b + (t − t_last) · r; t_last ← t.
   So T_b = (distance travelled) / 320 at steady speed: the bob is tied to
   distance, not time.
5. Cycle shape: P = `cl_bobcycle` (use 0.01 if ≤ 0), U = `cl_bobup` (use
   0.01 if ≤ 0). For a phase c in [0, 1):
   θ(c) = π·c/U if c < U, else π + π·(c − U)/(1 − U).
   With U = 0.5, θ = 2πc (a plain sine).
6. Vertical: c_v = frac(T_b / P) (T_b ≥ 0, so truncation and floor agree);
   V = 0.005·s·(0.3 + 0.7·sin θ(c_v)), then clamp to [−7, 4].
7. Lateral: c_l = frac(T_b / (2P)) (half the frequency);
   L = 0.005·s·(0.3 + 0.7·sin θ(c_l)), clamp to [−7, 4].

At defaults, the speed clamp 320 limits V and L to [−0.64, 1.6]; the
[−7, 4] clamp never acts. `cl_bob` is not used (measured: setting it to 0
or 0.02 left the CS:S bob unchanged).

**Applying it** (forward f and right r are taken from the view-model angles
Q **before** this step changes them):

- O ← O + f · 0.4V
- O.z ← O.z + 0.1V (world up, not view up)
- roll ← roll + 0.5V; pitch ← pitch − 0.4V (positive V tilts the muzzle up)
- yaw ← yaw − 0.3L
- O ← O + r · 0.2L

Mean values are positive (0.3 of the amplitude), so while moving the gun is
pushed forward, up and to the right (before mirroring; on screen with the
default flip: up and toward the screen centre/left), and it oscillates
around that. The vertical cycle repeats every 0.8 · 320 = 256 units of
travel and the lateral every 512 units, at any speed.

**Alternative (HL2MP base, if CS:S turns out to match it):** same formula
but P = 0.45 and U = 0.5 fixed (cvars ignored), no slew limit, forward
offset 0.1V instead of 0.4V and right offset 0.8L instead of 0.2L; state is
shared by all weapons.

**What resets it:** nothing in normal play. T_b never resets; when the
player stops, s (after the slew limit) falls to 0 within s/320 seconds and
V, L follow it to 0, freezing the phase. In TF2's version the state belongs
to the weapon, so each weapon keeps its own phase and, when re-drawn after
a while, its first frame sees a large t − t_last: no slew limit applies and
T_b jumps by (t − t_last)·r (a phase jump, invisible because it happens at
the current speed). CS:S may keep one state per weapon or one shared state
(Open question 2).

**In the air:** no ground check. The bob keeps cycling at the horizontal
speed while jumping or falling.

**Frame-rate dependence:** none beyond sampling; T_b integrates real time
and the slew limit is per second.

### 4. Sway (lag behind view rotation)

CS:S uses the history-based sway (`sv_wpn_sway_pred_legacy 1`, view model
entity class "predicted_viewmodel"). The older spring lag in the SDK base
(catch-up speed 5, max lag 1.5, pitch-dependent offsets) is **not** used in
CS:S.

Per frame, after the bob (so the angles include the bob's pitch/yaw/roll):

1. If `cl_wpn_sway_interp` (I) is 0, no sway.
2. Append the current view-model angles Q to a history with timestamp t.
   Drop entries older than t − I − 0.05.
3. Q_lag = the history's value at time t − I:
   - between two samples: spherical interpolation of the two orientations
     (angles → quaternions → slerp → angles), at fraction
     (t − I − t_old)/(t_new − t_old);
   - if t − I is older than every sample (the first 0.1 s): the oldest sample;
   - if the history has one sample: that sample.
4. D = Q_lag − Q, per component (pitch, yaw, roll as plain differences, no
   wrapping; the trigonometry below makes ±360° harmless).
5. g = forward vector of the angles −D = (cos(−D_p)·cos(−D_y),
   cos(−D_p)·sin(−D_y), −sin(−D_p)) (roll does not affect forward).
6. k = `cl_wpn_sway_scale` · ((1, 0, 0) − g).
7. O ← O + f·k.x + r·(−k.y) + u·k.z, where f, r, u are the forward, right and
   up vectors of Q.

Effect: turning left by δ degrees over the last 0.1 s moves the gun right by
sin δ and forward by 1 − cos δ (units, scale 1); looking down by δ moves it
up by sin δ. The offset is at most ≈ 1.4 units (90° turn in 0.1 s) at
scale 1. Steady view → zero offset. **Before the handedness mirror.** The
mirror (section 2) then negates the lateral part, so with the default flip
the gun moves to the left when turning left (Open question 4).

Frame-rate: one history sample per rendered frame; the lag is a fixed 0.1 s
of real time regardless of frame rate. Sway is client-only and cosmetic.

### 5. Projection, FOV and staying out of walls

**FOV.** Each frame:

1. fov_vm = `viewmodel_fov` − (`default_fov` − fov_player), where fov_player
   is the player's current FOV (90 normally; smaller when zoomed). So the
   view model zooms with the player by the same number of degrees.
2. Both the world FOV and fov_vm are horizontal values for a 4:3 screen.
   Each is converted for the real aspect a = width/height:
   fov' = 2·atan(tan(fov/2) · a/(4/3)).
   - The world FOV uses a capped aspect when `sv_restrict_aspect_ratio_fov`
     requires (≤ 1.85:1 in windowed multiplayer by default); the view model
     always uses the real aspect.
   - So the vertical view-model FOV is constant: 2·atan(tan 27° · 3/4) =
     41.828° for every aspect; horizontal 54° at 4:3, 62.886° at 16:10,
     68.382° at 16:9.
3. The projection is a normal symmetric perspective with that horizontal
   FOV, the screen's aspect, near 1 unit, far = the world's far plane.

**Pass.** The world is drawn first. Then, without clearing depth, the view
model is drawn with its own projection and the depth range mapped to
[0, 0.1] of the buffer, with depth test and write on. Opaque view-model
parts first, then translucent ones. Then the depth range is restored.

- Consequence: any view-model fragment has depth ≤ 0.1; a world fragment
  only beats it if its own depth is below that, i.e. closer than about
  7/0.9 ≈ 7.8 units from the eye (world near plane 7). The player hull keeps
  walls 16 units away horizontally, so in practice the gun never clips.
- The view model is not frustum-culled against the world view.

**Attachment positions for effects.** Effects attached to view-model
attachments (flash, brass) are spawned in the world pass, so attachment
positions are re-projected: in camera space (right, up, forward), the right
and up components are multiplied by tan(fov_world/2) / tan(fov_vm/2) (the
aspect cancels), forward is kept. At 90/54 the factor is 1/tan 27° =
1.96261. A world-pass object at the re-projected point appears on screen
exactly where the attachment appears in the view-model pass.

### 6. Animation events that drive effects

- Every shot restarts a fire sequence at cycle 0 (weapons.md 3.8). A restart
  always re-arms the sequence's events, even when the same sequence is
  picked again, and events at cycle 0 fire on the first frame.
- Events fire on the frame the client's cycle passes them (between the
  previous frame's cycle and the current one). Only events numbered 5000+
  or flagged client-side are handled by the client.
- View-model events are first offered to the weapon (CS:S's weapon code, not
  public); what it does not consume falls back to the generic handling below.
- `v_rif_ak47.mdl` data:
  - attachment 1, named `1`, on bone `v_weapon.AK47_Parent`, at (0, 3.5, 19)
    in that bone's space, identity rotation (the muzzle);
  - attachment 2, named `2`, same bone, at (0, 4.1, 5.0), rotated (axis
    columns x = (−0.837, 0.224, 0.5), y = (−0.259, −0.966, 0), z = (0.483,
    −0.129, 0.866)) (the ejection port);
  - `ak47_fire1..3`: at cycle 0, event 5001 with option `1` (muzzle flash
    on the first attachment) and a named client-effect event
    (AE_CLIENT_EFFECT_ATTACH) with options `EjectBrass_762Nato 2 150`
    (effect name, attachment 2, parameter 150 = brass speed);
  - `ak47_draw`: sound `Weapon_AK47.BoltPull` at cycle 0.3667;
    `ak47_reload`: `Weapon_AK47.Clipout` at 0.1444, `Weapon_AK47.Clipin`
    at 0.6333 (sound events, played to the owner only, from the view
    model's origin).
- `v_knife_t.mdl` has no attachments and no events.
- The world model `w_rif_ak47.mdl` has one attachment, `muzzle_flash`, on
  bone `ValveBiped.flash`, rotated so its forward (x) is the bone's z.

### 7. Muzzle flash

What is known for certain:

- In CS:S builds, the SDK's generic sprite muzzle flash from model events
  5001/5011/... does **nothing**; CS:S's own code (weapon or player) makes
  the flash. It is not public.
- Data in the install that CS:S's flash can use:
  - `materials/effects/muzzleflashx.vmt` (CS:S's own): UnlitGeneric,
    additive, vertex colour and vertex alpha, texture
    `effects/muzzleflashX`.
  - `materials/effects/muzzleflash1..4.vmt` and `_noz` variants (shared
    HL2): UnlitGeneric additive with vertex colour/alpha; the `_noz` ones
    ignore depth.
  - Weapon scripts' `MuzzleFlashScale` and `MuzzleFlashStyle` (names in
    weapons.md section 1; values not yet read).
  - `particles/muzzleflashes.pcf` (preloaded by the particle manifest)
    defines `muzzle_pistols`, `muzzle_smgs`, `muzzle_machinegun`,
    `muzzle_rifles`, `muzzle_shotguns`, `muzzle_autorifles`. All six:
    12 particles at once, lifetime 0.01–0.1 s, radius 3–5, random roll,
    offset 0..10 along **world** y, alpha fading in over the first half of
    life and out over the second, material
    `particle/muzzleflash/noisecloud1.vmt` (additive sprite card).
    Their colours are debug-looking pairs (e.g. rifles (255, 180, 0) to
    white; pistols green to yellow; machinegun to blue), which suggests
    they are unused (Open question 5).
- Observed in CS:S (AK, slowed time): on the shot frame a short bright
  plume extends forward from the muzzle, the gun is kicked up with the view
  punch, and a casing leaves the ejection side.

Generic SDK behaviour that applies if CS:S keeps it (unverified):

- When an entity's muzzle-flash counter changes and `muzzleflash_light` is
  1, a **model-only** light (lights studio models, not world surfaces) is
  placed at the entity's attachment 1: radius a random integer 32–64,
  colour (255, 192, 64) with exponent 5, it dies after 0.05 s and its radius
  shrinks linearly to 0 over that time (decay = radius / 0.05 per second).
- First person vs world: the view-model flash is spawned at the view
  model's (re-projected) muzzle attachment and seen only by its owner;
  others see the world model's `muzzle_flash` attachment.

### 8. Shell ejection

From the CS shell code that is in the SDK (the effect callbacks that call it
are CS:S code; the SDK template mod's callback is used as reference):

- Trigger: the fire sequence's `EjectBrass_<type> <attachment> <speed>`
  event at cycle 0, if `cl_ejectbrass` is 1. Origin and orientation: the
  view model's attachment 2, re-projected (section 5) and mirrored with the
  model.
- Shell model by type: 9mm `models/Shells/shell_9mm.mdl`, 57
  `shell_57.mdl`, 12gauge `shell_12gauge.mdl`, 556 `shell_556.mdl`,
  762nato `shell_762nato.mdl`, 338mag `shell_338mag.mdl` (same folder).
  Bounce sounds: pistol shells `Bounce.PistolShell`, rifle shells (556,
  762nato, 338mag) `Bounce.RifleShell`, 12gauge `Bounce.ShotgunShell`.
- Initial velocity, with f, r, u from the attachment's angles:
  v = f · speed · U(1.2, 2.8) + u · U(−10, 10) + r · U(−20, 20) + shooter's
  velocity. AK: speed 150 → 180..420 units/s forward.
- Initial angles: the shooter's eye angles. Spin: pitch and yaw rates
  U(−256, 256) deg/s, roll 0.
- Drawn in the **view-model pass** (its projection and depth range) until
  its first collision, then in the normal world pass.
- Each frame dt: prev = position; position += v·dt; angles += spin·dt;
  trace prev → position against solid world and entities (not the shooter).
  - No hit: v.z −= 800·dt (full `sv_gravity`; the 0.4 gravity factor the
    code sets is not used by this physics path).
  - Hit: position = hit point; damping 0.5; if the surface normal z > 0.9
    and −2400·dt ≤ v.z ≤ 0 (i.e. 0 ≥ v.z ≥ 3 × this frame's gravity
    step) the shell stops: v = 0, spin and gravity stop, pitch and roll set
    to 0. Otherwise v is reflected about the normal, yaw negated, then
    v ×= 0.5 and angles ×= 0.9. On every hit a bounce sound plays with
    probability 1/6 (the CS shell types are not treated as "casings" by
    the sound code), at the sound entry's volume × min(1, |v.z|/450)
    (v.z at impact), and with a random pitch from the entry's range one
    time in four, else its base pitch.
- Life 10 s; then alpha = 255 · (1 − 0.5·age_past_death), removed at 0
  (2 s fade).

### 9. Lighting

Shader and material (data):

- AK (`rif_ak47skin1.vmt`) and knife (`knife_t.vmt`): VertexLitGeneric with
  `$envmap env_cubemap` and an envmap mask texture. Hands (`v_hands.vmt`):
  VertexLitGeneric with `$bumpmap` (per-pixel lit variant), `$envmap
  env_cubemap`, mask in the normal map's alpha
  (`$normalmapalphaenvmapmask 1`). All per shaders.md section 4 (ambient
  cube by squared normal + up to 4 lights with distance attenuation, linear
  space, no overbright factor on dynamic/ambient light).

Inputs from the engine (closed source; inferred from the model format and
the engine's console variables):

- One lighting sample point per model per frame. By default it is the
  model's illumination centre (a point stored in the model header,
  transformed by the model's placement). For the view models:
  - `v_rif_ak47.mdl`: (10.60, 3.87, −6.35) in model space = 10.6 forward,
    3.9 left, 6.4 below the eye;
  - `v_knife_t.mdl`: (8.42, −2.68, −4.87);
  - (`w_rif_ak47.mdl`: (8.59, 0.17, −0.12).)
  The handedness mirror acts on bones, not on the placement, so the sample
  point is not mirrored. Either way it is within ≈ 13 units of the eye:
  **the view model is lit by the light at the player's eye**.
- At that point the engine builds an ambient cube from the map's baked
  ambient samples and picks up to `r_worldlights` (4) of the strongest
  static world lights (`light`, `light_spot`, `light_environment`) that can
  see the point, plus active dynamic lights; the rest is folded into the
  ambient cube. The same "light at a point" query lights props and ropes
  (ropes.md Open questions).
- Related engine cvars (defaults read from CS:S): `r_worldlights` 4,
  `r_ambientboost` 1, `r_ambientmin` 0.3, `r_ambientfactor` 5,
  `r_ambientfraction` 0.1 (boost the ambient cube, by at most ×5, when it is
  below 0.3 and below 0.1 × the direct light), `r_lightinterp` 5 (model
  lighting changes are smoothed over time), `r_lightaverage` 1,
  `r_worldlightmin` 0.0002.
- The view model receives no render-to-texture shadows (shadows_sky.md) and
  casts none in first person.
- The env_cubemap used for its reflections is the one the engine assigns
  for the sample point (nearest `env_cubemap`; engine-side).

## Per-frame order

For one rendered frame on the client (after prediction):

1. Compute the eye: position, angles + punch, shake, smoothing; FOV.
2. View model origin/angles := eye.
3. Bob: update state from speed and time, add offsets/angles.
4. Sway: push the (bobbed) angles into the history, read the value 0.1 s
   ago, add the offset.
5. Shake at 10 %.
6. Set the model's placement; compute bones for the current sequence and
   cycle; mirror bones in camera space if flipping; compute attachments
   (mirrored, then re-projected to world-FOV positions).
7. Fire animation events between the last and current cycle (flash, brass,
   sounds).
8. Render the world (world FOV, near 7).
9. Render view models (view-model FOV, near 1, depth range [0, 0.1], reversed
   culling if flipped; also brass still in its view-model phase).

## Edge cases

- Zoomed weapons: the view model FOV drops by the same degrees as the
  player's (AUG/SG552 at fov 55 → 54 − 35 = 19° before aspect scaling).
  Snipers hide the view model while scoped (CS:S code; not covered here).
- `viewmodel_fov` range is 0.1–179.9. At 0 the attachment re-projection
  would divide by zero; the code then collapses attachments to the screen
  centre line.
- Bob with the speed limit: going from standing to 250 units/s instantly,
  the bob's speed reaches 250 after 0.78 s.
- Sway in the first 0.1 s of existence returns the oldest sample (little or
  no sway).
- Pausing (frametime 0) freezes bob output; the sway still samples.
- Spectating in-eye: the spectated player's view model is drawn with the
  viewer's own handedness.

## Quirks

- Mirroring acts on final bones in camera space, so it also mirrors bob
  lateral offset/yaw and the sway's lateral offset (keep: it is what CS:S
  does by construction; see Open question 4).
- `cl_bob` exists and is listed but does nothing (keep: don't use it).
- The bob is phase-locked to distance (T_b = distance/320), so it never
  drifts with time at constant speed.
- Shell casings fall with full gravity despite a 0.4 gravity setting, and
  are drawn with the view-model projection until their first bounce.
- The world model light from the generic muzzle flash only lights models,
  not walls (if CS:S uses it at all).

## Test cases

Bob uses the primary (cvar-driven) model, defaults P = 0.8, U = 0.5.
Offsets are before handedness mirroring. Speeds in units/s.

| Setup | Input | After | Expected |
|---|---|---|---|
| B1 T_b = 0, s_last = 250, t_last = 0 | t = 0.3125, speed 250 | 1 frame | T_b = 0.244141 (r = 0.78125) |
| B2 T_b = 0.078125, speed 250 | evaluate | – | V = 0.878832, L = 0.639255 |
| B3 as B2, angles (0, 0, 0) | apply | – | origin += (0.351533 fwd, 0.127851 right) and z += 0.087883; roll +0.439416, pitch −0.351533, yaw −0.191777 |
| B4 T_b = 0.2, speed 250 | evaluate | – | V = 1.25 (peak), L = 0.993718 |
| B5 T_b = 0.6, speed 250 | evaluate | – | V = −0.5 (trough), L = 0.993718 |
| B6 T_b = 0, speed 250 | evaluate | – | V = L = 0.375 (0.3 × 1.25) |
| B7 T_b = 0.2, speed 221 (AK) | evaluate | – | V = 1.105, L = 0.878447 |
| B8 any T_b, speed 0 | evaluate | – | V = L = 0 |
| B9 cl_bobup 0.25, T_b = 0.3, speed 250 | evaluate | – | c_v = 0.375, θ = 7π/6, V = −0.0625 |
| B10 s_last = 0, t − t_last = 0.01 | measured speed 250 | 1 frame | speed used = 3.2, s_last = 3.2 |
| B11 steady 250 units/s | run 256 units | – | vertical bob back to the same value (one cycle); lateral needs 512 units |
| B12 frametime = 0 | any | – | V, L unchanged |
| B13 alternative HL2MP model, T_b = 0.1, speed 320 | evaluate | – | V = 1.582985, L = 1.199922, fwd 0.158298, right 0.959938 |
| S1 scale 1, angles now (0, 10, 0), 0.1 s ago (0, 0, 0) | sway | – | offset = 0.015192 fwd + 0.173648 right |
| S2 now (10, 0, 0), 0.1 s ago (0, 0, 0) | sway | – | offset = 0.015192 fwd + 0.173648 up |
| S3 now (0, 36, 0), 0.1 s ago (0, 0, 0) | sway | – | offset = 0.190983 fwd + 0.587785 right |
| S4 constant angles | sway | – | 0 |
| S5 as S1 with `cl_wpn_sway_scale` 2 | sway | – | doubled |
| S6 as S1 with `cl_wpn_sway_interp` 0 | sway | – | 0 |
| S7 samples (0,0,0) at t = 0.90 and (0,20,0) at t = 1.00, now t = 1.05 with (0,30,0) | sway, I = 0.1 | – | lagged yaw = 10 (halfway), D_y = −20 → offset 0.060307 fwd + 0.342020 right |
| F1 aspect 4:3, fov 90 | – | – | vm hfov 54°, vfov 41.828° |
| F2 aspect 16:9 | – | – | vm hfov 68.382°, vfov 41.828°; world hfov 106.260° (no cap) |
| F3 aspect 16:10 | – | – | vm hfov 62.886° |
| F4 player fov 40, default 90 | – | – | vm fov = 4° (before aspect scaling) |
| F5 world 90, vm 54 | attachment at camera (2, −3, 20) (right, up, fwd) | re-project | (3.92522, −5.88783, 20) |
| D1 world surface 7.5 units ahead | draw vm | – | world can occlude the view model (depth < 0.1) |
| D2 world surface 16 units ahead | draw vm | – | view model always in front |
| H1 `cl_righthand 1`, AK or knife | draw | – | mirrored: AK on the right, knife blade in the right hand |
| H2 `cl_righthand 0` | draw | – | as in the file: AK on the left, knife blade in the left hand |
| H3 point 5 units right of the eye in camera space, flipped | mirror | – | 5 units left; winding reversed |
| E1 AK fire, same sequence chosen twice in a row 0.1 s apart | events | – | flash and brass events fire on both shots |
| E2 AK brass | spawn | – | forward speed in [180, 420] plus up ±10, right ±20, plus player velocity |
| E3 brass falling, no hit, dt = 0.01 | 1 frame | – | v.z decreases by 8 |
| E4 brass hits floor (n.z = 1), dt = 0.01, v.z = −20 | collision | – | stops (−24 ≤ −20 ≤ 0) |
| E5 brass hits floor, dt = 0.01, v = (100, 0, −200) | collision | – | v = (50, 0, 100), angles × 0.9, yaw negated |
| E6 brass at age 11 s | – | – | alpha = 127 (integer of 255 × (1 − 0.5)); removed at 12 s |
| L1 eye at (0,0,64), angles (0,0,0), AK | lighting point | – | (10.60, 3.87, 57.65) in world (illum centre via placement) |

## Open questions

1. **Bob model.** CS:S's bob is not public. Measured on de_dust2 (AK,
   walking forward, screenshots tied to `getpos` distance): cl_bob 0, 0.002
   and 0.02 looked the same; the gun's top edge peaked near 40 units and
   bottomed near 150–165 units of travel (period ≈ 256 units, i.e.
   `cl_bobcycle` 0.8, not HL2MP's 0.45 → 144 units); vertical swing ≈ 20 px
   and lateral ≈ 6 px at 720p, which fits the TF2 helper's 0.2 lateral
   gain rather than HL2MP's 0.8. Confirm by changing `cl_bobcycle` to 0.4
   (period should halve) and `cl_bobup` (shape), and by a longer straight
   run with finer sampling. The run was cut short when the game stopped
   responding to RCON (another session was using it).
2. **Bob state per weapon or shared**, and whether CS:S has a ground check.
   Check: run, switch weapon, compare phase; jump while running.
3. **Sway in CS:S.** The cvars and the entity class match the SDK's
   history sway exactly; not measured. Check: `cl_wpn_sway_scale 20`, turn
   with `setang` steps and screenshot.
4. **Mirrored sway/bob direction.** By construction the flip mirrors the
   lateral sway and bob, so with the default flip a left turn moves the gun
   left. Verify with a screenshot pair (turning left/right with a large
   `cl_wpn_sway_scale`) and with `cl_righthand 0`.
5. **Muzzle flash look.** Which material and size CS:S uses
   (`muzzleflashx` vs `muzzleflash1..4`), how `MuzzleFlashScale`/`Style`
   apply, how long it lasts, whether the `muzzle_*` particle systems are
   used, and whether there is a world-lighting dynamic light (wall glow) or
   only the model light. Best checked with a RenderDoc capture of a firing
   frame (sprite quads, sizes, blend) plus a dark-room screenshot pair with
   `muzzleflash_light 0/1`. Also read `MuzzleFlashScale`/`MuzzleFlashStyle`
   from the decrypted scripts (scratch only).
6. **Brass in CS:S.** Shell code is in the SDK under the CS build, but the
   CS:S effect callback (attachment, which velocity parameter) is not.
   Check whether casings bounce off walls/players and whether they are
   drawn oddly before the first bounce.
7. **View-model lighting point.** Whether the engine uses the model's
   illumination centre or the eye/view origin for view models, how it picks
   the 4 lights, how `r_lightinterp` smooths changes, and how the ambient
   boost works exactly. Check: stand with the eye in shadow and the
   illumination centre in light (e.g. under an edge) and compare with
   `r_DrawModelLightOrigin 1`; `r_drawlightinfo` may show the chosen
   lights.
8. **Cross-fades between view-model sequences** (animation.md section 9)
   are not covered here.
