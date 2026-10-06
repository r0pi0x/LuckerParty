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
2. Buy zones (`func_buyzone`, else around the team's spawns), ammo buying
   (`primammo`/`secammo`), a buy menu (the game's `buymenu` layout or our
   own), bots buying.
3. Round sounds (`Event.TERWin`/`Event.CTWin`/`Event.RoundDraw` game
   sounds), the scoreboard's team scores, round restarts re-creating map
   entities (logic layer).
4. Objectives: bomb (plant/defuse, C4), hostages; mp_timelimit and
   mp_maxrounds; team switching rules (mp_limitteams, autoteambalance).

## Decision log

- 2026-10-06: rounds are a rules mode (off by default) rather than
  replacing deathmatch: minigame maps and quick playtests keep instant
  respawns. Money lives in the weapon layer next to buying, which the
  rules layer drives through `BuyWindow`.
