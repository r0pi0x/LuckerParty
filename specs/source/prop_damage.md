# Source engine: prop damage, breaking and prop inputs (as CS:S uses them)

Source basis: Valve's public Source SDK 2013 only (the current multiplayer
branch). I read the server prop entities (the shared base prop, breakable
prop, dynamic prop, physics prop and the multiplayer physics prop), the
shared prop-data loader and gib spawning, the server physics impact-damage
tables and stress code, the server collision-event and deferred-damage
queue, the generic damage entry points (filters, damage scaling, kill and
removal, hierarchy cleanup on removal), the explosion entity, the entity
flame, the light entity's on/off inputs, the base animating inputs, the
sequence-transition helper, the shared break-sound queue, and on the client
the client-side physics prop (its damage, breaking and break pieces), the
physics-prop temp entity and the explosion temp entity's push on client
objects. CS:S's own game DLL (weapons, its radius damage, its player class)
is not in the SDK; where CS:S code could change a result it is listed under
Open questions with an in-game check.

From the user's CS:S install I read `scripts/propdata.txt` (CS:S's own copy,
which differs from HL2's), the surface-property scripts, the physics sound
scripts, the `prop_data` / interaction sections of every model the stock
maps place as `prop_physics*` / `prop_dynamic*` (264 model/class pairs), their
`.phy` text sections (mass, surfaceprop, break pieces), a few models'
sequences and body groups, and the entity lumps of the 18 stock maps
(cs_assault, cs_compound, cs_havana, cs_italy, cs_militia, cs_office,
de_aztec, de_cbble, de_chateau, de_dust, de_dust2, de_inferno, de_nuke,
de_piranesi, de_port, de_prodigy, de_tides, de_train).
Status: draft

## Summary

A prop's health, damage multipliers, gib list and multiplayer behaviour come
from the model, not the map: the model's `prop_data` block names a template
in `scripts/propdata.txt` and may override its keys; the map's `health` key
is ignored except on the two `_override` classes. A prop takes real damage
only if it has health **and** something to break into (model break pieces,
template gib chunks, or a fire/explode interaction); every other prop takes
"events only" damage: it never loses health but fires OnTakeDamage and
OnHealthChanged (with 0) on every hit and still gets pushed. Damage is
scaled per prop by type (bullets, club, blast), integer health is reduced,
and at 0 the prop breaks: OnBreak fires, its break sound is queued, it may
explode, it is removed at once (name cleared, parented children deleted),
and in multiplayer its pieces are spawned **on each client only**, as
client-side physics debris that never affects gameplay.

Two other things matter for feel. Physics impacts damage props by the
kinetic energy the collision removed, read through a step table (5, 10, 50,
100, 500 hp). And 860 of the stock maps' 1954 `prop_physics_multiplayer`
(those with physics mode 3, 44 %) do not exist on the server
at all: each client simulates and breaks them by its own simple rule (a
bullet always does 30, a grenade push does about 32+).

## Units and conventions

Source units (in), Z up, seconds, kg. Server tick dt = 0.015 s. Health is an
integer; damage amounts are floats until subtracted. Impulses in kg·in/s
(specs/cs_source/physics_props.md). Damage-type bit values: crush 1,
bullet 2, slash 4, burn 8, blast 64, club 128, sonic 512, prevent-force
2048, drown 16384, the time-based group = bits 15–21 (paralyze, nerve gas,
poison, radiation, drown-recover, acid, slow burn), blast-surface 2^27,
direct 2^28, buckshot 2^29. Output, input, keyvalue, prop_data key and
template names are the game's data names.

"Server prop" = an entity simulated on the server. "Client prop" = a
client-only physics prop (section 2.3). The stock maps run in multiplayer
(more than one player slot), which selects the multiplayer paths below.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| default_dmg_mod | 1.0 | × | bullet/club/blast multiplier when the template sets none |
| func_breakdmg_bullet / club / explosive | 0.5 / 1.5 / 1.25 | × | cvars; used only by brush breakables (breakables.md), never by props |
| buckshot_factor | 2 | × | bullet multiplier is doubled when the buckshot bit is also set |
| slash_crush_factor | 10 | × | damage with both slash and crush bits (spinning blades) |
| default_impact_scale | 0.1 | – | `physdamagescale` 0 → 0.1 (every stock prop has 0.1 except 3 de_port crates with 1) |
| max_health_floor | 1 | hp | max health = health if > 0, else 1 |
| auto_clientside_volume | 15³ = 3375 | in³ | multiplayer prop with a smaller box becomes client-only (cvar `sv_pushaway_clientside_size` 15) |
| auto_nonsolid_mass | 8 | kg | lighter (and not small) → mode 2 |
| chunk_size_unit | 32 × 32 = 1024 | in² | template gib chunk size limit divisor |
| piece_burst | 100 | in/s | default outward speed added to break pieces |
| piece_fade_default | 20 | s | model break piece life if it gives no `fadetime` |
| chunk_fade | U(5, 10) | s | template chunk life |
| client_fadeout | 1.0 | s | client pieces fade alpha over this after their life |
| piece_vel_jitter | ±2.5 % | – | client piece velocity × (1 + U(−0.025, 0.025)) |
| cl_phys_props_max | 300 | props | client prop limit (map props ignore it, pieces don't) |
| client_bullet_damage | 30 | hp | every bullet impact on a client prop |
| client_other_damage | 50 | hp | any other non-blast impact effect on a client prop |
| client_bullet_push | 4000 | kg·in/s | along the shot, at the hit point |
| client_blast_push_scale | 500 | × | blast push factor (section 3.2) |
| client_blast_lift | 32 | in | blast source lowered by this for client objects |
| flame_tick | 0.2 | s | entity flame damage interval (first tick 0.1 s after ignition) |
| flame_self_damage | 1 | hp/tick | burn + direct, to the burning prop (5 hp/s) |
| flame_area_damage | 4 | hp/tick | burn, to others within flame_size / 2 |
| flame_size | max(16, (box_x + box_y) / 2) | in | |
| burn_time_blast | U(10, 15) | s | ignition time from non-lethal blast or burn |
| player_impact_scale | 1.0 | – | players' impact energy scale |
| player_stress_scale | 0.125 | × | applied to the player's scale for stress (crush) |
| stress_body_weights | 5 | – | cvar `phys_stressbodyweights` |
| stress_damage | 200 | hp | crush damage when stress is exceeded |
| large_object_mass | 500 | kg | "large object" for stress, impact tables, velocity restore |

## Behavior

### 1. Prop classes and what can be damaged

| Class | Takes damage | Notes |
|---|---|---|
| `prop_static` | never | baked into the map by the engine; not an entity; bullets and blasts treat it as world |
| `prop_dynamic` | events only, unless its model has prop_data (then deleted, below) | animated, not physically simulated |
| `prop_dynamic_override` | as prop_data says, with the **map's** health | becomes classname `prop_dynamic` after spawn |
| `prop_physics` | as prop_data says | a server rigid body |
| `prop_physics_override` | as prop_data says, with the **map's** health | becomes classname `prop_physics` after spawn |
| `prop_physics_multiplayer` | as prop_data says | server rigid body (modes 1, 2) or client-only (mode 3), section 2.3 |
| `prop_door_rotating` | – | specs/source/doors_buttons.md |

Spawn-time deletion (server), checked after reading the model:
- No model name → deleted.
- prop_data names a template that does not exist → deleted (any class
  except the two `_override` classes).
- `prop_physics` whose model has **no** prop_data → deleted. (cs_havana's 4
  `prop_physics` with `controlroom_desk001b.mdl` never exist in game.)
- `prop_dynamic` (not `_override`) whose model **has** prop_data → deleted,
  unless the prop_data (or a template in its chain) says `allowstatic` 1.
  None of the 172 stock `prop_dynamic` use such a model.
- `prop_physics_multiplayer` is exempt from the "no prop_data" and "has
  prop_data" checks (not from the missing-template one); both `_override`
  classes are exempt from all three.

### 2. Prop data: health, multipliers, gibs, physics mode

#### 2.1 Resolving a model's prop data

1. Before reading anything, the prop's multipliers are bullet 1, club 1,
   blast 1, and its health is the map's `health` keyvalue **only on the two
   `_override` classes**; every other class ignores the map's `health`
   (stock maps set it to 0 on 16 multiplayer props and 5 dynamic overrides;
   it has no effect).
2. The model's keyvalue text has a `prop_data` block. If it has a `base`
   key, the named template from `scripts/propdata.txt` is applied first,
   recursively (a template may have its own `base`). Then the model's own
   keys are applied on top. Each key overwrites only if present.
3. Keys: `health`, `dmg.bullets`, `dmg.club`, `dmg.explosive`,
   `physicsmode`, `breakable_model`, `breakable_count`, `breakable_skin`,
   `damage_table`, `explosive_damage`, `explosive_radius`,
   `multiplayer_break` (`server`, `client`, `both`; absent everywhere in
   CS:S data), `allowstatic`, `blockLOS`, `AIWalkable`.
   - On the `_override` classes, `health`, `explosive_damage` and
     `explosive_radius` from prop_data are **ignored** (the map's values
     stay); multipliers, gibs and the rest still apply.
   - A prop that ends up with health > 0 loses spawnflag 2 ("don't take
     physics damage").
4. Interactions are read from separate model blocks next to prop_data:
   `physgun_interactions` and `fire_interactions` (or from inside a template
   block). The ones that matter here: `fire_interactions` → `flammable`
   `yes`, `explosive_resist` `yes`, `ignite` `halfhealth`;
   `physgun_interactions` → `onbreak` `explode_fire`, `onfirstimpact`
   `break`. (Physgun launch/paint/stick/impale interactions need a gravity
   gun; CS:S has none, so they never trigger.)
5. Template gib size limit: from the prop's box size (sx, sy, sz), replace
   the smallest by 1 and take floor(product / 1024). (Smallest axis: x if
   x < y and x < z; else z if x < y; else y if y < z; else z. Ties pick the
   later axis.)

CS:S's `propdata.txt` templates (health, bullet/club/blast multipliers,
physics mode, gib chunks):

| Template | Health | Bullet | Club | Blast | Mode | Gib chunks / count | Table |
|---|---|---|---|---|---|---|---|
| Cardboard.Base | – | 0.5 | 1.25 | 1.5 | – | – | |
| Cardboard.Small / Medium / Large | 10 / 20 / 40 | ← | | | – / – / 1 | – | |
| Cardboard.break / breakclient | 10 / 10 | ← | | | 1 / 3 | – | |
| Cardboard.Indestructable | – | ← | | | – | – | |
| Cloth.Base | – | 0.5 | 0.75 | 1.5 | – | – | |
| Cloth.Small / Medium / Large | 30 / 50 / 100 | ← | | | – / – / 1 | – | |
| Cloth.Object | – | ← | | | 3 | – | |
| Wooden.Base | – | 0.75 | 2.0 | 1.5 | – | WoodChunks, skin 0 | |
| Wooden.Tiny / Small / Medium / Large / Huge | 6 / 20 / 30 / 50 / 130 | ← | | | 3 / – / – / 1 / 1 | count 0 / 2 / 4 / 6 / 10 | |
| Wooden.sticks / Barrel / Small2 / Barrel2 | – / 50 / 1 / 201 | ← | | | 1 | count 0 / 0 / 2 / 0 | |
| Stone.Base | – | 1 | 1 | 1 | – | – | |
| Stone.Small / Medium / Large / Huge / Gigantic | 50 / 100 / 200 / 400 / 600 | ← | | | 3 / – / 1 / 1 / 1 | – | |
| Glass.Base | – | 1 | 1 | 0.1 | – | – | |
| Glass.Small | 5 | ← | | | 3 | – | glass |
| Glass.Window | 1 | 0.5 | 1 | 1.0 | 1 | – | glass |
| Glass.CSWindow / CSWindow2 | 1 | (Window) | | | 1 | – | glass |
| Glass.picture | – | ← | | | 1 | – | |
| Metal.Base | 0 | 1 | 1 | 1 | – | – | |
| Metal.Small / Medium / Large | – | ← | | | – / 1 / 1 | – | |
| Metal.break / break2 / MediumClient | 10 / 100 / – | ← | | | 1 / 1 / 3 | – | |
| Plastic.Base | 0 | 1 | 1 | 1 | – | – | |
| Plastic.Small / Medium / Large / Small2 | – | ← | | | – / – / 1 / 1 | – | |
| Plastic.break / PlasticSmall.break | 10 / 10 | ← | | | 1 / 3 | – | |
| Item.Base (Small, Medium, Large) | 0 | 1 | 1 | 1 | Large 1 | – | |
| Pottery.Base | – | 1 | 1.25 | 1.5 | – | – | |
| Pottery.Small / Medium / Large / Huge | 5 / 40 / 70 / 100 | ← | | | – / – / 1 / 1 | – | Small: glass |
| Pottery.break / Plant / break2 / PlantBreak | 20 / – / 20 / 20 | ← | | | 3 / 3 / 1 / 3 | – | |
| Flesh.Base | – | 1.25 | 1 | 1.5 | – | – | |
| Flesh.Tiny / Small | 3 / 10 | ← | | | – | – | |

"←" = inherits the base's multipliers; "–" = key absent. Chunk lists
("BreakableModels"): WoodChunks = `models/gibs/wood_gib01e, 01d, 01c, 01b,
01a` (in that order), GlassChunks = `glass_shard01..06`, ConcreteChunks =
`props_debris/concrete_chunk08a, 09a, 03a, 07a, 09a, 02a`, MetalChunks =
`gibs/metal_gib1..5`. (A chunk list is found by name prefix: the first list
whose name starts with the requested name.)

#### 2.2 Damageable or events only (server props)

At spawn, after prop data:
1. `pieces` = the number of `break` blocks in the model's `.phy` text whose
   model resolves; if 0 and the prop data has a `breakable_model` and a
   non-zero `breakable_count`, `pieces` = that count.
2. If health = 0, **or** pieces = 0 and the prop has none of the
   interactions flammable / explosive_resist / ignite-halfhealth /
   onbreak-explode / onfirstimpact-break: health := 0 and the prop is
   **events only**. Otherwise it is **damageable**.
3. Max health = health if > 0, else 1.
4. Impact energy scale = `physdamagescale` keyvalue; 0 → 0.1.
5. A spawn tick is recorded (section 4, step 7).

Events-only props never lose health and never break from damage, but the
Break input and SetHealth/RemoveHealth still break them (section 8).

Effective stock examples: `oildrum001` (Metal.Medium, 177 placements),
`cardboard_box01` (Cardboard.Base) and `fire_extinguisher` are events only;
`chair_office` (Cloth.Large, health 100, no pieces) is events only too.

#### 2.3 Multiplayer physics mode (`prop_physics_multiplayer` only)

Mode: the prop data's `physicsmode` if set, else **auto**: box volume
sx·sy·sz < 3375 → 3; else mass < 8 kg → 2; else 1. Spawnflag 8192 ("force
server side") forces mode 2. Mode 1 = solid, pushes players away; mode 2 =
not solid to players, pushed by them; mode 3 = **client only**. Collision
behaviour of modes 1–3: specs/cs_source/physics_props.md 4.2.
(Box = the model's bounds; mass = the `.phy` mass × `massscale`.) The map
keyvalue `physicsmode` would set the mode before prop data, but stock maps
never write it (they write `multiplayer_physics_mode`, which the SDK code
does not read; the 7 props with value 1 resolve to mode 1 anyway).

- Mode 3 props are **removed on the server at spawn**; each client creates
  its own copy from the map's entity text (section 3). They have no name,
  no outputs and no server existence: server bullets pass through them,
  explosions don't touch them, map logic can't reach them.
- A model without a `.phy` in auto mode is removed on both sides.

Stock counts (multiplayer props only): 1037 mode 1, 57 mode 2, 860 mode 3.
Damageable (health > 0 and something to break into): 388 of mode 1, 8 of
mode 2; on clients, 326 mode-3 props have health > 0 (section 3.1).

### 3. Client-side props (mode 3)

#### 3.1 Spawning and health
Every client builds them at level load (cvar `cl_phys_props_enable` 1) from
the map's `prop_physics_multiplayer` entries: keyvalues `health`,
`physdamagescale`, `spawnflags`, `model`, `skin`, fades, `inertiaScale`,
`physicsmode` are read; prop data is applied after them, so a template's
`health` wins (the map's `health` key is used only if the prop data has
none). A client prop **takes damage iff health ≠ 0** (no pieces check, no
multipliers: the bullet/club/blast values are stored but never used).

#### 3.2 Damage
- **Bullet impact** (each impact effect the client draws on it, for its own
  and other players' shots): push 4000 kg·in/s along the shot direction at
  the hit point; damage 30.
- **Other impact effect types** (e.g. a knife, if CS:S's knife effect uses
  this path): damage 50, same push (Open question 4).
- **Explosion temp entity** with radius R and magnitude M (HE: R = 350,
  M = 100; specs/cs_source/grenades.md 5.3): for each client object within
  R of the blast point B whose centre C is in clear line (shot-hull mask)
  from B: `f = M − (M/R)·|C − B|`; skip if f ≤ 1. Let
  `v = f·normalize(C − B) + (0, 0, 32)`. Push 500·v at the mass centre;
  damage = trunc(|v|).
- Health −= damage (integer); at ≤ 0 it breaks (3.3).
- Client props take **no** physics-impact damage.

#### 3.3 Breaking
No outputs, no sound. Pieces as in section 7.3 (client branch), then the
prop is deleted. On a new map load all client props are recreated. Round
restarts: Open question 9.

### 4. Damage to a server prop

Inputs: amount A (float), type bits T, attacker, inflictor, force J, point
p. Order:

1. **Can be damaged at all**: bullets (trace attacks) and radius damage skip
   entities that take no damage; every prop entity takes at least events
   (a broken prop takes none).
2. **Damage filter** (`damagefilter` keyvalue, a filter entity, resolved at
   activation; it tests the **attacker**; specs/source/entity_io.md
   "Filters"): fail → nothing at all (no outputs, no push).
3. Game-rule veto and attacker/victim damage modifiers (none in CS:S data).
4. **Minimum damage**: if A < `minhealthdmg` (map keyvalue, raw amount
   before any prop multiplier) → nothing at all (no outputs, no push).
5. If the attacker is a player or NPC, remember it as the **last attacker**.
6. **Scale**: A′ = A ×
   - dmg.bullets if T has bullet (×2 more if also buckshot),
   - × dmg.club if T has club,
   - × dmg.explosive if T has blast,
   - × 10 if T has both slash and crush,
   - → 0 if T has any time-based bit.
   Each factor applies if its bit is set (they multiply). Slash, crush,
   burn, sonic and generic have no factor of their own.
7. **Spawn tick**: on the tick the prop was created, the hit is processed
   as events only (push and outputs, no health loss).
8. **Fire rules** (only with the matching interactions):
   - Already burning and T has burn without direct → ignore entirely
     (return, no outputs).
   - deadly := A′ ≥ health.
   - explosive_resist + deadly + blast + an inflictor: distance d from the
     inflictor's centre to the prop's centre. d ≥ 32: burn time =
     max(0.5, 5·min(d, 256)/256) + U(0, 0.5); else U(0.1, 0.2). Replace A′
     by health − min(health, 5·burn_time) and recompute deadly.
   - Not deadly and blast → ignite for U(10, 15) s (only if flammable;
     ignition does nothing otherwise).
   - Not deadly and burn → same ignition.
   - Not deadly and bullet and ignite-halfhealth: if not burning and
     health − A′ ≤ max_health/2 (integer division) → health := max health,
     ignite U(10, 15) s; if already burning → A′ := health (kills).
9. **Push**: if the prop is physically simulated and the damage type
   carries force (not generic, not burn/drown/time-based/crush/prevent-
   force/plasma/physgun... as physics_props.md 5.1) → apply J at p. A
   motion-disabled body ignores it (but see step 13).
10. **Health**: unless events only, health := trunc(health − A′) (toward
    zero). If ≤ 0 → **killed** (step 11). Otherwise continue.
11. **Killed**: if the physics body is motion-disabled, enable its motion
    (no OnMotionEnabled) and apply the push now; then **break** (section 7)
    with breaker = the **inflictor**; the prop stops taking damage.
12. **Outputs** (always, including events-only props, killing hits and the
    spawn-tick case): OnHealthChanged with value
    clamp(health / max_health, 0, 1) (events-only props: health 0 → value
    0), activator = attacker; then OnTakeDamage, activator = attacker. Both
    fire even if health did not change. On a killing hit they are queued
    **after** OnBreak.
13. **Motion enabling** (prop_physics*, after the above):
    - `forcetoenablemotion` > 0 and |J| ≥ it → enable motion (fires
      OnMotionEnabled) and clear the threshold. The triggering push is not
      re-applied.
    - `damagetoenablemotion` > 0 and health < it → clear it; if the body
      still exists, enable motion (OnMotionEnabled) and apply J.

Damage types CS:S produces (as far as the SDK shows):

| Source | Type | Prop multiplier used |
|---|---|---|
| Gun bullets | bullet (Open question 1: buckshot bit on shotguns) | dmg.bullets |
| Knife | Open question 2 (breakables.md Q2) | – |
| HE grenade / env_explosion / exploding props | blast | dmg.explosive |
| Physics impacts (section 5) | crush (+ slash for spin) | none |
| Entity flame | burn (+ direct on the burning prop) | none |
| Break-on-touch | crush | none |

### 5. Physics impact damage to props

Each collision between a server prop's body and another body (world,
another prop, a player's physics shadow) produces, for the prop (index
"self", other = "o"):

1. Skip if the prop has spawnflag 2 ("don't take physics damage"; cleared
   at spawn when the prop has health) or self and other are the same
   entity.
2. Other body has "no impact damage" or is a static constraint part →
   0. (A dissolving inflictor → 1000; not in CS:S.)
3. Energy scale s = the prop's impact energy scale (≤ 0 → no damage). If
   the other object is "heavy" (game flag; not set by stock CS:S data):
   other mass := the table's large-mass threshold, s := max(s, 2).
4. Props pass "allow static damage", so the table's minimum mass / small
   mass / minimum speed filters are **skipped**. (They apply to players,
   section 6.)
5. Spin damage: only if the other object is a "slicer" (`Damagetype` 1 on a
   physics prop, or a z-spin launch interaction; stock: none) and its
   squared angular speed > the table's minimum: read the angular table with
   |I_o · ω_o| (componentwise absolute dot product) × s; non-zero adds the
   slash bit.
6. Linear: with pre/post collision speeds (magnitudes) u, u′:

```math
\Delta_s = |u_s| - |u'_s| \quad(\text{set to } 0 \text{ if } 0 < \Delta_s < v_{min})
E = \Delta_s^2\, m_s + \Delta_o^2\, m_o \cdot L
E' = E \cdot \frac{s}{m_s}
```

   where Δ may be negative (the body gained speed; it is squared anyway),
   v_min = the table's "my min velocity" (0 for the prop tables), L =
   1 unless m_o ≥ the table's large-mass threshold, then L = large-mass
   scale × (1 + k·(falling scale − 1)) with k = |Δz_o / Δ_o| when Δ_o > 0,
   the other's z-velocity drop Δz_o = u_o,z − u′_o,z < 0 and u_o,z < 0
   (falling onto it); else k = 0. For a motion-disabled prop m_s is still
   its real mass (1/m_s is recomputed). The world's velocities are 0.
7. Damage = spin damage + table(E′); if the other body is not static and
   m_o < the table's small-mass limit (with a non-zero small cap), clamp to
   [0, small cap].
8. The damage is queued (not applied during the simulation step) as crush
   (| slash) damage with attacker = inflictor = the other entity (the world
   if none), force = the prop's post-collision velocity × its mass (if zero,
   the other's post velocity × the other's mass), point = the contact.
9. After the physics step, queued damage is applied in collision order
   through section 4. If an application changed the prop's state (killed,
   removed or made non-solid) the **other** body's velocity is restored
   toward its pre-collision value: blend b = 1 if m_o ≥ 500 kg or it is a
   slicer; else r = clamp(m_o / m_heaviest_damaged, 0.1, 10), b = remap(r:
   0.1 → 0, 1 → 0.5) for r < 1, remap(r: 1 → 0.5, 10 → 1) for r ≥ 1;
   v_o := b·v_o,pre + (1 − b)·v_o,now (angular too; applies to all bodies
   of that entity). So heavy objects plough through what they break.

Table lookup: walk the entries in order; stop at the first whose threshold
> E′; damage = the previous entry's damage (0 if none). Thresholds are
squared speeds (in²/s²):

| Table | Linear entries (threshold → damage) | Angular entries | min speed² / min rot² / min mass | small mass max / cap / min speed² | large mass / scale / falling | my min vel |
|---|---|---|---|---|---|---|
| default (all props without `damage_table`) | 150² → 5, 250² → 10, 350² → 50, 500² → 100, 1000² → 500 | 100² → 10, 150² → 25, 200² → 50, 250² → 500 | 24² / 360² / 2 | 5 / 5 / 36² | 500 / 4 / 5 | 0 |
| `glass` | 25² → 10, 50² → 20, 100² → 50, 200² → 75, 500² → 100, 250² → 500 | 50² → 25, 100² → 50, 200² → 100, 250² → 500 | 8² / 360² / 2 | 1 / 10 / 8² | 50 / 4 / **0** | 0 |
| `player` | 150² → 5, 250² → 10, 450² → 20, 550² → 50, 700² → 100, 1000² → 500 | 100² → 10, 150² → 20, 200² → 50, 300² → 500 | 24² / 360² / 2 | 5 / 5 / 36² | **0** / 1 / 2 | 320 |

An unknown `damage_table` name falls back to the default table (with a
warning). The `player_vehicle` table is irrelevant to CS:S.

### 6. Physics damage to players

- **Impacts** (players are hit through their physics shadow, physics_props
  4.1): only by moveable physics-simulated objects, not by the object they
  stand on, not by ragdolls flagged dying. Section 5 steps 2–7 with the
  `player` table, s = 1, and the filters of step 4 **active**: other mass
  < 2 kg → 0; m_o < 5 kg and speed² < 36² → 0; speed² < 24² and spin² <
  360² → 0. The `player` table's large-mass threshold is 0, so every object
  gets L = 1·(1 + k·(2 − 1)): falling objects do up to 2× energy.
  Damage: crush, attacker = the object (the player who last touched it
  within 1 s if any), force = the player's post velocity × 85 kg (cvar
  `phys_impactforcescale` 1). Queued and applied after the physics step.
- **Stress (crushing)**: each physics update, the player's received stress
  is measured from contact forces with moveable, non-friendly, not
  player-held objects (in kg after dividing by gravity). With scale
  1 × 0.125: stress' = received × 0.125 / 85. If the player touches at
  least one non-static object, stress' > 5 and one contact is a ≥ 500 kg
  object that is not the player's ground entity, and the player is not
  stuck in solid: 200 crush damage (attacker = world, force (0, 0,
  −received·gravity·frame time)). Whether CS:S's player keeps this is Open
  question 6.

### 7. Breaking

Triggered by: lethal damage (breaker = inflictor), the Break input
(breaker = activator), SetHealth/AddHealth/RemoveHealth reaching ≤ 0
(breaker = activator), pressure (breaker = the player who stood on it),
touch (breaker = the toucher). In this order, all at once:

1. Send the game event `break_prop` (userid of the breaker if it is a
   player, else 0; entindex).
2. The prop takes no more damage. Fire **OnBreak**, activator = breaker,
   caller = the prop (no value).
3. Make it non-solid. Capture its velocity, angular velocity, position and
   angles (from the body if it has one, else from the entity).
4. Queue the **break sound**: the "break" entry of the body's surface
   property (inherited through `base`), at the entity origin; none if the
   prop has no physics body or the surface has no break entry (section
   7.4).
5. **Explode** if explosive damage or radius > 0 (7.5). The blast's
   attacker = the last attacker (step 5 of section 4) if any; else the
   original damage's attacker (the prop itself for the Break and health
   inputs and for pressure; the toucher for touch).
6. **Pieces** (7.3).
7. If the prop has the onbreak-explode interaction: if step 5 did not
   explode, a 1-damage explosion of the same radius at the captured origin;
   then ask every player/NPC within explosive_radius of the origin that
   passes the damage filter to ignite for 30 s **with the NPC-only rule**:
   players refuse, so in the SDK no player is ignited (corrected by
   specs/source/fire.md section 4.2; Open question 5).
8. **Remove** the prop now: its name is cleared immediately (later inputs
   by name find nothing), it is deleted at the end of the frame; every
   entity parented to it (directly or deeper) is removed with it
   (sprites, spotlights, steam, a flame, a func_breakable...). Outputs
   already queued by it are still delivered.

#### 7.1 Pressure and touch spawnflags
- 16 "break on touch": when anything touches it and touch speed × 0.01 ≥
  health (speed = the toucher's smoothed velocity): it takes that much
  crush damage (forced damageable), and the toucher takes a quarter of it
  as slash damage with a melee push. Stock: unused.
- 32 "break on pressure": when a toucher has the prop as its ground entity,
  record it as breaker (every such touch updates it) and schedule a break
  after `PressureDelay` seconds (once; later touches don't reschedule). Stock: cs_militia's roof
  boards (below).

#### 7.2 Gib limits
- `PerformanceMode` 1 (no gibs): no pieces, unless the cvar
  `breakable_disable_gib_limit` is 1. 2 (full gibs): ignore the piece-count
  cap. Stock: one prop has 1.
- cvar `props_break_max_pieces` (−1 = no cap): at most that many pieces.
- cvar `props_break_max_pieces_perframe` (−1 = no cap): per frame across
  all breaks.
- Client: no piece if 300 client props exist.

#### 7.3 Pieces
**Where they spawn.** With more than one player slot and the cvar
`breakable_multiplayer` 1 (default), by the prop's multiplayer break mode
(prop data `multiplayer_break`, default → client):
- default / client: the server sends one "physics prop, break" temp entity
  (model, skin, origin, angles, the body's velocity, render effects) to
  every client in audio range (PAS) of the prop's centre; **each client**
  spawns the pieces itself (client branch below). The server spawns none.
- server: the server spawns only pieces whose break block says
  `multiplayer_break` `server`. both: temp entity and server pieces. (No
  CS:S data uses these.)
- Single player / `breakable_multiplayer` 0: the server spawns all pieces
  (server branch).

**Which pieces.** If the model's `.phy` has `break` blocks, one piece per
block, in order (block keys: `model` (gets `models/` and `.mdl` added if
missing), `ragdoll`, `offset`, `health` (default 1), `fadetime` (default
20; setting it without `debris` also makes the piece debris), `fademindist`,
`fademaxdist`, `debris` (> 0 debris, else interactive), `burst` (default
100), `placementbone`, `placementattachment`, `motiondisabled`,
`multiplayer_break`, `velocity`). Otherwise, if the prop data names a chunk
list and a count, that many template chunks.

**Model break pieces:**
- Position: the block's `offset` (minus the piece model's "placementOrigin"
  attachment relative to the parent's) transformed by the prop's captured
  position and angles; with a placement bone/attachment, that bone's or
  attachment's transform. Angles = the prop's.
- Skin = the prop's skin, or 0 if it is greater than the piece model's
  number of skin families.
- Velocity = the captured velocity (the body's velocity at that point if
  the breaking side has the body). Client: × (1 + U(−0.025, 0.025)), and
  angular velocity = the captured one (0 in the temp-entity path).
- Burst: if burst ≠ 0, add burst × normalize(position − origin) (if the
  position equals the origin, use the piece's centre − origin): pieces fly
  out from the prop at 100 in/s.
- `motiondisabled`: the piece is frozen in place.
- Client pieces: a client prop with the block's `health` (0 → takes no
  damage and stops colliding with players); fades out after `fadetime` s
  over 1 s (0 → never fades); `fademindist`/`fademaxdist` apply when both
  are set and max ≥ min. Pieces inherit the parent's render effects.
- Server pieces: physics props in the block's collision group.

**Template chunks** (`breakable_model` + `breakable_count`): for each of
`count` chunks: model = chunk list entry I(0, min(size_limit, list length −
1)) (section 2.1 step 5; so bigger props can use bigger chunks); position
= a uniformly random point of the prop's box with the smallest axis fixed
at its middle; velocity = 100 × normalize(position − origin); no angular
velocity; angles = the prop's, re-aligned by 90° turns so the chunk's
longest (then second longest) axis matches the prop's; skin =
`breakable_skin`; health 1; life U(5, 10) s; debris.

#### 7.4 Break sounds
Queued per physics frame and played at its end (sounds.md "Break sounds":
a fourth and later break of the same surface in one frame merges with an
existing one at the midpoint). Server props only; client props and client
pieces play none (Open question 7). Effective break entries of stock
surfaces: Wood_Crate → `Wood_Crate.Break`, Wood_Furniture →
`Wood_Furniture.Break`, wood → `Wood.Break`, Wood_Plank/Box/Panel/Solid →
own `.Break`, glass → `Glass.Break`, glassbottle → `GlassBottle.Break`,
pottery → `Pottery.Break`, computer / metalvent / metal_barrel / popcan →
`Metal_Box.Break`, plastic → `Plastic_Box.Break`, plastic_barrel →
`Plastic_Barrel.Break`, cardboard / paper → `Cardboard.Break`, flesh →
`Flesh.Break`; metal, metalpanel, tile, porcelain, brick, snow, carpet,
watermelon, rubber → none.

#### 7.5 Explosive props
- Explosion at the prop's centre with magnitude trunc(explosive_damage)
  and radius trunc(explosive_radius) (radius 0 → 2.5 × magnitude); blast
  damage; inflictor = the prop.
- With the onbreak-explode interaction: no sparks, no dynamic light, no
  smoke, surface-only, silent explosion, plus the sound
  `PropaneTank.Burst` (ambient/fire/gascan_ignite1.wav, volume 1, pitch
  95–105, 90 dB) from the prop. Without it: same flags but not silent, and
  magnitude/radius × the model scale (1).
- The explosion entity: scorch decal on a surface within 40 in below
  (8 in above → 32 in below the point), fireball temp entity (fireball,
  pushes client objects per 3.2 with its radius and magnitude), radius
  damage = the game's (CS:S: grenades.md 5.3 with D = magnitude, R =
  radius; Open question 3).
- Map keyvalues `ExplodeDamage` / `ExplodeRadius` are the defaults that
  prop data overrides (all 0 in stock maps).

#### 7.6 Fire
Ignite (flammable props only; not twice): an entity flame parented to the
prop, plays `General.BurningObject`, lasts the given time. Every 0.2 s (first
0.1 s after ignition): 4 burn damage to everything else within flame_size/2
of the flame (radius damage), then 1 burn + direct damage to the prop
(5 hp/s). When the time is up: `General.StopBurning`, removed 0.5 s later.
Removed with the prop when it breaks.

### 8. Inputs and outputs

| Input | Classes | Effect |
|---|---|---|
| Break | every prop class above (not prop_static) | break now (section 7), breaker = activator; works on events-only props too |
| SetHealth n / AddHealth n / RemoveHealth n | all | new = n / health + n / health − n; if different: OnHealthChanged(clamp(new/max,0,1), activator = activator); if ≤ 0 → break with breaker = activator. Works on events-only props (max health 1) |
| physdamagescale f | all | sets the impact energy scale |
| Wake | prop_physics* | wake the body |
| Sleep | prop_physics* | put the body to sleep |
| EnableMotion | prop_physics* | unfreeze (teleport to a saved fix-up position if any), wake, fire OnMotionEnabled (activator = caller = the prop) |
| DisableMotion | prop_physics* | freeze the body |
| SetAnimation name | prop_dynamic | section 9 |
| SetDefaultAnimation name | prop_dynamic | store the default sequence name only |
| TurnOn / Enable | prop_dynamic | draw it (collision unchanged) |
| TurnOff / Disable | prop_dynamic | stop drawing it (collision unchanged) |
| EnableCollision / DisableCollision | prop_dynamic | clear / set its not-solid flag (traces and movement); its static body stays |
| Skin n | every model entity | set the skin family index to n directly |
| SetBodyGroup n | every model entity | set the **combined body index** to n directly (not "group, value"); for a one-group model n = the sub-model index |
| Ignite | every model entity | ignite for 30 s (props: only flammable ones burn) |

| Output | Value | Activator | When |
|---|---|---|---|
| OnTakeDamage | – | attacker | every damage event that passes the filter and minhealthdmg |
| OnHealthChanged | float 0..1 | attacker / input activator | every such damage event (even unchanged health), and health inputs that change health |
| OnBreak | – | breaker | break |
| OnAwakened | – | the prop | prop_physics* with spawnflag 1 ("start asleep"): the first physics update in which the body is awake; then flag 1 is cleared (fires once) |
| OnMotionEnabled | – | the prop | EnableMotion input, force/damage-to-enable thresholds; not when lethal damage unfreezes it |
| OnPlayerUse | – | the prop | +use on a prop_physics* with spawnflag 256 |
| OnAnimationBegun / OnAnimationDone | – | none | prop_dynamic, section 9 |
| OnPhysGunPickup and the other physgun outputs | – | – | never in CS:S (no gravity gun) |
| OnOutOfWorld | – | the prop | a physics update finds it outside the world |

### 9. prop_dynamic animation

- Keyvalues: `DefaultAnim` (sequence started at spawn), `RandomAnimation`
  with `MinAnimTime`/`MaxAnimTime` (stock: unused), `StartDisabled`
  (not drawn), `SetBodyGroup`, `skin`, `solid` (0 none → an oriented box
  flagged not solid; 6 → static body from the `.phy`, or per-bone
  followers when the model lists them or the `.phy` has several solids;
  spawnflag 256 → not solid).
- SetAnimation name / DefaultAnim at spawn: look the sequence up by name
  (or activity name). Found: go to it (below) and fire OnAnimationBegun.
  Not found: warning, set sequence 0 (no output).
- Going to a sequence: if either the current or goal sequence has no
  transition node (entry node 0 — all stock models checked), switch at once:
  cycle 0, playback rate +1. (Models with node graphs walk transition
  sequences, possibly backwards; not used in stock CS:S.)
- The prop thinks every 0.1 s while animating: advance the cycle (rate =
  fps / (frames − 1) per second; clients animate it themselves from the
  sequence and cycle). When a non-looping sequence reaches cycle ≥ 0.999
  (or ≤ 0 backwards): if it is not the goal, take the next transition;
  otherwise fire OnAnimationDone and, if a default animation is set, start
  it (firing OnAnimationBegun again). A non-looping default animation
  therefore restarts forever, firing Begun/Done each pass.

### 10. What disappears when a prop breaks (stock patterns)

- Entities **parented** to the prop are deleted with it (section 7 step 8):
  cs_havana's monitor body `breen_monitor_1_body_1` (a damageable
  `tv_monitor01`) takes its spark, its screen `prop_dynamic` and its glass
  `func_breakable` with it; spotlights hanging from lamps (de_prodigy,
  de_tides, de_train, de_nuke, cs_militia) and cs_office/cs_assault's
  extinguisher steam would go with their props (those props are events
  only, so they never break).
- Through outputs (cs_office projector, on any damage): `Kill` removes 14
  `env_sprite` glows (gone at the end of the frame), the slide
  `func_illusionary` (gone), the `func_dustmotes` (the volume and its motes
  vanish), `TurnOff` on the `light_spot` named `projectorlight` (style 32
  set to "a": its baked contribution drops to 0), `StopSound` on the
  projector hum.

### 11. Stock map usage

Prop classes per map (multiplayer props split by resolved mode):

| Map | prop_physics | multiplayer (mode 1/2/3) | prop_dynamic | dynamic_override |
|---|---|---|---|---|
| cs_assault | 1 | 96 (39/5/52) | 2 | – |
| cs_compound | 7 | 106 (63/5/38) | – | 1 |
| cs_havana | 23 | 38 (32/3/3) | 1 | – |
| cs_italy | – | 119 (49/0/70) | – | – |
| cs_militia | 10 | 170 (36/11/123) | 3 | – |
| cs_office | – | 226 (154/11/61) | – | – |
| de_aztec | – | 16 (16/0/0) | – | – |
| de_cbble | – | 60 (36/0/24) | – | – |
| de_chateau | 8 | 108 (62/0/46) | – | – |
| de_dust | – | 162 (42/3/117) | – | – |
| de_dust2 | – | 75 (40/1/34) | – | – |
| de_inferno | 13 | 186 (64/3/119) | 6 | – |
| de_nuke | – | 73 (46/2/25) | 56 | – |
| de_piranesi | 2 | 33 (30/0/3) | – | – |
| de_port | 11 | 124 (110/0/14) | – | 4 |
| de_prodigy | – | 95 (67/0/28) | 96 | – |
| de_tides | – | 121 (60/10/51) | 8 | – |
| de_train | 2 | 146 (91/3/52) | – | – |

Prop I/O actually used:

| Feature | Where (count) |
|---|---|
| OnAwakened → own Break (start-asleep props that shatter when disturbed) | cs_office 24 (8 trash cans, 11 phones, 5 keyboards), cs_compound 2, de_port 1 |
| OnHealthChanged | de_nuke 6 extinguishers (4 outputs each, fire once: steam on/off after 4 s, hiss on/off); cs_office projector (5), 2 monitors → own Skin 1 |
| OnBreak | cs_office: 2 computer cases → monitor Skin 1, radio → StopSound, snowman face → hat Wake; cs_italy drawer → `killradio` Break (func_breakable that stops the opera) |
| OnMotionEnabled | cs_office and cs_assault extinguishers (`forcetoenablemotion` 2250) → steam on, off 4 s later |
| prop_dynamic Skin | de_nuke 34 (logic_case cars, logic_timer computer screens), de_prodigy 30 (timer screens) |
| prop_dynamic SetAnimation | de_nuke 16 (`car0x` "run", default "hide"), de_inferno 1 (elevator door "open") |
| SetBodyGroup | cs_havana 1 (func_breakable OnBreak → monitor screen 1) |
| Wake | cs_office 1 (snowman hat) |
| spawnflag 32 pressure | cs_militia 1 (roof boards, delay 1 s) |
| minhealthdmg | de_inferno 6 (200), de_tides 4 (200), de_train 2 (200) + 3 toilets (50) |
| forcetoenablemotion | cs_office 2 (2250, 100), cs_assault 1 (2250), others (10, 50) |
| Explosive / flammable props | 2 gas cans (`gascan001a`: cs_militia 1, de_inferno 1) |
| `damage_table` glass | windows (cs_militia 8, de_inferno 4), bottles/jugs/mugs (client props) |
| Never used | TurnOn, TurnOff, Enable, Disable, EnableCollision, DisableCollision, SetDefaultAnimation on props, Sleep, EnableMotion/DisableMotion inputs, OnTakeDamage, OnPlayerUse, damage filters on props, spawnflag 16, ExplodeDamage keyvalues |

Gameplay-relevant breakables (server props, motion disabled unless noted):
cs_militia windows (3 `wndw01`, 4 `skylight_glass`, 1
`militiawindow02_breakable`: Glass.CSWindow2/Window, health 1), the wall
hole boards and the pressure roof boards (Wooden.Medium, health 30);
de_chateau's 8 board barricades (Wooden.Medium/Large, 30/50); de_inferno's
4 windows (health 1, surface "Glass " with a trailing space); de_port's 3
loose crates with `physdamagescale` 1; many crates, barrels, pallets and
furniture (mode 1).

## Per-tick order

Server tick:
1. Player commands: bullets and knife hits apply damage immediately
   (section 4, all steps, outputs queued, breaking and removal marking
   included); the break sound is queued for the next physics frame end.
   Touch/pressure checks happen when players touch props during movement.
2. Other entities think: entity flames (burn damage), pressure break
   timers, prop_dynamic animation thinks, grenades (radius damage, which
   damages props in entity order).
3. Physics frame: simulation; collisions queue impact damage (props and
   players), wake-ups; then OnAwakened fires during the body-to-entity
   copy; then queued impact damage is applied in order (section 5 step 9,
   with velocity restoration); then queued impact sounds, then queued
   break sounds play.
4. Event queue: OnBreak, OnHealthChanged, OnTakeDamage and the rest are
   delivered (specs/source/entity_io.md); inputs from them (Break, Skin,
   Kill...) run here.
5. Entities removed this frame (broken props and their children, Kill
   targets) are deleted.
6. Temp entities (break pieces, explosions) and state go to clients; each
   client spawns pieces, plays the explosion and pushes/damages its client
   props when it receives them.

## Edge cases

- A prop with health but nothing to break into is events only (2.2), so
  OnHealthChanged reports 0 for it on every hit (not 1).
- Integer truncation: health 10, bullet 19 × 0.5 = 9.5 → trunc(0.5) = 0 →
  breaks.
- A zero multiplier (`Wooden.Barrel` bullets/club 0) still fires
  OnTakeDamage/OnHealthChanged and still pushes.
- minhealthdmg compares the raw damage: de_train's toilets (minhealthdmg
  50, health 40, bullets ×1) ignore everything below 50 raw (no outputs,
  no push) and break from one hit of 50 or more.
- minhealthdmg 200 (de_inferno's fruit crates and fountain, health
  200–201 with bullet/club ×0; its bombsite roof, de_tides' carts, de_train's
  biohazard tanks): unbreakable by guns, knives and the 100-damage HE; only
  impact damage of ≥ 200 per collision could hurt them, and they are
  motion-disabled.
- Second hit in the same tick on a broken prop (another pellet, a radius
  damage loop): ignored; outputs it already queued still deliver.
- Inputs by name to a broken prop in the same frame find nothing (name
  cleared at once); the monitor's OnHealthChanged → own Skin on a killing
  shot targets nothing.
- `forcetoenablemotion` props: the enabling hit's push is lost (the body was
  frozen when it was applied), so it starts at rest.
- Events-only Break: a fire extinguisher sent Break would break with no
  pieces (none listed) and vanish.
- prop_dynamic killed by an input on a model with a static body: breaks
  like any prop (no physics velocity).
- Client props never push or damage server props, and nothing that
  happens on the server (bullets, blasts, map logic) reaches them; only
  each client's own effects do (section 3.2).
- Break pieces spawned client side differ per client (random chunk choice,
  jitter) and are invisible to the server: they never block bullets or
  players.

## Quirks

- **Map `health` ignored** except on `_override` classes (keep).
- **OnHealthChanged fires on every hit, even when nothing changed**, and
  reports 0 for unbreakable props (keep: de_nuke's extinguishers and
  cs_office's projector depend on "any hit").
- **Outputs after OnBreak**: a killing hit queues OnBreak before
  OnHealthChanged/OnTakeDamage (keep; order matters only for chained
  inputs).
- **Client props ignore multipliers** and take a flat 30 per bullet (keep:
  every bottle, jug and mug dies to one bullet).
- **Client blast damage includes the 32-unit lift**: almost any client
  prop with a clear line inside the radius takes ≥ 32 (keep).
- **Glass table's last entry (250² → 500) is out of order**: energies
  ≥ 500² give 500 and the 100 entry is unreachable; between 200² and 500²
  give 75 (keep).
- **Speed gained counts as energy**: Δ is a signed speed drop that gets
  squared, so a body knocked from rest to speed w adds w²·m just like one
  that stopped from w (keep).
- **Player table's large-mass threshold is 0**: every object falling onto a
  player gets the ×2 falling emphasis (keep).
- **`Glass ` with a trailing space** on de_inferno's windows: an unknown
  surface, so (if the engine does not trim it) default physics and no break
  sound (Open question 8).
- **Activator of OnAwakened / OnMotionEnabled / OnPlayerUse is the prop
  itself** (keep).
- **SetBodyGroup takes the raw body index**, not group/value (keep).
- **TurnOff leaves collision on**; DisableCollision leaves the drawing on
  (keep).
- Shooting an asleep cs_office phone usually breaks it twice over: the
  bullet's damage (10 health) and the wake-up's Break input; the second is
  a no-op (keep).

## Test cases

Bullet damages are examples, not CS:S weapon values.

| Setup | Input | After | Expected |
|---|---|---|---|
| `wood_crate001a` as prop_physics_multiplayer | resolve | spawn | template Wooden.Medium: health 30, bullets 0.75, club 2.0, blast 1.5; mode 1 (40.5×40.3×40.2, 30 kg); 7 model break pieces (fadetime 20); damageable; surface Wood_Crate → `Wood_Crate.Break` |
| that crate, health 30 | bullet 36 | same tick | 36 × 0.75 = 27 → health 3; OnHealthChanged 0.1; OnTakeDamage |
| health 3 | bullet 36 | same tick | −24 → break: OnBreak (activator = inflictor), OnHealthChanged 0, OnTakeDamage queued after it; name cleared; 7 client pieces at 100 in/s outward |
| health 30 | club 16 | – | 32 ≥ 30 → breaks |
| `woodbarrel001` (Wooden.Large, health 50, no .phy pieces) | break | – | 6 WoodChunks; size limit floor(28.9 × 1 × 40.6 / 1024) = 1 → each chunk `wood_gib01e` or `wood_gib01d`; each lives U(5,10) s + 1 s fade |
| woodbarrel, health 50 | HE blast at 175 in (100 − 50 = 50 raw) | – | 50 × 1.5 = 75 → breaks |
| woodbarrel, health 50 | HE at 250 in (28.571 raw) | – | 42.857 → health trunc(7.14) = 7; OnHealthChanged 0.14 |
| `file_box` (Cardboard.break: health 10, bullets 0.5) | bullet 19 | – | 10 − 9.5 = 0.5 → 0 → breaks |
| `oildrum001` (Metal.Medium, health 0) | any bullet | – | events only: no health change; OnHealthChanged 0, OnTakeDamage; pushed by the bullet impulse |
| `chair_office` (Cloth.Large, health 100, no pieces) | spawn | – | events only (health 0) |
| de_nuke extinguisher at (1220.24, −912, −723), sf 264 | first bullet | delivery tick | OnHealthChanged(0) → steam13 TurnOn, steamsound1 PlaySound; 4 s later TurnOff/StopSound; second bullet: nothing (outputs fire once) |
| cs_office projector at (2118, −152, −130) (Plastic.break, health 10, mode 1, sf 264) | bullet 4 | – | health 6; OnHealthChanged 0.6 → 14 `projectorglow` sprites, `slideshow`, `projectordust` killed; `projectorlight` (style 32) off; hum stopped; projector stays |
| same, health 10 | bullet 30 | – | breaks: body unfrozen and pushed, OnBreak, then OnHealthChanged 0 (same kills); `Metal_Box.Break` (surface computer); 10 client pieces |
| cs_office monitor `monitor2` (Plastic.break 10) | bullet 5 | – | health 5; OnHealthChanged 0.5 → Skin 1 on itself |
| cs_office keyboard `keyboard1` (sf 257: asleep + use output) | an HE push wakes it without damage | next physics frame | OnAwakened → `keyboard1` Break → breaks (6 pieces), activator = the keyboard |
| cs_office extinguisher (`forcetoenablemotion` 2250, 10 kg, health 0) | hit with impulse |J| = 2400 | – | OnMotionEnabled → steam TurnOn now, TurnOff at +4 s; body unfrozen at rest |
| same | |J| = 2000 | – | stays frozen; OnHealthChanged 0, OnTakeDamage still fire |
| cs_office snowman face (Plastic.break 10, mass 200, force-to-enable 100) | bullet 30 | – | breaks; OnBreak → `hat` Wake (the hat, asleep debris, falls) |
| cs_italy drawer `FurnitureDrawer002a` at (1004.14, 2300.17, 145.07) (Wooden.Medium 30, frozen) | bullets 20, 20 | – | 15 → health 15; 15 → 0 → breaks; OnBreak → `killradio` Break → its OnBreak → `operaWav` StopSound |
| cs_militia roof boards at (406.3, 320.7, 158.7), sf 296, PressureDelay 1 | player lands on it at t | t + 1 s (nearest tick) | breaks, breaker = player; server prop gone (hole), client pieces (p1 frozen, never fades; p2–p4 fade after 10 s) |
| cs_militia `wndw01` (Glass.CSWindow2: health 1, bullets 0.5, glass table, frozen, 10 kg) | bullet 2 | – | 1 − 1 = 0 → breaks; `Glass.Break` |
| same | bullet 1.9 | – | 1 − 0.95 = 0.05 → 0 → breaks |
| window | a 30 kg crate hits it at 300 in/s and stops | after physics step | E′ = 300²·30 / 10 · 0.1 = 27000 → glass table 50 → breaks; the crate's velocity is restored with b = remap(3: 1→0.5, 10→1) = 0.611 of its pre-impact velocity |
| glass table | E′ = 600 / 625 / 40000 / 249999 / 250000 / 300000 | – | 0 / 10 / 75 / 75 / 500 / 500 |
| `wood_crate001a` (30 kg, scale 0.1, health 30) | falls onto the floor, Δv = 600 → 0 | after physics step | E′ = 600² · 0.1 = 36000 → 5 crush → health 25, OnHealthChanged 0.8333 |
| same crate | Δv thresholds | – | 5 at ≥ 474.34 in/s, 10 at ≥ 790.57, 50 at ≥ 1106.80, 100 at ≥ 1581.14 (drop heights at 800 in/s²: 140.6, 390.6, 765.6, 1562.5 in) |
| de_port crate at (1769, 1218, 532.6) (`physdamagescale` 1, health 30) | knocked off a 100 in ledge, lands at 400 in/s, stops | – | E′ = 160000 → 50 → breaks |
| default table, ≥ 500 kg falling object | a 600 kg prop falls straight down at 200 in/s onto a 30 kg crate (scale 0.1) on the floor; both end at rest | – | other term 200²·600 ·4·(1 + 1·4) = 4.8e8; E′ = 4.8e8 / 30 · 0.1 = 1.6e6 → 500 |
| player (85 kg), 30 kg crate hits at 500 in/s horizontally and stops | – | after physics step | E = 500²·30·1 / 85 = 88235 → 10 crush |
| player, same crate falling straight down at 500 onto them | – | – | ×2 → 176471 → 10; at 550: 213529 → 20 |
| player, 4 kg can at 800 in/s | – | – | 800²·4/85 = 30118 → 5 (cap 5 for < 5 kg); 1.5 kg object → 0 (under 2 kg) |
| player squeezed by a 600 kg object, received stress 4000 kg | physics update | – | 4000 × 0.125 / 85 = 5.88 > 5 → 200 crush |
| gas can (health 20, flammable, explode 25/80, mode 1) | bullet 19 | – | health 1; no ignition (bullet without half-health interaction) |
| gas can, health 1 | bullet 5 | – | breaks: silent explosion magnitude 25 radius 80 at its centre + `PropaneTank.Burst`; blast attacker = the shooter (last attacker); a player 40 in away is offered 25 − 40·25/80 = 12.5 blast damage; Open question 5 for ignition |
| gas can, health 20 | HE at 300 in (14.286) | – | health 5, ignites U(10,15) s; burn hits at +0.1, 0.3, 0.5, 0.7, 0.9 s → breaks at +0.9 s and explodes; flame radius damage 4 per tick within 8 in (size max(16, 14.4)/2) |
| client prop `glassjug01` (Glass.Small 5, mode 3) | a bullet on the client | – | 30 ≥ 5 → breaks, 3 pieces (client), no sound, no server effect |
| client prop at 300 in horizontally from an HE blast, clear line | explosion temp entity (R 350, M 100) | – | f = 14.286; v = (14.286 dir, 32); damage trunc(35.04) = 35; push 500·v |
| client prop 2 in from the radius edge (348 in) | same | – | f = 0.571 ≤ 1 → nothing |
| client `garbage_milkcarton002a` (prop data health 0) | bullets | – | never breaks; pushed 4000 per bullet |
| cs_havana `breen_monitor_1_body_1` (tv_monitor01, Metal.Medium → events only) | its child func_breakable glass broken | – | func_breakable OnBreak → screen `prop_dynamic` SetBodyGroup 1 → body index 1 (screen sub-model 1 of 3) |
| de_nuke `car01` (DefaultAnim hide: 600 frames at 30 fps, once) | spawn | – | OnAnimationBegun at spawn; done after 19.97 s (checked at 0.1 s thinks) → OnAnimationDone → restarts hide |
| `car01` | SetAnimation run (300 frames, 60 fps) | – | OnAnimationBegun now; Done ≈ 4.98 s later (≤ 0.1 s late), then hide restarts |
| de_inferno elevator door | SetAnimation open (61 frames at 80 fps) | – | done after 0.75 s; no default → holds the last frame |
| prop_dynamic | SetAnimation "nosuch" | – | sequence 0, no output |
| `prop_physics` with `controlroom_desk001b` (no prop_data), cs_havana | spawn | – | deleted |
| de_train toilet (Pottery.Medium 40, minhealthdmg 50) | bullet 36 | – | nothing (no outputs, no push) |
| same | bullet 60 | – | 60 × 1.0 ≥ 40 → breaks |

## Open questions

1. **Bullet damage type** to props in CS:S (bullet bit? buckshot bit on
   the M3/XM1014?). Shared with breakables.md Q1. Check: shoot de_dust's
   `woodbarrel001`-style prop (Wooden.Large 50) — on de_cbble or cs_italy —
   with one Glock round at point blank and read health through an
   OnHealthChanged → logic test map; expect 50 − 0.75 × damage.
2. **Knife damage type** (bullet, slash or club) and whether the client
   treats knife hits on client props as "other" (50). Check: knife a
   `wood_crate001a` (health 30: club ×2 breaks it with one 15+ hit, slash
   ×1 doesn't).
3. **CS:S radius damage** for exploding props (env_explosion goes through
   the game rules' radius damage): same formula as the HE grenade
   (grenades.md Q10)? Check: gas can on cs_militia next to a bot at
   measured distances.
4. **Client "other" impact damage** (50) and which effect types reach client
   props in CS:S (bullets certainly; knife, grenade bounce?). Check: knife a
   client-side cardboard (Cardboard.breakclient 10) vs a mug (Pottery 20).
5. **Gas can ignites players**: the onbreak-explode rule asks every
   combat character within 80 in to ignite for 30 s with the NPC-only rule,
   which the SDK's player refuses (specs/source/fire.md 4.2, Q3). Does
   CS:S's player override that? Check: break the de_inferno gas can next
   to a bot.
6. **Player stress (200 crush) and impact damage** from props: does CS:S's
   player keep the base rules? Check: drop a crate onto a bot from 390 in
   (≥ 10 by the table) and watch health.
7. **Client break sounds**: client props and pieces queue none in the SDK;
   does breaking a bottle in CS:S make a sound beyond the bullet impact?
   Check by ear on cs_italy.
8. **Surface name with a trailing space** (`Glass ` on de_inferno's
   windows): does the engine trim it (glass, `Glass.Break`) or fall back to
   default (no break sound)? Check by ear.
9. **Round restart**: are broken server props respawned each round (not in
   CS:S's preserve list?) and does the client recreate its client props
   (the SDK does it only on level load)? Check: break a client bottle and a
   server crate, then restart the round.
10. **`breakable_multiplayer`** default and existence in CS:S (`cvarlist`
    on a local server): if 0, pieces would be server-side.
11. **Bullet impulses** per CS:S calibre (physics_props.md Q): needed for
    the extinguishers' 2250 threshold. Check: shoot cs_office's
    extinguisher with each pistol and see which unfreeze it.
12. **Model box for size rules**: the auto mode (15³) and chunk size limit
    use the entity's collision box; I assumed it is the `.mdl` header's hull
    box. Check `ent_bbox`-style output on a cardboard box (24 placements,
    13.1×14.4×10.8 → mode 3).
13. **Player-shadow contacts as impacts**: does a player walking into a
    health-1 glass prop (mode 1, glass table) break it through the impact
    rule? Check: walk and sprint into cs_militia's `wndw01`.
14. **OnAwakened on cs_office's phones/keyboards/trash cans**: do players
    bumping them (push-away impulses) wake and so break them? Check by
    walking into one.

## For mashup's current code (docs/tech-debt.md "Model doors... Prop damage has no spec")

The current guesses differ from this spec in: using the map's `health` on
all classes; using breakable-brush multipliers (0.5/1.5/1.25) instead of the
prop's template multipliers; treating health-without-pieces props as
damageable; firing OnHealthChanged with 1.0 for events-only props and only
on change otherwise (here: every hit, value 0 for events-only); firing
OnTakeDamage before OnBreak; no impact damage, pieces, sounds, explosions,
mode-3 client props, pressure/touch flags, parented-child removal,
forcetoenablemotion, OnAwakened or OnMotionEnabled.
