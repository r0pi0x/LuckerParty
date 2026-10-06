//! Ragdolls in the running simulation (specs/cs_source/ragdolls.md): a
//! character with a small made-up skeleton and ragdoll is killed on a
//! floor; its body falls, settles without exploding, and goes away when
//! the character lives again.

use bevy::prelude::*;
use mashup::{
    core::{Damage, DamageKind, Health, Hitbox, Hitgroup},
    harness::Sim,
    map::{
        BoneBox, MapBone, MapBrush, MapCharacterModel, MapData, MapModel, MapPlugin, MapRagdoll, MapRagdollBody,
        MapRagdollJoint, Ragdoll, RagdollBody,
        ragdoll::{Ragdolled, RagdollJoint},
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
        collision_pairs: Vec::new(),
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
    let (lo, hi) = (Vec3::new(-20.0, -1.0, -20.0), Vec3::new(20.0, 0.0, 20.0));
    let data = MapData {
        name: "test:ragdoll".into(),
        collision_brushes: vec![MapBrush::from_box(lo, hi)],
        collision_hulls: vec![cube(lo, hi).iter().map(|v| v.to_array()).collect()],
        characters: vec![model()],
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
