# Sound

Started 2026-10-05, from specs/cs_source/sounds.md.

## Design

- **Map layer (any game):** `map/sound.rs`. The map carries sound entries
  (waves, volume/pitch/level intervals, channel) and decoded clips (16-bit
  PCM). Game code writes `PlaySound` messages (entry, position, optional
  volume override, source entity and channel). Playback resolves the
  entry (random wave, drawn pitch/volume/level), applies the distance
  model and left/right panning toward the listener (the active camera),
  and plays (the listener's own sounds, made by the player the camera
  belongs to, play centred as in Source; sounds within 16 units of the
  ear pan less: `sound::Ear`) through Bevy audio (pitch as playback speed). A new sound on
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
3. [x] Bullet impacts (hit surface's bulletimpact, pellet grouping) and
   physics impacts (avian's pre-solve approach speed, soft/hard choice,
   per-frame queue and merging), `games/cs_source/impacts.rs`. Scrapes
   (`games/cs_source/scrapes.rs`): the spec's rule (thresholds, rough or
   smooth, slots, 0.1 s ramps and timeout, the server's 0.5 s update)
   on a stand-in for Source's friction energy (spec open question 8):
   the friction work of one 0.015 s tick at the hardest-sliding contact,
   friction × normal force (avian's normal impulse / dt) × sliding speed,
   in kg·in²/s²; one loop per sliding body (props, pieces, ragdolls),
   following it. Breakables (specs/source/breakables.md): damage and
   break sounds draw the spec's volume and pitch; brush gibs bounce with
   their material's `Bounce.*` entry by the shared temporary-entity rule
   (1 in 6, volume by vertical speed, pitch drawn 1 in 4).
4. [x] Soundscapes: scripts flattened (nesting, volumes, position
   overrides), trigger_soundscape zones (most recent wins) and
   env_soundscape points, 3 s loop crossfades with reuse, random
   one-shots (fixed, positioned or random positions). env_soundscape
   selection follows the spec: in range, potentially visible from the
   ear's cluster and a clear line through the BSP's solid leaves; the
   current one stays until another qualifies (closer, or the current one
   out of range or sight). Loops are `live_sound` sounds (intro, then the
   loop point; live panning). Room DSP (`map/room.rs`): the soundscape's
   top-level "dsp" picks a preset from a 29-row table of our parameters
   (Source's are engine-side), played by one shared reverb bus that every
   clip sends into (more with distance; `#` waves and interface sounds
   dry); `dsp_off`, `dsp_volume`; `snd_show 1` shows the soundscape and
   preset. "dsp_volume" overrides the user's room level while its
   soundscape plays; "dsp_player" and "soundmixer" are read and shown,
   not played (their effects are engine-side). env_soundscape_proxy
   plays its main env_soundscape's soundscape and positions from its own
   place; Enable/Disable/ToggleEnabled go through the logic
   (`map::SoundscapeSwitches`): a disabled current one gives way to the
   next that qualifies; OnPlay fires when one becomes current. Not yet:
   the trace budget (not needed: one listener).
6. [x] ambient_generic (`logic/ambient.rs` on `map/live_sound.rs`):
   the logic keeps the entity's state (spawn keys, inputs, ramps and LFO
   at 5 Hz) and asks for long-lived sounds by entity (`Effect::Ambient*`
   -> `map::SoundControl`); the map layer plays them with live gains and
   pitch, looping from the wave's loop point after its intro, following
   the source entity's mover or prop node. Raw waves are loaded with the
   map as entries named by their path. Spec choices: script entries keep
   their own level (open question 11), input starts use the entry's
   volume/pitch (quirk kept), FadeIn/FadeOut 0 s fade in one step, LFO
   adds to pitch/volume each step as written. Not yet: presets (no stock
   map uses them), "!" sentences, the vo .wav -> .mp3 switch, what plays
   from a stock map trigger we don't have (prop OnHealthChanged, bomb
   explosion, env_fire).
7. [x] Damage and death (`games/cs_source/pain.rs`; the install's
   game_sounds.txt, the only manifest file defining them: CS:S's manifest
   leaves out hl2's game_sounds_player.txt). Entries found and used:
   `Player.DamageKevlar` (CHAN_BODY, kevlar1-5), `Player.DamageHelmet`
   (CHAN_BODY, bhit_helmet-1), `Player.DamageHeadShot` and
   `Player.DeathHeadShot` (CHAN_VOICE, headshot1-2), `Player.Death`
   (CHAN_VOICE, death1-6), `Player.FallDamage` (CHAN_BODY, damage1-3),
   all CompatibilityAttenuation 1.0; `Flesh.BulletImpact`
   (game_sounds_physics.txt, the `flesh` surface's bulletimpact) stays the
   shot's own client-side impact; shots on players now take the
   hitboxes' `flesh` (impact_effects.md 3), not the movement box's
   `player`, which has no bulletimpact (they were silent). Defined, unused: `Player.FallGib`,
   `Player.PlasmaDamage`/`SonicDamage` (null waves), `Player.DrownStart`/
   `DrownContinue` (no drowning yet). Rules (tech-debt.md): helmet ->
   DamageHelmet, bare head -> DamageHeadShot unless it kills, covered body
   -> DamageKevlar, fall damage taken -> FallDamage, death -> Death or
   DeathHeadShot; from the victim, positional for others and centred for
   the victim; hurt sounds are the server's `GameSound` (every client
   once), deaths play from `core::Died` on each side. Reference checks for
   the coordinator (CS:S, `sv_soundemitter_trace 1` on a listen server,
   bots as victims): (a) body shot with and without kevlar, (b) headshot
   with and without helmet, non-lethal and lethal (does a killing
   headshot play DamageHeadShot, DeathHeadShot or both; does a death by
   a headshot also play `Player.Death`), (c) a leg shot in kevlar, (d)
   knife and HE hits on kevlar, (e) a fall that hurts and one that kills,
   (f) whether the shooter's own client gets the hurt sound (trace on the
   shooter's client vs a third client), (g) a shotgun's pellets on one
   armoured player: one sound or several.
5. [x] Weapon sounds: fire, empty click, deploy, knife swings, reload parts at their animation times (sounds aren't yet tied to view-model events).

Measurements to take (probe server): the distance curves, CS:S footstep
rules (walking and crouching silent?), the jump sound, wave choice.
