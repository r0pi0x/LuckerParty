//! Ragdoll `.phy` test cases from specs/cs_source/ragdolls.md ("Test
//! cases"), on the player models read from the CS:S install (skipped
//! without one).

use bevy::math::{Quat, Vec3};
use mashup::{
    games::cs_source::{self, anim, mount, phy, props},
    map::MapBone,
    mount::config::LocalConfig,
};

fn read(path: &str) -> Option<Vec<u8>> {
    let config = LocalConfig::load().ok()?;
    let m = mount::open(&config.game_path(cs_source::GAME)?).ok()?;
    m.read(path).ok()
}

fn ragdoll_phy(model: &str) -> Option<phy::PhyRagdoll> {
    let bytes = read(&format!("models/player/{model}.phy"))?;
    Some(phy::parse_ragdoll(&bytes).expect("parses"))
}

fn skeleton(model: &str) -> Option<Vec<MapBone>> {
    let path = format!("models/player/{model}.mdl");
    let bones = anim::bones(&|p| read(p), &path).ok()?;
    Some(
        bones
            .into_iter()
            .map(|(name, parent, rotation, position)| MapBone {
                name,
                parent,
                position,
                rotation,
            })
            .collect(),
    )
}

fn near(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn ct_urban_counts_and_editparams() {
    let Some(p) = ragdoll_phy("ct_urban") else { return };
    assert_eq!(p.solids.len(), 16);
    assert_eq!(p.joints.len(), 15);
    assert_eq!(p.collision_pairs.len(), 24);
    assert!(p.has_collision_rules && p.self_collisions);
    assert!(!p.has_animated_friction);
    let edit = |k: &str| p.editparams.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    assert_eq!(edit("totalmass").and_then(|v| v.parse::<f32>().ok()), Some(100.0));
    assert_eq!(edit("rootname").as_deref(), Some("valvebiped.bip01_pelvis"));
    for (i, s) in p.solids.iter().enumerate() {
        assert_eq!(s.index, i, "index follows text order");
    }
}

#[test]
fn body_masses_and_parameters() {
    let Some(p) = ragdoll_phy("ct_urban") else { return };
    let total: f32 = p.solids.iter().map(|s| s.mass).sum();
    assert!(near(total, 102.636687, 1e-3), "{total}");
    // Body index → expected mass and rotdamping.
    let expected: [(f32, f32); 16] = [
        (1.912537, 3.0),  // pelvis
        (23.843636, 5.0), // Spine1
        (31.036686, 5.0), // Spine2
        (1.0, 6.0),       // R clavicle
        (1.0, 6.0),       // L clavicle
        (1.315652, 2.0),  // L upper arm
        (1.855846, 4.0),  // L forearm
        (1.0, 1.0),       // L hand
        (1.315652, 2.0),  // R upper arm
        (1.855846, 4.0),  // R forearm
        (1.0, 1.0),       // R hand
        (9.428412, 7.0),  // R thigh
        (6.050971, 5.0),  // R calf
        (9.428411, 7.0),  // L thigh
        (6.050970, 5.0),  // L calf
        (4.542068, 3.0),  // head
    ];
    for (s, (mass, rotdamping)) in p.solids.iter().zip(expected) {
        assert!(near(s.mass, mass, 1e-5), "{} mass {}", s.name, s.mass);
        assert_eq!(s.rotdamping, rotdamping, "{}", s.name);
        assert_eq!(s.damping, 0.05, "{}", s.name);
        assert_eq!(s.inertia, 10.0, "{}", s.name);
        assert_eq!(s.surfaceprop, "flesh", "{}", s.name);
    }
    if let Some(t) = ragdoll_phy("t_phoenix") {
        let total: f32 = t.solids.iter().map(|s| s.mass).sum();
        assert!(near(total, 102.636752, 1e-3), "{total}");
    }
}

#[test]
fn joint_limits_and_friction() {
    let Some(p) = ragdoll_phy("ct_urban") else { return };
    let limits = |parent: usize, child: usize| {
        p.joints
            .iter()
            .find(|j| j.parent == parent && j.child == child)
            .unwrap_or_else(|| panic!("joint {parent} <- {child}"))
            .limits
    };
    let table: [((usize, usize), [(f32, f32); 3]); 15] = [
        ((0, 1), [(-10.0, 10.0), (-16.0, 16.0), (-20.0, 30.0)]),
        ((1, 2), [(-10.0, 10.0), (-10.0, 10.0), (-20.0, 20.0)]),
        ((2, 3), [(-15.0, 15.0), (-10.0, 10.0), (0.0, 45.0)]),
        ((2, 4), [(-15.0, 15.0), (-10.0, 10.0), (0.0, 45.0)]),
        ((4, 5), [(-15.0, 20.0), (-40.0, 32.0), (-80.0, 25.0)]),
        ((3, 8), [(-15.0, 20.0), (-40.0, 32.0), (-80.0, 25.0)]),
        ((5, 6), [(-40.0, 15.0), (0.0, 0.0), (-120.0, 10.0)]),
        ((8, 9), [(-40.0, 15.0), (0.0, 0.0), (-120.0, 10.0)]),
        ((6, 7), [(-25.0, 25.0), (-35.0, 35.0), (-50.0, 50.0)]),
        ((9, 10), [(-25.0, 25.0), (-35.0, 35.0), (-50.0, 50.0)]),
        ((0, 11), [(-25.0, 25.0), (-10.0, 15.0), (-55.0, 25.0)]),
        ((0, 13), [(-25.0, 25.0), (-10.0, 15.0), (-55.0, 25.0)]),
        ((11, 12), [(-10.0, 25.0), (-5.0, 5.0), (-10.0, 115.0)]),
        ((13, 14), [(-10.0, 25.0), (-5.0, 5.0), (-10.0, 115.0)]),
        ((2, 15), [(-50.0, 50.0), (-20.0, 20.0), (-26.0, 30.0)]),
    ];
    for ((parent, child), want) in table {
        assert_eq!(limits(parent, child), want, "{parent} <- {child}");
    }
    for j in &p.joints {
        assert_eq!(j.friction, [0.0; 3]);
    }
    // Each body but the pelvis is the child of exactly one joint.
    for b in 1..16 {
        assert_eq!(p.joints.iter().filter(|j| j.child == b).count(), 1, "body {b}");
    }
}

#[test]
fn collision_pairs() {
    let Some(p) = ragdoll_phy("ct_urban") else { return };
    let want = [
        (14, 12),
        (13, 11),
        (2, 10),
        (2, 7),
        (1, 10),
        (1, 7),
        (2, 5),
        (2, 8),
        (9, 15),
        (9, 1),
        (6, 15),
        (6, 1),
        (6, 9),
        (6, 8),
        (10, 11),
        (10, 13),
        (10, 0),
        (7, 11),
        (7, 13),
        (7, 0),
        (15, 10),
        (15, 7),
        (15, 3),
        (15, 4),
    ];
    assert_eq!(p.collision_pairs, want);
}

#[test]
fn bodies_map_to_bones_and_anchors() {
    for (model, anchors) in [
        (
            "ct_urban",
            [
                (0, 1, Vec3::new(0.0, 7.620, -3.446)),
                (1, 2, Vec3::new(3.661, 0.0, 0.0)),
                (2, 4, Vec3::new(11.195, 1.396, 2.015)),
                (2, 3, Vec3::new(11.195, 1.396, -2.015)),
                (5, 6, Vec3::new(12.160, 0.0, 0.0)),
                (0, 11, Vec3::new(-4.046, 0.0, 0.0)),
                (13, 14, Vec3::new(18.562, 0.0, 0.0)),
                (2, 15, Vec3::new(15.765, 2.729, 0.0)),
            ],
        ),
        (
            "t_phoenix",
            [
                (0, 1, Vec3::new(0.0, 7.767, -3.513)),
                (1, 2, Vec3::new(3.732, 0.0, 0.0)),
                (2, 4, Vec3::new(11.410, 1.423, 2.054)),
                (2, 3, Vec3::new(11.410, 1.423, -2.054)),
                (5, 6, Vec3::new(12.394, 0.0, 0.0)),
                (0, 11, Vec3::new(-4.124, 0.0, 0.0)),
                (13, 14, Vec3::new(18.919, 0.0, 0.0)),
                (2, 15, Vec3::new(16.068, 2.782, 0.0)),
            ],
        ),
    ] {
        let (Some(p), Some(bones)) = (ragdoll_phy(model), skeleton(model)) else {
            return;
        };
        let r = props::ragdoll(&p, &bones, model).expect("a ragdoll");
        let bone_of: Vec<usize> = r.bodies.iter().map(|b| b.bone).collect();
        assert_eq!(bone_of, [0, 10, 11, 28, 15, 16, 17, 18, 29, 30, 31, 5, 6, 1, 2, 14], "{model}");
        assert_eq!(bones.len() - r.bodies.len(), 34, "{model}: unsimulated bones");
        assert_eq!(r.joints.len(), 15);
        // Anchors: the child bone's bind origin in the parent bone's frame.
        let mut global: Vec<(Quat, Vec3)> = Vec::new();
        for b in &bones {
            global.push(match b.parent.map(|i| global[i]) {
                Some((q, p)) => (q * b.rotation, p + q * b.position),
                None => (b.rotation, b.position),
            });
        }
        for (parent, child, want) in anchors {
            let (pq, pp) = global[r.bodies[parent].bone];
            let (_, cp) = global[r.bodies[child].bone];
            let a = pq.inverse() * (cp - pp);
            assert!((a - want).abs().max_element() < 0.002, "{model} {parent} <- {child}: {a}");
        }
        // Limits come out in radians.
        let knee = r.joints.iter().find(|j| j.parent == 11 && j.child == 12).unwrap();
        assert!(near(knee.limits[2].1, 115f32.to_radians(), 1e-6));
    }
}

#[test]
fn calf_body_lies_along_its_bone() {
    let Some(p) = ragdoll_phy("t_phoenix") else { return };
    let calf = p
        .solids
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("ValveBiped.Bip01_L_Calf"))
        .expect("left calf solid");
    let xs = calf.pieces.iter().flatten().flatten().map(|v| v.x);
    let (lo, hi) = xs.fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)));
    assert!(lo > -1.0 && near(hi, 22.2, 0.3), "calf spans {lo}..{hi} along X");
}
