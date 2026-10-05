# Sound

Started 2026-10-05, from specs/cs_source/sounds.md.

## Design

- **Map layer (any game):** `map/sound.rs`. The map carries sound entries
  (waves, volume/pitch/level intervals, channel) and decoded clips (16-bit
  PCM). Game code writes `PlaySound` messages (entry, position, optional
  volume override, source entity and channel). Playback resolves the
  entry (random wave, drawn pitch/volume/level), applies the distance
  model and left/right panning toward the listener (the active camera),
  and plays through Bevy audio (pitch as playback speed). A new sound on
  the same (entity, channel) replaces the old one.
- **Distance model:** one function. Until measured (spec open question
  1): sound levels use H1 (inverse distance from a 36-unit reference),
  CompatibilityAttenuation entries H2 (linear to 1000/a units), level 0
  everywhere.
- **CS:S layer:** `games/cs_source/wav.rs` (RIFF: PCM 8/16-bit, MS-ADPCM,
  loop points from `cue `/`smpl`), `games/cs_source/sound.rs` (the
  game_sounds manifest and scripts, the per-map level sounds file,
  precaching the entries a map uses).
- **Surfaces:** world meshes carry their material's `$surfaceprop`; a
  grid of world triangles answers "surface under this point"; surface
  properties give step sounds and the game material. Props carry their
  own (`.phy` surfaceprop, else the model's `$surfaceprop`) on their
  brushes and colliders, and the ground trace reports it, so standing on
  a prop uses the prop's steps. Names the scripts don't define step as
  "default".

## Slices

1. [x] Pipeline: decoder, scripts, playback, distance and panning.
2. [x] Footsteps (ground, ladder, wading, shallow water; bands, timers,
   left/right, duck volume), jump, landing tiers, wall slam, water
   entry/exit and swim strokes. CS:S's walk/duck silence assumed (walk
   key, duck key or ducked: silent) until measured. Not yet: the
   wading counter quirk shared by all players.
3. [ ] Physics impacts and scrapes (avian contact speeds).
4. [ ] Map ambience: ambient_generic, soundscapes (selection, crossfade,
   random sounds).
5. [x] Weapon sounds: fire, empty click, deploy, knife swings, reload parts at their animation times (sounds aren't yet tied to view-model events).

Measurements to take (probe server): the distance curves, CS:S footstep
rules (walking and crouching silent?), the jump sound, wave choice.
