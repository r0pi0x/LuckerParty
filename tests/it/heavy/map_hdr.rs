//! mat_hdr_level on de_dust2 from a real CS:S install, headless: level 2
//! loads the map's HDR lighting, sky and tone-map bounds; level 0 loads
//! exactly what the default load does (the LDR path refcmp matches).
//! Skipped without an install.

use mashup::{
    games::{self, cs_source},
    map::MapData,
    mount::config::LocalConfig,
};

fn load(level: u8) -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map_level("cs_source:de_dust2", level).expect("load de_dust2"))
}

fn bsp_bytes() -> Vec<u8> {
    let install = LocalConfig::load().unwrap().game_path(cs_source::GAME).unwrap();
    cs_source::mount::open(&install)
        .unwrap()
        .read("maps/de_dust2.bsp")
        .unwrap()
}

#[test]
fn level_zero_leaves_the_ldr_path_untouched() {
    let Some(ldr) = load(0) else { return };
    let default = games::load_map("cs_source:de_dust2").unwrap();
    assert!(ldr.look.hdr.is_none());
    assert!(ldr.look.source_ldr_lightmaps, "LDR lightmap encoding");
    assert_eq!(format!("{:?}", ldr.look), format!("{:?}", default.look));
    let (a, b) = (ldr.lightmap.as_ref().unwrap(), default.lightmap.as_ref().unwrap());
    assert_eq!((a.width, a.height), (b.width, b.height));
    assert!(a.rgb == b.rgb, "same lightmap texels");
    assert!(ldr.sky.as_ref().unwrap().hdr.is_none(), "LDR sky only");
}

#[test]
fn level_two_loads_hdr_lighting_sky_and_tonemap_bounds() {
    use mashup::games::cs_source::lightmap;

    let Some(hdr) = load(2) else { return };
    let look = &hdr.look;
    assert!(!look.source_ldr_lightmaps, "HDR lightmaps stay linear");
    let tone = look.hdr.as_ref().expect("an HDR look");
    // dust2's logic_auto: SetAutoExposureMax 1; min and bloom default.
    assert_eq!(tone.exposure, Some((0.5, 1.0)));
    assert_eq!(tone.bloom_scale, 1.0);

    // The atlas holds lump 53's samples: a face's first luxel matches.
    let bytes = bsp_bytes();
    let hdr_lump = lightmap::hdr_lighting_lump(&bytes).expect("dust2 has HDR lighting");
    assert!(!std::ptr::eq(hdr_lump.as_ptr(), lightmap::lighting_lump(&bytes).as_ptr()));
    let bsp = vbsp::Bsp::read(&bytes).unwrap();
    let face = bsp
        .models()
        .next()
        .unwrap()
        .faces()
        .find(|f| f.light_offset >= 0 && f.styles[1] == 255 && !f.texture().flags.contains(vbsp::TextureFlags::BUMPLIGHT))
        .unwrap();
    let want = lightmap::face_samples(hdr_lump, &face).unwrap().rgb[0];
    let atlas = hdr.lightmap.as_ref().unwrap();
    assert!(atlas.rgb.contains(&want), "HDR luxel {want:?} in the atlas");

    // The HDR sky: sky_dust_hdr*'s 16-bit faces, brighter than 1 somewhere.
    let sky = hdr.sky.as_ref().unwrap().hdr.as_ref().expect("HDR sky faces");
    assert!(sky.iter().all(|f| f.width > 0 && f.rgb.len() == (f.width * f.height) as usize));
    let peak = sky.iter().flat_map(|f| f.rgb.iter()).map(|c| c[0].max(c[1]).max(c[2])).fold(0.0, f32::max);
    assert!(peak > 1.0, "HDR sky peak {peak}");

    // Props are lit from the HDR ambient cubes and lights.
    assert!(hdr.light_field.is_some());
}

#[test]
fn maps_without_hdr_lighting_load_ldr_at_any_level() {
    let installed = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        return;
    }
    // cs_office (BSP v19) has no HDR lump.
    let office = games::load_map_level("cs_source:cs_office", 2).unwrap();
    assert!(office.look.hdr.is_none() && office.look.source_ldr_lightmaps);
}
