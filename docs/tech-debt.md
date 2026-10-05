# Known technical debt

Small, current, honest. Remove entries when fixed.

| Area | Debt | Why it's OK for now |
|---|---|---|
| movement | Placeholder movement has no step-up (stairs block it) and doesn't shrink the collider when crouching | It gets replaced by spec-based movement |
| movement | Ground check is one downward shape cast | Good enough until a spec defines grounding |
| client | Mouse capture fights the inspector (F1 releases it; click recaptures) | Debug-only |
| tests | `no_game_assets_tracked` blocks all .wav/.ogg/.mp3, including our own future sounds | Revisit when we have original audio |
| client | Local input overwrites the local player's `Intent` every frame, so remote tools can't drive it | Scripted intent playback is planned (docs/OBSERVABILITY.md) |
| build | Windows build not yet verified | Needs the Windows PC set up (see plans) |
