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
   punch). Armour (`Armor`, M7's split; `buy vest`/`vesthelm`) followed. Not yet: penetration (unmeasured),
   the landing rule and airborne/moving recoil sets (unmeasured), zoom
   and fire modes (no weapons using them yet).
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
4. [ ] Hitboxes from the player models instead of height bands; character
   models.
5. [ ] View models and world models; impact decals and effects; tracers.
6. [ ] The other CS:S weapons from the script tables.
