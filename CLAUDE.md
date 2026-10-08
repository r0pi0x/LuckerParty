# CLAUDE.md

Lucker Party (private monorepo; "mashup" is its engine and dev sandbox):
minigames built from other games' assets and behavior, with parts mixed
across games (e.g. a Combat Arms character on a Counter-Strike: Source map),
and later original minigames. Planned layout:
[docs/plans/active/engine-split.md](docs/plans/active/engine-split.md).

This file is a map. Follow the links for detail; keep it short.

## Where things are

| Need | Read |
|---|---|
| Design decisions and why (slots, mounts, weapon model, legal rules) | [README.md](README.md) |
| Code layout, layers, data flow | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| How to check your work (tests, screenshots, live ECS queries) | [docs/OBSERVABILITY.md](docs/OBSERVABILITY.md) |
| Current plan and progress | [docs/plans/active/](docs/plans/active/engine-split.md) |
| What to build next (to-do list) | [docs/backlog.md](docs/backlog.md) |
| Known shortcuts in existing code | [docs/tech-debt.md](docs/tech-debt.md) |
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
links (`tests/it/architecture.rs`). Its failure messages say how to fix things.

## Commands

```
cargo run --features dev       # day-to-day (dynamic linking + remote protocol)
cargo test --features dev -- --skip heavy::   # fast tier; before every commit
cargo nextest run --features dev   # full suite (or `cargo test --features dev`,
                                   # 2x slower); before pushing or merging
cargo test --features dev --test it weapons::   # one test file (tests/it/)
cargo run --features dev -- --help    # screenshot, spawn, look, movement options
cargo run --profile playtest   # optimized, quick to rebuild; for playtesting
```

Integration tests are one crate, `tests/it/` (slow ones in `tests/it/heavy/`);
docs/OBSERVABILITY.md section 1 says where new ones go.

## Working conventions

- Verify your own work with tests, screenshots or remote queries
  (docs/OBSERVABILITY.md) before calling it done. New behavior gets a
  scenario test.
- Commit at working milestones; the fast tier must pass, the full suite
  before a push or merge to main. `.githooks/pre-commit` and `pre-push`
  enforce it (enable per clone: `git config core.hooksPath .githooks`).
- Propose a plan before writing a new mount adapter or loader. Multi-session
  work gets a plan in `docs/plans/active/`, kept updated.
- When something was hard because a tool, doc or check was missing, add it.
- Keep docs true: when code changes a fact in a doc, change the doc in the
  same commit.

## Commit identity

Commits are by `r0pi0x <339403489+r0pi0x@users.noreply.github.com>` with
UTC dates: set that identity in the clone's local git config and commit
with `TZ=UTC`. `.githooks/pre-push` refuses anything else. No real names,
personal emails or machine paths in commits or files.

## Testing on Windows

Remote: https://github.com/r0pi0x/LuckerParty (private). The games and the
original source live on the Windows PC, which has its own checkout
(`git clone https://github.com/r0pi0x/LuckerParty`); `scripts\playtest.ps1`
pulls and runs an optimized build. Spec sessions there write into `specs/`
in that checkout and push; implementation sessions pull them.
