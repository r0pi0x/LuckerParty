# Custom maps and minigames

Started 2026-10-06. Goal after the foundations: play community CS:S
minigame maps (`mg_`, `deathrun_`, `surf_`, `jb_`), and later maps from
other games and our own mixes.

## Where maps come from and where they live

- **Read the game install, never write it.** CS:S keeps stock maps in
  `cstrike/maps` (and the VPKs), user content in `cstrike/custom`, and
  maps fetched from servers in `cstrike/download`. Players of community
  servers already have many minigame maps there. Mount all of them
  read-only.
- **Write only to mashup's own cache**, next to the config folder
  (`~/.local/share/mashup/`, `%APPDATA%\mashup\` on Windows), laid out like
  Source's download folder:

  ```
  mashup/content/cs_source/download/maps/<name>.bsp
  mashup/content/cs_source/download/{materials,models,sound}/...
  mashup/content/index.toml          # name -> hash, source URL, fetched date
  ```

  Not the Steam folder: it may need admin rights (Program Files), mixes
  our files into another product, and ties the cache to one game.
- **Search order** (extends `cs_source::mount::open`): the map's own
  pakfile, `custom/`, official CS:S content, HL2 content, then the
  install's `download/` and our cache last, so a custom map can't replace
  stock files (as in Source). Content packed in the BSP already works.
- **Versions:** Source keys maps by name only ("map differs from
  server"). We keep the name for lookup and record a content hash
  (SHA-256 of the `.bsp`) in the index; two versions of one name can sit
  side by side under `maps/<name>/<hash>.bsp` once that matters.
- **Distribution:** CS:S has no meaningful Workshop; servers send maps
  over HTTP (`sv_downloadurl`, `.bsp.bz2`). With networking, a server
  advertises `name + hash + URL`; the client uses the cached copy when
  the hash matches, else downloads, unpacks bz2, verifies and caches.
  Community maps are never committed to this repo; they still need the
  player's own CS:S content.

## Map IDs and other games

- `game:name` stays the ID. A custom CS:S map is `cs_source:mg_whatever`,
  found by the same search.
- **Source maps needing another game's content** (HL2:DM or CS:GO ports):
  a per-map list of extra installs to mount ("source BSP, mount
  `cs_source` + `hl2mp`"), not hard-coded into the CS:S module.
- **Mashup packages** (mixes, our own maps): a zip with a manifest
  (format, required installs, hash) and the map file. Design it when the
  first one exists.

## What minigame maps need (the bulk of the work)

Today only the static world, props, sounds and a few effects load. Minigame
maps are mostly entity logic. Each part gets a spec from the public SDK
first (specs/README.md), then a game-independent implementation:

- **Entity I/O:** named entities, outputs (`OnStartTouch`, `OnPressed`,
  `OnTrigger`...) firing inputs with parameters and delays; `logic_relay`,
  `logic_timer`, `logic_auto`, `math_counter`, `logic_case`,
  `logic_branch`, `filter_*`. A new layer (`logic`), below rules.
- **Triggers:** `trigger_teleport` (+ `info_teleport_destination`),
  `trigger_push`, `trigger_hurt`, `trigger_multiple`, `trigger_once`.
- **Moving brush entities:** `func_door`, `func_door_rotating`,
  `func_button`, `func_movelinear`, `func_rotating`, `func_tracktrain`,
  `func_breakable`. The movement code sweeps a static world; it needs
  moving solids (push the player, ride platforms, block).
- **Gameplay entities:** `game_player_equip`, `game_text`, `weapon_*`
  placed in the map, `point_servercommand` (a safe subset), round restart.
- **Out of scope:** maps that need SourceMod server plugins.

## Slices

1. [ ] Mount the install's `download/` and `custom/` maps plus the mashup
   cache; `map <name>` finds custom maps; `maps` lists them; `import
   <file.bsp|.bsp.bz2>` copies into the cache with a hash. Test: a map in
   the cache loads by name (a tiny generated BSP, or skip without one).
2. [ ] Spec entity I/O and triggers (specs/source/entity_io.md); the
   `logic` layer; `trigger_teleport`, `trigger_push`, `trigger_hurt`,
   `trigger_multiple/once`, `logic_*`, `math_counter`. Pick two real
   minigame maps from the user's downloads as targets.
3. [ ] Spec and build moving brush entities (doors, buttons, platforms,
   breakables) with movement support for moving solids.
4. [ ] Gameplay entities (`game_player_equip`, `game_text`, map weapons,
   rounds) until the two target maps play through.
5. [ ] Networking, then server-sent map download (hash check, bz2).
6. [ ] Per-map extra mounts; a mashup package format.

## Decision log

- 2026-10-06: maps the player downloads are cached in mashup's own data
  folder; the game install is mounted read-only (including its
  `download/` folder). Entity logic comes before networking, so the first
  custom map loaded actually plays.
