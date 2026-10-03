# CLAUDE.md

Private prototype for "mashup": minigames built from other games' assets and
behavior, with parts mixed across games (e.g. a Combat Arms character on a
Counter-Strike: Source map). It may later feed into Lucker Party
(https://github.com//LuckerParty), which is a separate public repo.
Never put prototype work there.

**Read [README.md](README.md) first.** It records every design decision: slots
and interfaces, mount adapters, the trigger/cost/delivery/effect weapon model,
normalization rules, and the legal rules for assets and decompiled code. Keep
it current when decisions change.

## Constraints

- Engine: Bevy (Rust), pinned in `Cargo.toml` (currently 0.19.1). Bevy's API
  changes between releases, so check the docs or source for the pinned version
  (docs.rs/bevy/<version>, or the `vX.Y.Z` tag's `examples/`) instead of
  relying on memory.
- Must build and run on Linux and Windows.
- Never ship or commit other games' assets, or files extracted from them.
  Load them at runtime from the user's own install. `.gitignore` blocks common
  Source/LithTech formats as a backstop.
- Decompiled or leaked code is never committed or translated line by line.
  Write a plain spec of the behavior first (formats, values, rules, edge
  cases), then implement from the spec in this project's ECS architecture.
- Scope for now: first-person games only. CS:S first, then Combat Arms.

## Building

```
cargo run --features dev     # day-to-day: Bevy dynamically linked
cargo run --release          # never with --features dev
```

- `dev` feature enables `bevy/dynamic_linking` for faster incremental links.
- Dev profile: our crate at opt-level 1, dependencies at 3.
- `.cargo/config.toml`: Windows uses `rust-lld.exe`; Linux uses rustc's
  default LLD (mold optional, see the comment there).

## Working conventions

- Commit at working milestones.
- Propose a plan before writing a new mount adapter or loader.
