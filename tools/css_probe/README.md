# CS:S movement probe

A SourceMod plugin that replays per-tick inputs on a bot in a real CS:S
dedicated server and logs the bot's position, velocity, ground and duck
state after every tick. `cargo run --features dev --bin movecmp` drives it
and compares the result with mashup's Source movement (see
docs/OBSERVABILITY.md).

Setup (once per machine; nothing here touches the CS:S game install):

1. Dedicated server, Steam app 232330, anonymous SteamCMD login:
   `steamcmd.sh +force_install_dir ~/games/css-ds +login anonymous +app_update 232330 validate +quit`
2. Metamod:Source 1.12 and SourceMod 1.12 (Linux builds from
   alliedmods.net) extracted into `~/games/css-ds/cstrike/`. The server is
   32-bit: delete `addons/metamod_x64.vdf`.
3. Compile and install the plugin:
   `cd ~/games/css-ds/cstrike/addons/sourcemod/scripting && ./spcomp <repo>/tools/css_probe/mashup_probe.sp -o ../plugins/mashup_probe.smx`
4. Copy `mashup_probe.cfg` (below) to `~/games/css-ds/cstrike/cfg/`.
5. In mashup.local.toml: `[games.cs_source_server]` `path = "~/games/css-ds"`.

movecmp starts the server itself (LAN only, 127.0.0.1:27030, `-insecure`)
when it isn't running, and stops it afterwards unless `--keep-running`.

`mashup_probe.cfg`:

```
sv_lan 1
sv_cheats 1
rcon_password mashup-probe
mp_freezetime 0
mp_roundtime 60
mp_ignore_round_win_conditions 1
mp_autoteambalance 0
mp_limitteams 0
mp_autokick 0
bot_join_after_player 0
bot_join_team ct
bot_zombie 1
bot_dont_shoot 1
bot_quota 1
bot_quota_mode normal
```

Notes:
- CS:S ignores `-tickrate`: the server always runs 0.015 s ticks.
- The plugin switches the bot to its knife (max speed 250) before a run,
  and on placement resets its move type, remembered ladder and jump
  stamina. Its log adds the water level and ladder state per tick.

## Weapon measurements

`weapcmp.py` (driver, Python 3, no dependencies; `rcon.py` is a minimal
Source RCON client) and `weapmeas.py` (one function per measurement) produce
the numbers in specs/cs_source/weapons.md, "CS:S values (measured)":

```
python3 tools/css_probe/weapcmp.py knife knife_range
python3 -c "import sys; sys.path.insert(0, 'tools/css_probe'); import weapmeas; weapmeas.timing('weapon_ak47')"
```

Plugin commands they use (all server commands, see the plugin header):
`mashup_wrun` (a run that keeps the bot's weapon and logs a `W` row with the
weapon state plus `E` rows for weapon events every tick; an optional sixth
input column selects weapon slot n-1 that tick), `mashup_give <weapon> [clip]
[reserve]`, `mashup_target x y z yaw health armor helmet hover` (the second
player; it is held at that yaw and its health/armour are restored after every
hit), `mashup_trace x y z pitch yaw [ignore_shooter]` (bullet-mask trace with
hitgroup), `mashup_scan` (finds long clear sightlines) and `mashup_info`.

Notes:
- The bot AI sets the bot's view angles and weapon on its own; weapon runs
  force `v_angle` to the input angles each command and block AI weapon
  selection, but the AI still switches back to its best gun after a forced
  switch to the knife and toggles silencers. Deploy timing is therefore
  measured by giving the gun mid-run (`weapmeas.deploy`).
- A bot that joins mid-round waits dead; `mashup_target` respawns it.
- Afterwards restore the movement setup: `bot_kick`, `bot_join_team ct`,
  `bot_quota 1`, `mp_friendlyfire 0`.
