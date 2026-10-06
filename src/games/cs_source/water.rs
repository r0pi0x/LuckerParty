//! Source Water materials (specs/cs_source/water.md) to the neutral
//! `map::water::MapWaterMaterial`: the material's keys at DX9 (its
//! `Water_DX90` block over the root), in-map patch materials merged over
//! the material they include (proxies kept, spec open question 8), the
//! proxies that animate it (TextureScroll on `$bumptransform`,
//! AnimatedTexture on `$normalmap`, WaterLOD), the map's
//! `water_lod_control` distances, and the `$bottommaterial`.

use std::collections::HashMap;

use bevy::math::Vec2;

use super::{bsp::METERS_PER_UNIT, material::MaterialLoader, surfaceprops::tokens};
use crate::map::water::{MapWaterMaterial, WaterEnvmap};

/// A KeyValues node: a value or a block of named children, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Value(String),
    Block(Vec<(String, Node)>),
}

/// Parse KeyValues text into its top-level entries.
pub fn parse(text: &str) -> Vec<(String, Node)> {
    let t = tokens(text);
    let mut i = 0;
    block(&t, &mut i)
}

fn block(t: &[String], i: &mut usize) -> Vec<(String, Node)> {
    let mut out = Vec::new();
    while *i < t.len() {
        if t[*i] == "}" {
            *i += 1;
            break;
        }
        let key = t[*i].clone();
        *i += 1;
        match t.get(*i).map(String::as_str) {
            Some("{") => {
                *i += 1;
                out.push((key, Node::Block(block(t, i))));
            }
            Some("}") | None => {}
            Some(v) => {
                out.push((key, Node::Value(v.to_string())));
                *i += 1;
            }
        }
    }
    out
}

/// `src` merged over `dst`: values replace (or are added), blocks merge
/// recursively. An empty block changes nothing.
fn merge(dst: &mut Vec<(String, Node)>, src: &[(String, Node)], only_new: bool) {
    for (k, v) in src {
        let existing = dst.iter_mut().find(|(dk, _)| dk.eq_ignore_ascii_case(k));
        match (existing, v) {
            (Some((_, Node::Block(d))), Node::Block(s)) => merge(d, s, only_new),
            (Some(slot), _) => {
                if !only_new {
                    slot.1 = v.clone();
                }
            }
            (None, _) => dst.push((k.clone(), v.clone())),
        }
    }
}

/// A material's shader name and body, with patch materials resolved.
pub fn resolve(text: &str, read: &dyn Fn(&str) -> Option<String>, depth: u32) -> Option<(String, Vec<(String, Node)>)> {
    let top = parse(text);
    let (shader, body) = top.into_iter().find_map(|(k, v)| match v {
        Node::Block(b) => Some((k, b)),
        Node::Value(_) => None,
    })?;
    if !shader.eq_ignore_ascii_case("patch") {
        return Some((shader, body));
    }
    if depth > 8 {
        return None;
    }
    let include = body.iter().find_map(|(k, v)| match v {
        Node::Value(v) if k.eq_ignore_ascii_case("include") => Some(v.clone()),
        _ => None,
    })?;
    let (shader, mut base) = resolve(&read(&include)?, read, depth + 1)?;
    for (k, v) in &body {
        if let Node::Block(b) = v {
            if k.eq_ignore_ascii_case("replace") {
                merge(&mut base, b, false);
            } else if k.eq_ignore_ascii_case("insert") {
                merge(&mut base, b, true);
            }
        }
    }
    Some((shader, base))
}

/// Keys by lower-case name.
pub type Keys = HashMap<String, String>;

/// The material's keys as DX9 sees them (lower-case): the root's values,
/// then a `<shader>_dx9`/`_dx90` block's over them; and its proxies.
pub fn keys_and_proxies(body: &[(String, Node)]) -> (Keys, Vec<(String, Keys)>) {
    let values = |b: &[(String, Node)]| -> HashMap<String, String> {
        b.iter()
            .filter_map(|(k, v)| match v {
                Node::Value(v) => Some((k.to_lowercase(), v.clone())),
                Node::Block(_) => None,
            })
            .collect()
    };
    let mut keys = values(body);
    let mut proxies = Vec::new();
    for (k, v) in body {
        let Node::Block(b) = v else { continue };
        let k = k.to_lowercase();
        if (k.ends_with("_dx9") || k.ends_with("_dx90")) && !k.contains("hdr") {
            keys.extend(values(b));
        } else if k == "proxies" {
            for (name, p) in b {
                if let Node::Block(p) = p {
                    proxies.push((name.to_lowercase(), values(p)));
                }
            }
        }
    }
    (keys, proxies)
}

/// A colour: `[r g b]` as given, `{r g b}` as 0..255 bytes; gamma 0..1.
pub fn colour(v: &str) -> Option<[f32; 3]> {
    let v = v.trim();
    let bytes = v.starts_with('{');
    let n: Vec<f32> = v
        .trim_matches(|c| c == '[' || c == ']' || c == '{' || c == '}')
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    let c = match n.as_slice() {
        [a, b, c, ..] => [*a, *b, *c],
        [a] => [*a; 3],
        _ => return None,
    };
    Some(if bytes { c.map(|x| x / 255.0) } else { c })
}

fn vec2(v: &str) -> Option<Vec2> {
    let n: Vec<f32> = v
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    match n.as_slice() {
        [a, b, ..] => Some(Vec2::new(*a, *b)),
        [a] => Some(Vec2::splat(*a)),
        _ => None,
    }
}

/// The map's `water_lod_control` distances (units): its keys, 1000 and
/// 2000 when unset; without the entity, 0 and 0.1.
pub fn lod_distances(entity: Option<(Option<&str>, Option<&str>)>) -> (f32, f32) {
    match entity {
        None => (0.0, 0.1),
        Some((start, end)) => {
            let num = |v: Option<&str>, d: f32| v.and_then(|v| v.trim().parse().ok()).unwrap_or(d);
            (num(start, 1000.0), num(end, 2000.0))
        }
    }
}

/// Everything a Water material says, before textures are loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterVmt {
    pub material: MapWaterMaterial,
    pub normal_map: String,
    pub envmap: Option<String>,
    pub bottom: Option<String>,
}

/// Read a Water material's keys and proxies (spec sections 1, 2 and 4).
/// `lod`: the map's cheap distances in units.
pub fn water_vmt(name: &str, body: &[(String, Node)], lod: (f32, f32)) -> WaterVmt {
    let (keys, proxies) = keys_and_proxies(body);
    let get = |k: &str| keys.get(k).map(|v| v.trim().to_string());
    let num = |k: &str, d: f32| get(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(d);
    let flag = |k: &str| get(k).is_some_and(|v| v != "0" && !v.is_empty());
    let texture = |k: &str| get(k).filter(|v| !v.is_empty());
    let mut m = MapWaterMaterial {
        name: name.to_string(),
        refract: texture("$refracttexture").is_some(),
        refract_amount: num("$refractamount", 0.0),
        reflect: texture("$reflecttexture").is_some(),
        reflect_entities: flag("$reflectentities"),
        reflect_amount: num("$reflectamount", 0.8),
        reflect_tint: get("$reflecttint").and_then(|v| colour(&v)).unwrap_or([1.0; 3]),
        // The `srgb?` form wins in CS:S (open question 9).
        fog_color: get("srgb?$fogcolor")
            .or_else(|| get("$fogcolor"))
            .and_then(|v| colour(&v))
            .unwrap_or([1.0, 0.0, 0.0]),
        fog_start: num("$fogstart", 0.0) * METERS_PER_UNIT,
        fog_end: num("$fogend", 0.0) * METERS_PER_UNIT,
        above_water: get("$abovewater").is_none_or(|v| v != "0"),
        force_cheap: flag("$forcecheap"),
        cheap_start: num("$cheapwaterstartdistance", 500.0) * METERS_PER_UNIT,
        cheap_end: num("$cheapwaterenddistance", 1000.0) * METERS_PER_UNIT,
        fixed_reflect_weight: flag("$nofresnel").then(|| num("$reflectblendfactor", 1.0)),
        scroll1: get("$scroll1").and_then(|v| vec2(&v)).unwrap_or_default(),
        scroll2: get("$scroll2").and_then(|v| vec2(&v)).unwrap_or_default(),
        ..Default::default()
    };
    for (proxy, p) in &proxies {
        let var = |k: &str| p.get(k).map(|v| v.trim().to_lowercase());
        let pnum = |k: &str| p.get(k).and_then(|v| v.trim().parse::<f32>().ok());
        match proxy.as_str() {
            "texturescroll" if var("texturescrollvar").as_deref() == Some("$bumptransform") => {
                let rate = pnum("texturescrollrate").unwrap_or(0.0);
                let angle = pnum("texturescrollangle").unwrap_or(0.0).to_radians();
                m.bump_scroll = rate * Vec2::new(angle.cos(), angle.sin());
            }
            "animatedtexture" if var("animatedtexturevar").as_deref() == Some("$normalmap") => {
                m.frame_rate = pnum("animatedtextureframerate").unwrap_or(15.0);
            }
            "waterlod" => {
                m.cheap_start = lod.0 * METERS_PER_UNIT;
                m.cheap_end = lod.1 * METERS_PER_UNIT;
            }
            _ => {}
        }
    }
    WaterVmt {
        material: m,
        normal_map: texture("$normalmap").unwrap_or_else(|| "dev/water_normal".to_string()),
        envmap: texture("$envmap"),
        bottom: texture("$bottommaterial"),
    }
}

/// The Water material `name` (as the BSP names it), with its normal-map
/// frames, cubemap and bottom material loaded. None when it isn't a Water
/// material.
pub fn load(materials: &mut MaterialLoader, name: &str, lod: (f32, f32)) -> Option<MapWaterMaterial> {
    load_inner(materials, name, lod, 0)
}

fn load_inner(materials: &mut MaterialLoader, name: &str, lod: (f32, f32), depth: u32) -> Option<MapWaterMaterial> {
    let path = format!(
        "materials/{}.vmt",
        crate::mount::normalize(name).trim_end_matches(".vmt")
    );
    let text = materials.read_text(&path)?;
    let read = |p: &str| materials.read_text(p);
    let (shader, body) = resolve(&text, &read, 0)?;
    if !shader.eq_ignore_ascii_case("water") {
        return None;
    }
    let vmt = water_vmt(name, &body, lod);
    let mut m = vmt.material;
    m.normal_frames = materials.texture_frames(&vmt.normal_map, false);
    m.envmap = vmt.envmap.map(|e| {
        if e.eq_ignore_ascii_case("env_cubemap") {
            WaterEnvmap::Nearest
        } else {
            materials.cubemap(&e).map_or(WaterEnvmap::Nearest, WaterEnvmap::Baked)
        }
    });
    if depth == 0
        && let Some(bottom) = vmt.bottom
    {
        m.bottom = load_inner(materials, &bottom, lod, depth + 1).map(Box::new);
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
"Water"
{
	"%tooltexture" "dev/water_normal"
	"$abovewater" 1
	"$bottommaterial" "dev/dev_waterbeneath2"
	"$fogcolor" "[.15 .1 0]"
	"$fogstart" 5
	"$fogend" 50
	"$envmap" "env_cubemap"
	"Water_DX90"
	{
		"$refracttexture" "_rt_WaterRefraction"
		"$refractamount" ".2"
		"$normalmap" "dev/water_normal"
	}
	"Proxies"
	{
		"AnimatedTexture"
		{
			"animatedtexturevar" "$normalmap"
			"animatedtextureframenumvar" "$bumpframe"
			"animatedtextureframerate" 16
		}
		"TextureScroll"
		{
			"texturescrollvar" "$bumptransform"
			"texturescrollrate" .01
			"texturescrollangle" 45
		}
		"WaterLOD" { }
	}
}
"#;

    const PATCH: &str = r#"
"patch"
{
	"include" "materials/liquids/test.vmt"
	"replace"
	{
		"$envmap" "maps/de_test/c0_0_0"
		"Proxies" { }
	}
}
"#;

    #[test]
    fn patch_keeps_proxies_and_dx90_wins() {
        let read = |p: &str| (p == "materials/liquids/test.vmt").then(|| BASE.to_string());
        let (shader, body) = resolve(PATCH, &read, 0).expect("resolves");
        assert_eq!(shader, "Water");
        let vmt = water_vmt("maps/de_test/liquids/test", &body, (500.0, 2000.0));
        assert_eq!(vmt.envmap.as_deref(), Some("maps/de_test/c0_0_0"));
        assert_eq!(vmt.bottom.as_deref(), Some("dev/dev_waterbeneath2"));
        let m = &vmt.material;
        assert!(m.refract && !m.reflect && m.above_water);
        assert!((m.refract_amount - 0.2).abs() < 1e-6);
        assert_eq!(m.fog_color, [0.15, 0.1, 0.0]);
        assert!((m.fog_end - 50.0 * METERS_PER_UNIT).abs() < 1e-6);
        assert_eq!(m.frame_rate, 16.0);
        let s = 0.01 * 45f32.to_radians().cos();
        assert!((m.bump_scroll - Vec2::splat(s)).length() < 1e-6);
        // The WaterLOD proxy takes the map's distances.
        assert!((m.cheap_start - 500.0 * METERS_PER_UNIT).abs() < 1e-6);
        assert!((m.cheap_end - 2000.0 * METERS_PER_UNIT).abs() < 1e-6);
    }

    #[test]
    fn srgb_fog_colour_and_bytes() {
        let text = r#""Water" { "$fogcolor" "{7 16 18}" "srgb?$fogcolor" "{21 48 52}" "$abovewater" 0 "$scroll1" "[.01 .01 0]" "$reflecttexture" "_rt_WaterReflection" "$nofresnel" 1 }"#;
        let (_, body) = resolve(text, &|_| None, 0).unwrap();
        let m = water_vmt("x", &body, (0.0, 0.1)).material;
        assert_eq!(m.fog_color, [21.0 / 255.0, 48.0 / 255.0, 52.0 / 255.0]);
        assert!(!m.above_water && m.reflect && !m.refract);
        assert_eq!(m.scroll1, Vec2::new(0.01, 0.01));
        assert_eq!(m.fixed_reflect_weight, Some(1.0));
        // No WaterLOD proxy: the material's own (default) distances.
        assert!((m.cheap_end - 1000.0 * METERS_PER_UNIT).abs() < 1e-6);
    }

    #[test]
    fn lod_defaults() {
        assert_eq!(lod_distances(None), (0.0, 0.1));
        assert_eq!(lod_distances(Some((None, None))), (1000.0, 2000.0));
        assert_eq!(lod_distances(Some((Some("500"), Some("2000")))), (500.0, 2000.0));
    }
}
