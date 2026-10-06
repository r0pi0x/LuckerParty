//! The stock CS:S maps other than dust2 (which has its own tests), from a
//! real install, headless: they load, and what they draw resolves.
//! Skipped without an install.
//!
//! `cargo test --features dev --test map_stock -- --nocapture --ignored
//! all_stock_maps_warnings` prints every map's load warnings (the catalog
//! in docs/plans/active/other-maps.md comes from it).

use mashup::{
    games::{self, cs_source},
    map::MapData,
    mount::config::LocalConfig,
};

fn installed() -> bool {
    let ok = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !ok {
        eprintln!("skipping: no CS:S install configured");
    }
    ok
}

fn load(name: &str) -> Option<MapData> {
    installed().then(|| games::load_map(&format!("cs_source:{name}")).expect(name))
}

/// Every stock map in a CS:S install.
const STOCK: &[&str] = &[
    "cs_assault",
    "cs_compound",
    "cs_havana",
    "cs_italy",
    "cs_militia",
    "cs_office",
    "de_aztec",
    "de_cbble",
    "de_chateau",
    "de_dust",
    "de_dust2",
    "de_inferno",
    "de_nuke",
    "de_piranesi",
    "de_port",
    "de_prodigy",
    "de_tides",
    "de_train",
];

/// Warnings other than the decal/overlay placement counts (tracked in
/// docs/plans/active/other-maps.md).
fn load_warnings(map: &MapData) -> Vec<&String> {
    map.warnings
        .iter()
        .filter(|w| !w.ends_with("decals found no surface to project onto") && !w.ends_with("overlays produced no geometry"))
        .collect()
}

/// de_aztec's walls (WorldTwoTextureBlend), de_nuke's HDR-capable sky and
/// self-lit decals, static props named `./models/...`, water normal maps:
/// everything these maps name resolves.
#[test]
fn materials_and_models_resolve() {
    for name in ["de_aztec", "cs_office", "de_nuke", "de_train", "de_inferno", "cs_italy"] {
        let Some(map) = load(name) else { return };
        let warnings = load_warnings(&map);
        assert!(warnings.is_empty(), "{name}: {warnings:#?}");
        assert!(map.sky.is_some(), "{name}: no sky");
    }
}

/// WorldTwoTextureBlend draws its second texture over the base by the
/// second texture's alpha (detail blend mode 2).
#[test]
fn aztec_walls_blend_two_textures() {
    let Some(map) = load("de_aztec") else { return };
    let wall = map
        .meshes
        .iter()
        .find(|m| m.material == "de_aztec/stn01grm01_s1")
        .expect("an aztec stone wall");
    assert!(wall.texture.is_some());
    let detail = wall.detail.expect("the stonework as the detail layer");
    assert_eq!(detail.mode, 2);
    assert_eq!(detail.scale, [4.0, 4.0]);
}

/// Older maps (BSP v19 like de_aztec and cs_office; early v20 like de_nuke)
/// store one ambient cube per leaf rather than sample lists: props there
/// must still get ambient light, not just the sun.
#[test]
fn old_maps_light_props_with_ambient_cubes() {
    for name in ["de_aztec", "cs_office", "de_nuke", "de_train"] {
        let Some(map) = load(name) else { return };
        let props: Vec<_> = map.props.iter().filter(|p| !p.skybox).collect();
        let lit = props
            .iter()
            .filter(|p| {
                p.lighting
                    .as_ref()
                    .is_some_and(|l| l.cube.iter().any(|c| c.max_element() > 0.0))
            })
            .count();
        assert!(
            lit as f32 > 0.9 * props.len() as f32,
            "{name}: {lit} of {} props have ambient light",
            props.len()
        );
    }
}

/// Brush entities draw and collide where they stand: cs_office's first
/// window (func_breakable_surf, model *1, Source x -628..-508 at y -344..-340,
/// z -148..-52) and its sliding door (func_door *7, stored around its origin
/// 584 -1872 -252).
#[test]
fn brush_entities_draw_and_collide() {
    use bevy::math::Vec3;
    let Some(map) = load("cs_office") else { return };
    let src = |x: f32, y: f32, z: f32| Vec3::new(x, z, -y) * 0.0254;
    for (what, at) in [("window", src(-568.0, -342.0, -100.0)), ("door", src(584.0, -1872.0, -252.0))] {
        let drawn = map
            .meshes
            .iter()
            .flat_map(|m| m.positions.iter())
            .any(|p| Vec3::from(*p).distance(at) < 100.0 * 0.0254);
        assert!(drawn, "{what} not drawn");
        let solid = map
            .collision_brushes
            .iter()
            .any(|b| (b.min - Vec3::splat(0.05)).cmple(at).all() && (b.max + Vec3::splat(0.05)).cmpge(at).all());
        assert!(solid, "{what} not solid");
    }
}

/// `$additive` materials (de_nuke's light glows) add to what's behind them
/// instead of drawing their black background.
#[test]
fn additive_materials_add() {
    let Some(map) = load("de_nuke") else { return };
    let additive = |m: &mashup::map::MapMesh| m.alpha == mashup::map::MapAlpha::Add;
    assert!(map.meshes.iter().any(additive), "no additive world surfaces");
    assert!(
        map.models.iter().any(|m| m.meshes.iter().any(additive)),
        "no additive prop surfaces"
    );
}

/// Debug aid: `MAP=de_aztec AT=-280,-1512,-160 R=600 cargo test ...
/// --ignored props_near -- --nocapture` lists props near a Source point.
#[test]
#[ignore = "debug aid"]
fn props_near() {
    let (Ok(name), Ok(at)) = (std::env::var("MAP"), std::env::var("AT")) else {
        return;
    };
    let Some(map) = load(&name) else { return };
    let a: Vec<f32> = at.split(',').map(|s| s.parse().unwrap()).collect();
    let p = bevy::math::Vec3::new(a[0], a[2], -a[1]) * 0.0254;
    let r: f32 = std::env::var("R").ok().and_then(|s| s.parse().ok()).unwrap_or(500.0) * 0.0254;
    for prop in map.props.iter().filter(|q| q.translation.distance(p) < r) {
        let m = &map.models[prop.model];
        let mats: Vec<_> = m
            .meshes
            .iter()
            .map(|x| (x.material.as_str(), x.texture.is_some()))
            .collect();
        let cube = prop.lighting.as_ref().map(|l| (l.cube, l.lights.len()));
        println!("{:?} {mats:?} light {cube:?}", prop.translation / 0.0254);
    }
}

#[test]
#[ignore = "slow: loads every stock map; run by hand for the catalog"]
fn all_stock_maps_warnings() {
    if !installed() {
        return;
    }
    for name in STOCK {
        let map = games::load_map(&format!("cs_source:{name}")).expect(name);
        println!("== {name}: {} warnings", map.warnings.len());
        for w in &map.warnings {
            println!("  {w}");
        }
    }
}
