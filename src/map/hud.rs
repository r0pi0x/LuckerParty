//! A game's own HUD look, for any game: fonts, panel layout in a virtual
//! 640x480 screen (scaled by the window height, as Source's proportional
//! HUD is), icon glyphs and colours. Games fill it from their HUD files;
//! the client draws with it when present.

use std::{collections::HashMap, sync::Arc};

use bevy::prelude::*;

/// A horizontal or vertical coordinate in the virtual 640x480 screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HudCoord {
    /// From the left (top) edge.
    Start(f32),
    /// From the centre: `c-28`.
    Centre(f32),
    /// From the right (bottom) edge, measured back: `r157`.
    End(f32),
}

impl HudCoord {
    /// Parse `8`, `c-28`, `r157` (Source HUD layout syntax).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some(r) = s.strip_prefix(['r', 'R']) {
            return r.trim().parse().ok().map(HudCoord::End);
        }
        if let Some(c) = s.strip_prefix(['c', 'C']) {
            return c.trim().parse().ok().map(HudCoord::Centre);
        }
        s.parse().ok().map(HudCoord::Start)
    }

    /// Pixels from the start edge on a screen side of `length` pixels, at
    /// `scale` pixels per virtual unit.
    pub fn resolve(self, length: f32, scale: f32) -> f32 {
        match self {
            HudCoord::Start(v) => v * scale,
            HudCoord::Centre(v) => length / 2.0 + v * scale,
            HudCoord::End(v) => length - v * scale,
        }
    }
}

/// One HUD panel's box and the offsets of what it draws, in virtual units.
#[derive(Clone, Debug, PartialEq)]
pub struct HudPanel {
    pub x: HudCoord,
    pub y: HudCoord,
    pub wide: f32,
    pub tall: f32,
    /// Rounded background colour (RGBA 0..255), when it has one.
    pub background: Option<[u8; 4]>,
    pub icon: Vec2,
    pub digit: Vec2,
    pub digit2: Vec2,
}

/// A HUD font: its file's bytes and the size the game asks for (virtual
/// pixels at 480 lines).
#[derive(Clone, Debug)]
pub struct HudFont {
    pub data: Arc<Vec<u8>>,
    pub tall: f32,
}

/// A game's HUD look.
#[derive(Clone, Debug, Default)]
pub struct GameHud {
    /// By the game's font name (e.g. `HudNumbers`, `Icons`).
    pub fonts: HashMap<String, HudFont>,
    /// By panel name (e.g. `HudHealth`).
    pub panels: HashMap<String, HudPanel>,
    /// Icon name -> (font name, glyph).
    pub icons: HashMap<String, (String, char)>,
    /// Named colours (RGBA 0..255).
    pub colors: HashMap<String, [u8; 4]>,
}

impl GameHud {
    pub fn color(&self, name: &str) -> Option<Color> {
        self.colors
            .get(name)
            .map(|[r, g, b, a]| Color::srgba_u8(*r, *g, *b, *a))
    }
}

/// The loaded map's game HUD (see `GameHud`).
#[derive(Resource, Clone, Debug)]
pub struct ActiveHud(pub Arc<GameHud>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_resolve_like_source() {
        // 1080 lines: 2.25 pixels per virtual unit; 1920 wide.
        let s = 1080.0 / 480.0;
        assert_eq!(HudCoord::parse("8").unwrap().resolve(1920.0, s), 18.0);
        assert_eq!(HudCoord::parse("c-28").unwrap().resolve(1920.0, s), 960.0 - 63.0);
        assert_eq!(HudCoord::parse("r157").unwrap().resolve(1920.0, s), 1920.0 - 157.0 * s);
        assert_eq!(HudCoord::parse(" R 12 "), Some(HudCoord::End(12.0)));
        assert_eq!(HudCoord::parse("x"), None);
    }
}
