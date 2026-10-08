# Plan: engine, Source and games as separate crates and repos

Status: proposed 2026-10-07. Nothing moved yet.

## Why

- Other people (a Discord server sharing game mashups) want to build on the
  engine: a Half-Life 2 group now, more games later. CS:S should be
  playable and buildable by others too.
- Lucker Party will move onto this engine, with friends writing original
  minigames.
- Before anything is shared, the work moves to an anonymous account
  (rewritten history; see "Order").

## Target layout

```
engine/              neutral: app shell, console, slots/mounts, physics,
                     movement framework, weapons/rounds/bots, HUD, net
source/              Source engine formats and shared Source behaviour:
                     BSP/VPK/VTF/VMT/MDL/PHY, entity I/O and logic classes,
                     soundscapes, Source materials and shaders
games/cs_source/     CS:S: library (plugins) + standalone app
games/combat_arms/   Combat Arms: library + standalone app (private)
games/originals/     Lucker Party's own games and assets
minigames/           minigame definitions + rule modules that mix games
apps/mashup/         dev sandbox: any game, any map, all debug tools
apps/lucker_party/   the party product
```

Rules:
- `engine` knows no game. `source` depends on `engine` only. Games depend
  on `engine` (and `source` if they're Source games), never on each other.
- Apps pick games with cargo features. A game that needs something the
  engine can't express gets a new engine abstraction, not a special case.
- Each game crate is both a library and a program: `cargo run -p
  cs_source` is a faithful CS:S (its menus, HUD, modes, loaded from the
  player's install); the same library supplies parts to minigames. A
  game's own modes are minigame definitions, so both uses run the same
  code.
- `tests/architecture.rs` already enforces most of this as layers;
  crates make the compiler enforce it.

## Minigames (the middle layer)

- A minigame is a data file picking pieces by id from any game (map,
  character, movement, weapons) plus a rule module by name:

  ```toml
  name = "Gun Game: Dust II"
  map = "cs_source:de_dust2"
  movement = "cs_source:movement"
  weapons = ["cs_source:weapon_*"]
  rules = "originals:gungame"
  players = "2-16"
  ```

- Rule modules (round flow, scoring, win conditions) are Rust code that
  games register by name; definitions load at run time, so new minigames
  need no rebuild. The existing modes (deathmatch, bomb and hostage
  rounds) become the first rule modules.
- No native dynamic plugins (Rust has no stable ABI; a Bevy dylib must
  match the compiler and Bevy version exactly). If rule code must ship
  without a rebuild later, scripted rule modules (WASM or Lua) behind the
  same interface.

## Repos

- Private (new account): the current repo, full history rewritten to the
  new identity. Holds everything, including `games/combat_arms` and the
  private workflow (spec sessions, probe server, refcmp references).
- Public, each starting from one clean initial commit of a checked tree
  (no old history): `engine` and `source` (one repo with two crates at
  first is simpler for contributors), then `cs_source`. Other groups'
  games (HL2) live in their repos, depending on these by git.
- The private repo depends on the public ones by git, with a local
  `[patch]` to sibling checkouts, so one `cargo run` builds everything and
  engine changes are visible immediately.

## Before anything goes public

- No game assets, extracted files or keys (tests enforce this; check by
  hand too).
- Specs are sorted by where they came from: anything written from Combat
  Arms material stays private; each Source/CS:S spec is listed with its
  source for the owner to decide.
- Docs describing the private workflow (CLAUDE.md, plans, backlog, Windows
  spec sessions, probe server) stay private; public repos get their own
  README and contributor guide.
- Scan for personal traces (names, usernames, machine paths, Discord ids,
  emails); commit only under the new identity; pick a licence
  (e.g. MIT/Apache-2.0).

## Order

1. [ ] New account: log it into `gh`; pick the commit name and use its
   noreply email.
2. [ ] Back up, rewrite history to the new identity, create the private
   repo under the new account, push, re-point `origin`; Windows
   re-clones; owner deletes the old repo when satisfied.
3. [ ] Cargo workspace in this repo, one crate at a time, tests passing
   at each step: `engine` out of `src/` first, then `source` (pull the
   Source formats and logic out of `games/cs_source`, `logic`, parts of
   `map`), then `games/*` and `apps/*`. Sort every module first (table
   below), then move.
4. [ ] Minigame definitions and rule modules; the existing modes as the
   first definitions.
5. [ ] Pre-publication check, then the public `engine`/`source` repos;
   later `cs_source`.
6. [ ] Networking; the Lucker Party app (lobby, party flow, rotation,
   launcher), ported from the Godot prototype.

## Module sorting (to fill in at step 3)

| Module | Goes to | Notes |
|---|---|---|
| `src/core.rs`, `console.rs`, `slots.rs`, `harness.rs` | engine | |
| `src/logic` | source | Source entity I/O and classes (func_door, triggers, ...) |
| `src/map` | split | neutral runtime (MapData, culling, particles) vs Source materials/shaders (world.wgsl, water) |
| `src/games/cs_source` | split | formats (bsp, vpk, material, mdl, phy, wav, sound) to source; CS:S rules, weapons, HUD, bots' CS:S parts stay |
| `src/games/combat_arms` | games/combat_arms | private |
| `specs/` | split | `source/` to source (after review), `cs_source/` to cs_source, `combat_arms/` private |
