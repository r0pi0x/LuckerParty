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

## Test map: mashup_logic_test

Every entity class we support gets an example in one map we generate
ourselves, so each can be checked in mashup and measured in real CS:S.

- **Two layers.** (1) Fixtures in Rust tests: entity key values and box
  brush models handed straight to the logic layer, no compiler needed;
  fast and deterministic, one per behaviour. (2) A real map: a script
  (`tools/testmap/`) writes a Hammer `.vmf` (text; our own geometry,
  stock CS:S texture names only), Valve's compilers (vbsp, vvis, vrad)
  build the `.bsp` into the mashup cache. The `.vmf` and script are
  committed; the `.bsp` never is (.gitignore).
- **Compilers.** The Linux CS:S install has none; the Windows install
  ships them in `bin/` (check), so `scripts/compile_testmap.ps1` builds it
  there. Linux would need Wine (the user's call) or the Windows build
  copied over.
- **Reference.** The compiled map runs on the local CS:S probe server, so
  door speeds, push strength, teleport and hurt timing, counter and relay
  ordering are measured from the real game (like the movement and weapon
  probes), then compared with mashup on the same map.
- **Rooms** (each with a visible result: a light, a toggled `func_brush`
  or a `game_text`, so a screenshot or a probe can read it):
  1. I/O: `logic_auto` → `logic_relay` chain with delays, `logic_timer`,
     `math_counter` (add to max fires `OnHitMax`), `logic_case`,
     `logic_branch`, `filter_activator_name` on a trigger.
  2. Triggers: `trigger_once`, `trigger_multiple` (wait), `trigger_teleport`
     to an `info_teleport_destination`, `trigger_push` (up and sideways),
     `trigger_hurt` (damage per second).
  3. Moving brushes: `func_door` (sliding, wait/return), `func_door_rotating`,
     `func_button` opening a door, `func_movelinear`, `func_rotating`,
     `func_tracktrain` on `path_track`s, `func_breakable` (glass and wood).
  4. Equipment and rounds: `game_player_equip`, placed `weapon_*`,
     `game_text`, a round restart from a trigger.

## Slices

1. [x] Mount the install's `download/` and `custom/` maps plus the mashup
   cache; `map <name>` finds custom maps; `maps` lists them; `import
   <file.bsp|.bsp.bz2>` copies into the cache with a hash. Test: a map in
   the cache loads by name (a tiny generated BSP, or skip without one).
   Done 2026-10-06: `games/cs_source/mount.rs` (layers and
   `import_map`), console `maps`/`import`; the index records an FNV-1a
   64 hash for now (SHA-256 when networking needs it).
2. [ ] Test map generator (`tools/testmap/`) and fixtures; compile on
   Windows; measure the reference behaviour on the probe server.
3. [ ] Spec entity I/O and triggers (specs/source/entity_io.md); the
   `logic` layer; `trigger_teleport`, `trigger_push`, `trigger_hurt`,
   `trigger_multiple/once`, `logic_*`, `math_counter`. Pick two real
   minigame maps from the user's downloads as targets.
   Draft specs written 2026-10-06 (need review and the probe-server
   measurements in their Open questions): specs/source/entity_io.md,
   specs/source/triggers.md. The install's `download/maps` is empty, so
   the target maps are still to be chosen.
   Built 2026-10-06 (src/logic, game-independent; `MapData::entities`
   from games): the event queue, target matching, AddOutput/Kill/
   FireUser, logic_auto/relay/timer/case/compare/branch, math_counter,
   filters, game_text (client draws it), point_servercommand/
   point_clientcommand with allowlists, trigger_multiple/once/hurt (moved
   from map/hurt.rs)/push/teleport/gravity/remove. The specs' test cases
   are unit tests (src/logic/tests.rs); tests/map_logic.rs runs teleport
   and push with Source movement. Open: the target maps, the probe-server
   measurements in the specs' open questions.
4. [x] Spec and build moving brush entities (doors, buttons, platforms,
   breakables) with movement support for moving solids.
   Draft specs: specs/source/doors_buttons.md (+use, pushers, doors,
   buttons, movelinear, rotating, tracktrain) and
   specs/source/breakables.md (func_breakable, func_breakable_surf;
   de_nuke's vents are func_breakable, material Metal, health 1).
   Built 2026-10-06 except breakables: `+use` (E) with the spec's use
   search; pushers that carry riders, push and are blocked (crush damage,
   reversal); func_door, func_door_rotating (away from the user,
   chainstodoor), func_button, func_movelinear, func_rotating,
   func_tracktrain + path_track, func_brush, and brushes parented to a
   mover. Movers have their own node (`map::MapBrushEntity`) and a
   `core::MovingSolid` that Source movement sweeps and rides (base
   velocity on leaving). Tests: unit tests per spec case, a door on use
   and a lift (tests/map_logic.rs), de_nuke's door pairs
   (tests/map_de_nuke_doors.rs). Breakables built 2026-10-06
   (src/logic/breakables.rs; nodes like movers, `core::Damageable`,
   weapon damage routed through `LogicSet::Damage`; gibs and window panes
   in `map::breakables`; tests: spec cases as unit tests,
   tests/map_breakables.rs shoots and knifes de_nuke's vents and shoots a
   cs_office window). Left: breakable follow-ups (backlog 7), prop_door_rotating,
   props parented to movers, train orientation and control.
5. [ ] Gameplay entities (`game_player_equip`, `game_text`, map weapons,
   rounds) until the two target maps play through.
6. [ ] Networking, then server-sent map download (hash check, bz2).
7. [ ] Per-map extra mounts; a mashup package format.

## Decision log

- 2026-10-06: maps the player downloads are cached in mashup's own data
  folder; the game install is mounted read-only (including its
  `download/` folder). Entity logic comes before networking, so the first
  custom map loaded actually plays.
