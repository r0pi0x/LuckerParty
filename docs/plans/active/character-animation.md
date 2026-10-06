# Character animation

Started 2026-10-06. Characters draw CS:S's player models in their reference
pose; this makes them move.

## Findings

- `models/player/ct_urban.mdl` (and the other player models) hold only a
  ragdoll sequence; their animations come from the included
  `models/player/cs_player_shared.mdl` (49 bones vs. the models' 50; match
  by bone name): 1382 animations, 723 sequences, 5 pose parameters, at
  30 fps, decoded by the `vmdl` crate (per-bone rotation and position per
  frame).
- Lower-body animations: `@Idle_lower` (61 frames), `@Crouch_Idle_Lower`,
  runs `a_Run{N,NE,E,SE,S,SW,W,NW}` (21 frames), walks `a_Walk*`; upper
  body and aim layers per weapon class (`Run_Upper_PISTOL`, ...).
- How CS:S picks and blends sequences (pose parameters move_yaw, body
  yaw, aim pitch/yaw, layers) is game code: a spec is needed for fidelity.

## Slices

1. [x] Skinned meshes: joints from the bones, inverse bind matrices from
   the reference pose, vertex weights from the VVD (looks unchanged).
2. [x] Sample animations from the shared model by bone name; play idle.
   Done: our own `.mdl` animation decoder and include merge
   (`games/cs_source/anim.rs`), the blending and layering math
   (`map/anim.rs`), `Animator` on characters and `pose_bodies`; all spec
   test values match (`tests/animation.rs`).
3. [x] Pick by movement: `games/cs_source/player_anim.rs` follows spec §12
   (9-way move grid, idle/walk/run/crouch/jump, feet yaw lagging the eyes,
   upper-body weapon layers with aim, fire layer). Scenario test
   `bodies_animate_with_movement`.
4. [ ] Hitboxes follow the animated bones (sim side).
5. [x] Spec the CS:S player animation state: specs/cs_source/animation.md.
6. [ ] Reload, grenade and death layers; check the template's numbers
   against CS:S (spec open questions).
