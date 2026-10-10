Lucker Party (playtest build)
============================

Counter-Strike: Source, rebuilt in a new engine. It loads maps, models,
sounds and menus from YOUR OWN copy of CS:S, so nothing from the game is
included in this download.

What you need
-------------
- Windows 10/11 (64-bit) or Linux.
- Counter-Strike: Source installed through Steam (Steam app 240). The game
  finds it on its own (Steam library folders). If it can't, it asks for the
  folder on first start: paste the folder that holds "cstrike" and "hl2",
  usually  C:\Program Files (x86)\Steam\steamapps\common\Counter-Strike Source
- Everyone playing together must use the same build (same download).
  A different build is refused when joining, with a message saying so.

Start
-----
Run mashup.exe (Linux: ./mashup). Windows may warn that the program is
unrecognised (it isn't signed): "More info" -> "Run anyway".

Play together
-------------
One player hosts:
  - Main menu -> Create Server, pick a map, Start. Or, without playing
    yourself, run the dedicated server:
      mashup_server.exe -port 27015 +map de_dust2 +maxplayers 8 +mashup_rounds 1 +bot_quota_mode fill +bot_quota 8
    (+sv_password <pw> for a password, +hostname "<name>" to name it).
  - Allow the Windows Firewall prompt the first time (private networks).
  - Over the internet: forward UDP port 27015 on your router to your PC and
    give friends your public IP address. If your router has no public
    address (carrier-grade NAT), use Tailscale or ZeroTier instead, or let a
    friend with a public address host.
  - Upload needed: about 0.1 Mbit/s per friend.

Everyone else joins:
  - Main menu -> Find Servers (Lan tab, or Favorites -> Add a Server with
    the host's address), or open the console (~) and type:
      connect <host address>:27015

Useful console commands (~)
---------------------------
  mashup_rounds 1        Counter-Strike rounds with money (0: deathmatch)
  bot_add / bot_kick     add or remove bots; bot_quota <n>
  changelevel <map>      change map (host); maps lists them
  kill                   suicide (ends the round if you're the last one)
  mashup_debughud 1      movement/speed readout at the top left
  name <new name>        your player name

Known rough edges
-----------------
This is a playtest: expect bugs. Things CS:S servers usually get from
plugins (gungame weapon ladders, surf/kz timers, noblock) are not there.
Please report problems with what you were doing, the map, and if possible
a screenshot.
