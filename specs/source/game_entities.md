# Source engine: player and game entities used by minigame maps (player_speedmod, game_ui, env_fade, game_score, env_hudhint, env_explosion, func_wall_toggle, func_conveyor)

Source basis: Valve's public Source SDK 2013 only (current GitHub release).
I read the server code for player_speedmod and the player's
"lagged movement" scale and button masks, how a player command is built
(forced/disabled buttons, the "at controls" and "frozen" flags) and how the
shared movement code scales its frame time, game_ui, env_fade with the
screen-fade message and the client fade arithmetic, game_score and the
player/team score helpers, env_hudhint and the client key-hint panel,
env_explosion with the temp-entity explosion flags and the generic radius
damage in the shared game rules, func_wall / func_wall_toggle,
func_conveyor (server push, client texture-scroll material proxy) and the
base-velocity handling at the start of a player command. CS:S's own game
rules, player, HUD and movement subclass are not public; CS:S overrides
radius damage (see specs/cs_source/grenades.md) and may override HUD
elements and the movement time scale. Those points are open questions.
Status: draft

## Summary

Minigame maps reach into the player with these entities. player_speedmod
scales how fast the player's own movement simulation runs (everything
about movement, including gravity and friction, as if the player's clock
ran faster or slower) and can take away weapons, HUD and keys. game_ui
turns a player's movement and attack keys into map outputs, which is how
mg_ maps build drivable karts and turrets (with phys_thruster). env_fade
fades the screen to or from a colour. game_score adds kills to a player or
points to a team. env_hudhint shows a key-hint text on the HUD.
env_explosion is a configurable explosion: effect, sound, scorch and
radius damage. func_wall_toggle is a brush that appears and disappears.
func_conveyor is a brush that carries what stands on it and scrolls its
texture.

## Units and conventions

Source units, Z up; angles in degrees (forward(angles) as usual). dt =
0.015 s server tick. Colours 0–255. Button names are the player's command
buttons: forward/back (W/S: +forward/+back), moveleft/moveright (A/D
strafe: +moveleft/+moveright), attack, attack2, jump, duck, use, speed (in
CS:S +speed is "walk"), zoom. "Activator" as in
specs/source/entity_io.md. Flags are Hammer spawnflag values.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| fade_time_unit | 1/512 | s | fade duration and hold are sent as unsigned 16-bit counts of this (truncated) |
| fade_time_max | 65535/512 ≈ 127.998 | s | longer values clamp |
| stayout_refresh | 0.1 | s | a "stay out" fade keeps pushing its end this far ahead |
| explosion_probe_up | 8 | in | ground probe starts this far above the origin |
| explosion_probe_len | 40 | in | and goes this far down (to origin − 32) |
| explosion_lift | 24 | in | effect origin = probe hit + 24 × surface normal |
| explosion_radius_per_mag | 2.5 | in | radius = magnitude × 2.5 unless overridden |
| explosion_remove_delay | 0.3 | s | non-repeatable env_explosion removes itself |
| conveyor_default_speed | 100 | in/s | "speed" 0 becomes this |
| conveyor_scroll_div | 128 | in | texture scrolls speed/128 texture widths per second |

## Behavior

### 1. player_speedmod

Input **ModifySpeed** (float k). Spawnflags: 1 "Suppress weapons", 2
"Suppress HUD", 4 "Suppress jump", 8 "Suppress duck", 16 "Suppress use",
32 "Suppress sprint" (+speed: walk in CS:S), 64 "Suppress attack" (both
attack buttons), 128 "Suppress zoom". No keyvalues or outputs.

Target player: the activator if it is a player. Otherwise, only outside
deathmatch, the single-player local player; in multiplayer games (CS:S is
treated as deathmatch, Open questions) **nothing happens**.

1. If k ≠ 1:
   - flag 1: remember the active weapon as "last weapon", holster it,
     leave the player with no active weapon, hide the view model;
   - turn the flashlight off and disable it (CS:S flashlight: same);
   - disable the buttons of flags 4–128 (they are removed from every
     later command until re-enabled);
   - flag 2: hide the whole HUD.
2. If k = 1: flag 1: if no weapon is active, re-deploy the last weapon;
   re-enable the flashlight; re-enable the flagged buttons; flag 2: show
   the HUD.
3. Movement scale := k (any k, including 1, 0 and negatives).

Effect of the movement scale (lagged movement): for each of the player's
commands, the shared movement code runs with frame time k × dt instead of
dt. Everything in it scales together: acceleration and friction per
command, gravity per command, position change per command, duck and jump
timers. Velocities themselves are not scaled. So with k = 2 the player
moves (and falls) as if two ticks passed per tick: apparent run speed 2 ×
250 = 500 in/s, same jump height but half the air time, twice the gravity
pull per tick; with k = 0.5 everything is half speed; k = 0 freezes the
player in place (no movement, no gravity) while view and weapons still
work. The scale is networked so the client predicts the same.
- Respawn resets the scale to 1 (base player spawn). The disabled buttons
  and the hidden HUD are **not** reset on respawn in the base code (Open
  questions for CS:S).
- The flags applied are the speedmod entity's own: two different speedmods
  can disable different buttons; re-enabling (k = 1) only re-enables that
  entity's buttons.

### 2. game_ui

Keyvalue "FieldOfView" (float, Hammer default −1). Spawnflags: 32 "Freeze
Player", 64 "Hide Weapon", 128 "+Use Deactivates", 256 "Jump Deactivates".
Inputs **Activate** (string, optional player name), **Deactivate**.
Outputs (activator = the player, caller = the game_ui): **PlayerOn**,
**PlayerOff**, **PressedMoveLeft**, **PressedMoveRight**,
**PressedForward**, **PressedBack**, **PressedAttack**, **PressedAttack2**,
**UnpressedMoveLeft**, **UnpressedMoveRight**, **UnpressedForward**,
**UnpressedBack**, **UnpressedAttack**, **UnpressedAttack2**, and the
value outputs **XAxis**, **YAxis**, **AttackAxis**, **Attack2Axis**
(their value is passed as the parameter of connected inputs).

**2.1 Activate:**
1. Player = the entity named by the parameter (activator-relative names
   allowed) if given, else the activator. Not a player: warning, nothing.
2. If the game_ui already has a **different** player: ignored silently
   (one driver at a time). Same player again: continues (PlayerOn fires
   again).
3. Remember the player, fire **PlayerOn**, think every tick from now.
4. Flag 32: put the player "at controls": its forward/side/up movement
   inputs are zeroed in every command (it cannot walk; buttons, view,
   jump, duck and weapon keys are not blocked by this).
5. Flag 64: hide the weapon-selection HUD; if a weapon is active: remember
   it, holster it, clear the active weapon, hide the view model.
6. Mark "force update".

**2.2 Each tick while active:**
1. No player (disconnected): stop thinking (no PlayerOff).
2. On a force update, last buttons := the player's current buttons (keys
   already held at activation do not produce Pressed outputs).
3. FieldOfView > −1: if dot(player eye forward, normalize(game_ui origin
   − player centre)) < FieldOfView: Deactivate and stop. (The game_ui
   entity's position matters: the player must keep facing it within
   acos(FieldOfView).)
4. Set the player's "on train" flag (Open questions: the base player
   clears it again before each command's movement, so it probably has no
   effect).
5. If the player pressed use this command and flag 128, or pressed jump
   and flag 256: Deactivate and stop.
6. Changed buttons (current XOR last), checked in this order: moveright,
   moveleft, forward, back, attack, attack2. For each changed one: was
   held → **Unpressed...**, else **Pressed...**.
7. Last buttons := current.
8. Axes: x = 1 if moveright held, else −1 if moveleft held, else 0
   (right wins); y = 1 forward, else −1 back, else 0 (forward wins);
   attack = 1/0; attack2 = 1/0. Each axis output fires when its value
   differs from the last value it fired, or on a force update (all four
   fire on the first tick after activation, even with 0).
9. Clear "force update".

**2.3 Deactivate** (input, FOV, use/jump):
1. No player: warning, stop thinking, nothing else.
2. Flag 32: remove "at controls". Flag 64: show the weapon-selection HUD,
   switch back to the remembered weapon, deploy the active weapon.
3. Fire **PlayerOff**, then set all four axes to 0, which **fires XAxis,
   YAxis, AttackAxis, Attack2Axis with 0** (always, even if already 0).
4. Forget the player, stop thinking.

The buttons game_ui reads are the player's processed buttons for its last
command (after player_speedmod's disabled buttons are removed), so a
speedmod that suppresses attack also hides PressedAttack from a game_ui.
A frozen player (point_viewcontrol flag 4) has all buttons zeroed.

### 3. env_fade

Keyvalues: "duration" (s), "holdtime" (s), "renderamt" (alpha 0–255),
"rendercolor". Spawnflags: 1 "Fade From" (fade in), 2 "Modulate", 4
"Triggering player only", 8 "Stay Out". Input **Fade**. Output
**OnBeginFade** (activator = the input's activator).

- Flag 4: send the fade to the activator only (if it is a connected
  player; no purge). Else: send to every player, **with "purge"** (the
  new fade replaces all fades they had).
- Message: duration D and hold H truncated to 1/512 s, colour, alpha a,
  flags (out unless flag 1; modulate; stay out; purge).

Client fade list, per frame at time t (a fade received at t₀):
- Fade **out** (D > 0): ramps 0 → a over [t₀, t₀ + D], holds a until
  t₀ + D + H, then disappears.
- Fade **in** (flag 1, D > 0): holds a during [t₀, t₀ + H], ramps a → 0
  over [t₀ + H, t₀ + H + D], then disappears.

```math
\alpha_{out}(t) = \operatorname{clamp}\!\left(a\left(1 - \frac{t_0 + D - t}{D}\right), 0, a\right), \qquad
\alpha_{in}(t) = \operatorname{clamp}\!\left(a\,\frac{t_0 + H + D - t}{D}, 0, a\right)
```

  truncated to an integer.
- D = 0: the fade's end and hold times are not offset by now, so it is
  removed at once (no visible effect) **unless** stay out is set, in which
  case a fade out shows alpha a immediately and forever, and a fade in
  shows 0.
- **Stay out**: the fade never expires (until purged by another purging
  fade or cleared by respawn); a stay-out fade out remains at alpha a.
- Several fades at once: colour = per-channel integer average of all
  active fades' colours; alpha = the largest alpha.
- Drawing: a full-screen quad over the 3D view, blended with alpha
  (colour·α + scene·(1 − α)); "modulate" multiplies the scene by the fade
  colour instead (exact modulate formula: Open questions).
- Player spawn clears all fades (base player).

### 4. game_score

Keyvalues: "points" (integer), "master" (optional master entity).
Spawnflags: 1 "Allow Negative", 2 "Team Points". Input **ApplyScore**
(also reacts to Use). No outputs.

ApplyScore with activator A:
1. No activator: nothing. "master" set and that master is a master-type
   entity that is not triggered for A: nothing (a missing or non-master
   entity counts as triggered).
2. A not a player: nothing.
3. Flag 2: A's team score += points (no negative check, team score can go
   below 0). Else A's frag count ("score" on the scoreboard) changes by
   points: positive always added; negative without flag 1: if frags < 0
   already, nothing; else frags = max(frags + points, 0).

### 5. env_hudhint

Keyvalue "message" (text or a "#localisation" token). Spawnflag 1 "All
Players". Inputs **ShowHudHint**, **HideHudHint**.

- All Players: a reliable key-hint message to everyone. Otherwise to the
  activator if it is a player (else the single-player local player:
  nobody in multiplayer).
- Show sends the message text; Hide sends an empty text.
- Client: an empty text hides the key-hint panel (with its hide
  animation). Otherwise the text is localised if it is a token the client
  knows; an unknown "#token" is treated as empty (hides); plain text is
  shown as is. Inside the text, "%name%" is replaced by the key bound to
  command "name" (e.g. "%+jump%" → SPACE), drawn in a larger font. Then
  the panel is shown (show animation). Position, font and whether it hides
  by itself after a time are CS:S HUD layout (Open questions).

### 6. env_explosion

Keyvalues: "iMagnitude" (damage, integer), "iRadiusOverride" (integer, 0 =
none), "fireballsprite" (sprite, default `sprites/zerogxplode.vmt`),
"rendermode" (5 additive default; 4 → alpha-blended sprite; others →
opaque), "DamageForce" (float), "ignoredEntity". Spawnflags: 1 "No
Damage", 2 "Repeatable", 4 "No Fireball", 8 "No Smoke" (unused), 16 "No
Decal", 32 "No Sparks", 64 "No Sound", 128 "Random Orientation", 256 "No
Fireball Smoke", 512 "No Particles", 1024 "No DLights", 2048 "Don't
clamp Min", 4096 "Don't clamp Max", 8192 "Damage above surface only",
16384 "Generic damage". Input **Explode**.

Spawn: sprite scale s = (magnitude − 50) × 0.6, clamped to [10, 50]
(min 1 with flag 2048, max 200 with flag 4096), truncated to an integer.

Explode:
1. Probe: trace from origin + (0, 0, 8) to origin − (0, 0, 32) against
   world brushes and water. If it hits: effect origin = hit + 24 × hit
   normal; else the origin.
2. Unless flag 16: the "Scorch" decal on the probe's hit surface (none if
   it hit nothing).
3. Temp-entity explosion at the effect origin to everyone in hearing
   range: sprite, scale s/10 (0 with flag 4), frame rate 15, radius,
   magnitude, flags from spawnflags (4 → no fireball, 64 → no sound, 128
   → rotate, 256 → no fireball smoke, 512 → no particles, 1024 → no
   dlights, render mode as above). The client effect is the one in
   specs/cs_source/grenades.md 5.4: "no fireball" removes the flash and
   the whole core (smoke, embers, fireballs), "no fireball smoke" removes
   the core's smoke puffs only, "no particles" removes the debris
   streaks and flecks, "no sound" the client explosion sound. In water:
   the water-explosion effect instead.
4. Unless flag 1: radius damage centred on the entity's **origin** (not
   the lifted effect origin), damage = magnitude, radius R = override if
   > 0 else int(magnitude × 2.5), type blast (generic with flag 16384;
   flag 8192 adds "surface only": players fully under water are not hurt),
   inflictor = the explosion, attacker = its owner or itself, skipping
   "ignoredEntity". The game rules' radius damage applies:
   specs/cs_source/grenades.md 5.3 (CS:S formula: open question there).
   If DamageForce ≠ 0, each victim's push = DamageForce × (magnitude/R)
   along the direction from the blast (the same for every distance), else
   the standard explosive force.
5. If the origin is in water and flag 32 is clear: 0–3 "spark_shower"
   entities at the effect origin (sparks only **under water**: inverted
   check, quirk).
6. 0.3 s later: removed, unless flag 2 (Repeatable).

### 7. func_wall_toggle

A func_wall (static solid brush, blocks everything, drawn) with one
spawnflag, 1 "Starts Invisible", and input **Toggle** (also reacts to
Use). Off = not solid, not drawn, no physics collisions; on = the reverse.
Toggle flips between them; there are no separate on/off inputs. It can be
moved by parenting like any brush entity.

### 8. func_conveyor

Keyvalues: "movedir" (angles; push direction = forward(movedir)), "speed"
(in/s; 0 → 100). Spawnflags: 1 "No push" (visual only), 2 "Not solid".
Inputs **ToggleDirection** (speed := −speed; also Use), **SetSpeed**
(float). Otherwise a func_wall.

**Push.** At the start of each player command (before movement, and
before the leftover base-velocity conversion of the trigger_push
section of specs/source/triggers.md): if the player is on the
ground and its ground entity is a conveyor without flag 1: base velocity
:= forward(movedir) × speed, added to an existing base velocity if a
push trigger set one during the previous command; marked "set this
command". From there the base-velocity rules of specs/source/triggers.md
(trigger_push) apply unchanged: the horizontal part moves the player
without accumulating while it stands on the belt; stepping or jumping off
turns the leftover base velocity into real velocity × (1 + dt/2) on the
first command where it is not refreshed. Other walking/stepping entities
whose ground entity is the conveyor get the same base velocity.

**Texture scroll.** Materials with the "ConveyorScroll" proxy on a
func_conveyor scroll their texture transform along the texture's s axis
(t offset 0), at |speed|/128 texture widths per second:
s_offset = frac(−t·|speed|/128) for speed ≥ 0, frac(+t·|speed|/128) for
speed < 0 (t = client time; frac maps into [0, 1)). The direction on
screen therefore follows the texture alignment, not movedir (mappers align
the texture). Materials without the proxy do not scroll. The speed is
networked, so SetSpeed and ToggleDirection change the scroll.

## Per-tick order

Player command (per player): (1) forced/disabled buttons applied
(player_speedmod), frozen/at-controls zeroing; (2) conveyor base velocity
and leftover base-velocity conversion; (3) movement with frame time k·dt;
(4) re-link to triggers (triggers.md). Then entity thinks: game_ui reads
the buttons of the last processed command and fires outputs. Then the
event queue (Fade, ApplyScore, ShowHudHint, Explode, Toggle, SetSpeed,
ModifySpeed take effect; Explode deals its damage immediately inside the
input). env_explosion removal 0.3 s later is a think.

## Edge cases

- game_ui driven by a player who dies: it keeps thinking and reading the
  dead player's buttons until Deactivate; there is no automatic
  PlayerOff on death (maps usually Deactivate from a death trigger).
- game_ui FieldOfView −1 (or anything ≤ −1): never deactivates by view.
  FieldOfView 0: deactivates when the game_ui is more than 90° off the
  player's view.
- player_speedmod with the activator a non-player (logic_auto): nothing in
  CS:S.
- env_fade with renderamt 0: nothing visible.
- env_explosion iMagnitude 0: no damage (damage ≤ 0 everywhere), radius 0
  → falloff 1 per unit; scale clamps to 10.
- env_explosion inside a solid: the probe starts in solid; follow the
  trace result (effect origin from the hit, likely the start).
- game_score with points 0: no change.
- func_conveyor with "No push": scrolls only.

## Quirks

- **player_speedmod scales the whole movement clock** (keep: kz_/mg_ maps
  depend on the exact feel, including faster falling).
- **Disabled buttons outlive respawn** in the base code (keep unless
  CS:S differs).
- **game_ui fires the axes with 0 on every Deactivate** (keep: maps stop
  their thrusters with it).
- **game_ui ignores a second player** while one is active (keep).
- **env_explosion sparks only under water** (keep; invisible anyway in
  most maps).
- **DamageForce does not fall off with distance** (keep).
- **env_fade without flag 4 purges every other fade** (keep).

## Test cases

dt = 0.015.

| Setup | Input | After | Expected |
|---|---|---|---|
| player running at 250 in/s on flat ground | ModifySpeed 2 (activator player) | 1 tick | moves 7.5 units per tick (apparent 500 in/s); velocity stays 250 |
| same | ModifySpeed 0.5 | 1 tick | moves 1.875 units per tick |
| CS:S jump (impulse ≈ 268 in/s, gravity 800), k = 2 | jump | — | apex height ≈ 268²/1600 ≈ 44.9 (unchanged), airtime ≈ 0.67/2 = 0.335 s |
| k = 0 | hold forward | — | no movement, no falling |
| speedmod flags 4 (no jump) | ModifySpeed 0.9, then jump | — | no jump; ModifySpeed 1 re-enables |
| speedmod fired by logic_auto | ModifySpeed 2 | — | no player affected |
| player respawns after ModifySpeed 2 | spawn | — | scale 1 |
| game_ui flags 32, player P | Activate (activator P) | first tick | PlayerOn; XAxis 0, YAxis 0, AttackAxis 0, Attack2Axis 0 fired once each |
| active | P presses W | that tick | PressedForward; YAxis 1 |
| active | P holds W and S | — | YAxis 1 (forward wins) |
| active | P releases W | — | UnpressedForward; YAxis 0 |
| active | P presses A then D held together | — | XAxis −1 then 1 (right wins while both held) |
| active, P walks (W held) | — | — | P does not move (at controls) but PressedForward/YAxis fire |
| active, flag 256 | P presses jump | — | PlayerOff, then XAxis/YAxis/AttackAxis/Attack2Axis each fire 0 |
| active for P | Activate from player Q | — | ignored |
| FieldOfView 0.5, game_ui 60° off P's view | tick | — | dot 0.5 ≥ 0.5 → stays (strictly less deactivates) |
| same, 61° off | tick | — | Deactivate |
| env_fade duration 2, hold 1, black, alpha 255 | Fade at t₀ | t₀+1 | alpha 127 |
| same | — | t₀+2.5 | 255 |
| same | — | t₀+3.01 | gone |
| flag 1 (fade from), duration 2, hold 1 | Fade | t₀+0.5 / t₀+2 / t₀+3.01 | 255 / 127 / gone |
| flags 8 (stay out), duration 1 | Fade | t₀+100 | 255 |
| duration 0, flag 8 | Fade | immediately | 255 forever |
| duration 0, no flag 8 | Fade | — | nothing |
| duration 0.3 | message | — | sent as 153/512 = 0.29883 s |
| two fades (255,0,0,a 100) and (0,0,255,a 200) active | — | — | colour (127, 0, 127), alpha 200 |
| game_score points 1, player frags 5 | ApplyScore (activator player) | — | frags 6 |
| points −10, frags 5, no flag 1 | ApplyScore | — | frags 0 |
| same, frags −3 | ApplyScore | — | frags −3 (unchanged) |
| points −10, flag 1, frags 5 | ApplyScore | — | frags −5 |
| points 3, flag 2, player on CT | ApplyScore | — | CT team score + 3 |
| env_hudhint "Press %+use% to drive", Use bound to E | ShowHudHint (activator P) | — | P's hint panel shows "Press E to drive" (E in large font) |
| env_hudhint "#Unknown_Token" | ShowHudHint | — | panel hidden |
| env_explosion iMagnitude 100, standing 1 unit above floor | Explode | — | effect origin = floor hit + 24 up; scorch on floor; R = 250; sprite scale 30 → 3.0 |
| same, player 100 units away, clear line (generic rule) | Explode | — | damage 100 − 100·100/250 = 60 before armour (CS:S rule per grenades.md) |
| iMagnitude 300 | spawn | — | scale (250·0.6 = 150) → clamp 50; with flag 4096 → 150 |
| iMagnitude 60 | spawn | — | scale 6 → clamp 10; with flag 2048 → 6 |
| DamageForce 1000, magnitude 100, R 250 | Explode | — | each victim's push 400 along blast → victim |
| not repeatable | Explode | 0.3 s | entity removed (second Explode does nothing) |
| flag 2 | Explode twice | — | two explosions |
| func_wall_toggle flag 1 | spawn | — | invisible, not solid |
| same | Toggle | — | visible, solid |
| func_conveyor movedir "0 90 0", speed 100, player standing still on it | 1 tick | — | player moves +1.5 Y; velocity 0 |
| same | player jumps off | first command not on it | velocity.y += 100.75 (100 × 1.0075) |
| func_conveyor speed 64, ConveyorScroll material | t = 1 s | — | s offset = frac(−0.5) = 0.5 |
| SetSpeed −64 | t = 1 s | — | s offset = 0.5 (moving the other way: 0.25 at t = 0.5 vs 0.75) |
| ToggleDirection on speed 100 | — | — | push −100 along movedir; scroll reversed |
| func_conveyor flag 1 | player stands | — | no push; texture scrolls |

## Open questions

1. **CS:S and lagged movement**: does CS:S's movement subclass keep the
   frame-time scaling (it is in the shared base routine, which the
   subclass is expected to call)? Probe: player_speedmod 2 on a test map;
   measure displacement per tick and jump airtime with the movement probe.
2. **CS:S "deathmatch" flag**: CS:S rules are multiplayer; confirm that a
   player_speedmod fired without a player activator affects nobody.
3. **Disabled buttons and HUD on respawn in CS:S**: fire a speedmod with
   flags 4 and 2 at value 0.5, die, respawn: can the player jump, is the
   HUD back?
4. **game_ui "on train" flag**: does it block player movement in CS:S at
   all (the base player clears it before each command)? Activate a
   game_ui without flag 32 and try to walk.
5. **Key-hint panel in CS:S**: position, font, and does it hide on its
   own after some seconds? Screenshot after ShowHudHint and wait 30 s.
6. **Modulate fade formula**: screenshot env_fade with flag 2 (modulate),
   colour (128, 128, 128) alpha 255 over a known scene.
7. **CS:S radius damage for env_explosion**: does CS:S's override treat
   env_explosion damage like HE (armour, player height sampling,
   friendly fire)? Probe: env_explosion magnitude 100 at 100 units from a
   bot with and without kevlar.
8. **Team score**: does CS:S's team score (rounds won on the scoreboard)
   change with game_score flag 2, and does it persist across rounds?
9. **Conveyor on physics props**: do physics props on a func_conveyor
   move (they have no ground entity in this code)? Drop a prop on a
   conveyor on a test map.
10. **Fade on round restart**: does CS:S clear stay-out fades at round
    start or only at respawn?
