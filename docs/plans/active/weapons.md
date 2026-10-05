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
2. [ ] Merge the probe measurements (M1–M17): inaccuracy model, recoil /
   punch (aim = view + punch), hitgroup multipliers, falloff, armour,
   penetration, knife numbers, fire timing rule.
3. [ ] Bot navigation: CS:S's `.nav` mesh (plan the loader first), paths
   to enemies and around the map.
4. [ ] Hitboxes from the player models instead of height bands; character
   models.
5. [ ] View models and world models; impact decals and effects; tracers.
6. [ ] The other CS:S weapons from the script tables.
