# Weapons, bots and deathmatch

Started 2026-10-05, from specs/cs_source/weapons.md. MVP plan items 4–5.

## Design

- **core:** `Damage` and `Died` messages (damage applied after
  `SimSet::Weapons`), `Hitgroup`, `MaxSpeed` (equipment speed cap; Source
  movement reads it), intent fields `secondary`, `select`, `last_weapon`.
- **weapon (any game):** a weapon is an entity with parts: `Trigger`
  (automatic, cycle, "set"/"accumulate" refire), `Magazine` (swap at the
  end of a reload), `Hitscan` (template spread) or `Melee` (line, then the
  SDK's hull fallback), `DamageEffect` (falloff, hitgroup scale, impulse on
  dynamic bodies), `WeaponSounds`. `Inventory` + `WeaponState` timers run
  the spec's frame order. Games register weapons by ID; `StartingWeapons`
  are given to every new character.
- **rules:** deathmatch (respawn after `mp_respawn_delay`, scores).
- **bot:** brains write `Intent`, so bots use the same movement and weapons.
- **CS:S:** knife and AK-47 from script values; unmeasured rules are marked
  `UNMEASURED` in `games/cs_source/weapons.md` with the measurement that
  will replace them.

## Slices

1. [x] Framework, knife, AK-47, HUD (crosshair, health, ammo, hit marker,
   killfeed), deathmatch respawns, a first bot brain (no navigation),
   `tests/weapons.rs` (spec T5, T6, T8, T10, T11).
2. [x] Merge the probe measurements: carry-over refire (7,7,7,6 ticks),
   dry fire on a held empty clip, whole-point damage with the measured
   hitgroups and falloff, the knife (follow-up slash, bearing-based
   backstab, generic hits, miss refires, box reach), AK-47 inaccuracy
   (`Inaccuracy`) and recoil (`Recoil`, view punch, shots along view + 2 x
   punch). Armour (`Armor`, M7's split; `buy vest`/`vesthelm`) followed. Not yet: zoom and fire modes (no
   weapons using them yet); penetration and the remaining recoil sets
   came in slice 5.
3. [x] Bot navigation from specs/cs_source/nav.md. (CS:S bots' own path
   cost and follower aren't in the SDK; ours use the stock cost and a
   simple follower: crossing points, jump up ledges and when stuck.)
   - `map/nav.rs` (any game): `NavMesh` in engine space: areas as XZ
     rectangles with four corner heights, directed links in file order
     (with the side they leave by), ladders; `height_at`, `area_at`,
     `nearest_area`, A* with the stock cost and the spec's tie rules,
     portal crossing points. Constants are Source's, in meters.
   - `games/cs_source/nav.rs`: parse `maps/<map>.nav` (versions 9 and 16)
     from the mount at map load into `MapData::nav`; the map plugin
     inserts it as a resource.
   - Bots: when no enemy is in sight, path toward the nearest enemy
     (repath every second), walk through portal crossing points, jump
     where the next point is above step height.
   - Tests: the spec's dust2 parse, height, area-at and A* cases.
4. [x] Hitboxes from the player models (reference pose; shots test every
   character's boxes along the ray, so boxes outside the hull count and
   hull-only grazes pass) and the models drawn for other characters
   (t_phoenix for team 1, ct_urban otherwise). Not yet: animation (bodies
   and hitboxes stay in the reference pose), crouch, per-player skins.
5. [x] Penetration and collaterals (M13): `Penetration` part (budget,
   object count, distance limit) and the `PassMaterials` resource (per
   material class scale and damage factor, characters' fixed cost, damage
   x 0.5 and range halving) in the weapon layer; CS:S's numbers in
   `games/cs_source/weapons.rs`. Wall thickness is stepped through the
   physics colliders (adjacent solids count as one wall); the material
   class comes from the prop's or nearest world triangle's surface
   property. Falloff applies again at every hit. Also the AK-47's moving
   and airborne kick sets (M3) and the landing rule scaled by fall speed
   (M1). Fits and guesses are in docs/tech-debt.md.
6. [ ] View models and world models; impact decals and effects; tracers.
7. [ ] The other CS:S weapons from the script tables.
   - [x] M4A1, AWP, USP, Glock, Deagle (2026-10-06). Guns are `Gun`
     rows of script values in `games/cs_source/weapons.rs` (the spec's
     tables; sound entries and model paths checked against the install
     by `tests/map_de_dust2.rs::gun_models_icons_and_handedness`; reload,
     draw and silencer sounds at the view models' event times from
     `dump --sequences`), built by `gun`. New weapon parts: `AltModes`
     (attack2 steps modes; `toggle_time`, silencers block both attacks),
     `Zoom` (FOV per mode, zoomed speed, unzoom after a shot until the
     next may fire; owner gets `Zoomed`, which the client camera, the
     scope overlay and the view-model hiding follow), `Burst` (rounds
     fired on their own after the pull), `WeaponSounds::fire_alt`,
     draw and mode sounds, `WeaponEventKind::ModeChanged`. `Inaccuracy`
     uses the `*Alt` keys in a non-zero mode. Starting weapons are per
     team (`StartingWeapons::team`): Terrorists knife + Glock, everyone
     else knife + USP, plus the AK-47 for all (deathmatch rifle, drawn).
     Tests: `tests/cs_guns.rs` (damage with falloff, armour and
     headshots; deploy, refire, reload ticks; first kicks; zoom levels,
     speed, unzoom/re-zoom; silencer and burst timing), view-model cases
     in `tests/view_models.rs`.
   - [ ] Real-game checks (probe): silenced damage/range (M16), scoped
     and silenced/burst recoil, the Glock's burst refire, the M4A1's
     moving/airborne kicks, pistol punch caps, whether reloading
     unzooms, 556MM/9MM penetration, 45ACP's distance limit.
   - [x] Grenades (2026-10-06, specs/cs_source/grenades.md): HE,
     flashbang and smoke as slot-3 weapons (`weapon::grenade::Throwable`:
     count and carry limit 1/2/1, prices 300/200/300, buying stacks and
     keeps the hand), pin on press, throw on release (+0.1 s), the
     spec's pitch bend and speed, the projectile's box sweep, 0.4 gravity,
     mirror bounce ×0.45 (×0.135 on players), stop under 30 units/s,
     glass, water, fuse checked every 13 ticks; HE radius damage traced
     against the world only, armour (H1), push, scorch, particles, kill
     credit (`Damage::weapon`, `d_hegrenade`); flash blindness
     (`core::Blinded`, a fitted model) and the white tint; smoke clouds
     (64 churning sprites, grey tint, `core::SightBlocker` for bots);
     view-model pin/throw/draw with the pin sound, the third-person
     grenade layer, a primed grenade dropped live on death, all cleared
     at a new round. Tests: `tests/cs_grenades.rs`, unit tests for the
     spec's G cases, `map_de_dust2::grenade_models_and_sequences`.
   - [x] The other 18 guns (2026-10-06): FAMAS (burst, M16's 0.08 s
     rounds and 0.55 s refire), Galil, AUG and SG552 (55 zoom, no
     overlay), scout, SG550, G3SG1 (40/15 scopes), MAC-10, TMP, MP5,
     UMP45, P90, M249, P228, Five-SeveN, Elites, M3 and XM1014, each a
     `Gun` row with the spec's script values, view-model durations and
     sound events, world and view models, prices and team limits. New
     parts: `ShellReload` (start, then a shell per insert a tick apart,
     fire interrupts, `ShellInserting` events; view-model start/insert/
     finish and the player's `_start/_loop/_end` gestures), `Hitscan::
     pellets` with one inaccuracy offset per shot and a Spread draw per
     pellet, timed fire and finish sounds (`WeaponSounds::shot`,
     `finish`), zoom without an overlay or speed change, a random-up
     kick (shotguns; once per shot, not per pellet). The Elites alternate hands by clip parity (view
     model left/right sequences, `_L`/`_R` player shots). The buy menu
     is CS:S's (`Prices::menu`: 1 pistols, 2 shotguns, 3 SMGs, 4 rifles,
     5 machine guns, 8 equipment; each team sees its own items); bots pick
     a primary at random by `Prices::bot_weights` among the dearer half
     they can afford and let go of the trigger between semi-automatic
     shots. Tests: `tests/cs_guns.rs` (buy by name, slot, clip/reserve,
     price, fire cycle, draw/reload ticks for all; pellets and cone, shell
     reload T25 timing and interruption, scope FOVs, FAMAS burst, Elite
     hands, shotgun kick), `tests/view_models.rs` (Elite sequences, shell
     reload sequences, AUG keeps its view model), `tests/map_de_dust2.rs`
     (every gun's models, icons, sounds, durations, handedness and body
     animations).
   - [ ] Real-game checks for the new guns (probe): recoil of every
     automatic but the AK-47/M4A1 (M3), the scoped/semi-automatic kicks
     of the scout, SG550, G3SG1, P228, Five-SeveN and Elites, the shotgun
     punch (M3/XM1014 kick per shot standing, moving, crouched and in the
     air, held and tapped; whether the decay is the M3 rule; whether
     pellets follow view + 2 x punch), the shotgun reload timing and
     interruption (M9), the pellet
     pattern and per-pellet inaccuracy (Q12), 357SIG/57MM/BUCKSHOT/
     556MM_BOX penetration (M13), the SG552's and G3SG1's zoomed speed
     (M15), the Elites' hand order; how far CS:S bots pull down against
     recoil (a bot spraying an AK-47 at a wall: its eye angles and
     `m_vecPunchAngle` per tick, to see whether eye + 2 x punch stays on
     the target, as `bot_recoil_control 1` assumes).
