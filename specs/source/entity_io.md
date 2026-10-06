# Source engine: entity input/output and logic entities

Source basis: Valve's public Source SDK 2013 only (the current GitHub
release). I read the shared server code for output definitions and their
parsing from map keyvalues, the global event queue and its servicing, entity
lookup by name and classname (including the "!" names), the base entity's
inputs (Kill, KillHierarchy, AddOutput, FireUser, Use) and keyvalue parsing,
the value-type conversion used when an output feeds an input, the server
frame order, think scheduling, and the logic entities logic_auto,
logic_relay, logic_timer, math_counter, logic_case, logic_compare,
logic_branch, the filter_* entities, point_servercommand,
point_clientcommand and game_text (server side plus the client HUD message
drawing). I also read real entity lumps from the user's CS:S install
(cs_office, de_nuke, cs_assault, de_prodigy, de_inferno, de_port) for
examples. CS:S's own game DLL (game rules, round restart, player) is not
public; where its behaviour can differ I say so and list it under Open
questions. The current SDK also contains VScript hooks (script functions
that can veto an input); CS:S has no VScript, so they are left out.
Status: draft

## Summary

Map logic is a message system. Every entity can have **outputs** (named
events such as OnTrigger, OnStartTouch, OnPressed). A mapper connects an
output to an **input** of other entities (by target name) with an optional
parameter, a delay and a fire count. When the output fires, one message per
connection is put into a single global, time-ordered **event queue**. Once
per server tick, after all entities have thought and moved, the queue
delivers every message whose time has come; a delivered input can fire more
outputs, and those with zero delay are delivered in the same tick. Logic
entities (relays, timers, counters, case/compare/branch, filters) are just
entities whose inputs and outputs do arithmetic and branching. The feel of a
minigame map's timing comes from four details: delays count from the tick
the output fired and are delivered on the first tick at or after the due
time; zero-delay chains finish in the same tick; thinks (timers) round to
the nearest tick while queue delays round up; and connections on one output
fire in **reverse** of the order they are listed in the map.

## Units and conventions

- Time in seconds. Server tick interval dt = 0.015 s (CS:S default 66.67
  ticks/s; see specs/cs_source/movement.md). Server time of tick N is
  N × dt (32-bit float).
- Names compare case-insensitively (ASCII only).
- "Activator" is the entity that started a chain (usually a player);
  "caller" is the entity whose output fired this particular message.
- Keyvalue, input and output names below are exactly what mappers type in
  Hammer; flag values are the spawnflags bit values.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| dt | 0.015 | s | server tick interval (CS:S 66.67 tick) |
| fire_always | −1 (also 0) | count | times-to-fire meaning "unlimited" |
| relay_refire_pad | 0.001 | s | extra time a relay stays locked after its longest delay |
| relay_onspawn_delay | 0.01 | s | logic_relay OnSpawn think delay (rounds to 1 tick) |
| auto_delay | 0.2 | s | logic_auto fires this long after activation (rounds to 13 ticks) |
| timer_min_interval | 0.01 | s | smallest logic_timer refire (rounds to 1 tick) |
| max_cases | 16 | — | logic_case Case01..Case16 |
| max_filters | 5 | — | filter_multi Filter01..Filter05 |
| hud_channels | 6 | — | game_text channels (value taken mod 6) |
| hud_slots | 16 | — | HUD messages a client draws at once |
| hud_text_max | 512 | bytes | longest game_text message the client keeps |
| hud_font | Trebuchet 24 | — | default HUD message font |

## Behavior

### Named entities and target matching

Every entity has an optional **targetname** ("targetname" keyvalue; it can
be changed at runtime with AddOutput, see below). A **target string** in an
output connection is resolved when the message is delivered, not when it is
queued:

1. **Empty** target: nothing.
2. Starts with **"!"** (procedural name). Exactly one entity at most:
   - `!activator`: the message's activator (may be none).
   - `!caller`: the message's caller.
   - `!self`: the *caller* too (in a queued message the searching entity is
     the caller). So `!self` and `!caller` are the same thing in outputs.
   - `!player`: the player in client slot 1 (the first slot, not "any
     player"; empty slot gives nothing). Quirk, keep.
   - `!pvsplayer`: the first player in the potential visibility set of the
     caller (or of the activator if no caller; slot 1 if neither).
   - `!picker`: the entity under slot-1 player's crosshair (debug use; can
     ignore).
   - Anything else after "!": nothing (a warning).
3. Otherwise, **every** entity whose targetname matches, in entity-list
   order (spawn order: map lump order for map entities, then creation
   order). Entities without a targetname never match.
4. **Classname fallback**: only if step 2 or 3 found no entity at all, every
   entity whose *classname* matches the same string receives the input.
   E.g. target `func_door` with input `Lock` locks every func_door when no
   entity is *named* "func_door".

**Match rule** (names and classnames): compare characters case-insensitively
from the start. It is a match if both strings end at the same point, or if
the query reaches a `*` while all previous characters matched. So the only
wildcard is "the rest", and anything after the `*` is ignored: `door*`
matches `door`, `Door_1`, `doorway`; `do*r` matches `dox`; `*` matches every
named entity. A `*` in the middle is not a pattern. Filters that test names
(filter_activator_name) use the same rule, with one extra: the literal
`!player` there means "is any player".

### Output connections (map syntax)

An output is a keyvalue whose key is the output name and whose value is
five fields: `target,input,parameter,delay,timestofire`. Maps compiled by
newer tools separate the fields with the ESC character (0x1B) instead of
commas; if the value contains 0x1B, split on it, else on commas. CS:S
stock maps use commas. The same key may appear many times (one per
connection). A `#` in a key name and everything after it is ignored (old
Hammer duplicate-key workaround).

- target: string as above.
- input: if empty, defaults to `Use`.
- parameter: if empty, there is **no override** (the output's own value, if
  any, is passed); if non-empty it **replaces** the value with this string.
- delay: float seconds (atof; empty = 0).
- timestofire: integer (atoi). Empty, 0 or −1 mean unlimited. N > 0: the
  connection is deleted from the output after it has fired N times. The
  count is per connection and lives as long as the entity.

**Order**: each new connection is put at the **front** of the output's
list. Firing walks the list front to back, so connections fire in reverse
of their order in the entity lump, and a connection added later with
AddOutput fires before all older ones. Because the queue keeps equal-time
messages in insertion order (below), three connections listed A, B, C with
equal delay are delivered C, B, A.

### Firing an output

When an entity fires an output (with an activator, itself as caller, and an
optional typed value), for each connection in list order:

1. If the connection has no parameter override, queue a message with the
   output's value (or "no value") and due time = now + connection delay +
   an extra delay the firing code may pass (only a few entity types pass
   one; most pass 0).
2. If it has an override, queue the override string as the value with due
   time = now + connection delay. **The extra delay is dropped** when there
   is an override (quirk; only matters for entities that pass an extra
   delay).
3. If the connection has a finite count, decrement it; at 0 remove it.

"now" is the current value of the game clock when the output fires. During
a player's movement command that clock is the player's own command time
(tick base × dt), which equals the server tick time for a player with no
backlog; when a player's client sends several commands in one server tick,
each command runs at its own (consecutive) time. Elsewhere it is the server
tick time.

Each queued message stores: due time, target string (or, for internal
messages, a direct entity reference), input name, value, activator, caller,
and the connection's unique id. Activator and caller are stored as entity
handles: if that entity is deleted before delivery, they become "none".

### The event queue

- One global list ordered by due time. A new message goes **after** every
  message whose due time is less than or equal to its own (stable,
  first-in first-out for equal times).
- **Servicing** happens once per server tick, after all entities have
  thought and moved, and after the touch/untouch pass (see Per-tick order).
  It repeatedly takes the head of the list while its due time ≤ the current
  server time:
  1. Resolve the target (rules above) and deliver the input to each match
     in order. Each delivery is immediate and may fire outputs, which queue
     new messages.
  2. If nothing matched, nothing happens (a developer message).
  3. Remove the message and restart from the head.
- Consequences:
  - A message is delivered on the **first tick whose time is ≥ due time**:
    tick = ceil((t_fire + delay) / dt), with float comparison. Delay 0.5
    fired at tick 100 (t = 1.5) arrives at tick 134 (t = 2.01, since
    tick 133 is 1.995).
  - **Zero delay** (or any delay that is already due) is delivered in the
    same tick, including messages created while servicing. A chain
    A → B → C with all delays 0 completes in one tick. Messages already
    due are delivered before ones created during servicing (breadth-first),
    because new ones go behind every earlier-or-equal one.
  - An infinite zero-delay loop (two relays triggering each other) hangs
    the server in the original. We should cap work per tick (see Quirks).
  - Delays that are an exact multiple of dt compare floats at the boundary;
    see Open questions.
- **Cancel**: some inputs remove every queued message whose caller is a
  given entity (logic_relay CancelPending). Removal checks the caller handle
  and that caller's current name and classname.
- The queue is not cleared by Kill: messages that target a killed entity by
  name later simply find nothing; messages the killed entity sent remain.

### Delivering an input; parameter typing

The target looks up the input name case-insensitively among its inputs
(its class, then base classes). Unknown inputs are ignored (developer
message). Each input has a declared type: none (void), string, integer,
float, boolean, vector, color, entity, or "any" (accepts the value as is).
If the value's type differs, it is converted:

| From | To | Rule |
|---|---|---|
| string | integer | atoi (leading integer, else 0); "2.7" → 2 |
| string | float | atof; "" → 0 |
| string | boolean | atoi ≠ 0. **"true" → false**, "1" → true |
| string | vector | "[x y z]" or "x y z" (missing parts 0) |
| string | color | "r g b a", missing alpha = 255 |
| string | entity | first entity whose name matches (no activator context) |
| integer | float, boolean | numeric; ≠ 0 is true |
| float | integer | truncate toward 0; float → boolean: ≠ 0 |
| entity | string | the entity's targetname |
| no value | string | allowed (empty string) |
| anything | void, any | allowed |
| **no value** | integer/float/bool/vector/color | **rejected**: the input does not run (warning "bad input/output link") |
| float/int/bool | string | not converted: **rejected** (e.g. math_counter OutValue into a string input fails; logic_case InValue is "any" and works) |

When an "any" input reads its value as text, numbers print as: integer
"%i", float "%g" (3 → "3", 2.5 → "2.5", 1000000 → "1e+06"), boolean
"true"/"false", vector "[x y z]", no value → "".

So a connection to math_counter `Add` with an empty parameter from an
output that carries no value (OnStartTouch, OnTrigger) does nothing. Map
authors always type a number there.

Inputs that are plain "set this field" (declared as inputs directly bound to
a keyvalue) just store the converted value.

### Activator and caller propagation

The caller of a message is always the entity that fired the output. The
activator is chosen by the firing entity; rules for the entities in these
specs:

| Entity / output | Activator passed on |
|---|---|
| triggers: OnStartTouch, OnEndTouch, OnTrigger, OnHurt... | the touching / hurt entity |
| logic_relay OnTrigger, math_counter, logic_case, logic_compare, logic_branch, filters, FireUserN→OnUserN | the activator of the input that caused it (pass-through) |
| logic_relay OnSpawn, logic_timer outputs | the relay / timer itself |
| logic_auto outputs | none |
| func_button OnPressed/OnIn/OnOut | the presser |
| func_door OnOpen, OnClose, OnFullyOpen | the door itself |
| func_door OnFullyClosed | the last user of the door |
| func_breakable OnBreak | the breaker |
| path_track OnPass | the train |

### Base inputs every entity has

- **Kill**: tells its owner (if any) and removes the entity. Removal is
  deferred to the end of the server frame: until then the entity still
  exists, can still be found by name and still receives inputs queued in
  the same tick (inferred from the deferred-delete design; see Open
  questions). It stops thinking/touching immediately only where the removal
  path says so (marked entities are skipped by touch handling).
- **KillHierarchy**: KillHierarchy every child (attached by parenting)
  first, depth first, then Kill itself.
- **Use**: calls the entity's use behaviour with activator and caller.
  Quirk: the "use type" passed is the queued message's connection id (a
  running counter starting at 1, one per parsed connection in the map), not
  a meaningful on/off/toggle value. Doors and buttons ignore the use type;
  func_movelinear only reacts to use type 2 and func_tracktrain only to 2,
  so an I/O `Use` on those is effectively random. Treat I/O `Use` as
  "toggle" and do not reproduce the id quirk.
- **AddOutput** (string): the text up to the first space is a key; every
  `:` in the rest becomes `,`; then it is applied as if it were a map
  keyvalue. With an output name as key it adds a connection
  (`OnTrigger target:Input:param:delay:times`, times −1 or 0 = unlimited),
  at the front of the list. With a normal keyvalue it sets it at runtime:
  `targetname` renames, `origin "x y z"` moves the entity instantly (no
  sweep, velocity unchanged), `angles` rotates, `rendercolor`, `renderamt`,
  and any field the class reads from a keyvalue. A keyvalue that only
  matters at spawn changes the field but has no visible effect. Because `:`
  is rewritten, a parameter can never contain a colon or comma. No space →
  ignored with a warning.
- **FireUser1..FireUser4**: fire OnUser1..OnUser4 with the same activator
  and **caller = the entity itself**. The parameter is ignored and the
  OnUser output carries no value.
- **SetParent** (name), **ClearParent**, **Alpha** (int 0..255 clamp),
  **Color** ("r g b"), **SetDamageFilter** (filter name), **AddContext**...
  exist on every entity; only Kill/KillHierarchy/Use/AddOutput/FireUser
  are in this spec's scope.

**Enable/Disable** are not universal. Each class that has them defines
them (triggers, logic_relay, logic_timer, math_counter, func_brush...). An
Enable sent to an entity without it is an unknown input and is ignored.

### logic_auto

Keyvalues: "globalstate" (optional env_global name), spawnflags 1 = "Remove
on fire".

- After the entity is activated (map load; in CS:S also every round
  restart, see Open questions), it thinks once 0.2 s later (rounded to the
  nearest tick: 13 ticks at dt 0.015).
- On that think, if no globalstate is set or that global is on:
  1. If the load is a level transition: OnMapTransition; new game:
     OnNewGame; saved-game load: OnLoadGame; background map:
     OnBackgroundMap. A multiplayer server map load counts as a **new
     game**, so OnNewGame fires on every normal map start (inferred: the
     engine sets the load type; see Open questions).
  2. OnMapSpawn (always).
  3. Multiplayer: if the game rules say a round restart is in progress,
     OnMultiNewRound, else OnMultiNewMap.
  4. If spawnflag 1: remove itself.
- If the global is off, nothing fires and it never retries.
- All outputs have no activator, caller = the logic_auto.
- Order of the queued messages: OnNewGame-type first, then OnMapSpawn, then
  OnMulti*; equal delays deliver in that order.

### logic_relay

Keyvalues: "StartDisabled" (0/1). Spawnflags: 1 = "Only trigger once"
(remove on fire), 2 = "Allow fast retrigger".

Inputs:
- **Trigger**: if enabled and not locked: fire OnTrigger (activator
  pass-through, caller = relay). Then: if flag 1, Kill itself; else if flag
  2 is not set, become **locked** and queue an internal unlock message to
  itself at now + (largest delay among its OnTrigger connections) + 0.001 s.
  Trigger inputs that arrive while locked are **dropped**, not queued.
- **Enable**, **Disable**, **Toggle**: the enabled state. Disabled relays
  drop Trigger.
- **CancelPending**: remove every queued message whose caller is this relay
  (its pending OnTrigger messages and its own unlock message) and unlock.
- **EnableRefire**: unlock now (the unlock message is this input).

Outputs: OnTrigger; **OnSpawn** fires once, 0.01 s (1 tick) after
activation, only if it has connections, with activator = caller = relay; if
flag 1 the relay then removes itself (so a flag-1 relay with OnSpawn
connections is gone after spawn).

### logic_timer

Keyvalues: "RefireTime" (s), "StartDisabled", "UseRandomTime" (0/1),
"LowerRandomBound", "UpperRandomBound" (s). Spawnflag 1 = "Oscillator"
(alternate OnTimerHigh and OnTimerLow).

- **Spawn**: if not random and RefireTime < 0.01, use 0.01. Enable unless
  StartDisabled (with the 0.01 floor it is always > 0, so "refire 0"
  becomes "every tick").
- **Scheduling** (reset): if random, draw RefireTime = uniform float in
  [Lower, Upper] (overwrites the stored value). Next fire at tick
  round_half_up((now + RefireTime) / dt). Thinks use **nearest tick**,
  unlike queue delays which round up. RefireTime 1.0 → 67 ticks; 0.1 → 7
  ticks; 0.02 → 1 tick; 0.01 → 1 tick.
- **Fire** (on its scheduled tick, if enabled): with the oscillator flag,
  fire OnTimerLow when the internal toggle is off, OnTimerHigh when on,
  then flip it; the toggle starts off, so the **first fire is OnTimerLow**.
  Without the flag, fire OnTimer. Activator = caller = timer. Then
  reschedule (a new random draw if random).
- Inputs:
  - **Enable**: enable and reschedule from now (also when already enabled:
    restarts the countdown).
  - **Disable**: stop; **Toggle**.
  - **FireTimer**: fire now if enabled (and reschedule).
  - **RefireTime** (float): clamp to ≥ 0.01; if different from the current
    value, store and reschedule (no reschedule if equal).
  - **ResetTimer**: reschedule from now (if enabled).
  - **AddToTimer** (float): next fire time += value (if enabled), rounded
    to nearest tick.
  - **SubtractFromTimer** (float): if (next − now) ≤ value, fire at the
    next think opportunity (this tick if the timer has not thought yet this
    tick, else next tick); else next −= value.
  - **UseRandomTime**, **LowerRandomBound**, **UpperRandomBound**: set the
    field (takes effect at the next reschedule).

### math_counter

Keyvalues: "startvalue" (parsed with **atoi**: "2.5" → 2), "min", "max"
(floats), "StartDisabled".

- Spawn: if min > max, swap. Clamping is active only if min ≠ 0 or max ≠ 0
  ("both zero" = unclamped). If clamping, clamp the start value. Hit flags
  start cleared (so a start value already at max does not fire until it
  leaves and returns).
- **Update(new value)**, used by every arithmetic input:
  1. If clamping: if new ≥ max: if the hit-max flag is off, set it and fire
     **OnHitMax**; else (new < max) clear it. Then the same for min: new ≤
     min fires **OnHitMin** once. Then clamp new to [min, max].
  2. Store and fire **OutValue** with the new value (float), even if
     unchanged.
  So OnHitMax/OnHitMin are queued before OutValue and (equal delays)
  delivered first. Activator pass-through, caller = counter.
- Inputs (float parameter unless noted):
  - **Add**, **Subtract**, **Multiply**, **SetValue**: value ± × or = then
    Update. Ignored while disabled.
  - **Divide**: if parameter ≠ 0, divide then Update; if 0, Update with the
    unchanged value (fires OutValue and hit checks). Ignored while disabled.
  - **SetValueNoFire**: set (clamped if clamping), no outputs, flags
    unchanged. Ignored while disabled.
  - **SetHitMax** / **SetHitMin**: set max (if max < min, min = max) / set
    min (if max < min, max = min), then Update with the current value.
    **Works while disabled.**
  - **GetValue** (no parameter): fire **OnGetValue** with the current value.
    Works while disabled.
  - **Enable**, **Disable**.
- Note the "both zero" rule also applies after SetHitMax 0 + SetHitMin 0.

### logic_case

Keyvalues: "Case01".."Case16" (strings). Outputs OnCase01..OnCase16,
OnDefault.

- **InValue** (any type): take the value as text (rules above) and compare
  it case-insensitively with Case01, Case02, ... Case16 in that order,
  skipping empty cases. The **first** match fires its OnCaseNN (activator
  pass-through, no value). No match: fire **OnDefault** carrying the input
  value.
- **PickRandom**: build the list of case numbers whose OnCaseNN output has
  at least one connection (not whose CaseNN is set), ascending; pick one
  uniformly; fire it. None: nothing.
- **PickRandomShuffle**: draws without replacement from a "deck":
  1. If the deck is empty, rebuild it like PickRandom (count n). If n > 1
     and there was a previous pick, move the previous pick to the last
     position and draw this time only from the first n − 1 (no immediate
     repeat across refills).
  2. Draw index r uniformly from the drawable range; fire that case; then
     replace deck[r] with the deck's last element and shrink the deck by
     one. (When the previous pick was held out it is the last element, so
     it moves into slot r and stays in the deck.)
  3. Remember the pick.

### logic_compare

Keyvalues: "InitialValue", "CompareValue" (floats).

- **SetValue**: store input value, no output. **SetCompareValue**: store
  compare value, no output. **SetValueCompare**: store input value, then
  compare. **Compare**: compare the stored input value.
- Compare (exact float equality): equal → **OnEqualTo**; else
  **OnNotEqualTo**, then **OnGreaterThan** (value > compare) or
  **OnLessThan**. All carry the input value (float); activator
  pass-through. OnNotEqualTo is queued before Greater/Less.

### logic_branch

Keyvalue "InitialValue" (boolean via atoi). Outputs OnTrue, OnFalse.

- **SetValue** (bool): store, no output. **SetValueTest**: store, then test.
- **Toggle**: invert, no output. **ToggleTest**: invert, then test.
- **Test**: fire OnTrue if true, else OnFalse (activator pass-through, no
  value).
- (logic_branch_listener integration exists in the SDK; out of scope.)

### Filters

All filters: keyvalue "Negated" (0 = pass entities that match, 1 = pass
entities that don't). Input **TestActivator**: test the activator; fire
**OnPass** or **OnFail** (activator pass-through). Triggers and damage
filters call the same test directly.

- **filter_activator_name**, keyvalue "filtername": passes if the tested
  entity's targetname matches (match rule above). `!player` means "is a
  player". An entity without a name only matches an empty or `*` pattern.
- **filter_activator_class**, keyvalue "filterclass": passes if the
  classname matches (same rule; `player`, `weapon_*`...).
- **filter_activator_team**, keyvalue "filterteam": passes if the team
  number equals it (CS:S: 2 = Terrorists, 3 = Counter-Terrorists).
- **filter_activator_mass_greater** ("filtermass"): physics mass > value;
  entities without physics fail.
- **filter_damage_type** ("damagetype"): only for damage filters: passes if
  the damage-type bits equal the value exactly.
- **filter_multi**: keyvalues "FilterType" (0 = AND, 1 = OR),
  "Filter01".."Filter05" (names). At activation each name is resolved once
  (first entity with that name; non-filters skipped with a warning) into a
  compact list. AND: passes if every listed filter passes (no filters →
  pass). OR: passes if any passes (no filters → fail). Negated applies to
  the combined result. Each sub-filter applies its own Negated.
- A null tested entity (e.g. TestActivator with no activator) crashes or is
  undefined in the original; we fail it.
- Filter names on triggers and filter_multi are resolved **once at
  activation**; a filter created or renamed later is not picked up.

### point_servercommand

Input **Command** (string). If the string is non-empty, it is appended
(with a newline) to the **server console** command buffer and runs when
the engine next executes that buffer (later in the same frame or the next).
Anything a server operator could type is possible: `sv_gravity 200`,
`mp_restartgame 1`, `changelevel`, `exec`, `rcon_password`, `quit`,
`sv_cheats 1`. The current SDK gates it with the server cvar
`sv_allow_point_servercommand` ("always" default outside TF2; "disallow"
blocks with a warning; TF2 adds "official"); older CS:S builds may have no
gate at all (Open questions).

**Security, for us**: a downloaded map is untrusted. We implement
point_servercommand as an **allowlist**: commands that change only
match/movement state of the current game (e.g. `say`, `sv_gravity`,
`sv_airaccelerate`, `sv_accelerate`, `sv_friction`, `sv_maxspeed`,
`mp_restartgame`, `mp_roundtime`, `mp_friendlyfire`, `sv_alltalk`,
`phys_pushscale`, `mp_freezetime`, `mp_timelimit`), applied through our
own console with values bounded; everything else is logged and ignored.
Settings changed by a map revert at map change. The exact list should be
grown from the target minigame maps (see Open questions).

### point_clientcommand

Input **Command** (string, non-empty). Multiplayer: if the activator is a
player, the string is sent to that player's client to run as if typed;
no player activator → nothing. (Single player: always player 1.) The
client decides what it accepts; the 2013 engine lets a server run only
commands marked as server-executable when the client restricts server
commands (Open questions). Typical map uses: `play <sound>`,
`playgamesound`, `r_screenoverlay <material>`, `echo`. Abuse seen in the
wild: `bind`, `disconnect`, `retry`, `cl_*` settings. **For us**: an
allowlist of harmless client commands (`play`, `playgamesound`,
`r_screenoverlay`, `echo`), others ignored and logged.

### game_text

Keyvalues: "message" (text), "x", "y" (floats), "channel" (int),
"effect" (0, 1, 2), "color" and "color2" ("r g b a" ints), "fadein",
"fadeout", "holdtime", "fxtime" (s), "master" (multisource name, rarely
used). Spawnflag 1 = "All players". Input **Display**; using the entity
(I/O `Use`) does the same. There is no SetText input in CS:S-era Source.

- **Who sees it**: flag 1 → every client. Otherwise only the activator, and
  only if it is a player; no activator (e.g. fired from logic_auto or a
  timer) → **nobody** sees it (common mapping mistake; keep).
- **Channel**: the client uses channel mod 6. A new message on a channel
  replaces that channel's text and parameters; a message still being shown
  on that channel picks up the new text (the client stores one message per
  channel and shows it from up to 16 draw slots; see Open questions).
- **Lifetime** (from when the client receives it): effect 0 or 1: fadein +
  holdtime + fadeout. Effect 2: fadein × (number of characters) + fadeout +
  holdtime.
- **Fade** for effects 0/1, with local time τ: τ < fadein: opacity =
  τ/fadein; fadein ≤ τ ≤ fadein + hold: 1; after: 1 − (τ − fadein −
  hold)/fadeout (fadeout 0 → 0). The colour's alpha component ("color" a)
  is **ignored**; opacity comes only from the fade. Effect 1 ("credits"
  flicker) draws the same as effect 0 (its flicker is disabled in code).
- **Effect 2 (scan out)**: characters appear one by one, character k
  (0-based, counting every character including spaces and newlines)
  appears at τ = (k+1)·fadein. A just-appeared character is drawn in color2
  and blends linearly to color with weight (τ − appear)/fxtime, i.e. at
  appear it is color2, after fxtime it is color. Not-yet-appeared
  characters are still emitted, in colour (0, 0, 0) at full opacity; with
  the HUD's additive text rendering that is invisible (see Open
  questions). After fadein·n + holdtime the whole text fades out over
  fadeout.
- **Position** (W, H screen size in pixels, w line width, h text block
  height = lines × font height): x = −1 centres each line separately:
  (W − w)/2. x ≥ 0: left edge at x·W. Other negative x: right-aligned,
  (1 + x)·W − (widest line). Then clamp so the line stays on screen.
  y = −1: (H − h)/2 for the block; y ≥ 0: top at y·H; other negative:
  (1 + y)·H − h; clamp to the screen. Lines split at "\n" characters
  present in the received text.
- The message is truncated to 511 characters by the client; the text is
  first looked up as a localization token (a message starting with "#" or
  equal to a token shows the localized string).

## Per-tick order

One server tick (frame), as far as entity logic is concerned:

1. Delete entities that were marked for removal outside the frame.
2. Every entity is simulated once, in the global think list order. For
   each: run its think if due (logic_timer fires, logic_auto, relay
   OnSpawn, trigger_hurt passes, door/button "move done" steps happen as
   part of the mover's push); movers (doors, trains) push; players run all
   their pending movement commands (each command: use key handling →
   movement → touch triggers → touch callbacks fire outputs). Outputs fired
   here are queued at the time described above.
3. Untouch pass: entities whose touch link was not refreshed get their end
   touch (OnEndTouch, OnEndTouchAll fire; see specs/source/triggers.md).
4. **Service the event queue** (all due messages, including zero-delay ones
   created during servicing).
5. Delete entities marked by Kill during this frame.
6. Send state to clients.

A consequence: an output fired by a think or touch at tick N with delay 0
is delivered at tick N; one fired during servicing at tick N with delay d
is delivered at ceil((N·dt + d)/dt).

## Edge cases

- A message whose target matches several entities delivers to all of them,
  in entity-list order, before the next message.
- Target matching happens at delivery: renaming an entity with AddOutput
  between fire and delivery changes who gets it.
- `!activator` when the activator was deleted (player disconnected, entity
  killed) resolves to nothing, and the classname fallback then looks for an
  entity of class "!activator" (none).
- An output fired by an entity that is then killed in the same tick still
  delivers (the queue keeps the message; the caller handle may read as
  none, which breaks `!caller` / `!self` targets).
- Times-to-fire counts per connection, even if the target did not exist.
- Delay is never negative in practice; a negative delay is "already due".
- Integer inputs fed "1.9" get 1 (atoi), but float inputs get 1.9.
- logic_timer RefireTime input with the same value does not reschedule;
  with a different value it restarts the countdown.
- math_counter with min 0 and max 10 started at 0: the first Update to 0
  (e.g. Subtract) fires OnHitMin even though it "was" at 0 (flags start
  cleared).

## Quirks

- **Reverse connection order** (keep: maps rely on it implicitly, e.g. a
  Kill listed first runs last).
- **Parameter override drops the extra firing delay** (keep; rarely
  matters).
- **"true" is false** for boolean parameters (keep).
- **`!player` is slot 1, not the activator** (keep; maps meant
  `!activator`).
- **game_text without "All players" and without a player activator shows
  nothing** (keep).
- **Use carries the connection id as use type** (do not keep; treat as
  toggle, see above).
- **Infinite zero-delay loops hang the server** (do not keep: cap
  deliveries per tick, e.g. 10,000, log and defer the rest to the next
  tick).
- **Timer think rounding vs queue rounding differ** (keep: a logic_timer at
  0.02 fires every tick, a relay delay of 0.02 waits 2 ticks).

## Test cases

dt = 0.015 s. "Tick N" means server time N × dt.

| Setup | Input | After | Expected |
|---|---|---|---|
| logic_relay with three OnTrigger connections in lump order to relays A, B, C (each `X,Trigger,,0,-1`) | Trigger at tick 100 | tick 100 | A, B, C receive Trigger in order C, B, A, all at tick 100 |
| relay OnTrigger `B,Trigger,,0.5,-1` | Trigger at tick 100 | — | B triggered at tick 134 (t 2.01), not 133 (1.995) |
| relay OnTrigger `B,Trigger,,0.02,-1` | Trigger at tick 100 | — | B triggered at tick 102 |
| relays A→B→C, all delay 0 | Trigger A at tick 50 | tick 50 | A, B, C all fire OnTrigger at tick 50 |
| trigger_multiple OnStartTouch `door,Open,,0,1` | two separate touches | — | door gets one Open; second touch queues nothing |
| connection with timestofire 0 | fire 5 times | — | 5 deliveries |
| connection with timestofire 2 | fire 3 times | — | 2 deliveries, connection gone |
| math_counter (min 0, max 0) | `Add` with empty parameter from OnTrigger | — | input rejected, value stays 0, no OutValue |
| math_counter (min 0, max 0) | Add "2.7" | — | value 2.7, OutValue 2.7 |
| math_counter "startvalue" "2.5" | spawn | — | value 2 |
| logic_branch | SetValueTest "true" | — | value false, OnFalse fires |
| entities named door, Door_1, doorway, mydoor | target `door*` Lock | — | first three locked, mydoor not |
| entity named dox | target `do*r` | — | dox receives (wildcard ends the match) |
| two func_door, none named "func_door" | target `func_door` Open | — | both open (classname fallback) |
| entity named "func_door" plus two unnamed func_doors | target `func_door` Open | — | only the named one |
| trigger touched by player P, OnStartTouch `!activator,SetHealth,50,0,-1` | P touches | same tick | P's SetHealth receives 50 |
| as above with delay 1, P disconnects at +0.5 s | — | +1 s | nothing receives it |
| relay R, OnTrigger `!self,FireUser1,,0,-1` | Trigger R | — | R gets FireUser1 (self = caller = R) |
| entity E | AddOutput "OnUser1 relay2:Trigger::0.5:1"; then FireUser1 twice, 2 s apart | — | relay2 triggered once, 0.5 s (34 ticks) after the first FireUser1 |
| entity E | AddOutput "targetname newname" | — | target `newname` now reaches E, old name does not |
| E with OnUser1 connections, activator P | FireUser1 from relay R | — | OnUser1 activator = P, caller = E |
| relay R | Kill (delay 0) and Trigger (delay 0) queued in that order in tick 10 | tick 10 | R still receives Trigger (removal at end of frame) |
| logic_auto, map loads, activation at tick 0 | — | — | OnMapSpawn queued at tick 13 (0.195 s), delivered tick 13 |
| logic_relay with OnSpawn connection, activated tick 0 | — | — | OnSpawn fires at tick 1 |
| logic_relay (flags 0) OnTrigger `B,Trigger,,1,-1` | Trigger at ticks 0, 34 and 68 | — | B triggered at ticks 67 and 135; the tick-34 Trigger is dropped; the tick-68 one is accepted (unlock due 1.001 s, delivered tick 67, after B's message) |
| same relay with spawnflag 2 | Trigger at ticks 0 and 34 | — | B at ticks 67 and 101 |
| relay spawnflag 1 | Trigger twice | — | one OnTrigger, relay removed |
| relay with pending OnTrigger (delay 5) | CancelPending at +1 s | — | the delayed message never arrives; relay unlocked |
| logic_timer RefireTime 1.0, enabled tick 0 | — | — | OnTimer at ticks 67, 134, 201 |
| logic_timer RefireTime 0 | — | — | OnTimer every tick |
| logic_timer RefireTime 0.02 | — | — | OnTimer every tick (nearest-tick rounding) |
| logic_timer spawnflag 1, RefireTime 1 | — | — | tick 67 OnTimerLow, tick 134 OnTimerHigh, tick 201 OnTimerLow |
| logic_timer random 5..30 (cs_office radiotimer) | — | — | first fire between ticks 333 and 2000 |
| logic_timer next fire at tick 200, now tick 100 | AddToTimer 1.0 | — | next fire tick 267 |
| logic_timer next fire at tick 200, now tick 100 | SubtractFromTimer 2.0 | — | fires next think (tick 100 or 101) |
| math_counter min 0 max 5 (cs_office soda) | Add 1 five times | — | OutValue 1,2,3,4,5; OnHitMax on the 5th, delivered before OutValue 5 |
| same, at 5 | Add 1 | — | value stays 5, no OnHitMax, OutValue 5 |
| same, at 5 | Subtract 1, then Add 1 | — | OnHitMax fires again on reaching 5 |
| math_counter min 0 max 10 at 4 | Divide 0 | — | value 4, OutValue 4 |
| math_counter disabled, at 3 | Add 1; GetValue | — | value 3; OnGetValue 3 |
| math_counter min 0 max 0 | Add 5 three times | — | 15 (no clamping) |
| math_counter OutValue → logic_case InValue, Case01 "3", Case02 "3.0" | value becomes 3 | — | OnCase01 |
| logic_case Case01 "abc" | InValue "ABC" | — | OnCase01 |
| logic_case Case01 "a", Case02 "a" | InValue "a" | — | only OnCase01 |
| logic_case no match | InValue "zz" | — | OnDefault carrying "zz" |
| logic_case with outputs on OnCase02, 05, 09 only | PickRandom 3000× | — | each of 2, 5, 9 about 1/3; never others |
| same | PickRandomShuffle 3× | — | 2, 5, 9 each once in some order |
| same | 4th PickRandomShuffle | — | not equal to the 3rd pick |
| logic_compare CompareValue 5 | SetValueCompare 5 | — | OnEqualTo (5) only |
| logic_compare CompareValue 5 | SetValueCompare 7 | — | OnNotEqualTo (7), then OnGreaterThan (7) |
| filter_activator_name "bob*", Negated 0 | test entity named "Bob1" | — | pass |
| same, Negated 1 | test "Bob1" | — | fail |
| filter_activator_name "!player" | test any player | — | pass |
| filter_multi AND with no valid filters | test anything | — | pass; with OR: fail |
| filter_multi OR of name "a" and class "player", Negated 1 | test player named "b" | — | fail |
| filter_activator_team 3 | test CT player | — | pass; T player: fail |
| game_text, flag 0 | Display from logic_auto (no activator) | — | no client shows anything |
| game_text effect 0, fadein 1.5, hold 2, fadeout 0.5, flag 1 | Display | — | visible 4.0 s; opacity 0.5 at τ 0.75, 1 at τ 2, 0.4 at τ 3.8 |
| game_text effect 2, fadein 0.05, hold 2, fadeout 1, message of 10 chars | Display | — | last char appears τ 0.5; total lifetime 3.5 s |
| game_text channel 7 | Display | — | uses channel 1 |
| game_text x −1, y 0.25, 1920×1080, one line 300 px wide | Display | — | left edge 810 px, top 270 px |
| point_servercommand | Command "sv_gravity 400" | — | our allowlist applies sv_gravity 400 |
| point_servercommand | Command "rcon_password x" | — | ignored and logged |
| point_clientcommand, activator a player | Command "play buttons/button1.wav" | — | that player hears it |
| point_clientcommand, no activator | Command "play x" | — | nothing |

## Open questions

1. **CS:S round restart**: CS:S re-creates most map entities at each round
   restart, keeping a preserve list (in the public HL2DM code that list
   includes func_brush, func_wall, func_buyzone, info_target,
   env_soundscape*, trigger_soundscape, keyframe_rope, move_rope, sky_camera,
   ...). Check in CS:S: does logic_auto fire OnMapSpawn every round? Does
   OnMultiNewRound ever fire (the shared code needs game rules to report
   "in round restart"; CS:S may not)? Does a func_brush disabled in round 1
   stay disabled in round 2? Test: map with logic_auto OnMapSpawn and
   OnMultiNewRound → `say` via point_servercommand, plus a func_brush
   toggled by a button; play two rounds on the probe server.
2. **OnNewGame on a dedicated server map load**: does it fire on
   `changelevel`/`map`? Same test map, print both.
3. **Float boundary**: is a delay that is an exact tick multiple (0.015,
   0.03, 0.3) delivered after n or n+1 ticks? Fire `say` from relays with
   delays 0.015, 0.03, 0.3 and log the server tick (probe plugin) on the
   probe server. Until measured, we compute due ticks in integer ticks:
   ceil(delay/dt − 1e−4).
4. **Kill then input in the same tick**: confirm a killed entity still
   accepts inputs queued in that tick (relay: Kill delay 0 then Trigger
   delay 0 → does OnTrigger fire?).
5. **sv_allow_point_servercommand in CS:S**: `cvarlist sv_allow` on the
   probe server; and which commands real minigame maps send (dump all
   point_servercommand / point_clientcommand Command parameters from the
   user's downloaded maps once there are some; none are in the install's
   download folder yet).
6. **Client command restriction**: does the CS:S client run
   `point_clientcommand` strings like `bind` or `r_screenoverlay`? Check
   `cl_restrict_server_commands` exists and its default (`cvarlist
   cl_restrict`).
7. **game_text scan-out**: are not-yet-revealed characters invisible
   (additive font) or drawn black? Screenshot an effect-2 message.
8. **game_text same channel**: send two messages on channel 1, 1 s apart,
   each with hold 5; does the first disappear, or do both slots show the
   second text? Screenshot on the reference client (user's PC).
9. **game_text newline**: can a map message contain a line break at all
   (the BSP keyvalue has no escape for "\n")? Look for multi-line game_text
   in downloaded maps.
10. **Use type through I/O**: harmless for doors/buttons; confirm a
   func_tracktrain ignores an I/O `Use`.
