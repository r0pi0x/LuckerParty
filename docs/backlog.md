# Backlog

Things to build, roughly in priority order within each section. Shortcuts
already in the code live in [tech-debt.md](tech-debt.md); the MVP plan is in
[plans/active/](plans/active/mvp-combat-arms-slice.md). Move items into a plan
when work starts; delete them when done.

## Tools

- **HDR parity**: CS:S defaults to mat_hdr_level 2 on dust2 (HDR lightmaps,
  tonemapping, bloom); the reference install runs LDR. Compare and match
  both if players use HDR.
- **More refcmp views** across dust2 (mid, long, B, spawns) and other maps.
- **Debug overlays, CS:S-style**, toggled by console cvars (and a key or
  flag until the console exists):
  - `cl_showpos 1`: map name, position, angles and velocity in the top
    corner, as CS:S draws it.
  - `cl_showfps 1` / `net_graph`-like FPS counter (frame time, min/max).
  - Sound emitter markers: where each sound was emitted, with the entry
    name (and wave) drawn at that spot for a few seconds after it plays,
    fading out; filters by channel/name. Source has `snd_show`/
    `snd_visualize` as a reference for the idea.
  - Other Source debug text worth copying as we go (`developer 1`
    notify lines, `cl_showpos 2`).

## Movement

- **Walk + crouch is extremely slow.** Holding walk (+speed) while ducked
  barely moves the player. specs/cs_source/movement.md ("Walking", walk +
  duck) says CS:S does this too: wish speed 44.2, one tick of acceleration
  smaller than friction's stop drop, so speed never builds. Check in the
  real game first; if CS:S moves at a normal crouch-walk pace, the
  measurement or the model is wrong and the spec needs a revision.

## Gameplay

- **In-game console** (toggle with `~`, Source-style), feature rich:
  - Commands and cvars from every system that registers them (movement
    cvars like `sv_enablebunnyhopping`, `sv_airaccelerate`, `sv_gravity`;
    `map`, `noclip`, `god`, `give`, `kill`, `setpos`/`getpos`, `exec`,
    `bind`, `alias`, `echo`, `clear`, `quit`), each with help text, type,
    default, min/max and "changed from default" marking.
  - Tab completion of names and values (enum values, map names from the
    install, file paths for `exec`), cycling with repeated Tab, and
    inline suggestions as you type, ranked fuzzy matches.
  - History (up/down, Ctrl+R reverse search), persisted across sessions;
    `;`-separated commands; quoted arguments; `+`/`-` actions for binds.
  - `find <text>` over names and help; `differences` lists cvars changed
    from defaults; `reset <cvar>`/`resetall`; `cvarlist <prefix>`.
  - Config files: autoexec.cfg at startup, `host_writeconfig` to save
    binds and changed cvars; `exec` reads CS:S-style .cfg (surf/bhop/kz
    configs work as-is, unknown cvars reported, not fatal).
  - Output: colored by severity, log lines (warnings/errors from `log`)
    mirrored in, scrollback with page up/down, selectable and copyable,
    a filter box; timestamps on demand.
  - Live values: `watch <cvar|expr>` overlays a value on the HUD (speed,
    position, velocity, water level, ladder state); `toggle`, `incrementvar`
    for binds; `wait` for scripted sequences.
  - Remote: the same commands over the dev remote protocol, so scripts
    and tests can drive it.
- **Sound effects**: an audio slot (Bevy audio or a mixer crate) playing the
  game's own sounds from the user's install: footsteps by surface material
  (Source surfaceprops), jump/land, ladder and water (splash, swim), weapon
  fire/reload/dry, impacts by material, player hurt/death; positional with
  distance falloff. Spec the CS:S sound rules (footstep cadence, volumes,
  `sv_footsteps`, walk/duck silence) from the SDK first.
- **Weapons**: the weapon slot from the MVP plan (trigger, cost, delivery,
  effect; hitscan, health, crosshair, ammo HUD), with CS:S weapons from
  their (locally decrypted) scripts and a spec for firing, spread/recoil,
  damage by hitgroup and range, penetration, reloads and weapon speeds;
  the knife first. View models and world models from the install.

## dust2 fidelity

Counts are from de_dust2's entity lump and static prop lump.

- **Baked per-vertex prop lighting (`.vhv`)** for maps that ship it (dust2
  doesn't; its props use the per-prop light probe, as in the game).
- **Remaining decals**: 6 of dust2's 135 sit on props or brush entities
  rather than world faces; decals on displacements (none on dust2).
- **Verify inferred Source rules** with the comparison tool, using a local
  copy of a map with test entities added where dust2 has no example: floor
  and ceiling decal orientation, decal reach, overall brightness/tonemapping.
- **Physics props, remaining**: bullet and explosion impulses (with
  weapons), the player physics shadow for `prop_physics` (dust2 has none),
  the model's `prop_data` physicsmode override, impact sounds.
- **Brush entities**: doors and visible `func_brush` if a map needs them
  (dust2's one `func_brush` is render mode 10, never drawn).
- **Prop collision from `.phy`** for physics-solid props (now the visible
  mesh).
- **Fire** (`env_fire`, 16) and other effects, if they show in normal play.
- **Tunnel lamp glows**: iterate on the brightness of the billboard
  glows (env_sprite) in dust2's tunnels against CS:S (refcmp
  `glow_lamp`, `glow_lamp_down`; RenderDoc a lamp draw for the sprite
  shader's colour, alpha and scale).
- **Reflective floors**: the lower tunnels' floor (and other `$envmap`
  materials, incl. map-patched ones pointing at `env_cubemap`) should
  reflect: read the BSP's cubemap lump and baked cubemaps, pick the
  nearest per surface, apply `$envmaptint`, `$envmapmask` / base or
  normal-map alpha masks and fresnel as the shader spec describes;
  compare in refcmp.
- **Materials**: env maps (2 dust2 materials, plus map-patched ones with
  `env_cubemap`); detail blend modes other than 0 and 1; `$basetexturetransform`
  (unused on dust2).
- **Lightmap styles** (switchable lights).
- **Tonemap** (`env_tonemap_controller`; HDR only). Fog on ropes.
- **Water surfaces**: swimming works (`MapWater`), but water faces draw as
  plain textured surfaces, without the Water shader's refraction,
  reflection or fog. Water currents (base velocity) aren't applied.
- **Sound** (map): `ambient_generic` and soundscapes, once there's an audio slot.
