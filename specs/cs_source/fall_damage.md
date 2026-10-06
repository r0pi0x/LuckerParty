# Counter-Strike: Source: fall damage and trigger_hurt

Source basis: measurements only, on a local CS:S dedicated server (the
tools/css_probe plugin over RCON, 0.015 s tick, de_dust2 and de_port),
plus the landing rules of the public SDK 2013 shared movement code already
in specs/cs_source/movement.md ("Falling and landing"). No game code was
read for this spec.
Status: implemented (src/games/cs_source/movement.rs, src/map/hurt.rs)

## Summary

- Landing faster than 580 units/s out of water costs
  floor((fall speed − 580) × 100 / 416) health. 100 damage (death at full
  health) comes at 996 units/s, not the shared code's 1024.
- Armour doesn't absorb it. There is no attacker (the event's attacker is
  0, "worldspawn" on death).
- `trigger_hurt` with damage D (per second) bites everyone touching it for
  D / 2 every half second (33 ticks at 0.015 s).

## Fall damage

**Fall speed** is −vz at the start of the landing tick (the movement spec's
"fall speed"), i.e. −vz logged at the end of the previous tick.

| Name | Value | Unit |
|---|---|---|
| fall_safe | 580 | units/s: no damage at or below |
| fall_damage_per_speed | 100 / 416 = 0.240385 | hp per unit/s above fall_safe |
| fall_fatal (100 hp) | 996 | units/s |

damage = floor((fall − 580) × 100 / 416), when fall > 580 and the player
isn't in water (water level 0; the movement spec's landing check). Health
loses exactly that many whole points.

Fit: 50 landings from 573 to 2012 units/s; floor with safe speed in
579.9..580.2 and slope 0.2404..0.2405 is the only rounding rule that fits
all of them (round and ceil need a safe speed of 582 or 584, which the
measured 0 at 584 and 1 at 585 then pin to the same curve). 100/416 lies in
the slope range; 1.25 × 100/444 (0.2815) and 100/444 (0.2252) don't.

| Fall speed | Damage | Health after (from 100) |
|---|---|---|
| 573 .. 584 | 0 | 100 |
| 585 | 1 | 99 |
| 648 | 16 | 84 |
| 836 | 61 | 39 |
| 984 | 97 | 3 |
| 992 | 99 | 1 |
| 996 | 100 | dead |
| 1024 | 106 | dead |
| 1124 | 130 | |
| 1512 | 224 | |
| 2012 | 344 | |

(Rows past 100 measured with health 1000.)

- Armour 100 with fall speed 836: damage 61, armour stays 100.

### Method

`python3 tools/css_probe/fallmeas.py [start speeds]`: `mashup_health 1000`
(plugin command: respawns the probe bot if dead, sets health and armour),
then a `mashup_wrun` whose start line places the bot 40 units above the
tile floor inside the CT spawn building on de_dust2 (352, 2384, −126.24)
with a downward start velocity. The run log's tick rows give the landing
tick (first grounded row) and the previous row's vz; its `player_hurt` row
gives `dmg_health`. `python3 tools/css_probe/fallmeas.py armor` repeats the
armour and death checks.

Not measured: landing on a moving or floating object (the movement spec's
adjustments), the landing view punch and the CS:S landing slowdown (already
in the movement spec's stamina values).

## trigger_hurt

Of the stock maps on the reference install, only de_port has one (the sea
under the map: model *11, a box −7424..7936 × −6816..2144 × −128..128,
`damage` 50, `damagetype` 16384 (drown), `damagemodel` 0, `damagecap` 20,
`spawnflags` 1, `StartDisabled` 0). dust2 has none.

Measured on de_port, the bot standing on the floor at (256, 2000, 0) inside
it for 300 ticks: 25 damage (half of `damage`) on tick 4, then every 33
ticks (0.495 s), 10 bites in 300 ticks. Armour 0, so armour's effect isn't
known.

Implemented: each volume's brushes (its brush model's convex brushes, moved
by the entity origin); a living character whose bounding box overlaps one
takes damage / 2 every 0.5 s, due within half a tick of the half second
(which gives 33 ticks at 0.015 s); the first touch after nobody was inside
bites at once. One timer per volume, shared by everyone in it (inferred).
Left out: `StartDisabled 1` (needs map logic), triggers that don't hurt
players (spawnflags without bit 1, when not 0).

## Open questions

1. `damagemodel` 1 (doubling damage up to `damagecap`) isn't used by any
   stock map; it hurts at the constant rate here.
2. Whether armour reduces trigger_hurt damage (drown type), and whether
   other damage types differ.
3. Whether the half-second timer is per trigger (shared) or per toucher.
