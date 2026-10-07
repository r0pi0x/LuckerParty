//! Sprites (`env_sprite`): lamp glows and similar, from
//! specs/cs_source/sprites_dust.md. Glow modes (3, 9) draw additively over
//! everything when visible; additive (5) is depth tested.

use bevy::prelude::*;
use vbsp::Bsp;

use super::{
    bsp::{METERS_PER_UNIT, to_engine},
    material::MaterialLoader,
};
use crate::map::{MapData, MapSprite};

fn parse3(v: &str) -> Option<Vec3> {
    let mut it = v.split_whitespace().filter_map(|p| p.parse::<f32>().ok());
    Some(Vec3::new(it.next()?, it.next()?, it.next()?))
}

/// Scale as the client sees it: clamped to 0..64, then sent in 0.25 steps
/// from 0.25 (0.8 becomes 0.75).
pub fn quantize_scale(scale: f32) -> f32 {
    ((scale.clamp(0.0, 64.0) / 0.25).floor() * 0.25).max(0.25)
}

pub fn add_sprites(bsp: &Bsp, materials: &mut MaterialLoader, data: &mut MapData) {
    let mut skipped = 0;
    for (index, e) in bsp
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.prop("classname") == Some("env_sprite"))
    {
        let (Some(origin), Some(model)) = (e.prop("origin").and_then(parse3), e.prop("model")) else {
            continue;
        };
        let mode: u32 = e.prop("rendermode").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let fx: u32 = e.prop("renderfx").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        // Only the modes dust2-era maps use for lights are drawn so far.
        if !matches!(mode, 3 | 5 | 9) {
            skipped += 1;
            continue;
        }
        // A named sprite without "Start on" starts hidden (the logic can
        // show it).
        let flags: u32 = e.prop("spawnflags").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let named = e.prop("targetname").is_some_and(|n| !n.is_empty());
        let start_on = !named || flags & 1 != 0;
        let path = model.to_lowercase().replace('\\', "/");
        let path = path
            .trim_start_matches("materials/")
            .trim_end_matches(".vmt")
            .trim_end_matches(".spr");
        let Some(texture) = materials.resolve(path).texture else {
            continue;
        };
        let t = &materials.textures[texture];
        let scale = quantize_scale(e.prop("scale").and_then(|v| v.trim().parse().ok()).unwrap_or(1.0));
        let amt = e
            .prop("renderamt")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(255.0)
            / 255.0;
        let rgb = e.prop("rendercolor").and_then(parse3).unwrap_or(Vec3::splat(255.0)) / 255.0;
        let glow = mode != 5;
        // Glows: vertex rgb = rendercolor x blend x glow factor, where the
        // blend is renderamt and Constant Glow (renderfx 14) multiplies by
        // renderamt again (no distance fade); vertex alpha is renderamt.
        // Additive: rendercolor, alpha renderamt, once each (confirmed in
        // a RenderDoc capture of CS:S: vertex colour = rendercolor/255,
        // alpha 25/255 for dust2's tunnel halos).
        let rgb = if glow {
            let factor = if fx == 14 { amt } else { 1.0 };
            (rgb * 255.0 * amt * factor).floor() / 255.0
        } else {
            rgb
        };
        data.sprites.push(MapSprite {
            position: to_engine(vbsp::Vector {
                x: origin.x,
                y: origin.y,
                z: origin.z,
            }),
            texture,
            size: Vec2::new(t.width as f32, t.height as f32) * scale * METERS_PER_UNIT,
            color: rgb.extend(amt),
            glow,
            proxy: e
                .prop("GlowProxySize")
                .or_else(|| e.prop("glowproxysize"))
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(2.0)
                .round()
                .clamp(1.0, 64.0)
                * METERS_PER_UNIT,
            entity: Some(index),
            start_on,
        });
    }
    if skipped > 0 {
        data.warnings
            .push(format!("{skipped} sprites in render modes not drawn yet"));
    }
}
