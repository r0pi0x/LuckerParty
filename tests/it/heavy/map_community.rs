//! Community maps from the user's content cache (never in the repo),
//! headless; each test is skipped when its map isn't cached.

use bevy::math::Vec3;
use mashup::{
    games::{self, cs_source},
    map::MapData,
    mount::config::{LocalConfig, content_dir},
};

/// The map, when the install and the cached map are both there.
fn cached(name: &str) -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    let present = content_dir(cs_source::GAME).is_some_and(|d| d.join("maps").join(format!("{name}.bsp")).is_file());
    if !installed || !present {
        eprintln!("skipping: {name} not in the content cache");
        return None;
    }
    Some(games::load_map(&format!("cs_source:{name}")).expect(name))
}

/// surf_boreas is repacked with LZMA-compressed lumps and its version-10
/// static prop lump sets the NO_DRAW bit on 670 of its 1587 props, its
/// surf ramps among them (`models/project_tendies/ramps/*`, 100 m long
/// models). The lighting must decode (not compressed bytes read as
/// luxels) and every static prop must be placed.
#[test]
fn surf_boreas_lighting_and_ramps() {
    let Some(map) = cached("surf_boreas") else { return };
    // Lightmaps: compressed bytes read as luxels gave random exponents.
    let lm = map.lightmap.as_ref().expect("lightmap");
    let wild = lm.rgb.iter().filter(|c| c.iter().any(|v| !v.is_finite() || *v > 16.0)).count();
    assert!(
        wild * 1000 < lm.rgb.len(),
        "{wild} of {} luxels out of range: lighting read from compressed bytes?",
        lm.rgb.len()
    );
    // Cubemap names come from the (compressed) cubemap lump.
    let bad: Vec<_> = map.warnings.iter().filter(|w| w.contains("materials/maps/surf_boreas/c")).collect();
    assert!(bad.is_empty(), "{bad:#?}");
    // Static props: all of them, the ramp models included.
    let statics = map.props.iter().filter(|p| p.entity.is_none()).count();
    assert!(statics >= 1587, "{statics} static props placed");
    let ramps = map
        .props
        .iter()
        .filter(|p| {
            let (lo, hi) = map.models[p.model].bounds;
            (hi - lo).max_element() > 90.0
        })
        .count();
    assert!(ramps >= 19, "{ramps} props over 90 m (the ramps)");

    // A snow cave whose displacement folds back over itself: from inside
    // it (the "transition" soundscape, Source 10446 8619 9860), the floor
    // ahead must face the eye. Re-winding each triangle to the base face's
    // normal turned the folded ones inside out (culled: holes to the sky).
    let engine = |x: f32, y: f32, z: f32| Vec3::new(x, z, -y) * 0.0254;
    let eye = engine(10446.0, 8619.0, 9860.0);
    for dir in [Vec3::new(-0.526, -0.728, -0.44), Vec3::new(-0.876, 0.163, -0.454)] {
        let dir = Vec3::new(dir.x, dir.z, -dir.y).normalize();
        let hit = first_hit(&map, eye, dir).expect("the cave floor is drawn there");
        assert!(hit.1.dot(dir) < 0.0, "the nearest drawn triangle faces away from the eye: {hit:?}");
        assert!((7.0..13.0).contains(&hit.0), "the floor 8-12 m away, not further: {hit:?}");
    }
}

/// The nearest world triangle on a ray (distance, its front normal by
/// winding: counter-clockwise faces the viewer), skybox meshes left out.
fn first_hit(map: &MapData, o: Vec3, d: Vec3) -> Option<(f32, Vec3)> {
    let mut best: Option<(f32, Vec3)> = None;
    for m in map.meshes.iter().filter(|m| !m.skybox) {
        for t in m.indices.chunks_exact(3) {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from_array(m.positions[t[k] as usize]));
            let (e1, e2) = (b - a, c - a);
            let h = d.cross(e2);
            let det = e1.dot(h);
            if det.abs() < 1e-9 {
                continue;
            }
            let s = o - a;
            let u = s.dot(h) / det;
            let q = s.cross(e1);
            let v = d.dot(q) / det;
            let t = e2.dot(q) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0 && best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, e1.cross(e2).normalize()));
            }
        }
    }
    best
}
