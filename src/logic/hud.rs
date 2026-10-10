//! HUD messages (game_text; specs/source/entity_io.md, "game_text"): what
//! the server sends and how a client fades, scans out and places them.
//! The client draws them (`client::game_text`).

use bevy::prelude::*;

/// A map's screen shake (env_shake), engine space: the client shakes
/// the local view with it (`client::senses`). Amplitude and radius in
/// metres; amplitude 0 stops the map's shakes.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct MapShake {
    pub at: Vec3,
    pub amplitude: f32,
    pub frequency: f32,
    pub duration: f32,
    pub radius: f32,
}

/// Channels a client keeps (a message's channel is taken mod this).
pub const HUD_CHANNELS: usize = 6;
/// Longest message a client keeps, characters.
pub const HUD_TEXT_MAX: usize = 511;

/// One HUD message as sent.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// Seconds per unit of a screen fade's duration and hold as sent: they go
/// as unsigned 16-bit counts of 1/512 s, truncated
/// (specs/source/game_entities.md 3).
pub const FADE_TIME_UNIT: f32 = 1.0 / 512.0;

/// A screen fade as sent (env_fade; specs/source/game_entities.md 3).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenFade {
    /// Seconds, already truncated to `FADE_TIME_UNIT`.
    pub duration: f32,
    pub hold: f32,
    pub color: [u8; 3],
    pub alpha: u8,
    /// Fade from the colour (in) rather than to it (out).
    pub fade_in: bool,
    pub modulate: bool,
    /// Never expires (until purged or a respawn clears it).
    pub stay_out: bool,
    /// Replaces every fade the player had.
    pub purge: bool,
}

impl ScreenFade {
    /// A time as the message carries it: truncated to 1/512 s, clamped to
    /// 65535 units.
    pub fn quantize(seconds: f32) -> f32 {
        ((seconds.max(0.0) / FADE_TIME_UNIT) as u32).min(65535) as f32 * FADE_TIME_UNIT
    }

    /// Its alpha `t` seconds after it arrived; None once it is gone.
    pub fn alpha_at(&self, t: f32) -> Option<u8> {
        let (d, h, a) = (self.duration, self.hold, self.alpha as f32);
        if d <= 0.0 {
            // The end and hold aren't offset by the arrival time: gone at
            // once unless it stays out.
            return self.stay_out.then_some(if self.fade_in { 0 } else { self.alpha });
        }
        if t > d + h && !self.stay_out {
            return None;
        }
        let v = if self.fade_in {
            a * (h + d - t) / d
        } else {
            a * (1.0 - (d - t) / d)
        };
        Some(v.clamp(0.0, a) as u8)
    }
}

/// The screen fades the local client shows, with when each arrived
/// (seconds, `Time` elapsed), and the key hint text (env_hudhint).
#[derive(Resource, Default, Clone, Debug)]
pub struct ScreenFades {
    pub fades: Vec<(ScreenFade, f64)>,
    /// The key hint text and when it came (empty: hidden).
    pub hint: Option<(String, f64)>,
}

impl ScreenFades {
    /// A fade arrives (a purging one replaces the rest).
    pub fn show(&mut self, fade: ScreenFade, now: f64) {
        if fade.purge {
            self.fades.clear();
        }
        self.fades.push((fade, now));
    }

    /// What the screen shows at `now`: colour (the per-channel integer
    /// average of the live fades), alpha (the largest) and whether any
    /// modulates; None with no live fade. Gone ones are dropped.
    pub fn at(&mut self, now: f64) -> Option<([u8; 3], u8, bool)> {
        self.fades.retain(|(f, t0)| f.alpha_at((now - t0) as f32).is_some());
        let live: Vec<(ScreenFade, u8)> = self
            .fades
            .iter()
            .filter_map(|(f, t0)| f.alpha_at((now - t0) as f32).map(|a| (*f, a)))
            .collect();
        if live.is_empty() {
            return None;
        }
        let n = live.len() as u32;
        let sum = live.iter().fold([0u32; 3], |s, (f, _)| {
            [s[0] + f.color[0] as u32, s[1] + f.color[1] as u32, s[2] + f.color[2] as u32]
        });
        let color = sum.map(|c| (c / n) as u8);
        let alpha = live.iter().map(|(_, a)| *a).max().unwrap_or(0);
        Some((color, alpha, live.iter().any(|(f, _)| f.modulate)))
    }

    /// A respawn clears the fades.
    pub fn clear(&mut self) {
        self.fades.clear();
    }
}

/// What map logic shows on one player's HUD (`to`) or everyone's (None):
/// the bridge shows it for the local player, and a network server sends
/// it to the client the player belongs to.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct HudEvent {
    pub to: Option<Entity>,
    pub what: HudShow,
}

/// One thing a `HudEvent` shows.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum HudShow {
    Text(HudMessage),
    Fade(ScreenFade),
    /// The key hint panel's text (env_hudhint); empty hides it.
    Hint(String),
}

/// Show a `HudShow` on this client's HUD at `now` (`Time` elapsed).
pub fn show_on_hud(world: &mut World, what: HudShow, now: f64) {
    match what {
        HudShow::Text(m) => world.get_resource_or_init::<HudMessages>().show(m, now),
        HudShow::Fade(f) => world.get_resource_or_init::<ScreenFades>().show(f, now),
        HudShow::Hint(text) => world.get_resource_or_init::<ScreenFades>().hint = Some((text, now)),
    }
}
