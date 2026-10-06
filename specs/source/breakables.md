# Source engine: func_breakable and func_breakable_surf

Source basis: Valve's public Source SDK 2013 only. I read the server
breakable brush code (func_breakable, and func_pushable which shares it),
the breakable glass surface code (func_breakable_surf and its falling
window_pane pieces, which has CS:S-specific lines), the shared breakable
damage scaling and prop data loader, the generic damage path, the breakable
gib temp-entity message and its client-side spawning (count, velocity,
lifetime, render mode). From the user's CS:S install I read
scripts/propdata.txt (gib model lists), the sound scripts (break sounds)
and the entity lumps of de_nuke, cs_office, cs_italy, de_chateau,
de_piranesi and cs_havana. The client's glass rendering (crack textures,
per-pane drawing) and the shard effect's physics are only outlined here;
shard visuals for bullet hits are in specs/cs_source/impact_effects.md
section 9.
Status: draft

## Summary

func_breakable is a solid brush with health and a material. Damage lowers
its health (bullets do half, clubs 1.5×, blasts 1.25×); at 0 it plays the
material's break sound, throws up to 15 gib models from the material's
gib list from random points inside it, becomes non-solid and loses its
name at once, and deletes itself 0.1 s later. de_nuke's vents are
func_breakable with material Metal and 1 health: one bullet opens them.
func_breakable_surf is a glass (or tile) window made of a grid of 12×12
unit panes: the first hit "breaks" the window as an entity (it becomes
non-solid at once, but its panes are still drawn), and from then on panes
shatter where they are hit or touched, and panes that lose support from
their neighbours collapse tick by tick.

## Units and conventions

Source units, Z up, seconds, dt = 0.015 s. Health is an integer. Damage
type bit values as in specs/source/triggers.md (bullet 2, club 128, blast
64, crush 1, slash 4, buckshot 536870912, burn 8, sonic 512). Material
numbers are the "material" keyvalue values Hammer writes.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| dmg_mod_bullet | 0.5 | × | func_breakdmg_bullet cvar: bullet damage multiplier |
| dmg_mod_buckshot | 1.0 | × | buckshot: 2 × bullet modifier |
| dmg_mod_club | 1.5 | × | func_breakdmg_club cvar |
| dmg_mod_blast | 1.25 | × | func_breakdmg_explosive cvar |
| max_pieces | 15 | gibs | func_break_max_pieces cvar (also caps the client) |
| reduction_factor | 0.5 | × | func_break_reduction_factor cvar (reduced-gibs mode) |
| shard_size | 12 | units | gib count reference cell |
| gib_speed_rand | 100 | units/s | random velocity spread per axis |
| gib_directed_speed | 200 | units/s | directed / precise gib speed |
| gib_life | 2.5 + U(0,1) | s | gib lifetime before removal (fades out) |
| gib_glass_alpha | 128 | 0..255 | glass gibs are translucent |
| gib_glass_bounce | 0.3 | — | glass gib bounce factor |
| remove_delay | 0.1 | s | broken func_breakable deleted after this (7 ticks) |
| pane_size | 12 | units | func_breakable_surf nominal pane size |
| max_panes_axis | 16 | panes | per axis |
| pane_full_support | 6.75 | — | support normaliser |
| pane_break_support | 0.2 | — | collapse threshold × fragility/100 |
| small_shard | 4 | units | shard size for single panes |
| large_shard | 7 | units | shard size for blast shatter |
| pane_bullet_force | 500 | × dir | force on panes shattered by a bullet |
| blast_force | 3000 × damage | units | CS:S: blast shatter force (generic code uses 3000) |

## Behavior

### func_breakable: keyvalues and spawn

Keyvalues: "material" (0 Glass, 1 Wood, 2 Metal, 3 Flesh, 4 CinderBlock, 5
CeilingTile, 6 Computer, 7 UnbreakableGlass, 8 Rocks, 9 Web, 10 None;
out-of-range → Wood), "health", "explosion" (gib direction: 0 random, 1
relative to attack, 2 use "gibdir"), "gibdir" (angles), "gibmodel"
(override gib list name or model), "spawnobject" (HL2 item index; ignore
in CS:S), "propdata" (index into a prop-data template table: 1 Wooden.Tiny
.. 24 Glass.Window; 0 none), "explodemagnitude", "ExplodeDamage",
"ExplodeRadius" (explode on break), "PressureDelay", "minhealthdmg" (hits
below this are ignored), "physdamagescale" (physics impact damage scale,
0 → 1), "PerformanceMode" (0 normal, 1 no gibs, 2 full gibs, 3 reduced
gibs), "nodamageforces", "damagefilter" (filter entity name for damage).
Spawnflags: 1 "Only Break on Trigger", 2 "Break on Touch", 4 "Break on
Pressure", 512 "Break immediately on Physics", 1024 "Don't take physics
damage", 2048 "Don't allow bullet penetration".

Spawn:
- A "propdata" template, if any, fills in health and other values first.
- If health is 0 **or** flag 1: glass gets health 1, and the entity is
  **not damageable** (only the Break input or touch/pressure flags can
  break it). Else damageable.
- Max health = health (≥ 1). It is a solid, static brush (blocks players,
  bullets and physics).
- UnbreakableGlass (7) never breaks (Break input does nothing) and shows a
  "BulletProof" decal; Glass shows "GlassBreak" decals.

### Damage

On a damage event with amount A and type T:
1. Ignore if not damageable or A < minhealthdmg.
2. If a damage filter is set and rejects the attacker: absorbed (no
   effect).
3. Scale A: bullet type: × 0.5 (× 1.0 if also buckshot); club: × 1.5;
   blast: × 1.25 (each modifier applies if its bit is set, multiplying).
4. New health = trunc_toward_zero(health − A) (integer health minus a
   float, truncated: 50 − 17.5 = 32.5 → 32).
5. If health changed: fire **OnHealthChanged** with
   clamp(health/maxhealth, 0, 1) (float), activator = attacker. If ≤ 0:
   **break** with the attacker as breaker. Else stay damageable (unless
   flag 1).
6. If still alive and the damage was not burn: play the material's damage
   sound ("Breakable.MatGlass", MatWood, MatMetal, MatConcrete for rocks
   and cinder, "Breakable.Computer"; computer 50 % uses metal), volume
   U(0.75, 1), pitch 100 two times in three, else 95 + I(0, 34).
- Bullet traces against Computer material: 50 % chance of sparks and a
  "Breakable.Computer" sound; UnbreakableGlass: 50 % ricochet effect.
- Physics impacts damage it unless flag 1024 (energy-based tables, see
  specs/cs_source/physics_props.md); with flag 512 the first impact breaks
  it outright. Glass hit by physics has its (static) mass set to 2.

Inputs: **Break** (break now, breaker = activator; works even if not
damageable), **SetHealth** n, **AddHealth** n, **RemoveHealth** n (set
health, fire OnHealthChanged, break at ≤ 0; note: these work while "not
damageable"), **SetMass** (physics only).

Touch flags (players only):
- 2 "Break on Touch": damage = player's smoothed speed × 0.01; if that is
  ≥ health: break it (crush) and give the player damage/4 slash damage
  with a melee push. (Runs at 100+ units/s per health point: a health-1
  pane breaks when touched at ≥ 100 units/s.)
- 4 "Break on Pressure": when a player stands on it, play the damage sound
  and break after "PressureDelay" seconds (nearest tick).

### Breaking (func_breakable)

1. Sound by material: Glass "Breakable.Glass", Wood "Breakable.Crate",
   Metal and Computer "Breakable.Metal", Flesh/Web "Breakable.Flesh",
   Rocks/CinderBlock "Breakable.Concrete", CeilingTile "Breakable.Ceiling";
   others none. Pitch 95 + I(0, 29), snapped to 100 if 98..102; volume
   U(0.85, 1) + |health|/100, capped at 1. The game event "break_breakable"
   (userid, entindex, material) is sent if a player broke it.
2. Gib velocity base v0: explosion 0 → (0,0,0); 1 → −200 × attack
   direction (the attack direction points from the breakable toward the
   attacker, so gibs fly away from the attacker); 2 → 200 × forward(gibdir).
3. Gib count n = floor((sx·sy + sy·sz + sz·sx) / 432) with (sx, sy, sz) the
   brush size (432 = 3 × 12²); n = min(n, 15). PerformanceMode 1 → 0; 3 →
   max(floor(n × 0.5), 1) (only when n > 0). (The cvar
   breakable_disable_gib_limit turns these modes off.)
4. Gib list by material (game data, scripts/propdata.txt "BreakableModels";
   override with "gibmodel"): Wood "WoodChunks" (5 models
   models/gibs/wood_gib01a..e), Glass/UnbreakableGlass "GlassChunks" (6
   models/gibs/glass_shard01..06), Metal "MetalChunks" (5
   models/gibs/metal_gib1..5), Rocks "ConcreteChunks" (6
   models/props_debris/concrete_chunkNNa), CinderBlock → ConcreteChunks;
   other materials → WoodChunks with a warning (Flesh, Web, Computer,
   CeilingTile in non-HL1 builds; None → no gibs in episodic builds only).
5. Send n gib messages (one gib each) to clients in the PVS of the centre.
   Each gib on the client: model chosen uniformly from the list; position =
   a uniformly random point of the brush's box (in its rotation); random
   body group; 78 % of gibs spin with angular velocity U(−256, 255) °/s per
   axis; velocity = v0 + (U(−100,100), U(−100,100), U(0,100)); half
   gravity; collides with the world and bounces; life 2.5 + U(0,1) s then
   fade out. Glass gibs: translucent (alpha 128), bounce factor 0.3. The
   break flag (glass/wood/metal/flesh/concrete) picks the gib's bounce
   sounds. Gibs are client-side only and never block anything.
6. Unground everything standing on it (entities with "on ground" whose
   origin is inside its box grown by 1 down and 8 up).
7. **Clear its targetname** (later outputs naming it find nothing), make it
   **non-solid immediately**, fire **OnBreak** (activator = breaker),
   remove its physics object, schedule deletion in 0.1 s (7 ticks).
8. If explodemagnitude/ExplodeDamage is set: an explosion at the centre
   (out of scope until grenades/explosions are specified).

So bullets that killed it and any player behind it pass through on the
same tick; the brush is still drawn until it is deleted 7 ticks later
(Open questions: CS:S may hide it at once).

### de_nuke's vents and windows (real data)

- 55 func_breakable on de_nuke: 50 are material 0 (glass windows), health 1;
  5 are **material 2 (Metal), health 1, spawnflags 0**: the vents (two at
  z ≈ −568 near the lower ramp/vent route, two at z ≈ −380, one at
  (640, −1535, −568)). One bullet (1 → 1 − 0.5·damage ≤ 0) breaks a vent;
  a knife slash (club? see Open questions) too. They play
  "Breakable.Metal" and throw MetalChunks gibs.
- 4 func_breakable_surf windows on de_nuke, 14 on cs_office, all glass
  (surfacetype 0), fragility 100, health 1.

### func_breakable_surf (glass panes)

Keyvalues: "surfacetype" (0 glass, 1 tile), "fragility" (0..100, default
100 in maps), "lowerleft", "upperleft", "lowerright", "upperright" (the
four corners of the drawn face, written by the compiler), "error" (0 ok, 1
more than one non-nodraw face, 2 face not a quad: both mean the entity
deletes itself at spawn), plus all func_breakable keyvalues (health,
material...). Spawnflags: 1 "Physics damage decals", 2 "Take damage from
held objects". The brush must use exactly one material on its drawn face
(else it deletes itself); the material's "$crackmaterial" is the broken
look.

**Unbroken state**: a solid, damageable func_breakable brush with its own
material drawn normally. Health from the keyvalue (glass with 0 → 1).

**Taking a hit** (every bullet/club trace hit, before any health check):
1. health −= damage; fire OnHealthChanged with the new (integer) health.
2. If the window is not yet broken: **break the window entity** with the
   attack direction = hit point − trace start (see "Window break").
3. Bullet or club: find the hit pane (column, row) = floor of the hit's
   position in pane units. Shatter that pane (if not already broken), with
   force 500 × shot direction; if it shattered, send the glass-impact
   effect (specs/cs_source/impact_effects.md section 9). Glass only:
   - if the hit is in the outer 20 % of the pane horizontally (fraction >
     0.8 or < 0.2), also shatter the neighbour on that side (not past the
     edge); same vertically;
   - 50 %: shatter the pane above (row + 1) with force 1000 × dir; and then
     50 %: also the one two above.
4. Sonic or blast type (glass): shatter everything: if fewer than 10 % of
   panes are broken, one large shard burst for the whole window (shard
   size 7), else one burst per vertical run of unbroken panes per column;
   then mark all panes broken. Force = ±3000 × normal × (CS:S: the damage
   amount; other games: 1), pointing along the attack. Tile with blast:
   breaks panes in a diamond around the attacker (|dx| + |dy| < I(2,5)
   within ±4).
- Damage events (not traces): crush on an unbroken window, blast or slash
  on glass → break the window entity; nothing else happens to the panes.

**Window break** (once):
1. Play the physics break sound for its surface; mark broken; health 0;
   fire **OnBreak** (activator = breaker, or the window itself if none).
2. Orient: if the attack came from the back side, flip the normal (and
   move the corners 1 unit along it). Compute width vector W = LL − LR,
   height vector H = LL − UL, normal = W × H (normalised); choose the
   reference corner and swap W/H if needed so that panes run along the
   window's right and up directions.
3. Panes: columns = floor(|W| / 12), rows = floor(|H| / 12), each at most
   16; pane size = |W|/columns by |H|/rows (so panes are slightly larger
   than 12 to fill the window exactly). All panes start "healthy" (support
   1).
4. Unground anything standing on it; delete its physics object; make it
   **non-solid and a trigger**: from now on it blocks nothing; a player
   walking into it triggers pane shattering by touch (below).
- Bullets still hit it after the break (the engine trace includes it for
  shots; Open questions), so later shots shatter more panes.

**Touch** (after break; any entity overlapping its box, every tick):
- Tile: only if the toucher moves at ≥ 500 units/s.
- Project the toucher's box onto the window plane and convert to a pane
  range [c0, c1) × [r0, r1) (floor/ceil, clamped). For each row in range:
  50 % shatter the pane left of the range; shatter every pane in range;
  50 % shatter the pane right of the range (note: the right one is c1 + 1,
  skipping c1, quirk). Force = toucher velocity × 5.

**Shatter a pane**: mark broken (count it), send a shard burst for that
pane (pane size, shard size 4, white front/back for glass; tile uses
green/brown and pushes the burst 8 units behind with −0.75 × force), play
the damage sound, and schedule a **support pass** for this tick.

**Support pass** (glass only; runs as a think, then again every tick as
long as something collapsed):
1. For every unbroken pane (c, r), compute
   S = 0.01 + up + 1.25·down + left + right + 1.0·downleft + 1.0·downright
   + 0.25·upright + 0.25·upleft, where each term is the neighbour's
   current support (0 if broken) or a fixed edge value if the neighbour is
   outside the window: top edge (up) 1.0, bottom edge (down) 1.25, left and
   right edges 1.0, any diagonal touching the bottom edge or a side edge
   1.0 (down-left/down-right) or 0.25 (up-left/up-right). All supports are
   computed from the old values first.
2. Then set each unbroken pane's support = S / 6.75 (so a fully supported
   pane has 6.76/6.75 ≈ 1.0015).
3. If a pane's new support < 0.2 × fragility/100: it collapses: 50 %
   shatter (as above, no force), 50 % **drop**: mark broken, small shard
   burst, damage sound, and spawn a falling pane piece (model
   models/brokenglass_piece.mdl, random body 0..2, spinning U(−120,120)
   °/s, falls with gravity, box collision) at the pane's corner. The piece
   shatters into shards when it touches anything that is not glass and
   then deletes itself. Each collapse re-schedules the pass for the next
   tick (cascades spread one ring per tick).
- Panes are held mostly from below (weight 1.25) and the sides; a pane
  with every neighbour broken has support 0.01/6.75 and always falls.

**Shatter input**: "Shatter" with a vector (x, y, r): break the window if
needed, then shatter every pane whose centre is within r units of the point
(x·columns, y·rows) in pane units scaled by pane size (x, y are 0..1
fractions across the window).

**What the client draws** (outline): before the break, the brush face;
after, each unbroken pane is drawn with the crack material, edges facing
broken neighbours get jagged edge pieces (glassbroken_* materials;
tilebroken_* for tile). Details of edge selection are not specced here.

## Per-tick order

1. Bullet traces and damage happen during the shooter's command (break,
   pane shatter, support pass scheduled).
2. Touch shattering happens when entities re-link (players: after
   movement).
3. Support passes run when the window entity thinks (once per frame, in
   think-list order), cascading one step per tick.
4. OnBreak / OnHealthChanged are delivered in the event queue at the end
   of the frame (specs/source/entity_io.md).
5. A broken func_breakable is deleted 7 ticks after breaking.

## Edge cases

- A func_breakable named "box" that broke cannot be targeted as "box"
  anymore, even during its 7 remaining ticks.
- Health 0 non-glass without flag 1 is unbreakable by damage (but Break
  works).
- RemoveHealth 5 on a not-damageable breakable with health 1 breaks it.
- Gib count 0 (small brush, e.g. 8×8×8: 192/432 → 0) → no gibs even in
  reduced mode.
- A window whose quad is smaller than 12 in a direction gets 0 panes in
  that direction (division by zero in the original; guard: treat as 1).
- Very large windows: more than 16 × 12 units per axis still use 16 panes
  (panes get bigger).

## Quirks

- **Bullets do half damage to breakables** (keep: affects how many shots
  vents/crates take).
- **Name cleared on break** (keep).
- **Window becomes non-solid at the first hit even though most panes are
  still drawn** (keep; players can walk through an intact-looking cracked
  window and shatter it by touch).
- **Touch shatter skips one column on the right side** (keep, cosmetic).
- **CS:S scales blast shatter force by damage** (keep).

## Test cases

| Setup | Input | After | Expected |
|---|---|---|---|
| func_breakable material 0, health 1 (nuke window) | one bullet of 26 damage | same tick | breaks: "Breakable.Glass", OnBreak, non-solid, name cleared; deleted 7 ticks later |
| func_breakable material 2, health 1 (nuke vent), 64×8×32 brush | one bullet | — | breaks; floor((512 + 256 + 2048)/432) = 6 MetalChunks gibs |
| func_breakable wood, health 50 | bullets of 36 damage | — | health 32, 14, then −4 → breaks on the 3rd shot |
| same | one club hit of 34 | — | 50 − 51 = −1 → breaks |
| same | blast 40 | — | 50 − 50 = 0 → breaks |
| func_breakable wood, health 50 | bullet 35 | — | health 32 (50 − 17.5 truncated) |
| func_breakable wood, health 50, minhealthdmg 20 | bullet 36 (raw 36 ≥ 20, scaled 18) | — | health 32 |
| same | bullet 18 (raw 18 < 20) | — | ignored, health 50 |
| func_breakable glass, health 0 | bullets | — | not damageable, never breaks; Break input breaks it |
| func_breakable flag 1 (trigger only), health 10 | shots | — | no effect; RemoveHealth 10 breaks it |
| brush 128×128×128 | break | — | 113 → 15 gibs; PerformanceMode 3 → 7; mode 1 → 0 |
| brush 8×8×8 | break | — | 0 gibs |
| explosion 2, gibdir "0 90 0" | break | — | gib velocities (U(−100,100), 200 + U(−100,100), U(0,100)) |
| explosion 1, attacker due −X of the breakable | break | — | gibs move +X on average (≈ 200) |
| flag 2 (touch), health 1, glass | player walks into it at 250 units/s | — | breaks; player takes 0.625 slash damage |
| same | player touches at 80 units/s | — | nothing |
| cs_office window *1: corners LL (−508,−344,−148), LR (−628,−344,−148), UL (−508,−344,−52) | first bullet | — | OnBreak; 10 columns × 8 rows of 12 × 12 panes; window non-solid; hit pane shattered |
| same window, bullet hits pane (4, 3) at fraction (0.9, 0.5) within the pane | — | — | panes (4,3) and (5,3) shatter; 50 % also (4,4), and then 50 % (4,5) |
| interior pane with all 8 neighbours broken | support pass | — | support 0.0015 < 0.2: collapses (50 % shatter, 50 % falling piece) |
| bottom-left pane (0,0), all inside neighbours broken | support pass | — | S = 0.01 + 1.25 + 1 + 1 + 1 + 0.25 = 4.51 → 0.668: stays |
| fully intact window | support pass | — | every pane support 1.0015 |
| player walks into a broken window (box 32 wide) | one tick of overlap | — | about 3 columns × all rows the box spans shatter (plus random edge panes) |
| unbroken glass window, HE grenade blast 100 damage (CS:S) | — | — | whole window shatters as one large-shard burst, force 300000 units along the blast |
| func_breakable named "vent" | OnBreak `vent,Break,,0.1,-1` loop attempt | — | after breaking, "vent" no longer resolves |

## Open questions

1. **CS:S bullet damage type** to breakables: does CS:S use the bullet bit
   (×0.5)? Probe: func_breakable wood health 100, shoot one Glock round
   (25 damage at short range); read health via `ent_text`-like output or
   OnHealthChanged → logic_compare → say. Expect 88 (100 − 12.5).
2. **CS:S knife** damage type on breakables (slash/club?) and multiplier;
   same probe with the knife.
3. **minhealthdmg**: is it compared before scaling? (Code: yes, the raw
   amount.) Confirm with minhealthdmg 20 and a 26-damage shot.
4. **Broken brush visibility**: is the broken func_breakable hidden the
   moment it breaks or drawn for 7 more ticks? Screenshot sequence on the
   reference client.
5. **Shots through a broken window**: do bullets keep hitting a
   func_breakable_surf after its first break (it is non-solid then)? Shoot
   the same cracked window twice and watch pane loss.
6. **Gib physics**: client gibs use "slow gravity" (half) and bounce with
   the material flag sounds; confirm visually that metal vent gibs fall at
   half gravity.
7. **de_nuke vent identification**: confirm the four material-2 brushes
   at z −568 and −380 are the vents players use (screenshot at those
   origins in mashup).
8. **Glass pane render details** (edge piece selection, crack material,
   back-face) for the client: needs a separate pass on the client glass
   code.
