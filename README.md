# Mashup prototype

Exploratory design for building minigames out of other games' assets, behavior
and UI, and mixing those parts across games, as a possible future for Lucker
Party (https://github.com//LuckerParty). Everything here is a
prototype: kept in its own private repository, separate from the shipping Godot
build, allowed to be messy, and expected to change as each imported game
teaches us something.

Status: design notes only. No code yet.

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

The current Godot game keeps working unchanged while this is explored.

## Architecture

### Mount layer

One adapter per game. It finds the player's install, reads its archives, and
converts assets into neutral data: mesh, skeleton, animation, texture,
material, sound, map, sprite, font, UI layout. Each adapter also owns the
game's units and coordinate conventions (a Source unit is about an inch; a
Minecraft block is a meter; some games are Z-up).

The engine only ever sees neutral data.

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
- Never commit decompiled or leaked code itself. Leaked source (Combat Arms)
  also carries trade-secret risk, so apply the same workflow with extra care.
- Keep this repository private.

### Per-game notes

- **Counter-Strike: Source.** Source 1 formats (VPK, BSP, MDL/VVD/VTX,
  VTF/VMT) are well documented. Valve's public Source SDK 2013 includes the
  shared movement code. Weapon stats live in `weapon_*.txt` scripts. Faithful
  rendering needs BSP lightmaps and Source material rules.
- **Combat Arms.** LithTech Jupiter: `.rez` archives and LithTech model/world
  formats. Thin community tooling; expect format reverse engineering from
  installed files. Now operated by Valofe.

## First steps

1. Create a Bevy project in this repository.
2. Load `de_dust2` from a local CS:S install: BSP geometry, VPK textures,
   lightmaps.
3. Source-style first-person movement as a Movement slot implementation.
4. One CS:S weapon from its script data, built from trigger/cost/delivery/
   effect parts, plus a CS crosshair as a HUD piece.
5. Load a Combat Arms map from installed files and run the same slots on it.

## Open questions

- Is this a new direction for Lucker Party or an experiment alongside it?
- Which third game tests the interfaces best: MW2 or Minecraft?
- How do players pick loadouts: fixed per minigame, chosen in a lobby, rolled
  by dice, or a mix?
