//! Diagnostic: our lightmap values at a world point, to compare with what
//! CS:S samples there (RenderDoc pixel trace). Ignored by default.

use bevy::math::{Vec2, Vec3};
use mashup::{
    games::cs_source::{lightmap, mount},
    mount::config::LocalConfig,
};

fn srgb_decode(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn bilinear(s: &lightmap::FaceSamples, at: Vec2) -> [f32; 3] {
    let (w, h) = (s.width as i32, s.height as i32);
    let x = at.x.clamp(0.0, (w - 1) as f32);
    let y = at.y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |x: i32, y: i32| s.rgb[(y * w + x) as usize];
    let mut out = [0.0; 3];
    for c in 0..3 {
        let top = px(x0, y0)[c] * (1.0 - fx) + px(x1, y0)[c] * fx;
        let bot = px(x0, y1)[c] * (1.0 - fx) + px(x1, y1)[c] * fx;
        out[c] = top * (1.0 - fy) + bot * fy;
    }
    out
}

#[test]
#[ignore]
fn lightmap_at_point() {
    let point = Vec3::new(-295.36871, 1575.36877, 86.58576);
    let config = LocalConfig::load().unwrap();
    let m = mount::open(&config.game_path("cs_source").unwrap()).unwrap();
    let bytes = m.read("maps/de_dust2.bsp").unwrap();
    let bsp = vbsp::Bsp::read(&bytes).unwrap();
    let lump = lightmap::lighting_lump(&bytes);
    let world = bsp.models().next().unwrap();
    for face in world.faces() {
        let n = face.normal();
        let n = Vec3::new(n.x, n.y, n.z);
        let verts: Vec<Vec3> = face
            .vertices()
            .map(|v| Vec3::new(v.position.x, v.position.y, v.position.z))
            .collect();
        if verts.len() < 3 || (point - verts[0]).dot(n).abs() > 0.5 {
            continue;
        }
        // Inside the convex polygon?
        let side = |i: usize| {
            let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
            (b - a).cross(point - a).dot(n)
        };
        let inside = (0..verts.len()).all(|i| side(i) >= -0.5) || (0..verts.len()).all(|i| side(i) <= 0.5);
        if !inside {
            let c = verts.iter().sum::<Vec3>() / verts.len() as f32;
            if c.distance(point) < 64.0 {
                println!("near: {} centre {c:?}", face.texture().name());
            }
        }
        if !inside {
            continue;
        }
        let tex = face.texture();
        let luxel = lightmap::luxel_coords(
            &tex,
            &face,
            vbsp::Vector {
                x: point.x,
                y: point.y,
                z: point.z,
            },
        );
        println!("face {} material {} luxel {luxel:?}", face.texture().name(), tex.name());
        let show = |label: &str, s: &lightmap::FaceSamples| {
            let l = bilinear(s, luxel);
            let enc = l.map(|v| mashup::map::source_ldr_texel(v));
            let dec = enc.map(|t| srgb_decode(t as f32 / 255.0));
            println!("  {label}: linear L {:.4?} -> texel {enc:?} -> decoded {:.4?}", l, dec);
        };
        if let Some(flat) = lightmap::face_samples(lump, &face) {
            show("flat", &flat);
        }
        if let Some(pages) = lightmap::face_bumped_samples(lump, &face, true) {
            for (i, p) in pages.iter().enumerate() {
                show(&format!("bump {}", i + 1), p);
            }
        }
    }
}

/// Our raw (linear) lightmap values for the flat page and 3 bump pages at
/// each point of a JSON list (MASHUP_POINTS: [{"pos": [x, y, z]}, ...]),
/// written to MASHUP_OUT as JSON.
#[test]
#[ignore]
fn lightmap_at_points() {
    let points: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(std::env::var("MASHUP_POINTS").unwrap()).unwrap()).unwrap();
    let config = LocalConfig::load().unwrap();
    let m = mount::open(&config.game_path("cs_source").unwrap()).unwrap();
    let bytes = m.read("maps/de_dust2.bsp").unwrap();
    let bsp = vbsp::Bsp::read(&bytes).unwrap();
    let lump = lightmap::lighting_lump(&bytes);
    let world = bsp.models().next().unwrap();
    let mut out = Vec::new();
    for p in &points {
        let pos: Vec<f32> = p["pos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap() as f32)
            .collect();
        let point = Vec3::new(pos[0], pos[1], pos[2]);
        let mut found = serde_json::Value::Null;
        for face in world.faces() {
            let n = face.normal();
            let n = Vec3::new(n.x, n.y, n.z);
            let verts: Vec<Vec3> = face
                .vertices()
                .map(|v| Vec3::new(v.position.x, v.position.y, v.position.z))
                .collect();
            if verts.len() < 3 || (point - verts[0]).dot(n).abs() > 1.0 {
                continue;
            }
            let side = |i: usize| {
                let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
                (b - a).cross(point - a).dot(n)
            };
            if !((0..verts.len()).all(|i| side(i) >= -0.5) || (0..verts.len()).all(|i| side(i) <= 0.5)) {
                continue;
            }
            let tex = face.texture();
            let luxel = lightmap::luxel_coords(
                &tex,
                &face,
                vbsp::Vector {
                    x: point.x,
                    y: point.y,
                    z: point.z,
                },
            );
            let flat = lightmap::face_samples(lump, &face).map(|s| bilinear(&s, luxel));
            let bumps: Vec<[f32; 3]> = lightmap::face_bumped_samples(lump, &face, true)
                .map(|ps| ps.iter().map(|s| bilinear(s, luxel)).collect())
                .unwrap_or_default();
            found = serde_json::json!({"material": tex.name(), "flat": flat, "bumps": bumps});
            break;
        }
        out.push(found);
    }
    std::fs::write(
        std::env::var("MASHUP_OUT").unwrap(),
        serde_json::to_string(&out).unwrap(),
    )
    .unwrap();
}

/// The whole lightmap block (flat + bump pages, raw linear) of the face at
/// a point, as JSON to MASHUP_OUT.
#[test]
#[ignore]
fn lightmap_block_at_point() {
    let point = Vec3::new(-295.36871, 1575.36877, 86.58576);
    let config = LocalConfig::load().unwrap();
    let m = mount::open(&config.game_path("cs_source").unwrap()).unwrap();
    let bytes = m.read("maps/de_dust2.bsp").unwrap();
    let bsp = vbsp::Bsp::read(&bytes).unwrap();
    let lump = lightmap::lighting_lump(&bytes);
    for face in bsp.models().next().unwrap().faces() {
        let n = face.normal();
        let n = Vec3::new(n.x, n.y, n.z);
        let verts: Vec<Vec3> = face
            .vertices()
            .map(|v| Vec3::new(v.position.x, v.position.y, v.position.z))
            .collect();
        if verts.len() < 3 || (point - verts[0]).dot(n).abs() > 0.5 {
            continue;
        }
        let side = |i: usize| {
            let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
            (b - a).cross(point - a).dot(n)
        };
        if !((0..verts.len()).all(|i| side(i) >= -0.5) || (0..verts.len()).all(|i| side(i) <= 0.5)) {
            continue;
        }
        let flat = lightmap::face_samples(lump, &face).unwrap();
        let bumps = lightmap::face_bumped_samples(lump, &face, true).unwrap();
        let tex = face.texture();
        let luxel = lightmap::luxel_coords(
            &tex,
            &face,
            vbsp::Vector {
                x: point.x,
                y: point.y,
                z: point.z,
            },
        );
        let out = serde_json::json!({
            "w": flat.width, "h": flat.height, "luxel": [luxel.x, luxel.y],
            "pages": [flat.rgb, bumps[0].rgb, bumps[1].rgb, bumps[2].rgb],
        });
        std::fs::write(std::env::var("MASHUP_OUT").unwrap(), out.to_string()).unwrap();
        return;
    }
    panic!("no face");
}

#[test]
#[ignore]
fn lightmap_face_raw() {
    let point = Vec3::new(-295.36871, 1575.36877, 86.58576);
    let config = LocalConfig::load().unwrap();
    let m = mount::open(&config.game_path("cs_source").unwrap()).unwrap();
    let bytes = m.read("maps/de_dust2.bsp").unwrap();
    let bsp = vbsp::Bsp::read(&bytes).unwrap();
    let lump = lightmap::lighting_lump(&bytes);
    for face in bsp.models().next().unwrap().faces() {
        let n = face.normal();
        let n = Vec3::new(n.x, n.y, n.z);
        let verts: Vec<Vec3> = face
            .vertices()
            .map(|v| Vec3::new(v.position.x, v.position.y, v.position.z))
            .collect();
        if verts.len() < 3 || (point - verts[0]).dot(n).abs() > 0.5 {
            continue;
        }
        let side = |i: usize| {
            let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
            (b - a).cross(point - a).dot(n)
        };
        if !((0..verts.len()).all(|i| side(i) >= -0.5) || (0..verts.len()).all(|i| side(i) <= 0.5)) {
            continue;
        }
        println!(
            "styles {:?} light_offset {} size {:?} texinfo flags {:?}",
            face.styles,
            face.light_offset,
            face.light_map_texture_size,
            face.texture().flags
        );
        let start = face.light_offset as usize;
        let lux: Vec<f32> = lump[start..(start + 60 * 4 * 8).min(lump.len())]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[r, _, _, e]| r as f32 * 2f32.powi(e as i8 as i32) / 255.0)
            .collect();
        for (i, chunk) in lux.chunks(6).enumerate().take(42) {
            println!(
                "row {i:2} (page {} row {}): {:?}",
                i / 10,
                i % 10,
                chunk.iter().map(|v| (v * 100.0).round() / 100.0).collect::<Vec<_>>()
            );
        }
        return;
    }
}
