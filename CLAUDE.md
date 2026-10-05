# CLAUDE.md

Private prototype for "mashup": minigames built from other games' assets and
behavior, with parts mixed across games (e.g. a Combat Arms character on a
Counter-Strike: Source map). It may later feed into Lucker Party
(https://github.com//LuckerParty), a separate public repo. Never put
prototype work there.

This file is a map. Follow the links for detail; keep it short.

## Where things are

| Need | Read |
|---|---|
| Design decisions and why (slots, mounts, weapon model, legal rules) | [README.md](README.md) |
| Code layout, layers, data flow | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| How to check your work (tests, screenshots, live ECS queries) | [docs/OBSERVABILITY.md](docs/OBSERVABILITY.md) |
| Current plan and progress | [docs/plans/active/](docs/plans/active/mvp-combat-arms-slice.md) |
| Known shortcuts | [docs/tech-debt.md](docs/tech-debt.md) |
| Writing behavior specs from original source | [specs/README.md](specs/README.md) |

## Hard rules

- Never commit other games' assets, extracted files or encryption keys. Load
  assets at runtime from the user's own install.
- Decompiled or leaked code never enters this repo and is never translated
  line by line. It reaches us only as specs written in a separate session.
  Implementation sessions read specs, not source.
- Bevy is pinned in `Cargo.toml` (0.19.1) and its API changes often: check the
  pinned crate source in `~/.cargo/registry/src/*/bevy_*-0.19.1/` or its
  examples instead of memory. Same for avian3d (0.7).
- Must build and run on Linux and Windows.
- Scope: first-person games only; CS:S and Combat Arms first.

`cargo test` enforces layering, the asset rule, the spec rule and this file's
links (`tests/architecture.rs`). Its failure messages say how to fix things.

## Commands

```
cargo run --features dev       # day-to-day (dynamic linking + remote protocol)
cargo test --features dev      # all tests; run before every commit
cargo run --features dev -- --help    # screenshot, spawn, look, movement options
cargo run --profile playtest   # optimized, quick to rebuild; for playtesting
```

## Working conventions

- Verify your own work with tests, screenshots or remote queries
  (docs/OBSERVABILITY.md) before calling it done. New behavior gets a
  scenario test.
- Commit at working milestones; `cargo test` must pass. `.githooks/pre-commit`
  enforces it (enable per clone: `git config core.hooksPath .githooks`).
- Propose a plan before writing a new mount adapter or loader. Multi-session
  work gets a plan in `docs/plans/active/`, kept updated.
- When something was hard because a tool, doc or check was missing, add it.
- Keep docs true: when code changes a fact in a doc, change the doc in the
  same commit.

## Testing on Windows

Remote: https://github.com/r0pi0x/LuckerParty (private). The games and the
original source live on the Windows PC, which has its own checkout
(`git clone https://github.com/r0pi0x/LuckerParty`); `scripts\playtest.ps1`
pulls and runs an optimized build. Spec sessions there write into `specs/`
in that checkout and push; implementation sessions pull them.
