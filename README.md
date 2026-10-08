# Lucker Party

> [!WARNING]
> ## 🤖⚠️ This entire repository is AI-generated ⚠️🤖
>
> **Every file here (code, shaders, tests, specs and docs) was written by AI
> coding agents (Claude), directed and reviewed by a human.** Expect the
> mistakes and quirks that come with that, and review anything before
> relying on it.

Exploratory design for building minigames out of other games' assets, behavior
and UI, and mixing those parts across games; "mashup" is the engine and dev
sandbox it runs on. Everything here is a prototype: allowed to be messy, and
expected to change as each imported game teaches us something.

Status: Bevy project set up (see CLAUDE.md for build commands). Working on the
foundation milestone below.

## Goal

Pull a "microcosm" out of an existing game (a map, its movement feel, a few
weapons, its HUD) and make it playable in Lucker Party. Then mix parts freely:

- Combat Arms rules, playing as a Modern Warfare 2 character with MW2 guns.
- A Combat Arms character in a Counter-Strike: Source defusal map.
- A Minecraft character on a CS:S minigame map.
- Smaller swaps: a Minecraft crosshair in CS:S, MapleStory damage numbers in
  Combat Arms.

Element swaps double as Roll the Dice modifiers.

## Scope for now

- First-person games only.
- First pair: Counter-Strike: Source, then Combat Arms. They are similar enough
  to test the shared core without fighting it, and different enough to expose
  CS-shaped assumptions.
- A third game (MW2 or Minecraft) is the next test of the interfaces.

Out of scope for now: 2D games such as MapleStory. Bringing 3D content into 2D
is workable (side-locked 3D or sprites rendered from 3D models); 2D into 3D
needs information the source does not have and is a gimmick at best.

## Engine direction

Leaning Bevy (Rust) for this prototype. The idea is essentially "an ECS with a
plugin per game", which is what Bevy is. Rust also has Source format parsers
(`vbsp`, `vmdl`, `vtf`, `vpk`). Known costs: Bevy API churn, immature UI, no
editor (Blender-as-editor tooling partly covers it). The editor matters less
here because imported content comes from importers, not hand placement.

## Architecture

### Mount layer

One adapter per game. It finds the player's install, reads its archives, and
converts assets into neutral data: mesh, skeleton, animation, texture,
material, sound, map, sprite, font, UI layout. Each adapter also owns the
game's units and coordinate conventions (a Source unit is about an inch; a
Minecraft block is a meter; some games are Z-up).

The engine only ever sees neutral data.

Mounts target the official, current release of each game, never a specific old
or modified build: users can't legitimately obtain old builds, and asking them
to undermines the "load from your own install" model. To survive game updates:

- Parse at the format level, not the build level. Formats change far less
  often than content.
- Read assets straight from the game's own archives at runtime and convert
  only what is actually loaded. Results that are expensive to produce
  (decrypting and parsing a world, BSP to meshes) go into a local cache.
- The cache is content-addressed: entries are keyed by a hash of the source
  file's bytes plus our converter version, so a game update only re-converts
  files whose bytes changed, and unchanged files are shared across builds.
  Each build has a small manifest (asset ID to hash); entries no current
  manifest references are deleted, and the cache has a size cap.
- A "mount doctor" per game checks expected files and reports what is mounted,
  what changed and what failed. The multiplayer pre-match check uses it too.

Supported sources are the Steam releases. An install is identified by its
Steam build ID plus file hashes. That identifies which build is installed
(the mount doctor warns on builds we haven't tested); it never names cache
folders, so an update doesn't duplicate anything.

Where imported data lives:

- Converted files are still the publisher's assets. They follow the same rule
  as the originals: never in git, never in a distributed build.
- The cache is per user, outside the repository:
  `~/.local/share/mashup/cache/` on Linux, `%LOCALAPPDATA%\mashup\cache\` on
  Windows.
- Development also has a `dump` tool that extracts an install's raw files into
  a local folder outside the repo, for exploring formats and writing specs.
  It shares the archive readers with the runtime mount; the game itself never
  needs a full dump.
- Moving assets between our own machines means copying the install or the
  cache directly (rsync, Syncthing), never through git.
- Install paths (and Combat Arms keys) live in the gitignored
  `mashup.local.toml`; see `mashup.local.example.toml`.
- Tests that need a real install skip when it isn't present.
- Our own original assets (greybox textures, our sounds, UI) may be committed,
  with Git LFS if they get large.

How code refers to assets:

- Game code uses stable namespaced IDs we own (`combat_arms:weapon/m4a1`),
  never archive paths.
- The importer builds the catalog mapping each ID to where the asset lives in
  this install, following the game's own data (a weapon's data file names its
  model and sounds) rather than hard-coded paths.
- When a release moves something, a committed per-fingerprint alias file
  records it; content hashes catch pure renames. File names and hashes are
  facts about the game and may be committed; contents may not.
- Importing writes a catalog snapshot (IDs, source paths, hashes; no content)
  that we commit, so a game update shows up as a reviewable diff.

### Slots

A match is a loadout that picks one implementation per slot. Each game
contributes implementations as a plugin.

| Slot | Contents | Examples |
|---|---|---|
| Map | Geometry, collision, lighting, spawns, objectives | de_dust2, a Combat Arms map |
| Ruleset | Rounds, win conditions, economy, respawns | CS defusal, CA elimination |
| Movement | Speeds, acceleration, jump, crouch, sprint, air control | Source, MW2 |
| Character body | Third-person model, skeleton, animations | CA soldier, MW2 operator, Steve |
| Weapon | Trigger/cost/delivery/effect plus feel | CS AK-47, MW2 M4 |
| Viewmodel | First-person hands and gun, reload/draw/fire animations | MW2 hands with M4 |
| HUD pieces | Crosshair, health, ammo, killfeed, damage numbers | CS crosshair, MapleStory numbers |
| Audio | Footsteps, gunshots, announcer, hit sounds | CA announcer |

Slots can get more granular as needed (the HUD row already is).

### Intent

Nothing reads the keyboard directly. Local input, bot brains and (later) the
network all write the same `Intent` component: move direction, look, jump,
crouch, sprint, fire, reload. The Movement slot turns intent into motion and
publishes movement state; weapons read intent plus that state. A bot is just
another intent source, so it uses exactly the same movement and weapons as a
player, and this matches the multiplayer model (clients submit intent).

### Interfaces

Each slot declares what it provides and what it needs, so any implementation
that satisfies the contract can be swapped in:

- A crosshair needs the current spread and aim state. Any weapon providing
  spread works with any crosshair.
- Movement provides state (sprinting, crouching, airborne, landing). Weapons
  read it, so "can't fire while sprinting" is a weapon rule, not a hard link
  between MW2 movement and MW2 guns.
- A character body maps its animations onto a standard action set (run,
  crouch, jump, aim, reload, die). That mapping is the retargeting work and
  probably the hardest single piece.
- A ruleset declares required capabilities (defusal needs bomb sites and a
  plant/defuse interaction). Loadouts are validated before a match starts;
  invalid combinations fail cleanly or use a declared fallback.

### Bundles

Some parts belong together: MW2's M4 model, reload animation, hands and fire
sound. Pick the bundle by default; allow overriding individual pieces for
chaos combinations.

### Weapons and items

"Weapon" is not a type. Any use action is four composable parts:

| Part | Question | Examples |
|---|---|---|
| Trigger | How is it activated? | primary/secondary, hold, charge, tap, toggle |
| Cost | What does it consume? | magazine and ammo type, energy, mana, durability, cooldown, nothing |
| Delivery | How does it reach things? | hitscan, projectile, beam, melee swing, area, self |
| Effect | What happens on hit? | damage, break block, knockback, stun, heal, ignite |

Examples: CS AK-47 is primary, magazine, hitscan with spread, damage. A
Minecraft pickaxe is primary, durability, melee swing, break block plus small
damage.

Each host declares which effects it supports. Unsupported parts do nothing or
go through a bridge (a pickaxe in CS:S deals melee damage; mining has no
blocks to act on unless a bridge maps it to doors and glass).

Feel that doesn't decompose (CS recoil patterns, MW2 aim-down-sights) lives in
per-game components that travel with the weapon.

Prior art: Minecraft's item data components; Unreal's Gameplay Ability System.

### Components and categorization

- Core vocabulary: Transform, Velocity, Health, Team, Hitbox, Inventory,
  Weapon parts. Small and normalized.
- Per-game components are namespaced (`cs_source::RecoilPattern`,
  `mw2::Perk`).
- Bridges translate between per-game and core components.
- Rule of three: a concept moves into the core only after two or three games
  need it. A slot interface is not proven until two games implement it.
- Things are described with tags, not a class hierarchy (an AWP is `weapon`,
  `ranged`, `hitscan`, `scoped`).
- Every imported item keeps a namespaced ID and its source path
  (`cs_source:weapons/awp` from `scripts/weapon_awp.txt`).
- The catalog is generated by importers, never hand-edited, so changing the
  categorization means changing importer code and regenerating.
- LLM help is used offline for bulk tagging and proposing normalized values;
  the rules are saved as importer code and spot-checked by a human.

### Normalization and mixing rules

- The host game's rules win; imported parts adapt to the host.
- Shared quantities are normalized: health and damage as a fraction of a
  standard player's health, speed relative to the host's walk speed, fire rate
  in seconds.
- Cross-game questions (whose sprint rules apply, what an MW2 perk does to
  Steve) are design decisions, made explicit as bridges or as ruleset options.
  Many make good dice modifiers.
- Cross-game kits (a CS:S character's moveset in another game) are generated
  ahead of time as data, reviewed and playtested, never generated during a
  match. A runtime "chaos kit" could exist later as a server-generated mode.

## Multiplayer

Same principles as the main game: the server owns the loadout, rules and
outcomes; clients submit intent. Every player must have each game the loadout
uses mounted; the server checks this before a match.

## Assets, code and legal

- Never ship other games' assets. Load them at runtime from the player's own
  install (the Garry's Mod model). Check each publisher's terms before a
  public release, especially Nexon/Valofe titles.
- Reverse engineering formats for interoperability is generally lawful; check
  each game's EULA.
- Facts and ideas (file layouts, values, formulas, how a mechanic behaves) are
  free to reuse. The specific code expressing them is not, and a line-by-line
  translation into Rust is still a copy.
- Workflow: read decompiled or reference code, write down what it does as a
  plain spec (formats, values, rules, edge cases), then implement from the spec
  in this project's ECS architecture rather than mirroring the original's
  structure.
- Never commit decompiled or leaked code itself.

### Per-game notes

- **Counter-Strike: Source.** Steam app 240, Linux build. Tested build
  25338318: `de_dust2.bsp` is BSP version 20 (loose in `cstrike/maps/`); VPK
  directories are version 2 (`cstrike/cstrike_pak_dir.vpk`, and
  `hl2/hl2_textures_dir.vpk`, `hl2/hl2_misc_dir.vpk` for shared content).
  Source 1 formats (VPK, BSP, MDL/VVD/VTX,
  VTF/VMT) are well documented. Valve's public Source SDK 2013 includes the
  shared movement code. Weapon stats live in `weapon_*.txt` scripts. Faithful
  rendering needs BSP lightmaps and Source material rules.
- **Combat Arms.** LithTech Jupiter (`.rez` archives, LithTech model and
  world formats). Its loader is developed locally and not part of this
  repository for now (`src/games/combat_arms/` is git-ignored; build it with
  `--features combat_arms` where the folder exists).

## Specs

Behavior taken from decompiled or leaked code reaches this repository only as
specs in `specs/`: constants with units, formulas as math, per-tick order of
operations, edge cases, deliberate quirks, and test cases with expected
numbers. Specs are written in sessions that read the source; implementation
happens in separate sessions that see only the specs. The test cases become
automated tests. See [specs/README.md](specs/README.md).

## First steps

MVP: a Combat Arms slice. Run, sprint, shoot and fight bots on one map, built
through the slots so another game's controller is a loadout change. Gameplay
never waits on asset formats.

1. ~~Create a Bevy project in this repository.~~
2. Foundation: core components, `Intent`, loadout resource with one plugin per
   slot, a greybox map built in code, collision queries, debug tools (inspector,
   free-fly camera), placeholder movement.
3. Combat Arms movement from its spec, as a kinematic character controller.
4. One Combat Arms weapon from its spec as trigger/cost/delivery/effect parts,
   plus hitscan, hit zones, health, crosshair and ammo HUD.
5. Bots (intent from a brain on a waypoint graph, navmesh later) and a simple
   deathmatch ruleset with respawns.
6. Real maps, in parallel with 2-5: `de_dust2` from a local CS:S install (BSP
   geometry, VPK textures, lightmaps), then a Combat Arms map once `.rez` and
   the world format are specced.
7. Prove the swap: Source movement (specced from Source SDK 2013) as a second
   Movement implementation, switchable live against Combat Arms movement on
   the same map. MW2 follows the same path.

## Open questions

- Which third game tests the interfaces best: MW2 or Minecraft?
- How do players pick loadouts: fixed per minigame, chosen in a lobby, rolled
  by dice, or a mix?

## License

The code in this repository is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution
you submit for inclusion is dual licensed as above, without any additional
terms or conditions.

This license covers this repository's own code and documents only. No game
files are included: maps, models, textures and sounds are loaded at run time
from your own installs of the games. Game names are trademarks of their
owners; this project is not affiliated with or endorsed by Valve or any
other game publisher.
