# Counter-Strike: Source: bullet and knife impact effects

Source basis: Valve's public Source SDK 2013 only. I read the shared client
impact code (the client's re-trace and decal step, choosing an effect by
surface material, the ragdoll check, the impact sound), the client impact
effects (debris flecks, dust puffs, glass shards, blood puffs and the
surface-colour lookup), the client spark, water-splash and blood effects, the
old-style client particle classes they use (simple sprites, camera-space
trails, 3D shards, flat quads, the "bounce plane" collision helper, the
global sprite singleton, the per-frame update and the particle cap), the
shared bullet-to-water handling, the shared "trace attack" and bleeding code,
the temp-entity host suppression, the SDK template mod's shot routine, shot
effects dispatch, melee base and impact callback (the template is a
stripped-down CS-derived example), and the breakable-glass entity. CS:S's own
client and game code (its impact callback, bullet routine, penetration, knife
and ragdolls), the engine (surface colour and lighting lookups) and the
particle-system library (the operators that run `.pcf` effects) are **not**
public; everything that depends on them is flagged.

Data read from the user's install with throwaway readers (nothing extracted
into the repo): `particles/particles_manifest.txt`,
`particles/blood_impact.pcf`, `particles/water_impact.pcf`, the base and CS
surface property files, `scripts/game_sounds_physics.txt`, the materials
named below (shader, blend flags, texture size) and `bin/dxsupport.cfg`. No
leaked CS:S/CS:GO code was used. The live game was not run (shared-machine
rule); checks that need it are under Open questions.

Status: draft

## Summary

- An impact effect is made **on each client, by that client**, once per
  bullet (so once per shotgun pellet), from that client's own re-trace of
  the shot. The surface's game material letter (from surface properties)
  picks one of four hard-coded "old style" effects: **debris flecks + dust**
  (concrete C, tile T, wood W), **dust puff** (dirt D, sand N), **metal
  sparks** (metal M, vent V) or **electric sparks** (computer P). Everything
  else (glass, grates, plastic, foliage, flesh...) gets no particles from
  the impact itself, only the decal and the impact sound.
- CS:S does **not** use the newer `impact_*` particle systems for surfaces:
  its particle manifest does not load them and the switch for them
  (`cl_new_impact_effects`) defaults to 0. The only `.pcf` systems on the
  impact path are the **blood burst** `blood_impact_red_01` (made by the
  server when a player takes bullet or knife damage) and, for slime only,
  `slime_splash_0x`.
- Dust and fleck colours are **tinted by the hit surface**: texture colour
  times the lighting at the hit point. Water splashes are tinted by the
  lighting only. Sparks are white and additive. No impact creates a
  dynamic light.
- The look is fast and small: flecks are 2–4 in squares that fly 64–256 in/s,
  fall under 800 in/s², bounce off nearby walls and fade over 3 s; dust
  sprites start a few inches wide, grow to roughly 10–130 in across and
  are gone within 1.5 s; sparks live 0.05–0.25 s.

## Units and conventions

- Distance: inches (Hammer units). Source axes: X forward, Y left, Z up.
- Time: seconds. These effects run on the client per **rendered frame**
  with `dt` = frame time clamped to at most 0.1 s; they are not tied to the
  66.67 Hz server tick.
- "Normal" `n` = unit normal of the hit surface, pointing out towards the
  shooter's side. "Shot direction" `d` = unit vector from shot start to hit
  point.
- `U(a,b)` = uniform random float in [a, b]. `I(a,b)` = uniform random
  integer in [a, b] inclusive. `U3(a,b)` = a vector of three independent
  `U(a,b)`. Every draw is a fresh draw.
- Colours are 0–255 per channel unless written as 0–1. "Clamp-ramp" of a
  0–1 colour `c` by `r` means `min(1, c·r)·255`, truncated to an integer.
- Old-style sprite "size" is a **half-width**: a sprite of size 4 is an 8 × 8
  in camera-facing square. Flat quads (glows, ripples) instead take the
  **full** side length. Particle-system radii are half-widths.
- Sprite roll angles in the old-style code are in **radians**, even where
  the random range looks like degrees (U(0,360)): treat them as "random
  angle" and roll speeds as rad/s.
- Throttle `θ`: a 0–1 particle budget factor from the particle manager; 1
  unless the client is over its particle rendering budget (closed code, Q4).

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| retrace_extra | 8 | in | client re-traces from shot start to 8 in past the hit point to find surface, normal and decal |
| effect_scale | 1 | – | the impact effects' scale argument for bullets and knife (template; Q2) |
| max_old_particles | 2048 | particles | total live old-style particles; further spawns silently fail |
| max_dt | 0.1 | s | per-frame time step clamp for old-style particles |
| near_fade | 16 → 64 | in | simple sprites fade from 0 (≤ 16 in from the camera plane) to 1 (≥ 64 in) |
| fleck_gravity | 800 | in/s² | flecks |
| fleck_bounce_keep | U(0.2, 0.4) | – | speed kept after a fleck bounce |
| fleck_life | 3.0 | s | flecks |
| fleck_speed | U(64, 128·s) × (3 − size) | in/s | s = effect scale; size 1 or 2 |
| fleck_spray | max(0.2, 0.6 − 0.2·s) | – | per-axis random added to the normal |
| fleck_merge_extent | 120 | in | flecks join an existing fleck group if the combined box diagonal stays under this |
| bounce_probe | 8 steps × 0.25 s | – | wall search for bounce planes |
| land_rule | n.z ≥ 0.5 and abs(v.z) ≤ 48 in/s | – | a fleck/trail hitting such a surface stops dead |
| dust_decay | v ×= 10^(−4·dt/0.5) | – | dust-puff velocity decay per frame |
| dust_min_speed | 32 | in/s | dust-puff speed floor |
| metal_spark_gravity | 400 | in/s² | |
| metal_spark_dampen | 8 | 1/s | velocity × max(0, 1 − 8·dt) per frame |
| metal_spark_glow | 24–28 → 0 | in (full side) | yellow flare quad, 0.1 s |
| splash_size | U(8, 12) | – | bullet water splash scale (template; Q8) |
| splash_drop_gravity | 800 | in/s² | |
| splash_gout_decay | v ×= 10^(−4·dt/3) | – | then v.z −= 800·dt |
| ripple_life | 1.5 | s | water ring |
| blood_offset | 0 or 4 | in | generic player: at the hit point; generic entity: 4 in back along the shot (Q6) |
| bleed_trace | 172 | in | blood-on-wall trace length behind the target |

## Behavior

### 1. Who makes impact effects, and when

Inputs: a fired shot (weapons.md section 4), every client's copy of it.

- **Your own shots** are traced and their effects made by your client when
  it predicts the shot, only on the **first** prediction of that command
  (re-predictions make no effects). The server makes no impact effects for
  your shots and sends you none.
- **Other players' shots** arrive as one compact "shots fired" message
  (shooter, eye origin, angles, weapon, mode, random seed, spread). It goes
  to clients in the PAS (potentially audible set) of the shooter's eye
  position, minus the shooter. Each receiving client re-runs the pellets
  with the same seed and traces them **against its own world state** (its
  view of other players is not lag-compensated), then makes the effects.
  A client outside the shooter's PAS sees no impacts from that shot even if
  the hit point is in view.
- One impact effect **per bullet trace that hits** (`fraction < 1`). A
  shotgun shot makes one per pellet (M3: 9, XM1014: 6). Penetration: every
  surface a bullet enters in CS:S makes an impact (weapons.md section 7,
  `bullet_impact` events); whether exit holes also make one is unknown (Q3).
- No impact effect when: the hit surface is sky, nodraw, hint or skip; the
  hit entity is a teammate and friendly fire is off (template rule, Q2); the
  bullet entered water (a splash is made instead, section 11).
- No distance limit. Old-style particles have no draw-distance cut; only the
  near-camera fade and normal view/leaf culling apply.
- The blood burst (section 10) is different: the **server** makes it during
  damage, and sends it to clients in the PAS of the hit point.

### 2. The per-impact procedure (client)

For one hit, with shot start `S`, server/predicted hit point `H`, the hit
entity, its hitbox (for static props: prop index + 1) and the surface
property id:

1. Material letter `m` = the `gamematerial` of the surface property (table
   in section 3).
2. Ragdoll check: if at most one client-side ragdoll is currently
   simulating, test the ray S → H against client-side ragdolls; the first
   one hit gets a push (force 4000 along `d` at the hit position) and the
   impact is marked "hit a ragdoll".
3. Decal and re-trace: trace from `S` to `S + d·(|H − S| + 8)` against the
   hit entity only (world, static prop or model). This gives the surface
   point `P`, normal `n` and surface flags, and places the decal
   (overlays_decals.md; decal name from the material letter). If the decal
   name is unknown, steps 4–6 are skipped (no particles, no ricochet); the
   impact sound (step 7) still plays (Q9).
4. If the re-trace missed, or a ragdoll was hit in step 2, skip step 5.
5. If the surface is not sky/nodraw/hint/skip: make the material effect
   (section 3) at `P` with scale 1.
6. Bullets only (template): with probability 3/10 play `Bounce.Shrapnel` at
   `H` (ricochet; sounds.md open question 7, Q2).
7. Always: play the surface's bullet-impact sound (sounds.md section 4,
   "Bullet impacts"; shotgun pellets are grouped there).

### 3. Effect by material letter

Letters are resolved through surface property inheritance (`base`), from
`scripts/surfaceproperties.txt` then `scripts/surfaceproperties_cs.txt`
(later definitions override).

| Letter | Effect | Surface properties in the CS:S data with this letter |
|---|---|---|
| C concrete | debris flecks (cement) + dust | default, concrete, concrete_block, rock, boulder, brick, gravel, porcelain, quicksand, ice, player, brass_bell_* |
| T tile | debris flecks (cement) + dust | tile |
| W wood | debris flecks (wood) + dust | wood, wood_*, watermelon, hay |
| D dirt | dust puff | dirt, mud, grass, snow, sand, carpet, plaster, cardboard, paper, papercup, ceiling_tile, rubber, rubber tyres, slipperyslime, floatingstandable, wasabiball |
| N sand | dust puff | none: CS:S's `sand` resolves to D |
| M metal | metal sparks | metal, solidmetal, metalpanel, metal_box, metal_barrel, canister, popcan, paintcan, grenade, roller, weapon, metalvehicle, slipperymetal, metal_bouncy, armorflesh |
| V vent | metal sparks | metalvent |
| P computer | electric sparks | computer |
| G grate | none | metalgrate, chainlink, chain |
| Y glass | none (but see section 9) | glass, glassbottle, pottery |
| L plastic | none | plastic, plastic_box, plastic_barrel, item |
| O foliage | none | foliage |
| F, B, H flesh | none from the impact (blood: section 10) | flesh, bloodyflesh, alienflesh |
| S slosh | none | water, slime |
| X, I, `-` | none | wade, ladder, woodladder, default_silent, player_control_clip, no_decal |

The player models' hitboxes use `flesh`. Antlion (A) and warp-shield (Z)
effects exist in the shared code but no CS:S surface uses those letters.

### 4. Shared machinery for the old-style effects

These rules apply to sections 5–9 and 11.

**Simulation.** Once per rendered frame, every old-style effect advances by
`dt = min(frame time, 0.1)`. A newly created effect group skips simulation
on the frame it was created (it is drawn once at its spawn state). Sprites
added to the global sprite pool (the dust "trail", "grit" and "cap" sprites
of section 5) do not skip: they move on their first frame.

**Plain sprite** (camera-facing square, "simple"): per frame,
`pos += v·dt`, `age += dt`, `roll += roll_speed·dt`; removed when
`age ≥ life`. No gravity, no drag. Drawn with
size `= start + (end − start)·age/life` and alpha
`= (start_a + (end_a − start_a)·age/life)/255`, times the near fade:

```math
\text{nearfade}(z) = \text{clamp}\left(\frac{z - 16}{64 - 16},\ 0,\ 1\right)
```

where `z` is the sprite's distance in front of the camera plane. Sprites
with alpha below 0.001 are not drawn. Colour is constant. Draw order within
a group is back to front (by camera depth, rounded to whole inches).

**Dust sprite** (a plain sprite with these changes; used only by the dust
puff, section 6):
- velocity: first `v ← v·k` with `k = exp(ln(10⁻⁴)·dt/0.5)`, then if
  `|v| < 32`, `v ← normalize(v_before_decay)·32` (zero stays zero). So a
  moving puff slows to 32 in/s and keeps drifting at 32 in/s.
- roll speed: `ω ← ω + ω·(−8·dt)`, then if `|ω| < 0.5`, `ω ← ±0.5` (same
  sign). Applied after the roll update.
- alpha **ignores the particle's start/end alpha**:
  `r = 1 − age/life`; alpha = `r` if `r ≥ 0.75`, else `r²`. (So it jumps
  from 0.75 to 0.5625 at a quarter of its life.) Near fade applies.

**Fleck** (camera-facing square): size 1 or 2 (half-width, constant),
colour constant, alpha `1 − age/life`, roll += roll speed·dt. Movement by
the bounce-plane rule below, gravity 800. No near fade. Material
`effects/fleck_cement1|2` or `effects/fleck_wood1|2` (random pick per
fleck; 32 × 32 textures, translucent, vertex colour; the cement ones use
vertex alpha, the **wood ones do not**, so wood flecks do not fade, Q10).

**Bounce planes.** A bouncing group finds up to 6 candidate planes when it
is set up, instead of tracing every particle every frame:
- Setup from origin `O`, an aim direction `a` (or none) and a test speed
  `s̄ = (min speed + max speed)/2`: probe along `a`, along the right vector
  of `a` and along its opposite (if no `a`: along ±X, ±Y, ±Z). A probe walks
  8 steps of `Δ = 0.25 s`: step `i` (1–8) goes from the previous point to
  previous + `dir·s̄·Δ`, with z lowered by `½·g·(Δ·i)²` (note: the drop is
  applied per step from the cumulative time, so the path falls faster than
  a true parabola). The first step that hits world brushes adds that
  surface's plane (duplicates ignored) and ends the probe.
- Each frame per particle (skip entirely if `v = 0`): `v.z −= g·dt`; test
  `O' = pos + v·dt`. If the segment crosses one of the planes (from its
  front side, with a 0.01 in tolerance), re-trace the segment against world
  brushes. If that trace hits at fraction `f`:
  - if the hit normal's z ≥ 0.5 and `|v.z| ≤ 48`: move to
    `pos + v·(f − 0.01)·dt`, set `v = 0` and roll speed 0 (landed, it stays
    until it fades);
  - else: move to `pos + v·(f − 0.01)·dt`, reflect
    `v ← (v − 2(v·n)n)·U(keep − 0.1, keep + 0.1)`, roll speed × −0.25.
  - Otherwise `pos ← O'`. Particles never collide with anything that was not
    found as a plane at setup (models, props, players) and planes are never
    re-probed.
- If the re-trace starts in solid, velocity and roll speed are zeroed.

**Fleck grouping.** A new fleck group merges into an existing live fleck
group if the union of the existing group's box and the new spawn box
(spawn point ± 5 in) has a diagonal under 120 in. The merged group then
**re-runs its bounce-plane setup from the new impact**, so older flecks in
it now collide against the new planes (quirk, keep).

**Trail sprite** (sparks, splash drops): drawn as a camera-facing strip
from the particle position along its velocity:
`length vector = v · L · max(0.01, 1 − age/life)` where `L` is the
particle's "length" in seconds; strip width = `min(width, |length vector|)`.
Colour × fade ramp where enabled (sparks and drops here: no fade, constant
colour and alpha). Movement: `v.z −= g·dt`; `pos += v·dt` (bounce planes
only if the group asked for collision); then, if velocity damping is on,
`v ← v·max(0, 1 − k·dt)` (frame-rate dependent; at `dt ≥ 1/k` the particle
stops dead). Removed at `age ≥ life`.

**Flat quad** (glows, ripples): a square lying in the plane with normal
`N`, full side `size`, rotated by `yaw` degrees about `N`
(`yaw += yaw_speed·frame_dt`, wrapped to [0, 360)). Each frame its age is
advanced *before* drawing. Size and alpha interpolate from start to end
over the life, optionally through `bias(t, b) = t^(log(b)/log(0.5))` (so
`bias(0.5, b) = b`). Alpha is clamped to [0, 1]; not drawn when 0.

**Surface colour** (used by flecks and dust): from the shot's hit, the
engine returns the lighting `L` (RGB, linear) at the hit and the surface
material's base colour `B` (RGB 0–1): world brush → the face's material;
static prop → the prop's material; other entities → the model's material
at the hit. Then per channel

```math
c = L^{1/2.2} \cdot B
```

If the entity cannot be resolved, `c = (255, 255, 255)` (out of the 0–1
range, so every clamp-ramp gives 255: bright white). What exactly `B` is
(texture average or the material's reflectivity) is engine code (Q5).

### 5. Debris flecks + dust (C, T, W)

Inputs: hit point `P`, normal `n`, letter, scale `s` = 1, throttle `θ`,
surface colour `c` (section 4). Switches: `fx_drawimpactdebris` (dev only,
1) turns the whole effect off; `r_drawflecks` 0 removes only the flecks.

1. **Flecks** (unless `r_drawflecks` is 0): spawn point `P + 1·n`.
   - Count `N = int(0.5 + θ·s·I(4, 16))`, 4–16 at full budget.
   - Spray `σ = max(0.2, 0.6 − 0.2·s)` = 0.4. Bounce setup with aim `n`,
     speeds 64 and `128·s`, gravity 800, keep 0.3.
   - Each fleck: life 3.0; direction `n + U3(−σ, σ)` (**not** normalized);
     size `I(1, 2)`; velocity = direction × `U(64, 128·s)` × (3 − size)
     (small flecks fly twice as fast); roll `U(0, 360)` rad, roll speed
     `U(0, 360)` rad/s (very fast spin); colour = clamp-ramp(`c`,
     `U(0.75, 1.25)`); material: wood letter → wood flecks, else cement.
2. **Dust trail**: 2 plain sprites (global pool) at `P + 2·n`, material
   `particle/particle_smokegrenade` (64 × 64, alpha-blended, vertex colour
   and alpha). For `i` = 0, 1: life 1.0; `dir = n + U3(−0.8, 0.8)`;
   start size `I(2, 4)·s`, end size `start·8·s`; velocity
   `dir·U(2, 24)·(i+1)` then `v.z −= U(8, 32)·(i+1)`; alpha `I(100, 200)`
   → 0; roll `U(0,360)`, roll speed `U(−1, 1)`; colour clamp-ramp(`c`,
   `U(0.5, 1.25)`).
3. **Grit**: 4 plain sprites at `P + 2·n`, material `effects/blood`
   (64 × 64, alpha-blended; despite the name it is used as a grit/speck
   sprite here). Each: life `U(0.25, 0.5)`; `dir = n + U3(−0.8, 0.8)`;
   start size `I(1, 4)`, end ×4; velocity `dir·U(8, 32)` then
   `v.z −= U(8, 64)`; alpha 255 → 0; roll speed `U(−2, 2)`; colour
   clamp-ramp(`c`, `U(0.5, 1.25)`).
4. **Hole cap**: 1 plain sprite at `P + 2·n`, `particle_smokegrenade`.
   Life `U(1, 1.5)`; `dir = n + U3(−0.8, 0.8)`; start size `I(4, 8)`, end
   ×4; velocity `dir·U(2, 24)` with `v.z` **replaced** by `U(−2, 2)`; alpha
   `I(100, 200)` → 0; roll speed `U(−2, 2)`; colour clamp-ramp(`c`,
   `U(0.5, 1.25)`).

The dust sprites (2–4) are always made, independent of `θ`.

### 6. Dust puff (D, N)

Inputs: as section 5. Switch: `fx_drawimpactdust` (dev only, 1). All
sprites are **dust sprites** (section 4) in one group, spawned at the hit
point `P` itself (no offset).

- Counts: with `f = 4·θ`: big puffs `int(0.50 + f)`, specks
  `int(0.83 + f)`, hit puffs `int(0.17 + f)` (4/4/4 at full budget).
- The `k`-th big puff and speck (k = 0, 1, 2, 3) use the "strength index"
  `j` = 3, 1, 2, 0 respectively, so a reduced count still covers the range.
- **Big puffs** (`particle_smokegrenade`): life `U(0.5, 1.0)`;
  `v = normalize(U3(−0.2, 0.2) + n·U(1, 6)) · U(250, 500)·j·s`; colour
  clamp-ramp(`c`, `U(0.75, 1.25)`); start size `int(s·I(3,4)·(j+1))`, end
  size `int(s·start·4)`; start alpha `I(32, 255)` (unused, see dust-sprite
  alpha); roll speed `U(−8, 8)`.
- **Specks** (`effects/blood` grit sprite): life `U(0.25, 0.75)`; velocity
  as for big puffs but **without** the `s` factor; colour as above; start
  size `I(2, 4)·(j+1)`, end ×2; roll speed `U(−2, 2)`.
- **Hit puffs** (`particle_smokegrenade`): life `U(0.5, 1.0)`;
  `v = normalize(U3(−1, 1) + n)·U(0, 50)`; start size `I(1, 4)`, end ×4;
  roll speed `U(−16, 16)`. A random XY offset of ±8 in is computed but not
  used: all hit puffs start exactly at `P` (quirk).
- Puffs with `j = 0` start with zero velocity and stay put (the speed floor
  keeps zero at zero); all others decelerate to 32 in/s and drift on along
  their direction until they die.

### 7. Metal sparks (M, V)

Inputs: `P`, `n`, `d`, scale `s` = 1, `θ`. Switch: `fx_drawmetalspark`
(dev only, 1).

- Aim: the reflection of the shot, jittered, not normalized:
  `a = d − 2(d·n)n + U3(−0.2, 0.2)`.
- Spawn point `O = P + 1·n`.
- Trail group: gravity 400, **no** collision, velocity damping `k = 8`;
  material `effects/spark` (32 × 64, additive, vertex colour).
- Count `N = int(0.5 + θ·I(4, 8)·2s)` (8–16).
- Each spark: `t = U(0, 2)`; `dir = normalize(a + U3(−0.5t, 0.5t))`;
  speed `U(128·(2−t), 512·(2−t))` (a wide spray is a slow spray: at `t = 2`
  the speed is 0); width `U(1, 4)`; length `U(0.025, 0.1)` s; colour white,
  alpha 255, no fade; life `U(0.05, 0.1)` (with `s > 1`, every third spark
  lives `U(0.15, 0.25)` instead).
- Glow: one flat quad at `O` facing `n`, material `effects/yellowflare`
  (64 × 64, additive), white, yaw `I(0, 360)`, size `I(24, 28)` → 0 and
  alpha 1 → 0, both linear over 0.1 s.

### 8. Electric sparks (P)

At `O = P + 1·n`, magnitude 1, trail length 1, no aim direction.

- **Big sparks**: trail group with bounce planes (probes ±X, ±Y, ±Z, test
  speed 182, gravity 800, keep 0.3); damping flag on with `k = 0` (no
  effect). Count `int(U(2, 4))` (2 or 3). Each: life `U(1, 2)`;
  `dir = (U(−1,1), U(−1,1), U(0.5,1))`, not normalized; width `U(2, 5)`;
  length `U(0.02, 0.05)`; velocity `dir·U(64, 300)`; white.
- **Small sparks**: trail group, gravity 400, no collision, no damping.
  Count `I(16, 32)`. Each: `dir = U3(−1, 1)` not normalized; width
  `U(2, 4)`; length `U(0.02, 0.03)`; life `U(0.1, 0.2)`; velocity
  `dir·U(128, 256)`; white. Material `effects/spark` for both.
- **Glow group** (all three sprites removed at creation time + 0.2 s, and
  only drawn if the spawn point passes a pixel-visibility test; the group
  is not simulated until it has been tested in view):
  - inner flare: `effects/yellowflare_noz` (additive, ignores depth),
    white, alpha 255 constant, size `I(4, 8)` → 0 over 0.2 s;
  - outer flare: same material, grey `g = I(32, 64)` (colour and start
    alpha both `g`) → alpha 0, size `I(32, 64)` → 0 over 0.2 s, roll speed
    `U(−1, 1)`;
  - smoke: `particle/particle_noisesphere` at `O + (U(−4,4), U(−4,4), 0)`,
    life 1.0 but cut at 0.2 s by the group, velocity
    `(U(−16,16), U(−16,16), 16)`, colour (255, 255, 200), alpha `I(16, 32)`
    → 0, size `I(4, 8)` → ×4, roll speed `U(−2, 2)`.

### 9. Glass panes (func_breakable_surf)

Bullets on glass make no material effect (section 3). A glass pane entity
(`func_breakable_surf`) that is damaged by a bullet or club hit and
**shatters a pane** sends a glass-shard effect to every client in the PAS
of the hit (including the shooter) at the hit point with the trace normal.
Breaking and gibbing glass are covered elsewhere (breakables); the shard
burst is:

- Light `ℓ` = engine lighting at the point (clamped), blended 30 %
  towards white: `ℓ' = 0.7·ℓ + 0.3`.
- Shards: `I(2, 4)` flat 3D squares (not camera-facing), material
  `effects/fleck_glass1|2` (translucent, two-sided). Shared size
  `σ = U(2, 6)`; each shard's side = `σ + U(−σ/2, σ/2)` in (truncated to
  integer), anchored at a corner; life `U(2.5, 5)`; velocity per axis
  `(n_i + U(−0.8, 0.8))·U(1, 300)` (each axis its own speed draw);
  angles random, all three angles spin together at `U(−800, 800)` deg/s;
  colour (200, 200, 210) × `ℓ'` (truncated). Bounce planes with aim `n`,
  speeds 1 and 300, gravity 800, keep 0.3. Alpha: 1 until the last 2 s of
  life, then `remaining/2`. A shard that has landed (spin 0) or bounced
  this frame is pulled towards lying flat: pitch `p ← 0.5p + 46` if
  `p < 180°` (settles at 92°), else `p ← 0.5p + 135` (settles at 270°),
  unless already within 0.5° of 90° / 270°; yaw is set to half the roll
  angle if it is more than 0.5° (quirk).
- Dust: 4 plain sprites at `P + 2·n`, grit material `effects/blood`.
  Colour: the code ramps the raw values 64, 64, 92 (not divided by 255) by
  `U(0.5, 1.25)` and clamps to 1, so every channel is 255: white (quirk).
  Life `U(0.1, 0.25)`; `dir = n + U3(−0.8, 0.8)`; start size
  `I(1, 4)`, end ×8; velocity `dir·U(8, 16)·(i+1)` then
  `v.z −= U(16, 32)·(i+1)`; alpha `I(128, 255)` → 0; roll speed `U(−1, 1)`.
- Cap: 1 plain sprite, `particle_smokegrenade`, same colour rule (white),
  life `U(1, 1.5)`, size `I(4, 8)` → ×4, velocity `dir·U(2, 8)` with z
  replaced by `U(−2, 2)`, alpha `I(32, 64)` → 0, roll speed `U(−2, 2)`.

### 10. Flesh: blood

Bullets and the knife hitting a player do not make a material effect
(letter F). Blood comes from the damage path:

- **When**: the victim takes damage from the hit (not refused by team rules)
  and human blood is enabled (`violence_hblood` 1, not low-violence). CS:S
  hostages bleed too (Q6).
- **Where**: generic player rule: at the hit point; generic entity rule:
  4 in back along the shot (`H − 4·d`). Orientation: the system's forward
  axis is `−d` (back towards the shooter).
- **What**: the particle system `blood_impact_red_01`
  (`particles/blood_impact.pcf`, preloaded by the manifest). The damage
  amount is passed but unused, so every hit, even 1 damage, looks the same.
  Who sees it: clients in the PAS of the blood point. The generic code
  would hide it from the shooter (the shot is processed with the shooter's
  own events suppressed), but CS:S shooters do see blood on hits, so CS:S
  must lift that for blood or make it client-side (Q6).
- **Blood on walls behind**: `n_t` traces from the hit point **onwards
  along the shot** (`d` plus `U3(−σ, σ)` noise), 172 in, against world
  brushes only and not grates; each trace that hits places the `Blood`
  decal. `n_t, σ` = 1, 0.1 for damage < 10; 2, 0.2 for < 25; else 4, 0.3.
  Same enable rule. (Decal projection: overlays_decals.md.)
- **The system's contents** (all children start at once; values from the
  install's `.pcf`; operator meanings in section 14):

| Part | Count (max) | Material | Life (s) | Radius (in) | Motion | Colour, alpha | Fade |
|---|---|---|---|---|---|---|---|
| smoke (root) | 7 (10) | `particle/smoke1/smoke1_nearcull2`, sprite card, random frame 0–15 | U(0.7, 1.2) | U(2, 3), grows to ×4 (bias 0.5) | speed U(3, 15) random dir + local U((10,−5,−5), (90,5,5)); gravity +5 up; drag 0.125 | between (205,188,173) and (124,106,86); alpha U(25, 40) | fade in over first 12.5 % of life, out from 25 % to 100 % |
| mist | 2 (5) | `particle/blood_core` (sprite card, max 0.25 screen) | U(0.25, 0.35) | U(9, 19), grows from ×0 to ×2 (bias 0.75) | speed U(3, 15) + local U((10,−5,−5),(30,5,5)); gravity 175 down; drag 0.125 | between (107,16,16) and (14,1,1), tint 0.1; alpha U(80, 255) | fade-in multiplier starts at 0 with a zero-length window (Q7); fade out from 50 % to 100 % |
| goop | 3 (5) | `particle/antlion_goop3/antlion_goop3` (sprite card sheet, max 0.35 screen), frame 0–5, animated at 35 fps | sequence length at 35 fps | U(8, 21) | within 2 in; speed U(0, 32) + local U((26,0,24),(42,0,50)); gravity 400 down; drag 0.025; spin 0–4 deg/s, stops at 1 s; random yaw flip 50 % | between (93,8,8) and (48,9,5), tint 0.1; alpha 255 | each fades out over a random last 25–50 % of life |
| small droplets | 40 (50) | `particle/particle_glow_03` (alpha-blended) | U(0.2, 0.8) | U(0.35, 0.65) (exponent 0.1: mostly near 0.65); shrinks to ×0.75 from 50 % of life | start 4–8 in out; speed U(75, 150) + local U((0,0,30),(0,0,90)); gravity 500 down; drag 0.015 | between (134,1,1) and (27,3,3); alpha U(128, 200) | fade out from 85 % to 100 % |
| chunks | 15 (15) | goop sheet, frames 8–12 | U(0.1, 0.7) | U(0.3, 2) | speed U(50, 150) + local U((0,0,20),(0,0,100)); gravity 400 down; drag 0.1 | between (0,0,0) and (17,53,81); alpha 255 | fade out from 50 % to 100 % |

  "Local" velocities are in the system's frame: +X = forward = back
  towards the shooter, +Z = the frame's up (Q7). Every part is drawn as
  camera-facing sprites; "maximum draw distance" is 100000 (no practical
  limit); time step at most 0.1 s per update.
- Only `blood_impact_red_01` is used for human (red) blood. The `.pcf`
  also contains a trail-rendered `blood_impact_red_01_droplets` that is not
  a child of the red system and is not used here.

### 11. Water splash

When a bullet's end point is inside water or slime, the client traces the
shot again including water. Unless that trace lies entirely inside water
(the shot started under water), a splash is made at the water entry point.
Either way the impact effect is **not** made (no decal, no flecks, no
impact sound); the underwater surface gets nothing. Scale `s = U(8, 12)` (template;
Q8). The splash is made by each client for its own copy of the shot.
Switch: `cl_show_splashes` (1).

Water (not slime), with `O` = entry point, up = (0, 0, 1) regardless of
the trace normal:

1. Lighting: `ℓ` = engine light at `O + s·up` (clamped). Tint
   `τ = ℓ / max(ℓ_r, ℓ_g, ℓ_b)` (zero stays zero). Luminosity
   `λ = clamp(4·(0.30ℓ_r + 0.59ℓ_g + 0.11ℓ_b), 0.25, 1)`. Splash colour
   `c = 0.25·τ + 0.75`. Scale factor `f = min(s/8, 4)`.
2. **Drops**: 16 trail sprites, material `effects/splash2` (64 × 64,
   alpha-blended). Group: gravity 800, no collision, damping `k = 2`.
   Each: position `O + (U(−8,8)·f, U(−8,8)·f, 0)`; life `U(0.25, 0.5)`;
   `v = (up + U3(−0.8, 0.8))·U(150f, 300f)`, then `v.z += U(32, 64)·f`;
   width `U(1, 3)`; length `U(0.025, 0.05)`; colour clamp-ramp(`c`,
   `U(0.75, 1.25)`), alpha `λ·255`.
3. **Gout**: 8 sprites (`effects/splash2`), all at `O`, for `i` = 0..7:
   `v = normalize(U3(−0.2, 0.2) + up·U(4, 6))·50·f·(8 − i)` (400f down to
   50f in/s); start size `int(24·f·r_i)` with `r_i` going linearly from
   0.5 (i = 0) to 1 (i = 7), end `min(255, 2·start)`; start alpha
   `int(a_i·λ)` with `a_i` going linearly from 32 (i = 0) to 255 (i = 7);
   colour clamp-ramp(`c`, `U(0.75, 1.25)`); roll `I(0,360)`, roll speed
   `U(−4, 4)`; life 2.0. Per frame: `v ← v·exp(ln(10⁻⁴)·dt/3)`, then
   `v.z −= 800·dt`, `pos += v·dt`; the sprite is removed as soon as
   `pos.z + size < O.z` (it has sunk below the surface); then `age += dt`,
   roll update with roll speed decay `ω ← ω − 4ω·dt` and floor `|ω| ≥ 0.5`.
   Alpha = start alpha × `clamp((pos.z − (O.z − size/2)) / (size/2), 0, 1)`
   (full above the surface, fading as it sinks; no fade with age). Not
   drawn beyond the group's cull radius `2s`. So the fast, small, faint
   gouts fly highest; the slow, big, opaque ones form the base.
4. **Ripple**: trace down from `O + 8 up` to `O − 64 up` against water; if
   found, a flat quad at the hit + 0.5·normal, facing the water normal:
   material `effects/splashwake1` (256 × 256, alpha-blended); size
   `16f → 128f` with bias 0.7; alpha `λ → 0` with bias 0.25; yaw
   `U(0, 360)`, yaw speed `U(−16, 16)` deg/s; colour `c`; life 1.5 s.
5. Sound `Physics.WaterSplash` at `O`, channel CHAN_VOICE, volume 1,
   level SNDLVL_NORM, local player only.

Slime: instead of steps 1–4, the particle system `slime_splash_01` if
`s < 2`, `slime_splash_02` if `s < 4`, else `slime_splash_03` (so bullets
always get `_03`), oriented along up; same sound. These are in
`particles/water_impact.pcf`. No stock CS:S map is known to use slime.

### 12. Knife

CS:S's knife code is not public. From the template melee base:

- If the swing trace starts outside water and its end point is inside
  water: a water splash with fixed `s = 8` (section 11, but sent by the
  server to the PAS) and nothing else.
- Otherwise a hit makes the same impact procedure as a bullet (section 2)
  with the knife's damage type: same material effects at scale 1, the
  knife's decal (overlays_decals.md, "knife slash marks"), the surface's
  bullet-impact sound, **no** ricochet sound. It is made by the server
  for every client in the PAS of the hit except the attacker, and by the
  attacker's client during prediction (Q11). The template's knife water
  splash is made by the server only, so the generic host suppression would
  hide it from the attacker (Q11).
- On a player: blood exactly as section 10.
- A miss into water also makes the splash.

### 13. Light and sound

- **No dynamic light** comes from any impact (the spark effects' light is
  disabled in the code). The additive yellow flares (sections 7, 8) only
  look bright. The muzzle flash light is view_models.md section 7.
- Sounds: the surface's bullet-impact sound per impact (sounds.md section
  4, including the 300 in grouping for pellets); `Bounce.Shrapnel` 30 % of
  bullet impacts (template, Q2): volume 0.8, pitch 90–124,
  random `weapons/fx/rics/ric*.wav`; `Physics.WaterSplash` for splashes
  (volume entry 0.8–1.0, pitch 85–115 in the script; the code requests
  volume 1).

### 14. Particle-system operator meanings (blood and slime)

The `.pcf` files are data in the user's install and should be loaded at
runtime rather than copied. The library that runs them is not public; this
is the interpretation used above, to be checked (Q7):

- **Emit instantaneously**: emit `num_to_emit` particles at
  `emission_start_time`. **Emit continuously**: `emission_rate`/s for
  `emission_duration`.
- **Position within sphere random**: position = control point + random
  direction × `U(distance_min, distance_max)` (per-axis bias 1 = sphere);
  velocity = random direction × `U(speed_min, speed_max)` + a per-axis
  random vector between the two "local coordinate system" speeds, rotated
  into the control point's frame.
- **Lifetime random**, **radius random**, **alpha random**, **rotation
  random**, **sequence random**: uniform picks (with the given exponent
  applied to the random fraction). **Lifetime from sequence**: life = frame
  count of the chosen sheet sequence / fps.
- **Color random**: a random blend between `color1` and `color2`;
  `tint_perc` blends that much towards the lighting at the control point.
- **Movement basic**: add `gravity·dt` to velocity, apply `drag`
  (fraction of velocity removed per step), move.
- **Alpha fade and decay**: alpha multiplier ramps from `start_alpha` to 1
  between the fade-in start/end life fractions, then from 1 to
  `end_alpha` between the fade-out start/end fractions; the particle dies
  at the end of its life. **Alpha fade out random**: fades to 0 over the
  last random fraction (`min`–`max`) of life. **Lifespan decay**: dies at
  end of life.
- **Radius scale**: radius × `lerp(start_scale, end_scale, bias(x, b))` (`b` = `scale_bias`)
  where `x` is the life fraction mapped from `[start_time, end_time]`.
- **Rotation spin roll**: roll speed in deg/s, stopping at
  `spin_stop_time`. **Rotation yaw flip random**: flips the sprite
  horizontally for that fraction of particles.
- **Render animated sprites**: camera-facing sprite from the material's
  sheet; `animation rate` frames/s when "use animation rate as FPS" is set.
- Sprite-card materials: `$maxsize` caps a sprite's size to that fraction
  of the screen; `$startfadesize`/`$endfadesize` fade a sprite out as its
  screen size grows from the first to the second fraction (so the blood
  smoke vanishes when it fills more than about 10 % of the screen, e.g. in
  your face); `$depthblend` softens the intersection with geometry.

## Per-frame order

Client, one rendered frame:

1. Process incoming messages: other players' shot messages → for each
   pellet: trace, then (water splash) or (section 2 steps 1–7). Server
   blood bursts, glass shard bursts and knife impacts arrive here too.
2. Run the local player's prediction; the local player's shots make their
   effects here (first prediction only).
3. Update old-style particles with `dt = min(frame time, 0.1)`: groups
   created this frame (except the global sprite pool) are not moved; then
   each particle: velocity rule (gravity/decay/bounce), position, age,
   roll; remove dead ones. Then update the particle systems (blood,
   slime).
4. Draw: particle groups sorted back to front within their group; flat
   quads advance their age then draw.

Within one impact (section 2): ragdoll push → decal and re-trace →
material effect → ricochet sound → impact sound. Within an effect, the
order of spawned sprites is the order listed in its section (only matters
for the 2048 cap: when the cap is hit mid-effect, the remaining particles
of that kind are skipped).

## Edge cases

- Shot hits sky, nodraw, hint or skip: nothing at all (no decal, no
  particles, no impact sound).
- Re-trace misses (e.g. the target moved between server and client
  traces): no particles; the sound plays at the server-reported point.
- A client-side ragdoll on the line (and at most one ragdoll simulating):
  the ragdoll is pushed, the wall behind gets a decal but no particles.
- Static props: surface colour and decal use the prop's own material; the
  effect is the prop's surface property letter.
- Flecks only bounce off world brushes found by the probes; they pass
  through props, players and brush entities, and through walls not in the
  probe directions. Flecks that land on a floor (normal z ≥ 0.5) slowly
  enough stay until they fade; on steep slopes they keep bouncing.
- Throttle: at `θ = 0.1` a dust puff spawns 0 big puffs, 1 speck, 0 hit
  puffs; flecks can drop to 0 (`int(0.5 + 0.4) = 0` for 4 at `θ = 0.1`).
- The 2048-particle cap is shared by all old-style sprite effects; a
  heavy firefight can starve impacts silently.
- Bullet entering water at a grazing angle still splashes straight up.
- A bullet whose end point is in water never makes an impact effect. If
  the whole shot is under water (fired from underwater at something
  underwater) there is no splash either, so nothing at all (template rule,
  Q8).

## Quirks

Keep all of these; they are part of the look.

- Wood flecks never fade (their material ignores vertex alpha); they pop
  out at 3 s. (Q10: confirm in game.)
- Dust-puff sprites ignore their random start alpha and always start fully
  opaque, with an alpha jump from 0.75 to 0.5625 at 25 % of their life.
- Dust puffs never stop: they slow to 32 in/s and drift until they die.
- Fleck and spark directions are "normal plus a random cube", not
  normalized: diagonal sprays are faster than straight ones.
- Small flecks fly twice as fast as big ones.
- Metal sparks sprayed wide are slow; at maximum spread they do not move.
- Hit puffs of the dust effect all spawn at the same point (the random
  offset is unused).
- Glass grit and cap sprites are pure white (the colour ramp is applied to
  0–255 values and clamps).
- Merged fleck groups re-probe their bounce planes from the newest impact.
- Electric spark "smoke" is cut off at 0.2 s by its group.
- The splash ignores the water surface normal and always goes straight up.
- Blood size does not depend on damage.

## Test cases

Values that use random draws state the draw. "s" = effect scale (1).

| Setup | Input | After | Expected |
|---|---|---|---|
| surface `brick` | bullet | impact | letter C → debris flecks (cement) + dust |
| surface `sand` | bullet | impact | letter D (CS data) → dust puff |
| surface `metalvent` | bullet | impact | letter V → metal sparks |
| surface `metalgrate` | bullet | impact | no particles; decal and `MetalGrate.BulletImpact` only |
| surface `glass` on a world brush | bullet | impact | no particles |
| surface `armorflesh` | bullet | impact | letter M → metal sparks |
| surface sky | bullet | impact | nothing |
| M3 shot, all pellets on concrete | 1 shot | impact | 9 fleck effects (one per pellet) |
| flecks, θ = 1, s = 1, I(4,16) = 4 | spawn | 0 s | 4 flecks |
| flecks, θ = 0.5, I(4,16) = 9 | spawn | 0 s | int(0.5 + 4.5) = 5 flecks |
| fleck spray | s = 1 / 2 / 3 | – | σ = 0.4 / 0.2 / 0.2 |
| fleck, n = (0,0,1), random cube = 0, U = 64, size 1 | spawn | 0 s | v = (0, 0, 128) |
| same, size 2 | spawn | 0 s | v = (0, 0, 64) |
| fleck alpha | life 3 | age 1.5 s | alpha 0.5 (cement); wood: 1.0 |
| fleck landing, floor n = (0,0,1) | v after this frame's gravity = (10, 0, −40), crosses the floor | that frame | v = 0, stays on the floor |
| fleck hitting floor fast | v after gravity = (0, 0, −100), keep draw 0.3 | that frame | v = (0, 0, 30) |
| fleck hitting wall n = (1,0,0) | v after gravity = (−100, 0, 0), keep draw 0.25 | that frame | v = (25, 0, 0) |
| surface colour | L = (0.5, 0.5, 0.5), B = (0.6, 0.5, 0.4) | – | c = (0.4378, 0.3649, 0.2919) |
| clamp-ramp | c = 0.9, ramp 1.25 | – | 255 |
| dust counts | θ = 1 / 0.5 / 0.25 / 0.1 | – | (4,4,4) / (2,2,2) / (1,1,1) / (0,1,0) |
| dust decay | dt = 0.01 | 1 frame | v × 0.83176 |
| dust decay | dt = 0.1 | 1 frame | v × 0.15849 |
| dust floor | |v| = 40, dt = 0.01 | 1 frame | 40 × 0.83176 = 33.27 (above floor); next frame 27.7 → set to 32 |
| dust alpha | life 1, age 0.2 / 0.25 / 0.26 / 0.5 | – | 0.8 / 0.75 / 0.5476 / 0.25 |
| big puff, j = 3, I(3,4) = 3, s = 1 | spawn | – | start size 12, end 48 |
| big puff, j = 0 | spawn | – | velocity 0, never moves |
| metal spark count | I(4,8) = 4 / 8, θ = 1 | – | 8 / 16 |
| metal spark speed | t = 0 / 1 / 2 | – | U(256, 1024) / U(128, 512) / 0 |
| metal spark damping | dt = 0.01 / 0.1 / 0.125 | 1 frame | v × 0.92 / × 0.2 / × 0 |
| metal glow | size 26 | 0.05 s | side 13, alpha 0.5 |
| electric big sparks | U(2,4) = 3.7 | – | 3 sparks |
| splash scale | s = 8 / 12 / 40 | – | f = 1 / 1.5 / 4 |
| gout, f = 1 | i = 0 / 7 | – | speed 400 / 50; start size 12 / 24; end 24 / 48; start alpha 32λ / 255λ |
| splash lighting | ℓ = (0.1, 0.1, 0.1) | – | λ = 0.4, c = (1, 1, 1) |
| splash lighting | ℓ = (0.02, 0.02, 0.02) | – | λ = 0.25 (floor) |
| splash lighting | ℓ = (0.5, 0.25, 0) | – | τ = (1, 0.5, 0), λ = 1 (clamped from 1.19), c = (1, 0.875, 0.75) |
| ripple, f = 1 | age 0.75 s of 1.5 | – | size 16 + 112 × 0.7 = 94.4; alpha λ × 0.75 |
| gout below surface | O.z = 0, size 10, pos.z = −11 | that frame | removed |
| gout sinking | O.z = 0, size 10, pos.z = −2.5 | – | alpha = start alpha × 0.5 |
| blood | player hit for 1 or for 100 | – | identical `blood_impact_red_01`: 7 smoke + 2 mist + 3 goop + 40 droplets + 15 chunks |
| blood decals | damage 9 / 10 / 24 / 25 | – | 1 / 2 / 2 / 4 traces, noise 0.1 / 0.2 / 0.2 / 0.3 |
| blood | `violence_hblood 0` | hit | no blood particles, no blood decals |
| `r_drawflecks 0` | concrete hit | – | no flecks; 2 + 4 + 1 dust sprites still spawn |
| bullet into water from air | s = 10 | – | splash at the surface, no decal or impact sound underwater |
| `cl_show_splashes 0` | bullet into water | – | nothing (and still no underwater impact) |
| frame time 0.25 s | any | 1 frame | particles advance 0.1 s |

## Open questions

1. **CS:S's own impact callback.** The client callback for impacts is
   game-specific and CS:S's is not public. This spec assumes it matches the
   SDK template (which mirrors CS). Check in game: shoot a concrete wall,
   a dirt floor, a metal door, a computer and a grate with `host_timescale
   0.1`, take screenshots and compare counts, colours and timings with
   sections 5–8.
2. **Template-only rules.** Scale 1, the 30 % `Bounce.Shrapnel` ricochet,
   and "no impact effect on teammates with friendly fire off" come from
   the template. Check: shoot a wall 50 times and count ricochet sounds
   (expect about 15); shoot a teammate with `mp_friendlyfire 0`.
3. **Penetration.** Does a bullet passing through a wall make an impact
   effect (and decal) where it exits? Check: shoot a thin wooden door and
   look at the far side.
4. **Throttle `θ`.** The particle library's budget is closed. Assume 1.
   Check whether fleck counts drop in a heavy smoke-grenade scene.
5. **Surface colour `B`.** Engine code: is it the base texture's average
   colour, the material's `$reflectivity`, or something else, and does the
   lighting include dynamic lights? Check: shoot a red and a white wall
   under the same light and compare fleck colours in screenshots.
6. **Blood details in CS:S.** Position (hit point or 4 in back), whether
   the shooter's own client makes it or receives it from the server, and
   hostage blood. Check: `cl_predict 0` vs 1 with a bot, `host_timescale
   0.1`, and `sv_showimpacts 1` to see where the burst centres relative to
   the hit marker.
7. **Particle operator semantics** (section 14), especially the frame of
   the "local" velocities (which way +Y and +Z point when the system is
   oriented by `−d` only), drag units, `tint_perc`, and the mist's start
   alpha 0 (is it invisible until the fade-out starts, or does fade-in to
   1 happen instantly?). Check: record `blood_impact_red_01` with
   `host_timescale 0.05` from the side.
8. **Splash size.** Template: U(8, 12) for every gun. CS:S may use per-ammo
   sizes (a commented CS line in the template gives 10–14 for .50 AE).
   Check: compare splash height of a Glock and a Deagle shot into water.
   Also check the template's underwater rule: standing in deep water, shoot
   an underwater wall; expect no decal, no particles and no impact sound.
9. **Unknown decal name.** If the decal lookup fails, the generic code
   returns before making particles; whether this happens for any CS:S
   surface (e.g. `no_decal`, letter X) is not known. Check: shoot a ladder
   (`ladder`, X) and a `no_decal` surface if a map has one.
10. **Wood fleck fade.** The wood fleck material has no vertex alpha, so
    by the material rules the fade is ignored. Check: watch wood flecks at
    `host_timescale 0.2`: do they fade or pop?
11. **CS:S knife**: its damage type (decides the decal and whether the
    ricochet rule could apply), whether it uses the template's water check,
    and whether the attacker's client predicts the impact. Check: knife a
    concrete wall and a metal wall; look for sparks/flecks and listen.
12. **`cl_new_impact_effects 1`**: does the cvar exist in CS:S, and if set,
    do impacts vanish (the `impact_*` systems are not in CS:S's manifest)?
    Harmless either way for us; default behaviour is what we implement.
13. **Effect detail setting.** `r_drawflecks` is set to 0 only for DirectX
    levels 7.0 and below (`bin/dxsupport.cfg`); whether the video menu's
    "Effect detail" changes anything here (e.g. `mat_reduceparticles`) is
    unknown.
