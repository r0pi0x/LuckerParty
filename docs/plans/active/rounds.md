# Rounds, money and buying

Started 2026-10-06. Counter-Strike's match flow as a rules mode beside
deathmatch, so CS:S maps play like the game and minigame maps get round
restarts.

## Done (slice 1)

- `rules::rounds` (`mashup_rounds 1`): freeze (`mp_freezetime`, nobody
  moves or shoots), a timed round (`mp_roundtime`), elimination wins,
  defenders win on time, a pause (5 s), then everyone back at their
  spawns: the dead with starting weapons, survivors with theirs; loose
  weapons cleared. `RoundEnded` messages, `RoundState` (phase, number,
  wins).
- Money (`weapon::economy`): `Money` per character (`mp_startmoney`,
  capped at 16000), kill reward 300, team-kill penalty 3300, win bonus
  3250, loss bonus 1400 + 500 per consecutive loss up to 3400 (a draw pays
  both the loss bonus).
- Buying: `buy` charges the price (spec weapons.md's `WeaponPrice`;
  kevlar 650, kevlar+helmet 1000, helmet alone 350), refuses without the
  money or outside the freeze and the buy period (`mp_buytime`), and drops
  the weapon it replaces in the same slot.
- HUD: `HudAccount` (money) and `HudRoundTimer` panels, the round result
  across the screen.
- Tests: tests/rounds.rs, `weapon::economy` unit tests.

## Next

1. Measure on the probe server: the defaults (CS:S's own mp_roundtime and
   mp_freezetime defaults, the end delay), whether the buy period counts
   from the freeze's end, the clock during the freeze, a draw's bonus,
   rewards per weapon (CS:S pays 300 for every gun and the knife?).
2. Buy zones: `func_buyzone` by team is in (`economy::in_buy_zone`; maps
   without zones let you buy anywhere, CS:S would make zones around the
   spawns), the buy menu doesn't yet say why a buy failed. Ammo buying
   (`primammo`/`secammo`), bot buying preferences (ours: `economy::autobuy`, a primary for the team by `Prices::bot_weights` among the dearer half it can afford, then armour), the game's own VGUI buy-menu look
   (ours: `client/buy_menu.rs`, B or `buymenu`, CS:S's category keys
   and order, each team its own guns; rounds start from the team pistol
   and knife).
3. Round restarts re-creating map entities: done. `start_round` counts
   `core::RoundRestarts` up; the logic bridge re-creates its world from
   the map's entities (`LogicWorld::round_restart`: doors, buttons,
   movers and trains at their start, triggers re-armed, timers,
   counters and relays as spawned, the event queue dropped, logic_auto
   firing again 0.2 s later) and shows the nodes of broken or killed
   entities again (they are hidden, no longer despawned); the map layer
   clears gibs and particles and makes broken windows whole; game_text
   messages are cleared. Kept as they are: `world::ROUND_KEEP`, the
   preserve list from the public multiplayer code the spec quotes
   (func_brush, func_wall, func_buyzone, info_target, soundscapes,
   ropes, sky_camera). Tests: logic unit tests (`restart_tests.rs`),
   tests/map_logic.rs, de_nuke vents and doors, cs_office windows.
   Open (probe server, entity_io.md open question 1): CS:S's own keep
   list (does a func_brush disabled in round 1 stay disabled?); which
   logic_auto outputs fire each round (ours: OnNewGame, OnMapSpawn,
   OnMultiNewRound; OnMultiNewMap only at map load); whether decals,
   prop_physics and env_global states reset; whether the restart runs
   at the freeze's start (ours) or the round's end. Also done: the
   announcer (`map::RoundSounds`: `Event.TERWin`/`Event.CTWin`/
   `Event.RoundDraw`, a radio line when the round goes live; whether the
   game plays it at freeze end for both teams is to check) and team
   scores on the scoreboard.
4. Objectives: bomb (plant/defuse, C4), hostages; mp_timelimit and
   mp_maxrounds; team switching rules (mp_limitteams, autoteambalance).

## Decision log

- 2026-10-06: rounds are a rules mode (off by default) rather than
  replacing deathmatch: minigame maps and quick playtests keep instant
  respawns. Money lives in the weapon layer next to buying, which the
  rules layer drives through `BuyWindow`.
- 2026-10-06: round restarts reach the logic and map layers through a
  core counter (`RoundRestarts`) rather than a message, so a restart is
  never missed between fixed ticks and layers that load later see the
  current count; rules run in `SimSet::Rules`, before the logic.
  Re-creating the whole logic world (instead of resetting each class)
  matches CS:S, which re-creates map entities from the entity lump.
