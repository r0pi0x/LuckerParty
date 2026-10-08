# First-person view models

Started 2026-10-06 (backlog 1C). The local player sees no weapon; this
draws CS:S's view models (`v_knife_t.mdl`, `v_rif_ak47.mdl`) in front of
the camera, playing their own draw, idle, fire and reload sequences.
Slice 4 follows specs/cs_source/view_models.md (draft).

## Plan

A loader (new model kind), so the plan comes first (CLAUDE.md).

- **Data** (`map`, game-neutral): `MapViewModel` in `MapData::view_models`
  keyed like `MapHeldModel` (the weapon ID): skinned meshes, its own
  skeleton (`MapBone`s), the root transform from the skeleton's space to
  the eye's (Source: x forward, y left, z up, inches; ours: -Z forward,
  meters), its `AnimSet` (sequences with their animation events), its
  handedness (`right_handed`, `allow_flipping`), attachments and lighting
  origin (the model's illumination centre).
- **Loading** (`games/cs_source/props.rs::load_view_model`): the same
  vmdl mesh conversion as player bodies (reference-pose vertices,
  skinning weights) and our own `.mdl` decoder (`anim::load`) for the
  sequences and events, `anim::attachments` for attachments and the
  illumination centre. Paths and handedness in `weapons.rs::VIEW_MODELS`,
  loaded with the map's characters.
- **Sequence state** (simulation side, testable headless): a
  `ViewAnimator` component on characters (the weapon key and an
  `Animator` on the view model's `AnimSet`). `games/cs_source/view_anim.rs`
  drives it from `WeaponEvent`s (spec weapons.md 3.3–3.8): a new active
  weapon or `Deployed` plays `ACT_VM_DRAW`; `Shot` plays
  `ACT_VM_PRIMARYATTACK` (weighted random among the model's sequences with
  that activity, restarted at cycle 0); knife `Swing`s play the hit or
  miss activity for slash/stab; `ReloadStarted` plays `ACT_VM_RELOAD`.
  Non-looping sequences hold their last frame; the idle activity follows
  once the sequence has finished and (for weapons with a script
  `TimeToIdle`) that long after the last attack. Animation events the
  cycle passes (re-armed by every restart) become `map::ViewModelEvent`s:
  5001 → muzzle flash on attachment `1`, `AE_CLIENT_EFFECT_ATTACH
  EjectBrass_<type> <attachment> <speed>` → a shell.
- **Bob and sway** (`games/cs_source/view_motion.rs`, spec 3–4): the
  cvar-driven bob (tied to distance, slew-limited speed) then the
  history-based sway (`cl_wpn_sway_interp` 0.1 s lag), on the eye angles
  including view punch; written as the local player's
  `map::ViewModelOffset` (camera space), which the drawing mirrors with
  the model.
- **Drawing** (`map/view_model.rs`, render only): the client marks its
  first-person camera with `ViewModelAnchor`; the map adds a child camera
  on its own render layer (`VIEW_MODEL_LAYER`), drawn after the world with
  its own depth buffer (so the model never clips into walls) and its own
  FOV (`ViewModelSettings`: `viewmodel_fov` 54 at 4:3, zoomed by as many
  degrees as the player's view, constant vertically), and the skinned
  model under it, posed each frame from the owner's `ViewAnimator`,
  placed by the offset and mirrored (scale −1 on X, front faces culled)
  when `right_handed != cl_righthand`.
- **Lighting** (spec 9): `PropMaterial` with a per-pixel probe
  (`PropParams::set_probe`: ambient cube + up to four directional lights,
  the same model props bake per vertex), sampled from the map's
  `LightField` (CS:S: `props::probe_with`, the props' query, at run time)
  at the model's lighting origin placed at the eye (not mirrored); the
  nearest `env_cubemap` for reflections; plus the scene's point lights.
- **Muzzle flash** (spec 7; look is our choice, see open questions):
  `MapMuzzleFlash` from cs_source (`view_anim::muzzle_flash`): three
  view-facing additive sprites of CS:S's `effects/muzzleflashx` strung
  along the model's forward from attachment `1`, re-projected into the
  world pass (spec 5), 0.05 s; and a `DynamicLight` (a pooled Bevy
  `PointLight`) that `world.wgsl` and `prop.wgsl` read from Bevy's light
  clusters, so it lights world surfaces, props and view models. Other
  characters (and the local player in third person) flash at their held
  world model's `muzzle_flash` attachment.
- **Shells** (`map/shells.rs`, spec 8): on the brass event, the shell
  model (`models/shells/shell_<type>.mdl`) leaves the view model's
  attachment `2` (re-projected), flies under full gravity with the spec's
  speeds and spin, bounces at half speed (a bounce sound one hit in six),
  rests on floors, fades after 10 s; drawn in the view-model pass until
  its first bounce.
- **Console** (cs_source): `viewmodel_fov`, `cl_righthand`,
  `muzzleflash_light` (archived), `r_drawviewmodel`, `cl_bobcycle`,
  `cl_bobup`, `cl_wpn_sway_interp`, `cl_wpn_sway_scale`, `cl_ejectbrass`.
- **Tests**: the real AK view model loads with a mesh, a skeleton, the
  expected activities, attachments, events and lighting origin, and the
  knife/AK handedness in the files (`tests/it/heavy/map_de_dust2.rs`; skipped
  without an install); scenario tests on the greybox with stand-in
  `AnimSet`s (`tests/it/view_models.rs`: deploy/fire/reload/idle, spec E1);
  the spec's B, S, F, H and E cases as pure maths
  (`tests/it/view_model_spec.rs`); cvars (`view_anim` unit test).

## Findings

- `dump cs_source --sequences <model>` lists a model's bones, sequences
  with their events, attachments and illumination position.
  `v_rif_ak47`: 63 bones; `ak47_idle` (looping 0.5 s),
  `ak47_fire1..3` (`ACT_VM_PRIMARYATTACK`, 0.75 s), `ak47_draw` (1.0 s),
  `ak47_reload` (2.4324 s). `v_knife_t` (the only knife view model in the
  install, both teams): 60 bones; `idle` (non-looping, 12.5 s), `draw`
  (1.0 s), `stab`, `midslash1`, `midslash2` (all `ACT_VM_HITCENTER`),
  `stab_miss` (`ACT_VM_MISSCENTER`). All activity weights are 1.
- The knife's attacks can't be told apart by activity, so the driver picks
  them by sequence name (spec 6.2 names them).
- **Handedness.** The AK is held left of the eye in its file (left-handed,
  gripped by `Left_Hand`), but the knife is held right of the eye
  (`knife_Parent` at y = −6.2, in `Right_Hand`; its illumination centre is
  on the right too). The spec's rule (mirror when `AllowFlipping` and
  `BuiltRightHanded ≠ cl_righthand`) plus the observed result (both in the
  right hand at `cl_righthand 1`, both in the left at 0) therefore means
  the knife's script says `BuiltRightHanded 1`, not 0 as the spec guessed.
  We mirrored both, which put the knife blade in the left hand; now
  `VIEW_MODELS` carries the handedness per weapon
  (`view_model_handedness_in_the_files` checks the files).
- The AK's attachment axes point sideways in model space (attachment `1`'s
  X is the model's left), so the flash plume follows the model's forward,
  not the attachment's X.
- Skinned meshes keep the reference pose's bounds, which lie outside the
  view; view models skip frustum culling.
- New Bevy point lights reach the light clusters only a frame or two after
  they are spawned, longer than a flash lasts; flash lights are pooled
  (kept, turned off) instead of spawned per shot.

## Slices

1. [x] Load `v_rif_ak47` / `v_knife_t` (meshes, skeleton, sequences);
   test (`tests/it/heavy/map_de_dust2.rs::view_models_load_with_their_sequences`).
2. [x] `ViewAnimator` and the CS:S driver from weapon events; scenario
   tests (`tests/it/view_models.rs`).
3. [x] Draw it over the world at the eye (separate camera and layer);
   screenshots idle, firing and the knife.
4. [x] Spec view_models.md: `viewmodel_fov`/`cl_righthand` cvars,
   handedness per model (knife fixed), bob and sway, muzzle flash with a
   light on world/props/view model, lighting at the eye, shells.
5. [ ] Check the open questions against CS:S (see below) and promote the
   spec from draft.

## Open questions

- **Muzzle flash look** (spec Q5). Our choice: `effects/muzzleflashx`, three
  sprites 8, 6.5 and 5 units wide at 0, 3 and 6 units along the model's
  forward, random size ×0.8–1.2, white, 0.05 s; the `muzzle_*` particle
  systems are not used. The light is the SDK's generic one (colour
  (255, 192, 64) exponent 5, radius 32–64 units, 0.05 s, radius shrinking
  to 0) with its colour scaled to 1/16 (our choice: ×32 burns walls out)
  and falloff 1 − d²/r² × cosine (our choice; the engine's dynamic-light
  falloff is not public); it lights world, props and view models, not
  just models as the generic light does, because CS:S visibly lights
  walls. It is kept 0.2 m in front of a wall the barrel pokes into.
  `MuzzleFlashScale`/`Style` are not read. Check: dark-room screenshot
  pair with `muzzleflash_light 0/1`, RenderDoc capture of a firing frame.
- **Shell direction** (spec Q6). Following attachment `2`'s axes, the AK's
  casings leave to the right of the gun in the file, so with the default
  mirror they fly left across the screen. Real AKs eject right; CS:S's
  brass callback isn't public. Check in CS:S which way casings fly with
  `cl_righthand 1`. Also unchecked: yaw negation and ×0.9 angles on
  bounce (we damp the rotation toward rest), random bounce pitch.
- **Near plane.** Spec: 1 unit. That cuts into the knife's hands in its
  idle pose here, so we use 1 cm. Our view-model pass also has its own
  depth buffer instead of the [0, 0.1] depth range, so walls closer than
  ~7.8 units never occlude it (spec D1).
- **View model FOV.** `viewmodel_fov` 54 is now from the spec; vertical
  41.8°. The world camera is now exactly `fov 90` at 4:3 (73.74°
  vertical, was 74°) so the zoom rule sees no zoom by default.
- **Lighting** (spec Q7). We sample at the illumination centre, as the
  spec infers; `r_lightinterp` smoothing and `r_ambient*` boosts are not
  done; the probe is re-sampled when the point moves 0.5 units. The hands'
  `$envmap` mask is in their normal map's alpha, which `PropMaterial`
  doesn't sample, so the hands get no reflection.
- **Bob** (spec Q1–2): the TF2-style model the spec measured; state is one
  per local player (not per weapon); no ground check. The HL2MP
  alternative (B13) is not implemented.
- **Cross-fades between view-model sequences.** Spec animation.md §9
  describes the client cross-fade for any model; whether CS:S applies it
  to view models (and with which fade time) isn't specced. We use the
  file's fade-in/fade-out values like bodies do.
- **Idle timing.** Spec weapons.md 3.7: idle after `TimeToIdle` (AK 1.9 s)
  "probably"; the knife has no listed value, so it idles when its
  sequence ends.
- **Not done:** sound events on view-model sequences (5004:
  `Weapon_AK47.BoltPull` etc.), shake at 10 %, brass for other players
  (world models have no ejection attachment), `viewmodel_fov` is archived
  here though cheat-protected in CS:S.
