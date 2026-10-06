//! HUD messages (game_text; specs/source/entity_io.md, "game_text"): what
//! the server sends and how a client fades, scans out and places them.
//! The client draws them (`client::game_text`).

use bevy::prelude::*;

/// Channels a client keeps (a message's channel is taken mod this).
pub const HUD_CHANNELS: usize = 6;
/// Longest message a client keeps, characters.
pub const HUD_TEXT_MAX: usize = 511;

/// One HUD message as sent.
#[derive(Clone, Debug, PartialEq)]
pub struct HudMessage {
    pub text: String,
    /// Screen fractions; -1 centres, other negatives count from the
    /// right/bottom.
    pub x: f32,
    pub y: f32,
    pub channel: usize,
    /// 0 fade in/out, 1 credits (drawn as 0), 2 scan out.
    pub effect: u8,
    /// RGB, 0..255 (alpha is ignored by the client).
    pub color: [u8; 3],
    pub color2: [u8; 3],
    pub fade_in: f32,
    pub fade_out: f32,
    pub hold: f32,
    pub fx_time: f32,
}

impl HudMessage {
    /// Channel the client stores it in.
    pub fn slot(&self) -> usize {
        self.channel % HUD_CHANNELS
    }

    /// The text as the client keeps it (truncated).
    pub fn shown_text(&self) -> String {
        self.text.chars().take(HUD_TEXT_MAX).collect()
    }

    fn chars(&self) -> usize {
        self.shown_text().chars().count()
    }

    /// Seconds from receipt until it is gone.
    pub fn lifetime(&self) -> f32 {
        if self.effect == 2 {
            self.fade_in * self.chars() as f32 + self.fade_out + self.hold
        } else {
            self.fade_in + self.hold + self.fade_out
        }
    }

    /// Opacity of the whole message at local time `t` (effects 0 and 1;
    /// for effect 2, the fade out after the scan and hold).
    pub fn opacity(&self, t: f32) -> f32 {
        let (fade_in, hold) = if self.effect == 2 {
            (self.fade_in * self.chars() as f32, self.hold)
        } else {
            (self.fade_in, self.hold)
        };
        if t < 0.0 || t > self.lifetime() {
            return 0.0;
        }
        if self.effect != 2 && t < fade_in {
            return if fade_in > 0.0 { t / fade_in } else { 1.0 };
        }
        if t <= fade_in + hold {
            return 1.0;
        }
        if self.fade_out <= 0.0 {
            return 0.0;
        }
        (1.0 - (t - fade_in - hold) / self.fade_out).clamp(0.0, 1.0)
    }

    /// Effect 2: character `k`'s colour at `t` (None before it appears):
    /// colour 2 when it appears, blending to colour over fx_time.
    pub fn scan_color(&self, k: usize, t: f32) -> Option<Vec3> {
        let appear = (k + 1) as f32 * self.fade_in;
        if t < appear {
            return None;
        }
        let w = if self.fx_time > 0.0 {
            ((t - appear) / self.fx_time).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let c = |c: [u8; 3]| Vec3::from(c.map(|v| v as f32 / 255.0));
        Some(c(self.color2).lerp(c(self.color), w))
    }

    /// Top-left of a line `w` pixels wide in a block `h` pixels high, on a
    /// `screen` (pixels), clamped on screen.
    pub fn place(&self, screen: Vec2, w: f32, widest: f32, h: f32) -> Vec2 {
        let x = if self.x == -1.0 {
            (screen.x - w) / 2.0
        } else if self.x >= 0.0 {
            self.x * screen.x
        } else {
            (1.0 + self.x) * screen.x - widest
        };
        let y = if self.y == -1.0 {
            (screen.y - h) / 2.0
        } else if self.y >= 0.0 {
            self.y * screen.y
        } else {
            (1.0 + self.y) * screen.y - h
        };
        Vec2::new(
            x.clamp(0.0, (screen.x - w).max(0.0)),
            y.clamp(0.0, (screen.y - h).max(0.0)),
        )
    }
}

/// Messages the local client shows: one per channel, with when it
/// arrived (seconds, `Time` elapsed).
#[derive(Resource, Default, Clone, Debug)]
pub struct HudMessages {
    pub channels: [Option<(HudMessage, f64)>; HUD_CHANNELS],
}

impl HudMessages {
    pub fn show(&mut self, message: HudMessage, now: f64) {
        let slot = message.slot();
        self.channels[slot] = Some((message, now));
    }
}
