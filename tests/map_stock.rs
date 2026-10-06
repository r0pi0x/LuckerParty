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
/// docs/plans/active/other-maps.md) and the AWP view model (MDL v48, not
/// read yet: docs/backlog.md section 3).
fn load_warnings(map: &MapData) -> Vec<&String> {
    map.warnings
        .iter()
        .filter(|w| {
            !w.ends_with("decals found no surface to project onto")
                && !w.ends_with("overlays produced no geometry")
                && !w.starts_with("models/weapons/v_snip_awp.mdl")
        })
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

/// WorldTwoTextureBlend's walls use the 2x grime mask (detail mode 4,
/// specs/cs_source/shaders_two_texture_blend.md): the stone is the detail.
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
    assert_eq!(detail.mode, 4);
    assert_eq!(detail.scale, [4.0, 4.0]);
    // The canals (Water, no base texture) draw their fog colour and
    // reflect the baked cubemap.
    let water: Vec<_> = map
        .meshes
        .iter()
        .filter(|m| m.material.contains("aztecwater"))
        .collect();
    assert!(!water.is_empty());
    for m in water {
        assert!(m.texture.is_some() && m.envmap.is_some(), "{}", m.material);
    }
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
/// z -148..-52), baked into the world; its sliding door (func_door *7,
/// stored around its origin 584 -1872 -252) is a mover: drawn and solid
/// through its own node, so its meshes and volumes are local to it.
#[test]
fn brush_entities_draw_and_collide() {
    use bevy::math::Vec3;
    let Some(map) = load("cs_office") else { return };
    let src = |x: f32, y: f32, z: f32| Vec3::new(x, z, -y) * 0.0254;
    let at = src(-568.0, -342.0, -100.0);
    let drawn = map
        .meshes
        .iter()
        .filter(|m| m.entity.is_none())
        .flat_map(|m| m.positions.iter())
        .any(|p| Vec3::from(*p).distance(at) < 100.0 * 0.0254);
    assert!(drawn, "window not drawn");
    let solid = map
        .collision_brushes
        .iter()
        .any(|b| (b.min - Vec3::splat(0.05)).cmple(at).all() && (b.max + Vec3::splat(0.05)).cmpge(at).all());
    assert!(solid, "window not solid");

    let (index, door) = map
        .entities
        .iter()
        .enumerate()
        .find(|(_, e)| e.get("model") == Some("*7"))
        .expect("cs_office's door");
    assert_eq!(door.classname(), "func_door");
    assert!(door.mover, "the door moves");
    assert_eq!(door.origin(), Vec3::new(584.0, -1872.0, -252.0));
    assert!(!door.hulls.is_empty(), "door has volumes");
    // Local: around the origin, within the door's size.
    let far = door.hulls.iter().flat_map(|h| &h.points).map(|p| p.length()).fold(0.0, f32::max);
    assert!(far < 200.0, "door volume reaches {far} units from its origin");
    assert!(map.meshes.iter().any(|m| m.entity == Some(index)), "door drawn on its node");
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

/// Infodecals land on displacement terrain too (cs_compound: 4 of its 6
/// previously unplaced decals; de_port 5 of 8).
#[test]
fn decals_project_onto_displacements() {
    for (name, most) in [("cs_compound", 2), ("de_port", 3)] {
        let Some(map) = load(name) else { return };
        let unplaced: usize = map
            .warnings
            .iter()
            .find_map(|w| w.strip_suffix(" decals found no surface to project onto")?.parse().ok())
            .unwrap_or(0);
        assert!(unplaced <= most, "{name}: {unplaced} decals unplaced");
    }
}

/// Switchable lights that start on (cs_office's projector: style 32, no
/// "starts dark" flag) add their own lightmap style to the faces they light.
#[test]
fn switchable_lights_that_start_on_are_baked_in() {
    use cs_source::lightmap;
    if !installed() {
        return;
    }
    let install = LocalConfig::load().unwrap().game_path(cs_source::GAME).unwrap();
    let mount = cs_source::mount::open(&install).unwrap();
    let bytes = mount.read("maps/cs_office.bsp").unwrap();
    let bsp = vbsp::Bsp::read(&bytes).unwrap();
    let lump = lightmap::lighting_lump(&bytes);
    let (mut faces, mut brighter) = (0, 0);
    for face in bsp.models().next().unwrap().faces() {
        if !face.styles.contains(&32) {
            continue;
        }
        faces += 1;
        let bumped = face.texture().flags.contains(vbsp::TextureFlags::BUMPLIGHT);
        let total = |s: &lightmap::FaceSamples| s.rgb.iter().map(|c| c[0] + c[1] + c[2]).sum::<f32>();
        let base = lightmap::face_samples(lump, &face).map_or(0.0, |s| total(&s));
        let on = lightmap::face_samples_lit(lump, &face, bumped, &|_| false)
            .0
            .map_or(0.0, |s| total(&s));
        let off = lightmap::face_samples_lit(lump, &face, bumped, &|_| true)
            .0
            .map_or(0.0, |s| total(&s));
        assert!(
            (off - base).abs() <= 1e-3 * base.max(1.0),
            "style 0 alone when the light is off"
        );
        if on > base * 1.01 {
            brighter += 1;
        }
    }
    assert!(faces > 0, "the projector lights some faces");
    assert!(
        brighter * 2 > faces,
        "{brighter} of {faces} faces brighter with the projector on"
    );
}

/// Debug aid: `MAP=cs_office cargo test ... --ignored dark_textures --
/// --nocapture` lists world and prop materials whose base texture is
/// nearly black, or missing.
#[test]
#[ignore = "debug aid"]
fn dark_textures() {
    let Ok(name) = std::env::var("MAP") else { return };
    let Some(map) = load(&name) else { return };
    for m in map.meshes.iter().chain(map.models.iter().flat_map(|m| m.meshes.iter())) {
        let Some(t) = m.texture.map(|i| &map.textures[i]) else {
            if m.alpha != mashup::map::MapAlpha::Add {
                println!("{} untextured", m.material);
            }
            continue;
        };
        let n = (t.rgba8.len() / 4).max(1) as f32;
        let mean = t
            .rgba8
            .chunks(4)
            .map(|p| p[0] as f32 + p[1] as f32 + p[2] as f32)
            .sum::<f32>()
            / (3.0 * 255.0 * n);
        if mean < 0.05 {
            println!("{} {} {mean:.3} {}x{}", m.material, t.name, t.width, t.height);
        }
        if let Some(d) = m.detail {
            let t = &map.textures[d.texture];
            let n = (t.rgba8.len() / 4).max(1) as f32;
            let dm = t
                .rgba8
                .chunks(4)
                .map(|p| p[0] as f32 + p[1] as f32 + p[2] as f32)
                .sum::<f32>()
                / (3.0 * 255.0 * n);
            if d.mode == 0 && dm < 0.3 {
                println!("{} detail {} mod2x mean {dm:.3}", m.material, t.name);
            }
        }
    }
    // Shipped mips much darker than the full-size image (distant surfaces
    // turn black).
    let mean = |px: &[u8]| {
        let n = (px.len() / 4).max(1) as f32;
        px.chunks(4)
            .map(|p| p[0] as f32 + p[1] as f32 + p[2] as f32)
            .sum::<f32>()
            / (3.0 * 255.0 * n)
    };
    for t in &map.textures {
        let full = mean(&t.rgba8);
        for (level, mip) in t.mips.iter().enumerate() {
            let m = mean(mip);
            if (m - full).abs() > 0.25 * full.max(0.05) {
                println!("{} mip {} mean {m:.3} vs {full:.3}", t.name, level + 1);
                break;
            }
        }
    }
}

/// Debug aid: which world mesh a `--views` pixel shows. `MAP=cs_office
/// EYE=x,y,z LOOK=yaw,pitch PX=250,330 cargo test ... --ignored pick --
/// --nocapture` (engine space and angles as in views.json; 1280x720).
#[test]
#[ignore = "debug aid"]
fn pick() {
    use bevy::math::{Quat, Vec3};
    let (Ok(name), Ok(eye), Ok(look), Ok(px)) = (
        std::env::var("MAP"),
        std::env::var("EYE"),
        std::env::var("LOOK"),
        std::env::var("PX"),
    ) else {
        return;
    };
    let Some(map) = load(&name) else { return };
    let nums = |s: &str| s.split(',').map(|v| v.parse::<f32>().unwrap()).collect::<Vec<_>>();
    let (e, l, p) = (nums(&eye), nums(&look), nums(&px));
    let eye = Vec3::new(e[0], e[1], e[2]);
    // 74 degrees vertical (CS:S's 90 at 4:3), 16:9.
    let ty = (73.74f32 / 2.0).to_radians().tan();
    let (x, y) = ((p[0] + 0.5) / 640.0 - 1.0, 1.0 - (p[1] + 0.5) / 360.0);
    let local = Vec3::new(x * ty * 16.0 / 9.0, y * ty, -1.0).normalize();
    let rot = Quat::from_rotation_y(l[0].to_radians()) * Quat::from_rotation_x(l[1].to_radians());
    let dir = rot * local;
    let mut hits = Vec::new();
    for m in &map.meshes {
        for tri in m.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(m.positions[tri[k] as usize]));
            let (e1, e2) = (b - a, c - a);
            let h = dir.cross(e2);
            let det = e1.dot(h);
            if det.abs() < 1e-9 {
                continue;
            }
            let s = eye - a;
            let u = s.dot(h) / det;
            let q = s.cross(e1);
            let v = dir.dot(q) / det;
            let t = e2.dot(q) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0 {
                hits.push((
                    t,
                    m.material.clone(),
                    m.skybox,
                    m.alpha,
                    m.texture.map(|i| map.textures[i].name.clone()),
                ));
            }
        }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    for h in hits.iter().take(5) {
        println!("{h:?}");
    }
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
