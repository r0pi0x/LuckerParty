# Counter-Strike: Source: bullet tracers and whizz sounds

Source basis: Valve's public Source SDK 2013. I read the shared bullet-firing
routine's tracer step and its tracer start-point helper, the entity and
weapon tracer dispatch, the shared tracer-effect sender, the client tracer
effect handlers (tracer, particle tracer, tracer sound), the client tracer
drawing helpers (the moving "discreet line" and the static player line), the
bullet near-miss sound helper, and the SDK template mod's CS-derived bullet
routine (which draws no tracers). CS:S's own client bullet code is **not**
public, so which of these paths CS:S takes, and how often, is marked
*inferred* or listed in Open questions. Data from the user's install (values
only): the tracer materials and texture headers in the HL2 packages CS:S
mounts, CS:S's sound-script manifest and entries, HL2's near-miss sound
entry, the view-model and world-model attachments (view_models.md), and the
player-visible console variable names `r_drawtracers` and
`r_drawtracers_firstperson`.
Status: draft

## Summary

- Not every bullet draws a tracer: a global shot counter draws one every
  N bullets (generic default N = 4 per bullet-firing call, HL2 weapon base
  N = 2; CS:S's N is Q1). A shot that broke glass draws none.
- A tracer is a thin, short streak (64–128 in long, ~1.5–1.8 in wide,
  additive `effects/spark`, warm white) that flies from the gun's muzzle
  attachment (view model for your own gun, world model for others) to the
  hit point at 5000 in/s; nothing is drawn if the shot is shorter than
  256 in.
- Whizz-by sounds come from tracer bullets only, when the bullet's line
  passes within 24 in of the listener, at most one per 0.1 s. CS:S does not
  load the sound entry they use, so CS:S has no bullet whizz (Q4).

## Units and conventions

- Distance inches; Source axes X forward, Y left, Z up. Time seconds.
- "Frame" = one client render frame (the tracer animates on frame time, not
  ticks).
- `U(a, b)` uniform float.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| tracer_freq_generic | 4 | bullets | default in the shared bullet-firing parameters |
| tracer_freq_hl2_weapon | 2 | bullets | generic weapon base's primary attack |
| tracer_attachment | 1 | index | muzzle attachment used in multiplayer |
| tracer_speed | 5000 | in/s | streak speed when none is given |
| tracer_min_distance | 256 | in | shorter shots draw no streak |
| tracer_length | U(64, 128) | in | streak length |
| tracer_half_width | U(0.75, 0.9) | in | half-width of the core quad (full width 1.5–1.8 in) |
| tracer_material | `effects/spark` | – | unlit, additive, vertex colour; texture 32 × 64, reflectivity (0.089, 0.078, 0.064) |
| tracer_min_px | 0.5 | px | half-width floor on screen ("extra visibility" mode) |
| tracer_outline_scale | 2 | × | soft outline quad width |
| tracer_outline_grey | 64/255 | – | outline brightness (core 255) |
| own_line_min | 256 | in | own-player static line: minimum shot length |
| own_line_offset | 8 + U(−24, 64) | in | own static line starts this far along the shot |
| own_line_length | U(0.1, 0.6) × shot length | in | |
| own_line_width | U(0.5, 0.75) | in | full width (half = ½ of it) |
| own_line_life | 0.01 | s | |
| own_line_shift | 4 right, −0.5 up | in | own static line start offset from the shot start |
| sp_tracer_offset | (0, 0, −4) + 2·right + 16·forward | in | single-player start offset (not used in multiplayer) |
| whiz_distance | 24 | in | bullet line must pass this close to the listener |
| whiz_listener_drop | 24 | in | listener point may slide down this far toward the bullet line |
| whiz_interval | 0.1 | s | minimum time between whizzes (global, per client) |
| whiz_entry | `Bullets.DefaultNearmiss` | – | HL2: static channel, volume 0.7, 140 dB, 8 waves `weapons/fx/nearmiss/bulletLtoR*.wav` |

## Behavior

### 1. Which bullets draw a tracer

- Each bullet trace (each pellet of a shotgun counts) increments one
  counter shared by every shooter in the process; a tracer is drawn when
  `counter mod N == 0` before the increment, and N ≠ 0. The counter is never
  reset (it survives rounds and maps), so which bullet of a burst gets the
  tracer depends on everything fired before.
- No tracer for a bullet that broke a pane of breakable glass on its way.
- In the shared routine the tracer is only made where "server effects" are
  allowed; in multiplayer the client predicts its own shots and the server
  suppresses events back to the shooter, so every client draws the tracers
  of the shots it simulates (its own predicted shots, and others' shots from
  the fire-bullets event). Each client therefore has its own counter phase.
- *CS:S (Q1)*: weapon scripts carry no tracer key; CS:S's client fires from
  the fire-bullets event as the template does, so N and the counter are
  CS:S code. Hypotheses: H1 one tracer every 4 bullets per client (shared
  default), H2 every 2, H3 per-weapon values.

### 2. Start and end points

- End: the bullet trace's end point (where it hit, or its maximum range).
  Underwater: the end becomes the water surface point when the shot entered
  water.
- Start in multiplayer: the tracer is sent with "use attachment 1" and the
  entity is the **weapon**; the client resolves the start each time the
  effect is created:
  - if the weapon is drawn through a view model (the local player's own
    gun in first person), attachment 1 of the **view model** (CS:S view
    models: attachment `1`, the muzzle, view_models.md);
  - else attachment 1 of the world model (CS:S world models:
    `muzzle_flash`, their only attachment);
  - if the attachment can't be found, the sent start point (the eye) is
    used.
- First-person tracers from the view-model muzzle are drawn in the world
  pass, so they start where the gun appears on screen, not where the
  bullet starts (the eye); with a 54° view-model FOV the start is off the
  bullet's line (view_models.md).
- Own-player branch: if the effect's entity is the local **player** itself
  (single-player style dispatch), instead of a moving streak a static line
  is drawn (section 3.2) from `shot start + 4·right − 0.5·up`. With weapon
  dispatch in multiplayer this branch is not taken; CS:S's choice is Q2.

### 3. Look

#### 3.1 Moving streak

Inputs: start `A`, end `B`, speed `v` (5000), material `effects/spark`.

1. `dir = normalize(B − A)`, `D = |B − A|`. If `D < 256`, draw nothing.
2. `L = U(64, 128)`; `w = U(0.75, 0.9)` (half-width); life
   `T = (D + L)/v` (so the tail also reaches the end).
3. Each frame, with `τ` = time since creation (advanced by frame time):
   head distance `h = v·τ`, tail distance `t = h − L`, both clamped to
   `[0, D]`. Not drawn while both are 0. Head point `A + h·dir`, tail
   `A + t·dir`.
4. The quad faces the camera: its side axis is
   `normalize((head − tail) × (head − eye))`.
5. Screen-size floor: with `z` = depth of the tail along the view forward
   and `W/2` half the screen width in pixels, the on-screen half-width is
   `p = w·(W/2)/z`. If `p < 0.5`: half-width becomes `0.5·z/(W/2)` (half a
   pixel) and brightness `b = clamp(remap(p; 0.25 → 0.3, 2.0 → 1.0), 0.25,
   1)`; else `b = 1`.
6. Draw two additive quads: the core (half-width as above, vertex grey
   `trunc(255·b)`) and a soft outline at twice the half-width with vertex
   grey `trunc(64·b)`. Texture coordinates: across the width u = 1 → 0;
   along the length v = 0 at the tail to `(h − t)/L` at the head (the
   texture is revealed as the streak leaves the muzzle).
7. The effect dies when `T` has elapsed.

With the material's additive blend and the texture's warm white, a tracer
reads as a short pale-yellow spark streak; it is not lit and casts no light.

#### 3.2 Static own-player line (alternative path)

If `D < 256` nothing. Else start `A + (8 + U(−24, 64))·dir`, end that point
`+ U(0.1, 0.6)·D·dir`; a camera-facing quad of half-width `U(0.5, 0.75)/2`,
`effects/spark`, white, living 0.01 s (one or two frames).

### 4. Console variables (player-visible)

- `r_drawtracers` (cheat, default 1): 0 hides all tracers.
- `r_drawtracers_firstperson` (archived, default 1, "Toggle visibility of
  first person weapon tracers"): 0 hides tracers whose entity is a player
  that the local view doesn't draw (your own shots in first person). With
  weapon-entity dispatch the check finds no player and does nothing; how
  CS:S applies it is Q2.
- The SDK's "extra visibility" switch for step 3.5 is a client cvar in the
  SDK; whether CS:S has it (and its default) is Q3. Default behaviour here:
  on.

### 5. Whizz sounds

Only for tracer bullets (the whizz flag rides on the tracer effect; weapon
tracers always set it). On the listening client:

1. Listener point `P` = the main view origin.
2. Default bullets: intersect the bullet line `A → B` with the vertical
   segment from `P` down 24 in; slide `P` down by `t·24` where `t ∈ [0, 1]`
   is the parameter of the closest point on that segment. (Your "body"
   extends 24 in below the eye.)
3. If the previous whizz was less than its interval ago (global timer), skip.
4. If the distance from `P` to the segment `A → B` is ≥ 24 in, skip.
5. Play the near-miss entry on the static channel **at the tracer's start
   point** `A` (not at the closest point), with the shot direction, to the
   local player only.
6. Next allowed whizz: now + 0.1 s.

Water bullets (the part of a shot under water) use `Underwater.BulletImpact`
(CS:S defines it: volume 0.9, pitch 95–105, 3 waves) with distance 48 in and
interval `U(0.3, 0.6)` s, sent as a separate "tracer sound" effect from the
water entry point to 400 in further along the shot.

CS:S: `Bullets.DefaultNearmiss` is defined only in HL2's
`game_sounds_weapons.txt`, which CS:S's sound manifest does not load
(sounds.md), so the lookup fails and no whizz is heard (*inferred*, Q4). The
wave files exist in the HL2 sound package. For mashup content that wants
whizz, the HL2 entry above is the reference.

## Per-tick order

Server tick: the shot is fired during the shooter's user command (weapons.md
§4); the tracer decision happens right after each bullet's trace and impact
handling, before the next pellet. Client: the tracer effect is created when
the predicted shot runs or when the fire-bullets event arrives, resolves its
start from the attachment at that moment (the view model's current animation
pose), then animates on frame time; the whizz test runs once at creation.

## Edge cases

- Shots shorter than 256 in (close walls, point-blank) never show a tracer
  but still advance the counter.
- A tracer is not occluded early: it is drawn along the full line to the
  hit point; past the hit point nothing is drawn.
- Penetrating shots (weapons.md M13): the generic code has no penetration;
  whether CS:S draws one tracer per bullet (to its final stop) or one per
  penetrated segment, and whether each segment advances the counter, is
  part of Q1.
- Shotguns: each pellet is a bullet for the counter (with N = 4, a 9-pellet
  M3 shot draws 2 or 3 tracers).
- Tracers of a dormant (out-of-PVS) shooter are skipped in HL2MP/TF builds;
  CS:S unknown.

## Quirks

- **Global counter**: tracer frequency is not per weapon or per player; the
  phase drifts with everyone's shots. Keep (or document if we use a
  per-weapon counter).
- **Whizz from the muzzle**: the near-miss sound is positioned at the
  tracer's start, so its loudness depends on the shooter's distance even
  though the bullet passed by your head. Moot for CS:S (no whizz).
- **Tracer start ≠ bullet start**: first-person tracers start at the view
  model's muzzle and converge on the hit point; they visibly do not come
  from the crosshair.

## Test cases

| Setup | Input | After | Expected |
|---|---|---|---|
| T1 counter, N = 4, counter starts at 0 | 10 single bullets | – | tracers on bullets 1, 5, 9 |
| T2 counter, N = 4, counter at 2 | M3 shot, 9 pellets | – | tracers on pellets 3 and 7 (counter values 4 and 8) |
| T3 short shot | wall 200 in away | – | no streak; counter advanced |
| T4 streak, D = 1000, L = 100, v = 5000 | – | – | life 0.22 s; at τ = 0.01 head 50, tail 0 (clamped); at 0.1 head 500, tail 400; at 0.21 head 1000 (clamped), tail 950 |
| T5 streak, D = 300, L = 128 | – | – | life 0.0856 s |
| T6 screen floor, w = 0.8, z = 2000, screen 1920 wide | – | – | p = 0.8·960/2000 = 0.384 px < 0.5 → half-width 1.0417 in; brightness = clamp(0.3 + (0.384 − 0.25)/1.75·0.7, 0.25, 1) = 0.3536 → core grey 90, outline grey 22 |
| T7 near streak, w = 0.8, z = 100, 1920 wide | – | – | p = 7.68 px ≥ 0.5 → half-width 0.8, core grey 255, outline 64 at half-width 1.6 |
| T8 whizz distance | bullet passes 20 in beside the eye, horizontal | – | whizz (if the entry exists) |
| T9 whizz below eye | bullet passes 20 in below the eye, horizontal, directly under | – | listener slides down 20 in to the line → distance 0 → whizz |
| T10 whizz rate | two whizzing tracers 0.05 s apart | – | one sound |
| T11 CS:S manifest | look up `Bullets.DefaultNearmiss` in CS:S's loaded scripts | – | not found → silent |
| T12 tracer start, own gun first person | fire | – | start = view-model attachment 1 position this frame |

## Open questions

Tracers and whizz sounds are client-only, so most checks need the CS:S
client (Windows PC, listen server or demo playback); `bullet_impact` events
from the probe server give the bullet end points to compare against.

1. **Q1 Frequency.** Fire 60 single shots of each weapon class (pistol,
   SMG, rifle, AWP, M3 pellets) at a far wall with `host_timescale 0.1`
   and record (`startmovie` or screenshots per frame); count tracers. Fire
   from two players alternately to see if the counter is shared. Check
   penetrating shots (one or two tracers?).
2. **Q2 First-person path.** Is your own tracer a moving streak from the
   view-model muzzle, or the 0.01 s static line? Does
   `r_drawtracers_firstperson 0` hide your own tracers in CS:S?
3. **Q3 Look.** `cvarlist tracer` on the client: is there an
   "extra visibility" cvar and what is its default? Compare a screenshot of
   a distant tracer with T6/T7 (width floor, outline). Confirm the material
   (`mat_texture_list` while firing).
4. **Q4 Whizz.** Stand 10–20 in beside a bot's line of fire with
   `snd_show 1` / `snd_visualize 1` (or `sv_soundemitter_trace 1` on a listen
   server); does any near-miss sound play? Expected: none in CS:S.
5. **Q5 Dormant shooters.** Are tracers of shooters outside your PVS (but
   whose bullets cross your view) drawn?
