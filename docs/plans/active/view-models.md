# First-person view models

Started 2026-10-06 (backlog 1C). The local player sees no weapon; this
draws CS:S's view models (`v_knife_t.mdl`, `v_rif_ak47.mdl`) in front of
the camera, playing their own draw, idle, fire and reload sequences.

## Plan

A loader (new model kind), so the plan comes first (CLAUDE.md).

- **Data** (`map`, game-neutral): `MapViewModel` in `MapData::view_models`
  keyed like `MapHeldModel` (the weapon ID): skinned meshes, its own
  skeleton (`MapBone`s), the root transform from the skeleton's space to
  the eye's (Source: x forward, y left, z up, inches; ours: -Z forward,
  meters) and its `AnimSet`.
- **Loading** (`games/cs_source/props.rs::load_view_model`): the same
  vmdl mesh conversion as player bodies (reference-pose vertices,
  skinning weights) and our own `.mdl` decoder (`anim::load`) for the
  sequences. Paths in `weapons.rs::VIEW_MODELS` (the scripts'
  `viewmodel`), loaded with the map's characters.
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
  `TimeToIdle`) that long after the last attack.
- **Drawing** (`map/view_model.rs`, render only): the client marks its
  first-person camera with `ViewModelAnchor`; the map adds a child camera
  on its own render layer (`VIEW_MODEL_LAYER`), drawn after the world with
  its own depth buffer (so the model never clips into walls) and its own
  FOV, and the skinned model under it, posed each frame from the owner's
  `ViewAnimator`. Lights include the layer.
- **Tests**: the real AK view model loads with a mesh, a skeleton and the
  expected activities (skipped without an install); a headless scenario on
  the greybox with a stand-in `AnimSet`: deploying plays draw, firing
  switches to a fire sequence, reloading to reload, then idle.

## Findings

- `dump cs_source --sequences <model>` lists a model's bones and
  sequences. `v_rif_ak47`: 63 bones; `ak47_idle` (looping 0.5 s),
  `ak47_fire1..3` (`ACT_VM_PRIMARYATTACK`, 0.75 s), `ak47_draw` (1.0 s),
  `ak47_reload` (2.4324 s). `v_knife_t` (the only knife view model in the
  install, both teams): 60 bones; `idle` (non-looping, 12.5 s), `draw`
  (1.0 s), `stab`, `midslash1`, `midslash2` (all `ACT_VM_HITCENTER`),
  `stab_miss` (`ACT_VM_MISSCENTER`). All activity weights are 1.
- The knife's attacks can't be told apart by activity, so the driver picks
  them by sequence name (spec 6.2 names them).
- The models are left-handed in the files (the AK sits left of the eye);
  CS:S shows them in the right hand by default (`cl_righthand 1`), so they
  are drawn mirrored (scale -1 on X, front-face culling).
- Skinned meshes keep the reference pose's bounds, which lie outside the
  view; view models skip frustum culling.

## Slices

1. [x] Load `v_rif_ak47` / `v_knife_t` (meshes, skeleton, sequences);
   test (`tests/map_de_dust2.rs::view_models_load_with_their_sequences`).
2. [x] `ViewAnimator` and the CS:S driver from weapon events; scenario
   tests (`tests/view_models.rs`).
3. [x] Draw it over the world at the eye (separate camera and layer);
   screenshots idle, firing and the knife.
4. [ ] Bob and sway, muzzle flash and shell ejection at the attachments,
   a `cl_righthand` cvar, hiding it in third person (remove
   `ViewModelAnchor` from the camera, or move it).

## Open questions

- **View model FOV.** CS:S's `viewmodel_fov` default is taken as 54
  (horizontal, at 4:3, like `fov 90`), so vertically 2·atan(tan(27°)·3/4)
  ≈ 41.8°, kept constant on wider screens as the world camera does. Not
  from a spec; check against a CS:S screenshot (refcmp could capture one).
- **Cross-fades between view-model sequences.** Spec animation.md §9
  describes the client cross-fade for any model; whether CS:S applies it
  to view models (and with which fade time) isn't specced. We use the
  file's fade-in/fade-out values like bodies do.
- **Idle timing.** Spec weapons.md 3.7: idle after `TimeToIdle` (AK 1.9 s)
  "probably"; the knife has no listed value, so it idles when its
  sequence ends.
- **Lighting.** Source lights view models from the light at the eye
  (ambient cube and world lights, like props). Ours uses the same
  `StandardMaterial` + sun + ambient as character bodies, unshadowed.
- **Handedness.** Mirroring is from memory of CS:S (`cl_righthand 1`
  default, left-handed models); it puts the AK in the right hand. Whether
  the knife looks right mirrored (its draw shows the blade in the left
  hand) needs a CS:S screenshot.
- **Bob and sway** (view model lag behind the view, walking bob) are
  client code with no spec yet.
