//! Ragdolls in the running simulation (specs/cs_source/ragdolls.md): a
//! character with a small made-up skeleton and ragdoll is killed on a
//! floor; its body falls, settles without exploding, and goes away when
//! the character lives again.

use std::sync::Arc;

use avian3d::prelude::Position;
use bevy::prelude::*;
use mashup::{
    core::{Damage, DamageKind, Health, Hitbox, Hitgroup},
    harness::Sim,
    map::{
        BoneBox, MapBone, MapBrush, MapCharacterModel, MapData, MapModel, MapPlugin, MapRagdoll, MapRagdollBody,
        MapRagdollJoint, Ragdoll, RagdollBody,
        RagdollShot,
        anim::{AnimSet, Animation, Sequence, Track},
        ragdoll::{DeathSide, RagdollJoint, Ragdolled, death_pose_frame, needs_repair},
    },
    movement::noclip,
    rules::Deathmatch,
};

const SCALE: f32 = 0.0254;

/// A box's corners, inches.
fn cube(lo: Vec3, hi: Vec3) -> Vec<Vec3> {
    (0..8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
        })
        .collect()
}

fn bone(name: &str, parent: Option<usize>, at: Vec3) -> MapBone {
    MapBone {
        name: name.into(),
        parent,
        position: at,
        rotation: Quat::IDENTITY,
    }
}

fn body(bone: usize, lo: Vec3, hi: Vec3, mass: f32) -> MapRagdollBody {
    MapRagdollBody {
        bone,
        pieces: vec![cube(lo, hi)],
        mass,
        damping: 0.05,
        rotdamping: 3.0,
        inertia: 10.0,
        surfaceprop: "flesh".into(),
        friction: 0.8,
        elasticity: 0.0,
    }
}

fn joint(parent: usize, child: usize, deg: [(f32, f32); 3]) -> MapRagdollJoint {
    MapRagdollJoint {
        parent,
        child,
        limits: deg.map(|(a, b)| (a.to_radians(), b.to_radians())),
    }
}

/// A Source-style skeleton (inches, Z up, facing +X): pelvis, spine,
/// head, two legs of thigh and calf, and an unsimulated foot.
fn model() -> MapCharacterModel {
    let bones = vec![
        bone("pelvis", None, Vec3::new(0.0, 0.0, 38.0)),
        bone("spine", Some(0), Vec3::new(0.0, 0.0, 6.0)),
        bone("head", Some(1), Vec3::new(0.0, 0.0, 20.0)),
        bone("l_thigh", Some(0), Vec3::new(0.0, 4.0, -2.0)),
        bone("l_calf", Some(3), Vec3::new(0.0, 0.0, -18.0)),
        bone("r_thigh", Some(0), Vec3::new(0.0, -4.0, -2.0)),
        bone("r_calf", Some(5), Vec3::new(0.0, 0.0, -18.0)),
        bone("l_foot", Some(4), Vec3::new(0.0, 0.0, -17.0)),
    ];
    let ragdoll = MapRagdoll {
        bodies: vec![
            body(0, Vec3::new(-4.0, -6.0, -3.0), Vec3::new(4.0, 6.0, 6.0), 10.0),
            body(1, Vec3::new(-4.0, -7.0, 0.0), Vec3::new(4.0, 7.0, 20.0), 30.0),
            body(2, Vec3::new(-4.0, -3.5, 0.0), Vec3::new(4.0, 3.5, 9.0), 5.0),
            body(3, Vec3::new(-3.0, -3.0, -18.0), Vec3::new(3.0, 3.0, 0.0), 9.0),
            body(4, Vec3::new(-2.5, -2.5, -18.0), Vec3::new(2.5, 2.5, 0.0), 6.0),
            body(5, Vec3::new(-3.0, -3.0, -18.0), Vec3::new(3.0, 3.0, 0.0), 9.0),
            body(6, Vec3::new(-2.5, -2.5, -18.0), Vec3::new(2.5, 2.5, 0.0), 6.0),
        ],
        joints: vec![
            joint(0, 1, [(-10.0, 10.0), (-20.0, 30.0), (-16.0, 16.0)]),
            joint(1, 2, [(-30.0, 30.0), (-30.0, 30.0), (-30.0, 30.0)]),
            joint(0, 3, [(-20.0, 20.0), (-60.0, 30.0), (-20.0, 20.0)]),
            joint(3, 4, [(-5.0, 5.0), (0.0, 115.0), (-5.0, 5.0)]),
            joint(0, 5, [(-20.0, 20.0), (-60.0, 30.0), (-20.0, 20.0)]),
            joint(5, 6, [(-5.0, 5.0), (0.0, 115.0), (-5.0, 5.0)]),
        ],
        collision_pairs: Some(Vec::new()),
    };
    let boxes = vec![
        BoneBox {
            bone: 2,
            center: Vec3::new(0.0, 0.0, 4.5),
            half: Vec3::new(4.0, 3.5, 4.5),
            group: Hitgroup::Head,
        },
        BoneBox {
            bone: 1,
            center: Vec3::new(0.0, 0.0, 10.0),
            half: Vec3::new(4.0, 7.0, 10.0),
            group: Hitgroup::Chest,
        },
    ];
    // Source axes (x, y, z) to ours (x, z, -y), turned to face -Z, inches
    // to meters: as the CS:S loader does.
    let face = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let axes = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    MapCharacterModel {
        team: None,
        model: MapModel::default(),
        hitboxes: boxes
            .iter()
            .map(|b| Hitbox {
                center: Vec3::ZERO,
                half: b.half * SCALE,
                rotation: Quat::IDENTITY,
                group: b.group,
            })
            .collect(),
        bones,
        root: Transform::from_rotation(face * axes).with_scale(Vec3::splat(SCALE)),
        animations: None,
        boxes,
        ragdoll: Some(ragdoll),
    }
}

/// A floor whose top is at y = 0 and the model, with a character standing
/// on it (noclip: it stays put).
fn sim() -> (Sim, Entity) {
    sim_with(model())
}

fn sim_with(model: MapCharacterModel) -> (Sim, Entity) {
    let (lo, hi) = (Vec3::new(-20.0, -1.0, -20.0), Vec3::new(20.0, 0.0, 20.0));
    let data = MapData {
        name: "test:ragdoll".into(),
        collision_brushes: vec![MapBrush::from_box(lo, hi)],
        collision_hulls: vec![cube(lo, hi).iter().map(|v| v.to_array()).collect()],
        characters: vec![model],
        ..default()
    };
    let mut sim = Sim::new(MapPlugin::new(data));
    // The dead stay dead for the test.
    sim.app.world_mut().resource_mut::<Deathmatch>().respawn_delay = 1000.0;
    let p = sim.spawn_character(Vec3::new(0.0, 0.9, 0.0), noclip::ID);
    sim.ticks(10);
    (sim, p)
}

fn kill(sim: &mut Sim, p: Entity, hitgroup: Hitgroup, point: Vec3, dir: Vec3) {
    sim.app.world_mut().write_message(Damage {
        target: p,
        attacker: None,
        amount: 5.0,
        point,
        dir,
        hitgroup,
        kind: DamageKind::Bullet,
    });
}

fn ragdoll(sim: &mut Sim) -> Option<Ragdoll> {
    let world = sim.app.world_mut();
    world.query::<&Ragdoll>().iter(world).next().cloned()
}

fn bodies(sim: &mut Sim) -> Vec<(usize, Transform, Vec3)> {
    let world = sim.app.world_mut();
    let mut q = world.query::<(&RagdollBody, &Transform, &avian3d::prelude::LinearVelocity)>();
    let mut out: Vec<_> = q.iter(world).map(|(b, t, v)| (b.index, *t, v.0)).collect();
    out.sort_by_key(|b| b.0);
    out
}

/// The largest distance between a joint's anchor on its parent body and
/// its child body's origin, m.
fn worst_separation(sim: &mut Sim) -> f32 {
    let world = sim.app.world_mut();
    let joints: Vec<RagdollJoint> = world.query::<&RagdollJoint>().iter(world).cloned().collect();
    joints
        .iter()
        .map(|j| {
            let p = world.get::<Transform>(j.parent).unwrap();
            let c = world.get::<Transform>(j.child).unwrap();
            p.transform_point(j.anchor).distance(c.translation)
        })
        .fold(0.0, f32::max)
}

#[test]
fn a_killed_character_falls_and_settles_on_the_floor() {
    let (mut sim, p) = sim();
    assert!(ragdoll(&mut sim).is_none());
    // Shot in the head from behind (its back faces +Z): pushed along -Z.
    let head = Vec3::new(0.0, 1.6, 0.0);
    kill(&mut sim, p, Hitgroup::Head, head, Vec3::NEG_Z);
    sim.ticks(1);
    let r = ragdoll(&mut sim).expect("a ragdoll");
    assert_eq!(r.bodies.len(), 7);
    assert_eq!(r.joints.len(), 6);
    assert!(sim.app.world().get::<Ragdolled>(p).is_some());
    // Bodies start at the bones: the head body at its bone, 64 in up.
    let start = bodies(&mut sim);
    assert!((start[2].1.translation.y - 64.0 * SCALE).abs() < 0.05, "{:?}", start[2].1);
    // The hit body got the push.
    assert!(start[2].2.z < -1.0, "head velocity {}", start[2].2);
    let mut worst = 0.0f32;
    for _ in 0..(64 * 4) {
        sim.ticks(1);
        worst = worst.max(worst_separation(&mut sim));
    }
    assert!(worst < 0.03, "joints pulled apart by {worst} m");
    for (i, t, v) in bodies(&mut sim) {
        assert!(t.translation.is_finite(), "body {i}");
        assert!(t.translation.y > -0.05, "body {i} sank through the floor: {}", t.translation);
        assert!(t.translation.y < 0.4, "body {i} is still up: {}", t.translation);
        assert!(t.translation.xz().length() < 2.0, "body {i} flew off: {}", t.translation);
        assert!(v.length() < 0.2, "body {i} still moving at {v}");
    }
    // It fell the way it was pushed.
    let end = bodies(&mut sim);
    assert!(end[2].1.translation.z < 0.0, "head ended at {}", end[2].1.translation);
}

#[test]
fn the_ragdoll_goes_when_its_owner_lives_again() {
    let (mut sim, p) = sim();
    kill(&mut sim, p, Hitgroup::Chest, Vec3::new(0.0, 1.2, 0.0), Vec3::X);
    sim.ticks(10);
    assert!(ragdoll(&mut sim).is_some());
    sim.app.world_mut().get_mut::<Health>(p).unwrap().current = 1.0;
    sim.ticks(2);
    assert!(ragdoll(&mut sim).is_none());
    assert!(bodies(&mut sim).is_empty());
    assert!(sim.app.world().get::<Ragdolled>(p).is_none());
}

/// The model plus a left upper arm hanging at the shoulder, left of the
/// torso (Source +Y), that swings freely; `pairs` are its self-collision
/// rules (body 7 is the arm, 1 the torso).
fn armed_model(pairs: Vec<(usize, usize)>) -> MapCharacterModel {
    let mut m = model();
    m.bones.push(bone("l_upperarm", Some(1), Vec3::new(0.0, 10.0, 12.0)));
    let r = m.ragdoll.as_mut().unwrap();
    r.bodies
        .push(body(8, Vec3::new(-2.0, -2.0, -14.0), Vec3::new(2.0, 2.0, 0.0), 3.0));
    r.joints.push(joint(1, 7, [(-170.0, 170.0), (-170.0, 170.0), (-170.0, 170.0)]));
    r.collision_pairs = Some(pairs);
    let arm = BoneBox {
        bone: 8,
        center: Vec3::new(0.0, 0.0, -7.0),
        half: Vec3::new(2.0, 2.0, 7.0),
        group: Hitgroup::LeftArm,
    };
    m.hitboxes.push(Hitbox {
        center: Vec3::ZERO,
        half: arm.half * SCALE,
        rotation: Quat::IDENTITY,
        group: arm.group,
    });
    m.boxes.push(arm);
    m
}

/// How deep the arm's axis got inside the torso body over `ticks`, inches
/// (0: never inside; the arm is 2 in thick, so touching leaves its axis
/// 2 in outside).
fn deepest_arm_in_torso(sim: &mut Sim, ticks: usize) -> f32 {
    let mut deepest = 0.0f32;
    for _ in 0..ticks {
        sim.ticks(1);
        let b = bodies(sim);
        let (torso, arm) = (b[1].1, b[7].1);
        for z in [2.0, 5.0, 8.0, 11.0, 14.0] {
            let p = arm.transform_point(Vec3::new(0.0, 0.0, -z * SCALE));
            let l = torso.rotation.inverse() * (p - torso.translation) / SCALE;
            // Distance inside each face of the torso box (x ±4, y ±7, z 0..20).
            let inside = (4.0 - l.x.abs()).min(7.0 - l.y.abs()).min(l.z).min(20.0 - l.z);
            deepest = deepest.max(inside);
        }
    }
    deepest
}

/// Spec 1.3: a shot in the arm throws it across the chest as the body
/// falls. With the arm-torso pair listed it stays outside the torso;
/// without it (the control) it passes through.
#[test]
fn listed_pairs_keep_limbs_out_of_the_torso() {
    let hit = Vec3::new(-10.0 * SCALE, 49.0 * SCALE, 0.0);
    let (mut sim, p) = sim_with(armed_model(vec![(1, 7)]));
    // The arm is at world -X (the character faces -Z); push it toward +X.
    kill(&mut sim, p, Hitgroup::LeftArm, hit, Vec3::X);
    let with_pair = deepest_arm_in_torso(&mut sim, 128);
    let (mut sim, p) = sim_with(armed_model(Vec::new()));
    kill(&mut sim, p, Hitgroup::LeftArm, hit, Vec3::X);
    let without = deepest_arm_in_torso(&mut sim, 128);
    assert!(without > 2.0, "control: the arm should cross the chest, got {without} in");
    assert!(with_pair < 0.5, "the arm went {with_pair} in into the torso");
}

#[test]
fn collision_masks_follow_the_rules() {
    let m = armed_model(vec![(1, 7), (7, 0)]);
    let r = m.ragdoll.as_ref().unwrap();
    let masks = r.collision_masks();
    assert_eq!(masks[7], 0b11);
    assert_eq!(masks[1], 1 << 7);
    assert_eq!(masks[2], 0);
    // No rules: everything but joint neighbours.
    let mut none = r.clone();
    none.collision_pairs = None;
    let masks = none.collision_masks();
    assert_eq!(masks[0] & (1 << 1), 0, "pelvis-spine are joined");
    assert_ne!(masks[0] & (1 << 2), 0, "pelvis-head collide");
    assert_eq!(masks[3] & (1 << 4), 0, "thigh-calf are joined");
    assert_ne!(masks[4] & (1 << 6), 0, "calf-calf collide");
}

/// Spec 6.2: a bullet through a lying body pushes its pelvis at the hit
/// point and wakes it; a shot that misses does nothing.
#[test]
fn bullets_push_a_dead_body() {
    let (mut sim, p) = sim();
    kill(&mut sim, p, Hitgroup::Chest, Vec3::new(0.0, 1.2, 0.0), Vec3::NEG_Z);
    // Settled and asleep (5 s of stillness).
    sim.seconds(8.0);
    let root = ragdoll(&mut sim).unwrap().bodies[0];
    assert!(sim.app.world().get::<avian3d::prelude::Sleeping>(root).is_some(), "asleep before the shot");
    let before = bodies(&mut sim)[0].1;
    // Across the pelvis at its height, toward +X.
    let at = before.transform_point(Vec3::new(0.0, 0.0, 1.5 * SCALE));
    sim.app.world_mut().write_message(RagdollShot {
        from: at - Vec3::X,
        to: at + Vec3::X,
        blast: false,
    });
    sim.ticks(1);
    assert!(sim.app.world().get::<avian3d::prelude::Sleeping>(root).is_none(), "woken");
    let v = bodies(&mut sim)[0].2;
    assert!(v.x > 1.0, "pelvis pushed along the shot: {v}");
    sim.ticks(16);
    let after = bodies(&mut sim)[0].1;
    assert!(
        after.translation.x - before.translation.x > 0.03,
        "moved {}",
        after.translation - before.translation
    );
    // A shot that misses does nothing.
    let (mut sim, p) = self::sim();
    kill(&mut sim, p, Hitgroup::Chest, Vec3::new(0.0, 1.2, 0.0), Vec3::NEG_Z);
    sim.seconds(8.0);
    let still = bodies(&mut sim)[0].1;
    sim.app.world_mut().write_message(RagdollShot {
        from: Vec3::new(-1.0, 1.5, 0.0),
        to: Vec3::new(1.0, 1.5, 0.0),
        blast: false,
    });
    sim.ticks(8);
    assert!(bodies(&mut sim)[0].1.translation.distance(still.translation) < 1e-3);
}

/// The model with a front death pose: frames 1–6 bend the spine 30° back
/// (its top, and the head on it, toward Source -X).
fn posed_model() -> MapCharacterModel {
    let mut m = model();
    let bind: Vec<(Quat, Vec3)> = m.bones.iter().map(|b| (b.rotation, b.position)).collect();
    let upright = bind[1].0;
    let bent = Quat::from_rotation_y(-30f32.to_radians()) * upright;
    let set = AnimSet {
        defaults: bind.clone(),
        bases: vec![bind],
        animations: vec![Animation {
            name: "@deathpose_front".into(),
            fps: 1.0,
            frames: 7,
            delta: false,
            base: 0,
            tracks: vec![Track {
                bone: 1,
                rotation: Some((0..7).map(|f| if f == 0 { upright } else { bent }).collect()),
                position: None,
            }],
        }],
        sequences: vec![Sequence {
            name: "deathpose_front".into(),
            activity: "ACT_DIE_FRONTSIDE".into(),
            grid: (1, 1),
            anims: vec![0],
            bone_weights: vec![1.0; m.bones.len()],
            ..Default::default()
        }],
        ..Default::default()
    };
    m.animations = Some(Arc::new(set));
    m
}

/// Spec 2.3: shot from the front, the bones' velocities aim at the front
/// death pose: the spine turns back and the head goes back with it (world
/// +Z: the character faces -Z); without the pose they stay put.
#[test]
fn a_death_pose_seeds_the_velocities() {
    let head_speed = |m: MapCharacterModel| {
        let (mut sim, p) = sim_with(m);
        // The stomach has no hitbox here: no impulse, only the pose.
        kill(&mut sim, p, Hitgroup::Stomach, Vec3::new(0.0, 1.0, 0.0), Vec3::Z);
        sim.ticks(1);
        let r = ragdoll(&mut sim).expect("a ragdoll");
        (r.death_pose, bodies(&mut sim)[2].2)
    };
    let (pose, head) = head_speed(posed_model());
    assert_eq!(pose, Some((DeathSide::Front, 2)));
    // The spine's top moves 10 in over the 0.05-0.0625 s look-back: about
    // 4 m/s, shared with the rest of the body through the joints.
    assert!(head.z > 1.5, "head thrown back: {head}");
    let (pose, still) = head_speed(model());
    assert_eq!(pose, None);
    assert!(still.z.abs() < 0.3, "no pose, no throw: {still}");
    // Shot from behind: the back pose, which this model lacks: no pose.
    let (mut sim, p) = sim_with(posed_model());
    kill(&mut sim, p, Hitgroup::Stomach, Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z);
    sim.ticks(1);
    assert_eq!(ragdoll(&mut sim).unwrap().death_pose, None);
}

#[test]
fn death_pose_side_and_frame() {
    // Source axes, yaw 0: facing +X, right -Y (spec test cases).
    assert_eq!(DeathSide::from_force(Vec3::NEG_X), Some(DeathSide::Front));
    assert_eq!(DeathSide::from_force(Vec3::Y), Some(DeathSide::Right));
    assert_eq!(DeathSide::from_force(Vec3::NEG_Y), Some(DeathSide::Left));
    assert_eq!(DeathSide::from_force(Vec3::X), Some(DeathSide::Back));
    assert_eq!(DeathSide::from_force(Vec3::ZERO), None);
    // Ties go to front/back.
    assert_eq!(DeathSide::from_force(Vec3::new(-1.0, 1.0, 0.0)), Some(DeathSide::Front));
    assert_eq!(death_pose_frame(Hitgroup::Head), 1);
    assert_eq!(death_pose_frame(Hitgroup::Stomach), 2);
    assert_eq!(death_pose_frame(Hitgroup::LeftLeg), 5);
    assert_eq!(death_pose_frame(Hitgroup::Generic), 1);
    assert_eq!(DeathSide::Left.activity(true), "ACT_DIE_CROUCH_LEFTSIDE");
    assert_eq!(DeathSide::Back.sequence(false), "deathpose_back");
}

/// Spec 6.3 test cases: a light child is put back, a heavy one in the
/// open isn't.
#[test]
fn separation_repair_rules() {
    let gap = 2.0 * SCALE;
    assert!(needs_repair(gap, 4.542068, 31.036686, false, false), "head on Spine2");
    assert!(!needs_repair(gap, 23.843636, 1.912537, false, false), "Spine1 on the pelvis");
    assert!(needs_repair(gap, 23.843636, 1.912537, false, true), "unless something is in between");
    assert!(needs_repair(0.0, 23.843636, 1.912537, true, false), "after its parent");
    assert!(!needs_repair(0.5 * SCALE, 1.0, 30.0, false, false), "within an inch");
}

/// A head held 0.3 m from its joint for 15 ticks is moved back onto it.
#[test]
fn a_joint_held_apart_gets_repaired() {
    let (mut sim, p) = sim();
    kill(&mut sim, p, Hitgroup::Chest, Vec3::new(0.0, 1.2, 0.0), Vec3::NEG_Z);
    sim.ticks(2);
    let head = ragdoll(&mut sim).unwrap().bodies[2];
    let mut repaired_at = None;
    for tick in 0..30 {
        sim.app.world_mut().get_mut::<Position>(head).unwrap().0 += Vec3::Y * 0.3;
        sim.ticks(1);
        if ragdoll(&mut sim).unwrap().repairs > 0 {
            repaired_at = Some(tick);
            break;
        }
    }
    let tick = repaired_at.expect("repaired");
    assert!(tick >= 14, "repaired after {tick} ticks, before the error lasted 15");
    assert!(worst_separation(&mut sim) < 0.03, "{}", worst_separation(&mut sim));
}

#[test]
fn bodies_take_their_surface_friction() {
    let mut m = model();
    for b in &mut m.ragdoll.as_mut().unwrap().bodies {
        b.friction = 0.35;
    }
    let (mut sim, p) = sim_with(m);
    kill(&mut sim, p, Hitgroup::Chest, Vec3::new(0.0, 1.2, 0.0), Vec3::NEG_Z);
    sim.ticks(1);
    let pelvis = ragdoll(&mut sim).unwrap().bodies[0];
    let f = sim.app.world().get::<avian3d::prelude::Friction>(pelvis).unwrap();
    assert_eq!(f.dynamic_coefficient, 0.35);
}
