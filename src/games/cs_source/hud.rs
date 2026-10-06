//! CS:S's HUD look, read from the install: panel layout
//! (`scripts/hudlayout.res`), fonts and colours (`resource/clientscheme.res`
//! and the TTFs it lists), icon glyphs (`scripts/mod_textures.txt`).

use std::{collections::HashMap, sync::Arc};

use bevy::prelude::*;

use super::material::MaterialLoader;
use crate::map::hud::{GameHud, HudCoord, HudFont, HudPanel};

/// A KeyValues tree: each key holds a string or a block.
#[derive(Debug, Clone)]
enum Kv {
    Value(String),
    Block(Vec<(String, Kv)>),
}

impl Kv {
    fn get(&self, key: &str) -> Option<&Kv> {
        match self {
            Kv::Block(items) => items.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Kv::Value(_) => None,
        }
    }
    fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Kv::Value(v) => Some(v),
            Kv::Block(_) => None,
        }
    }
    fn items(&self) -> &[(String, Kv)] {
        match self {
            Kv::Block(items) => items,
            Kv::Value(_) => &[],
        }
    }
}

/// Parse KeyValues text; platform conditionals (`[$WIN32]`) are dropped.
fn parse(text: &str) -> Kv {
    let tokens: Vec<String> = super::surfaceprops::tokens(text)
        .into_iter()
        .filter(|t| !(t.starts_with('[') && t.ends_with(']')))
        .collect();
    fn block(t: &[String], i: &mut usize) -> Kv {
        let mut items = Vec::new();
        while *i < t.len() && t[*i] != "}" {
            let key = t[*i].clone();
            *i += 1;
            if *i >= t.len() {
                break;
            }
            if t[*i] == "{" {
                *i += 1;
                let b = block(t, i);
                *i += 1; // the closing brace
                items.push((key, b));
            } else {
                items.push((key, Kv::Value(t[*i].clone())));
                *i += 1;
            }
        }
        Kv::Block(items)
    }
    let mut i = 0;
    block(&tokens, &mut i)
}

fn color(text: &str) -> Option<[u8; 4]> {
    let v: Vec<u8> = text.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    match v.as_slice() {
        [r, g, b] => Some([*r, *g, *b, 255]),
        [r, g, b, a] => Some([*r, *g, *b, *a]),
        _ => None,
    }
}

/// A TrueType file's family name (name table, ID 1).
fn family_name(ttf: &[u8]) -> Option<String> {
    let u16_at = |o: usize| ttf.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |o: usize| ttf.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let tables = u16_at(4)? as usize;
    let name = (0..tables)
        .map(|i| 12 + i * 16)
        .find(|&o| ttf.get(o..o + 4) == Some(b"name"))
        .and_then(|o| u32_at(o + 8))? as usize;
    let count = u16_at(name + 2)? as usize;
    let strings = name + u16_at(name + 4)? as usize;
    let mut best: Option<String> = None;
    for r in 0..count {
        let rec = name + 6 + r * 12;
        let (platform, id) = (u16_at(rec)?, u16_at(rec + 6)?);
        if id != 1 {
            continue;
        }
        let (len, off) = (u16_at(rec + 8)? as usize, u16_at(rec + 10)? as usize);
        let bytes = ttf.get(strings + off..strings + off + len)?;
        let text = if platform == 0 || platform == 3 {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            bytes.iter().map(|&b| b as char).collect()
        };
        if best.is_none() || platform == 3 {
            best = Some(text);
        }
    }
    best
}

/// The HUD look from the install, or None when its files are missing.
pub fn load(materials: &mut MaterialLoader) -> Option<GameHud> {
    let text = |m: &MaterialLoader, p: &str| m.read(p).map(|b| String::from_utf8_lossy(&b).into_owned());
    let scheme = parse(&text(materials, "resource/clientscheme.res")?);
    let scheme = scheme.items().first().map(|(_, v)| v.clone()).unwrap_or(scheme);
    let layout = parse(&text(materials, "scripts/hudlayout.res")?);
    let layout = layout.items().first().map(|(_, v)| v.clone()).unwrap_or(layout);
    let mut hud = GameHud::default();

    // Colours: named ones, then base settings that refer to them by name.
    if let Some(c) = scheme.get("Colors") {
        for (k, v) in c.items() {
            if let Kv::Value(v) = v
                && let Some(rgba) = color(v)
            {
                hud.colors.insert(k.clone(), rgba);
            }
        }
    }
    if let Some(b) = scheme.get("BaseSettings") {
        for (k, v) in b.items() {
            if let Kv::Value(v) = v {
                let rgba = color(v).or_else(|| hud.colors.get(v).copied());
                if let Some(rgba) = rgba {
                    hud.colors.insert(k.clone(), rgba);
                }
            }
        }
    }

    // Font files by family name, then the scheme's fonts by their name.
    let mut files: HashMap<String, Arc<Vec<u8>>> = HashMap::new();
    for (_, path) in scheme.get("CustomFontFiles").map(Kv::items).unwrap_or_default() {
        let Kv::Value(path) = path else { continue };
        let Some(bytes) = materials.read(path) else { continue };
        if let Some(family) = family_name(&bytes) {
            files.insert(family.to_lowercase(), Arc::new(bytes));
        }
    }
    for (name, f) in scheme.get("Fonts").map(Kv::items).unwrap_or_default() {
        // The first size entry (the 480-line one).
        let Some((_, first)) = f.items().first() else { continue };
        let (Some(family), Some(tall)) = (first.str("name"), first.str("tall").and_then(|t| t.parse().ok())) else {
            continue;
        };
        if let Some(data) = files.get(&family.to_lowercase()) {
            hud.fonts.insert(
                name.clone(),
                HudFont {
                    data: data.clone(),
                    tall,
                },
            );
        }
    }

    // Icons drawn with font glyphs.
    if let Some(t) = text(materials, "scripts/mod_textures.txt") {
        let t = parse(&t);
        // Entries sit in a "TextureData" block (under the file's root key).
        fn find<'a>(kv: &'a Kv) -> Option<&'a Kv> {
            kv.get("TextureData")
                .or_else(|| kv.items().iter().find_map(|(_, v)| find(v)))
        }
        let t = find(&t).cloned().unwrap_or(t);
        for (name, v) in t.items() {
            if let (Some(font), Some(ch)) = (v.str("font"), v.str("character").and_then(|c| c.chars().next())) {
                hud.icons.insert(name.clone(), (font.to_string(), ch));
            }
        }
    }

    // Panels.
    let num = |p: &Kv, k: &str| p.str(k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
    for (name, p) in layout.items() {
        let (Some(x), Some(y)) = (
            p.str("xpos").and_then(HudCoord::parse),
            p.str("ypos").and_then(HudCoord::parse),
        ) else {
            continue;
        };
        hud.panels.insert(
            name.clone(),
            HudPanel {
                x,
                y,
                wide: num(p, "wide"),
                tall: num(p, "tall"),
                background: p.str("bgcolor_override").and_then(color),
                icon: Vec2::new(num(p, "icon_xpos"), num(p, "icon_ypos")),
                digit: Vec2::new(num(p, "digit_xpos"), num(p, "digit_ypos")),
                digit2: Vec2::new(num(p, "digit2_xpos"), num(p, "digit2_ypos")),
            },
        );
    }
    (!hud.panels.is_empty()).then_some(hud)
}

/// The map's radar picture: `resource/overviews/<map>.txt` (upper-left
/// world corner `pos_x`/`pos_y`, `scale` world units per pixel of a
/// 1024-wide image, `rotate`) and its material, preferring the `_radar`
/// variant the game's radar uses.
pub fn overview(materials: &mut MaterialLoader, map: &str) -> Option<crate::map::hud::MapOverview> {
    let text = materials.read(&format!("resource/overviews/{}.txt", map.to_lowercase()))?;
    let kv = parse(&String::from_utf8_lossy(&text));
    let o = &kv.items().first()?.1;
    let num = |k: &str| o.str(k).and_then(|v| v.trim().parse::<f32>().ok());
    let material = o.str("material")?.to_string();
    // The _radar variant only when it exists (resolving a missing material
    // counts as a load warning).
    let radar = format!("{material}_radar");
    let name = if materials.read(&format!("materials/{radar}.vmt")).is_some() {
        radar
    } else {
        material
    };
    let texture = materials.resolve(&name).texture?;
    let size = {
        let t = &materials.textures[texture];
        Vec2::new(t.width as f32, t.height as f32)
    };
    let (pos_x, pos_y, scale) = (num("pos_x")?, num("pos_y")?, num("scale")?);
    let unit = super::bsp::METERS_PER_UNIT;
    // Source x is engine x, Source y is engine -z; scale is for a 1024-pixel
    // image.
    Some(crate::map::hud::MapOverview {
        texture,
        origin: Vec2::new(pos_x * unit, -pos_y * unit),
        meters_per_pixel: scale * unit * 1024.0 / size.x,
        rotate: num("rotate").is_some_and(|r| r != 0.0),
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyvalues_with_conditionals() {
        let kv = parse(
            r#""Layout" { HudHealth { "xpos" "8" [$WIN32] "ypos" "446" "wide" "80" } // c
               Colors { "Orange" "255 176 0 255" } }"#,
        );
        let root = &kv.items()[0].1;
        let h = root.get("hudhealth").unwrap();
        assert_eq!(h.str("xpos"), Some("8"));
        assert_eq!(h.str("ypos"), Some("446"));
        assert_eq!(
            color(root.get("Colors").unwrap().str("Orange").unwrap()),
            Some([255, 176, 0, 255])
        );
    }
}
