//! Community maps from the user's content cache (never in the repo),
//! headless; each test is skipped when its map isn't cached.

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
}
