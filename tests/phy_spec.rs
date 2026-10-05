//! `.phy` test cases from specs/cs_source/physics_props.md, on the plastic
//! crate's collision model read from the CS:S install (skipped without one).

use bevy::math::Vec3;
use mashup::{
    games::cs_source::{self, mount, phy},
    mount::config::LocalConfig,
};

fn read(path: &str) -> Option<Vec<u8>> {
    let config = LocalConfig::load().ok()?;
    let m = mount::open(&config.game_path(cs_source::GAME)?).ok()?;
    m.read(path).ok()
}

fn crate_phy() -> Option<phy::Phy> {
    let bytes = read("models/props_junk/plasticcrate01a.phy")?;
    assert_eq!(bytes.len(), 2555, "the spec's crate .phy");
    Some(phy::parse(&bytes).expect("parses"))
}

#[test]
fn crate_pieces_and_points() {
    let Some(p) = crate_phy() else { return };
    assert_eq!(p.pieces.len(), 5, "five convex pieces (leaves)");
    for piece in &p.pieces {
        assert_eq!(piece.len(), 12, "each piece is a box: 12 triangles");
    }
    // First point of the first piece, converted to Source inches.
    let first = p.pieces[0][0][0];
    assert!(
        (first - Vec3::new(-8.71210, -12.10720, 7.39240)).abs().max_element() < 1e-3,
        "{first}"
    );
    let all = p.pieces.iter().flatten().flatten();
    let (lo, hi) = all.fold((Vec3::MAX, Vec3::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!((lo - Vec3::new(-8.832, -12.875, -7.463)).abs().max_element() < 2e-3, "{lo}");
    assert!((hi - Vec3::new(8.795, 12.819, 7.392)).abs().max_element() < 2e-3, "{hi}");
}

#[test]
fn crate_volume_and_winding() {
    let Some(p) = crate_phy() else { return };
    let mut volume = 0.0;
    for piece in &p.pieces {
        let n = (piece.len() * 3) as f32;
        let centroid = piece.iter().flatten().copied().sum::<Vec3>() / n;
        for [a, b, c] in piece {
            assert!((*b - *a).cross(*c - *a).dot(*a - centroid) > 0.0, "wound outward");
            volume += a.dot(b.cross(*c)) / 6.0;
        }
    }
    assert!((volume - 1144.14).abs() < 0.05, "volume {volume}");
}

#[test]
fn crate_text_section() {
    let Some(p) = crate_phy() else { return };
    assert_eq!(p.mass, 10.0);
    assert_eq!(p.surfaceprop, "plastic");
    assert_eq!((p.damping, p.rotdamping, p.inertia), (0.0, 0.0, 1.0));
}

#[test]
fn legacy_solid_without_vphy_tag() {
    let Some(bytes) = read("models/props_combine/pod_extractor.phy") else { return };
    let p = phy::parse(&bytes).expect("legacy layout parses");
    assert!(!p.pieces.is_empty());
}
