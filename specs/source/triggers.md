# Source engine: brush triggers

Source basis: Valve's public Source SDK 2013 only. I read the shared server
trigger code (the base trigger, trigger_multiple, trigger_once,
trigger_hurt, trigger_push, trigger_teleport, info_teleport_destination,
trigger_gravity, trigger_remove), the touch-link bookkeeping (start touch,
touch, end touch, the untouch pass), how a player's movement command
re-links the player against triggers, the base-velocity handling in the
shared player movement and in the command runner, entity teleporting, the
generic damage path (fractional damage accumulation) and the damage-force
helpers. The engine's trigger enumeration (which triggers a box overlaps)
and CS:S's own player damage code are not in the SDK; those parts are
marked *inferred* or listed under Open questions. Real examples come from
the entity lumps of the user's CS:S install (de_port trigger_hurt,
de_prodigy and de_inferno trigger_multiple/trigger_once, cs_office
trigger_once).
Status: draft

## Summary

A trigger is an invisible brush volume (non-solid, flagged as a trigger)
that notices entities overlapping it. Each overlap gets a **start touch**
once, a **touch** every time the entity is re-linked while still inside
(for a player: every movement command, i.e. every tick), and an **end
touch** when it is no longer inside. Spawnflags pick which kinds of
entities count; an optional filter entity narrows it further. On top of
that base, trigger_multiple/once fire OnTrigger with a wait, trigger_hurt
damages everything inside every 0.5 s, trigger_push feeds a "base
velocity" into the player's movement every tick, trigger_teleport moves the
toucher to a named point keeping its velocity, and trigger_gravity sets the
toucher's gravity multiplier. Touch detection is a static box test at the
end of each move, so a fast player can skip a thin trigger.

## Units and conventions

Source units (inches), Z up, angles (pitch, yaw, roll) in degrees with
positive pitch looking down. Velocities in units/s. dt = 0.015 s (CS:S
66.67 tick). sv_gravity = 800 units/s². Player box in CS:S: (−16,−16,0) to
(16,16,62) standing, top 45 ducked (measured, specs/cs_source/movement.md).
Spawnflag values are what Hammer shows.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| multiple_default_wait | 0.2 | s | trigger_multiple "wait" 0 becomes this |
| once_remove_delay | 0.1 | s | trigger_once deletes itself this long after firing (7 ticks) |
| hurt_interval | 0.5 | s | trigger_hurt damage pass period (33 ticks) |
| hurt_per_pass | damage × 0.5 | hp | damage applied per pass ("damage" is per second) |
| hurt_forgive | 3.0 | s | doubling model resets after this long with no victims |
| radiation_think | 0.25 | s | radiation damage-type trigger re-check period |
| push_default_speed | 100 | units/s | trigger_push "speed" 0 becomes this |
| push_lift | 1 | units | grounded entity pushed upward is raised this much |
| exit_momentum | 1 + dt/2 | — | scale applied to leftover base velocity when it is turned into real velocity |

## Behavior

### Which entities can touch a trigger (spawnflags and filter)

A toucher passes the trigger's filters when **all** of these hold:

1. Class flags (any one suffices):
   - 64 "Everything (not including physics debris)": any entity;
   - 1 "Clients": players;
   - 2 "NPCs": NPCs;
   - 4 "Pushables": func_pushable;
   - 8 "Physics Objects": anything that moves with physics (props, dropped
     weapons in CS:S, gibs that are physics props);
   - 1024 "Physics debris": entities in the debris collision groups (this
     flag only works in episodic/TF builds; treat as unsupported in CS:S).
2. Player extras: the player must be **alive**; flag 32 "Only clients in
   vehicles" requires a vehicle (always fails in CS:S); 512 "Only clients
   *not* in vehicles" requires none (always passes in CS:S). At spawn, flags
   32 or 512 also switch on flag 1. Flag 4096 "Disallow bots" exists only
   in newer builds (Open questions).
3. NPC extras (16 "Only player ally NPCs", 2048 "Only NPCs in vehicles"):
   irrelevant for us, NPCs are out of scope.
4. If the keyvalue "filtername" names a filter entity, that filter must
   pass the toucher (specs/source/entity_io.md, Filters). The name is
   resolved once at activation (first entity with that name that is a
   filter).

Dead players never pass, so a trigger_hurt does not keep damaging corpses
and OnStartTouch does not fire for dead bodies.

### How overlap is detected

- A trigger is a brush model, made non-solid with a "trigger" flag; it
  blocks nothing. Entities with no solidity do not touch triggers. A
  trigger does not touch other triggers.
- Re-linking: whenever an entity's position is set by its own movement it
  is re-tested against triggers. For a player this happens **after each
  movement command** (once per tick), at the final position of the move,
  with the player's current box (standing or ducked). The test is the
  player's axis-aligned box against the trigger's **actual brush shape**
  (not just the brush's bounding box): *inferred* from the engine's
  "accurate trigger box checks" switch being on and the game's own
  box-against-brush helper. There is **no sweep** for players: the box at
  the end position must overlap; a player crossing a trigger thinner than
  the distance moved in one tick (speed × 0.015) can skip it. Entities
  pushed by movers are re-tested with their previous position given
  (possibly swept; Open questions).
- Each re-link marks every overlapped trigger as touched "now":
  - first time (no existing link): the trigger's **start touch** runs, then
    its **touch** runs, in that order, in the same call;
  - existing link: only **touch** runs.
  The same happens in the other direction (the player is told it touches
  the trigger).
- **End touch**: once per frame, after all entities have simulated, every
  entity that re-linked this frame drops links that were not refreshed;
  the trigger's end touch runs for each (players also drop stale links
  when they re-link). So OnEndTouch fires in the frame the player stopped
  overlapping, before that frame's event queue.
- Enabling a trigger (input Enable, or Toggle to on) re-tests it against
  everything overlapping it at once, so entities already inside get start
  touch immediately. Disabling (Disable, or Toggle to off) removes its
  trigger flag: entities inside are no longer refreshed and get end touch
  the next time they re-link (a moving player: next tick; an entity at
  rest: possibly never). DisableAndEndTouch ends every current touch
  immediately, then disables.

### Base trigger (all classes below)

Keyvalues: "StartDisabled" (0/1), "filtername". Spawnflags as above.

Inputs: **Enable**, **Disable**, **Toggle** (flips the trigger flag; does
*not* change the stored enabled state that other code reads),
**DisableAndEndTouch**, **TouchTest** (fires OnTouching if anything is in
the touching list, else OnNotTouching; nothing if disabled), **StartTouch**
/ **EndTouch** (run start/end touch with the *caller* as toucher).

Outputs (activator = toucher, caller = trigger):
- **OnStartTouch**: every start touch by an entity that passes the filters.
  The trigger keeps a list of touching entities; OnStartTouch fires even if
  the entity is already in the list (duplicate start touches can happen
  after teleports and re-enables).
- **OnStartTouchAll**: when the list goes from empty to one entry.
- **OnEndTouch**: on end touch of an entity that is in the list (filters
  are **not** re-checked; an entity that was in the list always gets it).
- **OnEndTouchAll**: when the list becomes empty after an end touch. Before
  deciding, stale entries are pruned: deleted entities and dead players are
  removed and do not count as "still touching". So the last living player
  leaving fires OnEndTouchAll even if a corpse is "inside".
- **OnTouching** / **OnNotTouching**: only from TouchTest.

### trigger_multiple

Keyvalue "wait" (s): 0 becomes 0.2; negative means "fire once". Output
**OnTrigger** (plus the base outputs).

On every **touch** call (not start touch) by an entity passing the filters:
1. If the trigger is still waiting (its next think is later than now), do
   nothing.
2. Fire OnTrigger (activator = toucher).
3. If wait > 0: schedule the end of waiting at round_half_up((now +
   wait)/dt) ticks (nearest tick). If wait ≤ 0: stop reacting to touches,
   and delete the trigger 0.1 s later (7 ticks).

So a player **standing inside** a trigger_multiple fires OnTrigger again
every wait seconds (wait 1 → every 67 ticks), while OnStartTouch fires only
once on entry. When the wait ends at tick T, a touch processed in tick T
fires again (the check is "next think strictly later than now").

### trigger_once

A trigger_multiple whose wait is forced to −1: OnTrigger fires on the first
touch, then it stops reacting and is deleted 7 ticks later. Its base
outputs still work until deletion: a second player entering in those 7
ticks still fires **OnStartTouch** (not OnTrigger). cs_office guards
against that with timestofire 1 on its OnStartTouch connections.

### trigger_hurt

Keyvalues: "damage" (per second; negative heals), "damagecap",
"damagetype" (bitfield), "damagemodel" (0 normal, 1 doubling),
"nodmgforce" (0/1). Input-set field **SetDamage** (float). Outputs
**OnHurt** (non-player victims) and **OnHurtPlayer** (players), activator =
victim, fired once per victim per pass.

Damage passes:
- A touch while the trigger has no pass scheduled starts the pass cycle
  with the first pass scheduled for "now" (this tick if the trigger has not
  thought yet this frame, else next tick).
- Each pass: per = damage × dt_pass where dt_pass = 0.5 (fixed, even though
  the real spacing is 33 ticks = 0.495 s). For every entity currently in
  the touch links: if it can take damage, passes the filters and (for
  players) is still connected:
  - per < 0: heal by −per (capped at max health by the heal code);
  - else apply per damage of type "damagetype", attacker = inflictor = the
    trigger, damage position = the point of the victim's box nearest to the
    trigger's centre; damage force: none if nodmgforce, else the generic
    "guessed" force along (damage position − trigger centre): bullet type →
    bullet force, blast type → blast force (75·4·damage, max 75·400,
    ×U(0.85,1.15)), anything else → 75·4·damage (all × phys_pushscale 1);
    whether CS:S turns that force into player velocity is an open question;
  - fire OnHurtPlayer or OnHurt; remember the victim as "hurt this pass".
- If at least one victim was hurt, schedule the next pass 0.5 s later
  (nearest tick: 33 ticks). If nobody was hurt, the cycle stops until the
  next touch restarts it (the next touch can be the very next tick, so the
  cadence can restart off the 33-tick grid).
- Doubling model (damagemodel 1), after each pass: if someone was hurt,
  damage = min(damage × 2, damagecap) and the forgive deadline = now + 3 s;
  if nobody was hurt and now > deadline, damage returns to its original
  value. With damagecap 0 the first doubling makes damage 0 (mapper error;
  keep).
- **Leaving**: when an entity that passes the filters ends touch and was
  **not** hurt in the most recent pass, it takes one half-strength hit,
  damage × 0.5, right away (so a quick dash through still hurts).
- Radiation type (damagetype contains 262144): instead of the touch-started
  cycle, the trigger thinks every 0.25 s (17 ticks) from a random first
  delay U(0, 0.5) s, and hurts all touchers when at least 0.5 s passed since
  the last pass, with per = damage × (actual elapsed time). It also drives
  the HL2 Geiger counter (ignore).
- **Player health** (generic path; CS:S may differ, Open questions): the
  fractional part of each hit accumulates; health drops by the integer part
  plus 1 whenever the accumulator reaches 1. So 2.5 per pass takes 2, 3, 2,
  3...

Damage types (bit values as typed in Hammer): 0 generic, 1 crush, 2 bullet,
4 slash, 8 burn, 16 vehicle, 32 fall, 64 blast, 128 club, 256 shock, 512
sonic, 1024 energy beam, 16384 drown, 32768 paralyse, 65536 nerve gas,
131072 poison, 262144 radiation, 524288 drown recover, 1048576 acid,
2097152 slow burn. de_port's water kill trigger uses 16384 (drown), damage
50, damagecap 20, damagemodel 0.

### trigger_push

Keyvalues: "pushdir" (pitch yaw roll), "speed" (0 → 100), spawnflag 128
"Once Only", plus the base flags. ("alternateticksfix" exists in newer
builds; ignore.)

- At spawn the push direction is the forward vector of the pushdir angles,
  stored relative to the trigger's own orientation; at each touch it is
  turned back into world space with the trigger's current orientation (so
  a parented, rotating push trigger turns its push). For a static trigger:
  world direction = forward(pushdir). pushdir "−90 0 0" is straight up,
  "0 90 0" is +Y.
- Touch (every tick the toucher is inside), skipped for non-solid
  entities, entities that are themselves movers (doors, trains) or have no
  movement, entities attached to a parent, and entities failing the
  filters. With push = speed × direction:
  - **Once Only (128)**: add push directly to the toucher's velocity; if
    push.z > 0 unground it; then delete the trigger. One-shot launch.
  - Physics objects: apply a force speed × direction × 100 × dt to the
    centre each tick (props drift).
  - Noclip: nothing.
  - Players and other walking/flying entities: **base velocity**. If the
    toucher already has a base velocity contributed this tick (another push
    trigger or a conveyor), add to it (push += current base velocity), else
    replace it. If push.z > 0 and the toucher is on the ground: unground it
    and raise it 1 unit. Then set base velocity = push and mark "base
    velocity set this tick". (Players on ladders are pushed too: the
    ladder exemption is HL2-only code.)

How the player's movement uses base velocity (this is what makes push
triggers feel like conveyors, not launchers):

1. **Start of each command**: if *no* push touched the player during the
   previous command (the mark is clear), the leftover base velocity is
   converted to real velocity: v += (1 + dt/2) · basevel, then basevel = 0.
   The mark is then cleared for this command.
2. **Gravity step** (start of the airborne/ground move): v.z += basevel.z ·
   dt; then basevel.z = 0. So the vertical part acts as an **acceleration**
   of basevel.z per second (it accumulates into real velocity every tick
   inside).
3. **Move**: for the position integration only, v += basevel (horizontal);
   after the move, v −= basevel. So the horizontal part is a constant
   **offset velocity** while inside: it moves the player but does not
   accumulate, and friction/acceleration act on v without it.
4. **After the move** the player re-links; the push trigger's touch sets
   basevel again and sets the mark, so step 1 next command does nothing.
5. On leaving: the first command after the last touch still has the mark
   set (it was set during the previous command), so it moves with the old
   horizontal basevel once more; the command after that converts the
   leftover horizontal basevel into velocity with the factor 1.0075 (at
   dt 0.015). Net effect: you **keep the push's horizontal speed as
   momentum** when you leave.

Gravity in CS:S movement is applied as two half-steps of sv_gravity × dt/2
around the move (see specs/cs_source/movement.md); a vertical push only
lifts the player if speed > sv_gravity (800).

### trigger_teleport and info_teleport_destination

Keyvalues: "target" (destination name, usually an
info_teleport_destination, any entity works), "landmark" (optional entity
name). Base flags apply (typically 1 = clients, 8 = physics). Newer builds
define spawnflag 32 as "Preserve angles"; in CS:S-era builds 32 is "Only
clients in vehicles", which blocks every player (Open questions).

On every **touch** by an entity passing the filters (so a player standing
inside a teleport whose destination is also inside gets teleported every
tick):
1. Find the destination: first entity matching "target", resolved with the
   toucher as activator (so `!activator` works). None → nothing.
2. If "landmark" is set and found: offset = toucher origin − landmark
   origin, else offset = 0.
3. Unground the toucher.
4. Position = destination origin + offset. Without a landmark and for a
   player, z is lowered by the player box's min z (0 for players, so no
   change).
5. Angles: **without a landmark**, set the toucher's angles to the
   destination's angles, and for players snap the **view angles** to them
   (pitch included; info_teleport_destination usually has pitch 0, so the
   player looks level). With a landmark (or the newer preserve-angles
   flag), angles and view are kept.
6. **Velocity is kept** in all cases, in world space (it is not rotated to
   the new facing; a player running +X into a teleport whose destination
   faces +Y keeps moving +X while looking +Y). Base velocity is also kept.
7. The move is instant: attached children move with it, interpolation is
   reset, collision is re-evaluated (the toucher is re-linked against
   triggers at the destination right away).

### trigger_gravity

Keyvalue "gravity" (multiplier; it is the generic entity gravity field).
Only players are affected; other entities are ignored regardless of flags.
On every touch the player's gravity multiplier is set to the trigger's
value. **It persists after leaving** until another trigger_gravity (or the
game) changes it. The player movement treats a multiplier of 0 as 1, so a
"gravity 0" trigger means normal gravity (use a tiny value for
weightlessness). Whether CS:S resets it on respawn is an open question.

### trigger_soundscape

Already implemented: src/games/cs_source/soundscape.rs and
src/map/sound.rs, behaviour in specs/cs_source/sounds.md
("env_soundscape_triggerable + trigger_soundscape"). It uses the same touch
rules: start touch pushes the soundscape onto the player's list, end touch
pops back; dead/spectating players are re-checked every 0.2 s by an overlap
test instead of touch. When the general trigger system lands, that code
should take its start/end touch from it.

### trigger_remove (bonus, trivial)

On touch by an entity passing the filters: delete that entity (players
too, in the original; we refuse to delete players).

## Per-tick order

For one player command (one tick), trigger-relevant steps:

1. Leftover base velocity → velocity if no push touched last command
   (above); clear the push mark.
2. Use key, think, weapon logic.
3. Movement (gravity half-step includes basevel.z; move uses basevel).
4. Re-link at the final position: start touch (new triggers) then touch for
   every overlapped trigger, in the engine's enumeration order (Open
   questions). Touch callbacks act immediately: push sets base velocity,
   teleport moves the player (and re-links again), gravity sets the
   multiplier, trigger_multiple fires OnTrigger, trigger_hurt starts its
   cycle.
5. After all entities simulated this frame: untouch pass → end touch →
   OnEndTouch / OnEndTouchAll; trigger_hurt half-hit on exit.
6. Event queue (specs/source/entity_io.md): outputs from 4 and 5 with delay
   0 are delivered now.

trigger_hurt passes and trigger_once deletion are thinks; they run when the
trigger entity is simulated in step 2 of the server frame (entity_io.md),
which can be before or after a given player's command in the same frame.

## Edge cases

- Touching vs. just outside: whether boxes that only share a face count as
  touching is engine-side (Open questions). Our rule: overlap requires
  positive-volume intersection with a tolerance of 0.
- A trigger_multiple with wait −1 behaves like trigger_once.
- A trigger_teleport whose destination is inside the same trigger loops
  every tick; whose destination is inside *another* teleport chains in the
  same tick (each teleport re-links).
- Teleporting out of a trigger_hurt removes the player from its links at
  the end of the frame; since the player was hurt in the last pass or not,
  the exit half-hit rule applies normally (it may hit once more).
- Two push triggers overlapping: base velocities add, in touch order.
- A push trigger while ducking or on a ladder still pushes (CS:S).
- Filter entity missing at activation: the trigger has no filter (passes
  everything allowed by the flags).
- Disabled triggers keep their touching list; re-enabling fires start
  touch again for entities inside (OnStartTouch twice for the same player
  is possible).

## Quirks

- **Standing in a trigger_multiple refires OnTrigger every wait** (keep;
  many maps rely on it, e.g. de_prodigy's door triggers with wait 1).
- **trigger_hurt real period is 33 ticks but deals the 0.5 s amount**
  (keep; 1 % more DPS than "damage").
- **Exit half-hit** (keep).
- **Teleport keeps world-space velocity and snaps view** (keep; surf and
  bhop maps depend on it).
- **Horizontal push is momentum on exit, vertical push is acceleration
  inside** (keep).
- **Gravity persists after leaving the trigger** (keep).
- **Thin triggers can be skipped at speed** (keep: matching the original
  matters more than fixing it; mappers already compensate).

## Test cases

dt = 0.015, sv_gravity 800 unless noted. Player box 32×32×62 standing.

| Setup | Input | After | Expected |
|---|---|---|---|
| trigger box (0,0,0)–(10,10,100), flag 1 | player origin (−17,5,0) | re-link | no touch (player x ∈ [−33,−1]) |
| same | player origin (−15,5,0) | re-link | start touch, then touch, OnStartTouch once |
| trigger flag 1 only | physics prop enters | — | no touch |
| trigger flag 8 | physics prop enters | — | touch |
| trigger flag 1, dead player inside | — | — | no OnStartTouch, no hurt |
| trigger_multiple wait 1, OnTrigger and OnStartTouch connected, player walks in at tick 0 and stays | — | 300 ticks | OnStartTouch once (tick 0); OnTrigger at ticks 0, 67, 134, 201, 268 |
| trigger_multiple wait 0 | player stands inside | — | OnTrigger every 13 ticks |
| trigger_once | player enters tick 0; second player enters tick 3 | — | OnTrigger once (tick 0); OnStartTouch twice (ticks 0, 3); trigger gone at tick 7 |
| trigger_multiple OnEndTouchAll, two players inside | one leaves; then the other dies inside | — | OnEndTouch for the leaver; OnEndTouchAll when the survivor's link drops (dead player pruned) |
| trigger_hurt damage 50 (de_port), player 100 hp enters tick 0 and stays | — | — | 25 damage at ticks 0, 33, 66, 99: health 75, 50, 25, 0 (dead at tick 99) |
| trigger_hurt damage 20, player enters tick 0, leaves at tick 40 | — | — | hits of 10 at ticks 0 and 33; no exit hit (hurt in last pass): total 20 |
| trigger_hurt damage 20, player enters tick 0, leaves tick 10, nobody else inside | — | — | 10 at tick 0; exit at tick 10: was hurt in last pass → none; total 10 |
| trigger_hurt damage 20, thin trigger, player passes through in one tick and the pass has not run yet | — | — | first pass (10) if the trigger thinks before the end-touch pass, else exit half-hit 10: total 10 either way |
| trigger_hurt damage −10, player 50/100 hp | stand 2 s | — | +5 per pass: 55, 60, ... capped at 100 |
| trigger_hurt damage 10, damagemodel 1, damagecap 50 | stand inside | — | per-pass hits 5, 10, 20, 25, 25, ... |
| same | leave for 3.1 s, re-enter | — | back to 5 |
| trigger_hurt damage 5 (2.5 per pass), player 100 hp | 4 passes | — | health 98, 95, 93, 90 (fraction accumulates; generic path) |
| trigger_push pushdir "0 0 0" speed 300, player standing still on flat ground, no input | 1 tick inside | — | player moves +4.5 units in X; stored velocity stays ≈ 0 |
| same | 10 ticks inside, then outside | — | +45 units while inside; on the 2nd tick outside velocity.x += 302.25 (300 × 1.0075), then friction acts |
| trigger_push pushdir "−90 0 0" speed 1000, sv_gravity 0, player airborne at rest | 10 ticks inside | — | v.z = 150 (15 per tick) |
| same with sv_gravity 800, player on ground | first tick | — | player raised 1 unit, ungrounded; v.z grows 3 per tick (15 − 12) |
| trigger_push speed 100 up, sv_gravity 800, player on ground | stand inside | — | ungrounded each tick but never rises (net −10.5 per tick), lands again |
| trigger_push "Once Only", pushdir up, speed 500 | player touches | — | v.z += 500 immediately, trigger deleted |
| two overlapping pushes +X 100 and +Y 100 | player inside both | — | base velocity (100, 100, 0) |
| trigger_teleport target D (origin (1000,0,64), angles (0,90,0)), no landmark | player v = (400,0,0), view (30, 0, 0) touches | same tick | origin (1000,0,64), v (400,0,0), view (0,90,0) |
| same with landmark L at (0,0,0), player at (5,3,0) | touch | — | origin (1005,3,64), view unchanged, v unchanged |
| trigger_teleport target `!activator` | touch | — | player "teleported" onto itself (no move), view set to its own angles |
| trigger_teleport target missing | touch | — | nothing |
| trigger_teleport spawnflags 33 (1+32), CS:S-era meaning | player on foot touches | — | nothing (only clients in vehicles) |
| trigger_gravity gravity 0.5 | player enters then leaves | — | player gravity multiplier 0.5 inside and after |
| trigger_gravity gravity 0 | player enters | — | multiplier 0 → treated as normal gravity |
| trigger_multiple with filtername naming filter_activator_team 2 | CT player enters | — | no OnStartTouch, no OnTrigger |
| player moving 3000 units/s through a 16-unit-thick trigger_teleport aligned with motion | — | — | may be skipped (45 units per tick > 16 + 32) |

## Open questions

1. **Face contact**: does a player box exactly touching a trigger face
   (shared plane, zero overlap) start a touch? Probe: place a player with
   setpos at x = −16.0 and −15.9 against a trigger's −X face on the test
   map and read OnStartTouch (via `say` from point_servercommand) on the
   probe server.
2. **Sweep for pushed entities**: is a player carried by a mover tested
   with a swept box (previous position given)? Test with a fast func_door
   carrying a player through a thin trigger.
3. **Touch order** of several overlapping triggers (engine enumeration):
   matters for overlapping pushes plus teleports. Test map room with
   overlapping push + teleport, compare outcomes.
4. **CS:S player damage path**: does CS:S armour absorb trigger_hurt
   damage (which types)? Does the fractional accumulator apply? Does damage
   force move CS:S players (knockback)? Probe: trigger_hurt damage 5, 10
   with and without kevlar, read health/armour per pass; and damage 100
   with nodmgforce 0 vs 1, read velocity.
5. **Gravity reset**: does a CS:S player's trigger_gravity multiplier reset
   on respawn / round start?
6. **Spawnflag 32 and 4096 on CS:S**: does CS:S's trigger_teleport treat
   32 as "only clients in vehicles" (never fires) or as "preserve angles"?
   Does 4096 (disallow bots) exist? Test both flags on the probe server
   with a bot.
7. **trigger_push in CS:S**: CS:S's movement is its own subclass of the
   shared movement; confirm it handles base velocity as above (measure the
   horizontal push test: +4.5 units per tick, and 302.25 momentum after
   exit) with the movement probe.
8. **First hurt pass timing**: same tick as the first touch or next tick?
   Read health per tick with the probe.
9. **AddOutput basevelocity on a player.** (Added by an implementation
   session, not from the source.) Bhop, kz and surf boosters send
   `!activator AddOutput basevelocity x y z` (most on OnEndTouch of a pad,
   some on OnStartTouch). We read it as setting the base velocity field
   itself (replacing it, no push mark), so the player's next command turns
   it into velocity × (1 + dt/2) by step 1 of trigger_push above, and it
   ungrounds nobody by itself (a resulting vz over 250 does: movement.md,
   per-tick order 7). Our reading; before, we added it to the velocity
   at once. Test: probe server, a trigger_multiple with that output,
   `cl_showpos` speed on the ticks after it fires, standing and airborne.
