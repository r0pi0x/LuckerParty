//! CS:S player animations decoded from a real install, checked against
//! the values in specs/cs_source/animation.md. Skipped without one.

use bevy::prelude::*;
use mashup::{
    games::cs_source::{self, anim},
    map::anim::{AnimSet, BonePose},
    mount::config::LocalConfig,
};

fn read() -> Option<impl Fn(&str) -> Option<Vec<u8>>> {
    let path = LocalConfig::load().ok()?.game_path(cs_source::GAME)?;
    let mount = cs_source::mount::open(&path).ok()?;
    Some(move |p: &str| mount.read(&p.to_lowercase().replace('\\', "/")).ok())
}

fn load(path: &str) -> Option<AnimSet> {
    let Some(read) = read() else {
        eprintln!("skipping: no CS:S install configured");
        return None;
    };
    Some(anim::load(&read, path).expect(path))
}

fn bone(set: &AnimSet, names: &[(String, Option<usize>, Quat, Vec3)], name: &str) -> usize {
    let _ = set;
    names
        .iter()
        .position(|b| b.0 == name)
        .unwrap_or_else(|| panic!("no bone {name}"))
}

#[track_caller]
fn rot(got: Quat, want: [f32; 4]) {
    let w = Vec4::from_array(want);
    let g = Vec4::from(got);
    assert!(
        g.distance(w).min(g.distance(-w)) < 1.5e-3,
        "rotation {got}, want {want:?}"
    );
}

#[track_caller]
fn pos(got: Vec3, want: [f32; 3]) {
    assert!(
        got.distance(Vec3::from_array(want)) < 1.5e-3,
        "position {got}, want {want:?}"
    );
}

fn sample(set: &AnimSet, name: &str, frame: f32) -> Vec<BonePose> {
    let a = set.animations.iter().position(|a| a.name == name).expect(name);
    let mut out = vec![(Quat::IDENTITY, Vec3::ZERO); set.defaults.len()];
    let n = set.animations[a].frames;
    set.sample(a, frame / (n - 1).max(1) as f32, &mut out);
    out
}

#[test]
fn shared_model_decodes() {
    let path = "models/player/cs_player_shared.mdl";
    let Some(set) = load(path) else { return };
    let read = read().unwrap();
    let bones = anim::bones(&read, path).unwrap();
    assert_eq!(bones.len(), 49);
    assert_eq!(set.animations.len(), 1382);
    assert_eq!(set.sequences.len(), 723);
    assert_eq!(set.params.len(), 5);
    let b = |n| bone(&set, &bones, n);
    let calf = b("ValveBiped.Bip01_L_Calf");
    let s = sample(&set, "@Idle_lower", 3.0);
    rot(s[calf].0, [0.0, 0.0, 0.374228, 0.927337]);
    pos(s[calf].1, [19.0976, 0.0, 0.0]);
    pos(s[b("ValveBiped.Bip01_Pelvis")].1, [-0.410149, 0.002761, 37.420749]);
    rot(sample(&set, "@Idle_lower", 4.5)[calf].0, [0.0, 0.0, 0.375253, 0.926922]);
    rot(
        sample(&set, "@Idle_lower", 60.0)[calf].0,
        [0.0, 0.0, 0.373931, 0.927456],
    );
    let s = sample(&set, "@Idle_lower", 0.0);
    let pelvis = b("ValveBiped.Bip01_Pelvis");
    pos(s[pelvis].1, [-0.4101, 0.0028, 37.4364]);
    rot(s[pelvis].0, [0.6258, 0.3292, 0.3773, 0.5980]);
    rot(s[b("ValveBiped.Bip01_L_Thigh")].0, [-0.7435, 0.4042, -0.4644, 0.2611]);
    pos(s[b("ValveBiped.Bip01_L_Thigh")].1, [4.1628, 0.0, 0.0]);
    rot(s[b("ValveBiped.Bip01_Head1")].0, [0.3161, -0.0216, 0.2915, 0.9025]);
    let weapon = b("ValveBiped.weapon_bone");
    pos(s[weapon].1, [18.2926, 24.1148, -1.2414]);
    rot(s[weapon].0, [-0.4368, -0.2736, -0.5558, 0.6522]);
    let s = sample(&set, "a_RunN", 2.5);
    pos(s[pelvis].1, [0.3877, -0.3000, 33.9363]);
    rot(s[pelvis].0, [0.4431, 0.5638, 0.5156, 0.4690]);
    rot(s[b("ValveBiped.Bip01_L_Thigh")].0, [-0.3920, 0.5228, -0.4234, 0.6275]);
    let s = sample(&set, "Run_Pistol_aim_up_left", 0.0);
    pos(s[pelvis].1, [1.5762, -2.7930, 0.0]);
    rot(s[pelvis].0, [0.0, 0.1736, 0.0, 0.9848]);
    rot(s[b("ValveBiped.Bip01_Spine4")].0, [0.0679, -0.0302, -0.0728, 0.9946]);
    rot(s[b("ValveBiped.Bip01_L_Toe0")].0, [0.0, 0.0, 0.0, 1.0]);
    let s = sample(&set, "Stand_Shoot_Pistol_layer", 5.0);
    pos(s[weapon].1, [0.5391, -2.4649, -0.2930]);
    rot(s[weapon].0, [-0.0672, 0.0, 0.0, 0.9977]);
}

#[test]
fn player_model_merges_its_includes() {
    let path = "models/player/ct_urban.mdl";
    let Some(set) = load(path) else { return };
    let read = read().unwrap();
    let bones = anim::bones(&read, path).unwrap();
    assert_eq!(bones.len(), 50);
    assert_eq!(set.sequences.len(), 757);
    assert_eq!(set.animations.len(), 1445);
    assert_eq!(set.sequence("Idle_lower"), Some(1));
    assert_eq!(set.sequence("Run_lower"), Some(3));
    assert_eq!(set.sequence("Run_Aim_AK"), Some(724));
    let names: Vec<&str> = set.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["move_yaw", "body_pitch", "body_yaw", "move_y", "move_x"]);
    // Idle_lower at cycle 0: shared bone lengths, ValveBiped.forward keeps
    // the player model's default.
    let mut pose = set.defaults.clone();
    let params = set.default_params();
    set.accumulate(&mut pose, 1, 0.0, 1.0, &params);
    pos(
        pose[bone(&set, &bones, "ValveBiped.Bip01_L_Thigh")].1,
        [4.1628, 0.0, 0.0],
    );
    let forward = bone(&set, &bones, "ValveBiped.forward");
    pos(pose[forward].1, [2.0, -3.0, 0.0]);
    rot(pose[forward].0, [-0.5572, 0.4353, -0.4353, 0.5572]);
}

fn set_params(set: &AnimSet, values: &[(&str, f32)]) -> Vec<f32> {
    let mut params = set.default_params();
    for (n, v) in values {
        let i = set.param(n).unwrap();
        params[i] = set.params[i].encode(*v);
    }
    params
}

#[test]
fn grids_blend_by_pose_parameters() {
    let path = "models/player/cs_player_shared.mdl";
    let Some(set) = load(path) else { return };
    let read = read().unwrap();
    let bones = anim::bones(&read, path).unwrap();
    let b = |n| bone(&set, &bones, n);
    let aim = set.sequence("Idle_Aim_Pistol").unwrap();
    let p = set_params(&set, &[("body_yaw", 20.0), ("body_pitch", -35.0)]);
    let (i0, s0) = set.axis(&set.sequences[aim], 0, &p);
    let (i1, s1) = set.axis(&set.sequences[aim], 1, &p);
    assert_eq!((i0, i1), (1, 0));
    assert!((s0 - 1.0 / 3.0).abs() < 1e-4 && (s1 - 0.5).abs() < 1e-4, "{s0} {s1}");

    let run = set.sequence("Run_lower").unwrap();
    let p = set_params(&set, &[("move_x", 0.8660), ("move_y", -0.5)]);
    let mut pose = vec![(Quat::IDENTITY, Vec3::ZERO); set.defaults.len()];
    set.sequence_pose(run, 0.0, &p, &mut pose);
    let pelvis = b("ValveBiped.Bip01_Pelvis");
    pos(pose[pelvis].1, [-0.4101, -0.0011, 33.7320]);
    rot(pose[pelvis].0, [0.5146, 0.4902, 0.4904, 0.5043]);
    rot(
        pose[b("ValveBiped.Bip01_L_Thigh")].0,
        [-0.6194, 0.4318, -0.5525, 0.3531],
    );
    rot(pose[b("ValveBiped.Bip01_L_Calf")].0, [0.0, 0.0, 0.5696, 0.8219]);
    pos(pose[b("ValveBiped.Bip01_Spine")].1, [0.0, 3.5793, -3.1906]);
    set.sequence_pose(run, 0.25, &p, &mut pose);
    pos(pose[pelvis].1, [0.1756, -0.4573, 36.7036]);
    rot(pose[b("ValveBiped.Bip01_L_Calf")].0, [0.0, 0.0, 0.4473, 0.8944]);

    let rate = |seq: &str, x: f32, y: f32| {
        let p = set_params(&set, &[("move_x", x), ("move_y", y)]);
        set.cycle_rate(set.sequence(seq).unwrap(), &p)
    };
    assert!((rate("Run_lower", 0.7071, -0.7071) - 1.414214).abs() < 1e-3);
    assert!((rate("Idle_lower", 0.0, 0.0) - 0.5).abs() < 1e-4);
    assert!((rate("Run_lower", 1.0, 0.0) - 1.5).abs() < 1e-4);
    assert!((rate("walk_lower", 1.0, 0.0) - 0.9375).abs() < 1e-4);
    assert!((rate("Run_lower", 0.0, 0.0) - 0.5).abs() < 1e-4);
    assert!((rate("Crouch_walk_lower", 0.5, 0.0) - 0.875).abs() < 1e-4);
}

#[test]
fn layers_accumulate() {
    let path = "models/player/cs_player_shared.mdl";
    let Some(set) = load(path) else { return };
    let read = read().unwrap();
    let bones = anim::bones(&read, path).unwrap();
    let b = |n| bone(&set, &bones, n);
    let p = set_params(&set, &[("body_yaw", 20.0), ("body_pitch", -35.0)]);
    let base = || {
        let mut pose = set.defaults.clone();
        set.accumulate(&mut pose, set.sequence("Idle_lower").unwrap(), 0.0, 1.0, &p);
        pose
    };
    let (pelvis, spine4, head) = (
        b("ValveBiped.Bip01_Pelvis"),
        b("ValveBiped.Bip01_Spine4"),
        b("ValveBiped.Bip01_Head1"),
    );
    let aim = set.sequence("Idle_Aim_Pistol").unwrap();
    let mut pose = base();
    set.accumulate(&mut pose, aim, 0.0, 1.0, &p);
    pos(pose[pelvis].1, [-0.6500, -0.9282, 36.6908]);
    rot(pose[pelvis].0, [0.5426, 0.4493, 0.4738, 0.5284]);
    rot(pose[spine4].0, [0.0029, 0.0012, 0.0443, 0.9990]);
    rot(pose[head].0, [0.2748, 0.0130, 0.4177, 0.8659]);
    let mut pose = base();
    set.accumulate(&mut pose, aim, 0.0, 0.5, &p);
    pos(pose[pelvis].1, [-0.5301, -0.4627, 37.0636]);
    rot(pose[pelvis].0, [0.5868, 0.3910, 0.4275, 0.5657]);

    let mut pose = base();
    set.accumulate(&mut pose, set.sequence("Idle_Upper_PISTOL").unwrap(), 0.0, 1.0, &p);
    pos(pose[pelvis].1, [-0.6500, -0.9282, 36.6908]);
    rot(
        pose[b("ValveBiped.Bip01_L_Thigh")].0,
        [-0.6673, 0.2997, -0.6039, 0.3166],
    );
    rot(pose[head].0, [0.2741, 0.0127, 0.4176, 0.8662]);
    rot(pose[b("ValveBiped.Bip01_R_Hand")].0, [-0.3304, -0.0804, 0.2841, 0.8965]);
    let weapon = b("ValveBiped.weapon_bone");
    pos(pose[weapon].1, [31.6520, 23.5236, 1.0931]);
    rot(pose[weapon].0, [-0.5305, -0.1653, -0.6531, 0.5145]);

    let mut pose = base();
    set.accumulate(
        &mut pose,
        set.sequence("Run_Upper_PISTOL_staticlayer").unwrap(),
        0.0,
        1.0,
        &p,
    );
    rot(
        pose[b("ValveBiped.Bip01_L_UpperArm")].0,
        [-0.8185, -0.1809, -0.4614, 0.2907],
    );
    rot(pose[b("ValveBiped.Bip01_R_Hand")].0, [-0.5164, -0.0840, 0.4447, 0.7270]);
}

/// The AWP's view model is MDL version 48 (specs/cs_source/mdl_v48.md):
/// it reads like 44. Values from the spec's test cases.
#[test]
fn awp_view_model_version_48() {
    let Some(set) = load("models/weapons/v_snip_awp.mdl") else {
        return;
    };
    let frames: Vec<(&str, usize)> = set.animations.iter().map(|a| (a.name.as_str(), a.frames)).collect();
    for (name, n) in [
        ("@awm_idle", 11),
        ("@awm_fire", 42),
        ("@awm_draw", 31),
        ("@awm_reload", 111),
    ] {
        assert!(frames.contains(&(name, n)), "{name} {n} in {frames:?}");
    }
    assert_eq!(set.sequences.len(), 4);
    let fire = set.animations.iter().find(|a| a.name == "@awm_fire").unwrap();
    let track = |bone: usize| fire.tracks.iter().find(|t| t.bone == bone).expect("track");
    let q = |bone: usize, f: usize| {
        let r = track(bone).rotation.as_ref().unwrap();
        r[f.min(r.len() - 1)]
    };
    let p = |bone: usize, f: usize| {
        let v = track(bone).position.as_ref().unwrap();
        v[f.min(v.len() - 1)]
    };
    rot(q(0, 0), [0.5, 0.5, 0.5, 0.5]);
    assert!(
        p(0, 0).distance(Vec3::new(9.1839, 5.5584, -7.0011)) < 2e-3,
        "{}",
        p(0, 0)
    );
    rot(q(0, 10), [0.5181, 0.4958, 0.5081, 0.4771]);
    assert!(p(0, 10).distance(Vec3::new(11.2582, 5.8709, -7.3214)) < 2e-3);
    rot(q(37, 10), [-0.0409, -0.4096, 0.0667, 0.9089]);
    rot(q(34, 0), [0.1122, 0.7034, -0.6770, -0.1851]);
}

/// Compare every zero-frame entry of `path`'s animations with the decoded
/// animation at frame j · span (mdl_v48.md §6: they are copies of the
/// first frames, so both readings must agree). Returns how many entries
/// were checked.
fn zero_frames_match(read: &dyn Fn(&str) -> Option<Vec<u8>>, path: &str) -> usize {
    let set = anim::load(read, path).expect(path);
    let zeros = anim::zero_frames(read, path).expect(path);
    let mut checked = 0;
    let (mut worst_deg, mut worst_pos) = (0f32, 0f32);
    for z in &zeros {
        let a = set
            .animations
            .iter()
            .position(|a| a.name == z.animation)
            .unwrap_or_else(|| panic!("{path}: no animation {}", z.animation));
        let n = set.animations[a].frames;
        let count = z.bones.iter().map(|(_, p, q)| p.len().max(q.len())).max().unwrap_or(0);
        for j in 0..count {
            let f = (j * z.span).min(n - 1);
            let mut out = vec![(Quat::IDENTITY, Vec3::ZERO); set.defaults.len()];
            set.sample(a, f as f32 / (n - 1).max(1) as f32, &mut out);
            for (bone, ps, qs) in &z.bones {
                if let Some(q) = qs.get(j) {
                    let deg = out[*bone].0.angle_between(*q).to_degrees();
                    worst_deg = worst_deg.max(deg);
                    assert!(
                        deg < 0.1,
                        "{path} {} frame {f} bone {bone}: rotation {} vs zero frame {q} ({deg}°)",
                        z.animation,
                        out[*bone].0
                    );
                    checked += 1;
                }
                if let Some(p) = ps.get(j) {
                    let d = out[*bone].1.distance(*p);
                    worst_pos = worst_pos.max(d);
                    assert!(
                        d < 0.05,
                        "{path} {} frame {f} bone {bone}: position {} vs zero frame {p}",
                        z.animation,
                        out[*bone].1
                    );
                    checked += 1;
                }
            }
        }
    }
    eprintln!(
        "{path}: {} animations with zero frames, {checked} values, worst {worst_deg}° {worst_pos} units",
        zeros.len()
    );
    checked
}

/// The hostages' animations live in HL2's `humans/male_*` include models,
/// in sections of external `.ani` blocks (mdl_v48.md §8). Their decoded
/// frames agree with the zero-frame copies the `.mdl`s keep (§6), as the
/// spec found for `crow.mdl`.
#[test]
fn hostage_animations_from_ani_blocks() {
    let Some(read) = read() else {
        eprintln!("skipping: no CS:S install configured");
        return;
    };
    let set = anim::load(&read, "models/characters/hostage_01.mdl").expect("hostage");
    for (name, activity) in [
        ("idle_subtle", "ACT_IDLE"),
        ("walk_all", "ACT_WALK"),
        ("run_all", "ACT_RUN"),
    ] {
        let s = set.sequence(name).unwrap_or_else(|| panic!("no {name}"));
        assert_eq!(set.sequences[s].activity, activity);
        // Animated: some bone moves over the cycle.
        let mut a = vec![(Quat::IDENTITY, Vec3::ZERO); set.defaults.len()];
        let mut b = a.clone();
        let params = set.default_params();
        set.sequence_pose(s, 0.0, &params, &mut a);
        set.sequence_pose(s, 0.5, &params, &mut b);
        let moved = a
            .iter()
            .zip(&b)
            .map(|(x, y)| x.0.angle_between(y.0))
            .fold(0.0, f32::max);
        assert!(moved > 0.05, "{name} doesn't move ({moved} rad)");
    }
    // Spot checks against an independent decoder written from the spec
    // (a throwaway script outside the repo), records in `.ani` blocks 1,
    // 5 and 8.
    let path = "models/humans/male_shared.mdl";
    let shared = anim::load(&read, path).expect(path);
    let bones = anim::bones(&read, path).unwrap();
    let b = |n| bone(&shared, &bones, n);
    let (pelvis, spine, neck) = (
        b("ValveBiped.Bip01_Pelvis"),
        b("ValveBiped.Bip01_Spine"),
        b("ValveBiped.Bip01_Neck1"),
    );
    let s = sample(&shared, "@idle_subtle", 0.0);
    rot(s[pelvis].0, [0.485858, 0.513752, 0.513752, 0.485859]);
    pos(s[pelvis].1, [0.378913, -0.003725, 38.139629]);
    let s = sample(&shared, "@idle_subtle", 140.0);
    rot(s[pelvis].0, [0.476656, 0.540943, 0.516319, 0.462162]);
    pos(s[pelvis].1, [0.296879, -0.390456, 38.045877]);
    rot(s[neck].0, [0.980019, 0.198903, 0.0, 0.0]);
    pos(s[neck].1, [3.307273, 0.0, 0.0]);
    let s = sample(&shared, "a_WalkN", 20.0);
    rot(s[pelvis].0, [0.480605, 0.484827, 0.521431, 0.511929]);
    pos(s[pelvis].1, [0.378913, 1.121309, 36.577082]);
    rot(s[spine].0, [0.473920, 0.525485, 0.479089, 0.519364]);
    let s = sample(&shared, "a_RunN", 7.0);
    rot(s[pelvis].0, [0.551407, 0.524051, 0.481201, 0.435622]);
    pos(s[pelvis].1, [0.378913, 1.773673, 35.354388]);

    let walk = set.sequence("walk_all").unwrap();
    let speed = set.ground_speed(walk, &set.default_params());
    assert!(speed > 30.0, "walk_all ground speed {speed}");
    let run = set.ground_speed(set.sequence("run_all").unwrap(), &set.default_params());
    eprintln!("ground speeds: walk_all {speed}, run_all {run}");
    // The HL2 humans are version 44: external blocks but no zero frames.
    for path in [
        "models/humans/male_gestures.mdl",
        "models/humans/male_postures.mdl",
        path,
    ] {
        assert_eq!(anim::zero_frames(&read, path).unwrap().len(), 0, "{path}");
    }
    // The spec's crow.mdl check: version 48, sectioned, zero frames.
    let crow = "models/crow.mdl";
    if read(crow).is_none() {
        eprintln!("{crow}: not mounted, skipped");
        return;
    }
    assert!(zero_frames_match(&read, crow) > 800);
    // Without its `.ani`, the crow plays from the zero-frame cache: equal
    // at the stored frames (§6).
    let hidden = |p: &str| if p.ends_with(".ani") { None } else { read(p) };
    let cached = anim::load(&hidden, crow).unwrap();
    let full = anim::load(&read, crow).unwrap();
    for z in anim::zero_frames(&read, crow).unwrap() {
        let a = full.animations.iter().position(|a| a.name == z.animation).unwrap();
        let n = full.animations[a].frames;
        let count = z.bones.iter().map(|(_, p, q)| p.len().max(q.len())).max().unwrap();
        for j in 0..count {
            let f = (j * z.span).min(n - 1);
            let at = f as f32 / (n - 1).max(1) as f32;
            let mut x = vec![(Quat::IDENTITY, Vec3::ZERO); full.defaults.len()];
            let mut y = x.clone();
            full.sample(a, at, &mut x);
            cached.sample(a, at, &mut y);
            for (bone, ps, qs) in &z.bones {
                if !qs.is_empty() {
                    let deg = x[*bone].0.angle_between(y[*bone].0).to_degrees();
                    assert!(deg < 0.1, "{} frame {f} bone {bone}: {deg}°", z.animation);
                }
                if !ps.is_empty() {
                    assert!(x[*bone].1.distance(y[*bone].1) < 0.05, "{} frame {f}", z.animation);
                }
            }
        }
    }
}
