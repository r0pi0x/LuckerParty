//! The window's and taskbar's icon: the game's own, read from the CS:S
//! install at run time (`cstrike/resource/game.ico`, which Linux and
//! Windows installs both have; never committed). Without an install, no
//! icon. Wayland compositors take icons from desktop files, not from the
//! window, so there it shows only under X11 and on Windows.

use std::path::{Path, PathBuf};

use bevy::{ecs::system::NonSendMarker, prelude::*, winit::WINIT_WINDOWS};

pub struct WindowIconPlugin;

impl Plugin for WindowIconPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, set_icon);
    }
}

/// The icon file in a CS:S install (Linux and Windows installs ship it).
const ICON_FILE: &str = "cstrike/resource/game.ico";

/// The install's icon file, if it has one.
pub fn icon_file(install: &Path) -> Option<PathBuf> {
    Some(install.join(ICON_FILE)).filter(|p| p.is_file())
}

/// An icon file's largest image as RGBA: (pixels, width, height).
pub fn decode(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    decode_ico(&bytes).ok_or_else(|| format!("{}: no icon image this reader knows", path.display()))
}

/// The largest image of a `.ico` file (PNG entries, or bitmaps of 24 or
/// 32 bits per pixel with their transparency mask), as RGBA.
fn decode_ico(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let u16_at = |o: usize| Some(u16::from_le_bytes(bytes.get(o..o + 2)?.try_into().ok()?));
    let u32_at = |o: usize| Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?));
    if u16_at(0)? != 0 || u16_at(2)? != 1 {
        return None;
    }
    let mut best: Option<(Vec<u8>, u32, u32)> = None;
    for i in 0..u16_at(4)? as usize {
        let entry = 6 + 16 * i;
        let (size, offset) = (u32_at(entry + 8)? as usize, u32_at(entry + 12)? as usize);
        let Some(data) = bytes.get(offset..offset.checked_add(size)?) else {
            continue;
        };
        let image = if data.starts_with(b"\x89PNG") {
            image::load_from_memory_with_format(data, image::ImageFormat::Png).ok().map(|i| {
                let rgba = i.to_rgba8();
                let (w, h) = rgba.dimensions();
                (rgba.into_raw(), w, h)
            })
        } else {
            decode_dib(data)
        };
        if let Some(image) = image
            && best.as_ref().is_none_or(|b| image.1 * image.2 > b.1 * b.2)
        {
            best = Some(image);
        }
    }
    best
}

/// An icon's bitmap: a BITMAPINFOHEADER, rows bottom-up (24 or 32 bits),
/// then the 1-bit mask (set = transparent). Its height counts both.
fn decode_dib(d: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let i32_at = |o: usize| Some(i32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?));
    let header = i32_at(0)? as usize;
    let (w, h2) = (i32_at(4)?, i32_at(8)?);
    let bits = u16::from_le_bytes(d.get(14..16)?.try_into().ok()?) as usize;
    if i32_at(16)? != 0 || !(bits == 24 || bits == 32) || w <= 0 || h2 <= 0 || w > 256 {
        return None;
    }
    let (w, h) = (w as usize, h2 as usize / 2);
    let stride = (w * bits).div_ceil(32) * 4;
    let mask_stride = w.div_ceil(32) * 4;
    let pixels = d.get(header..header + stride * h)?;
    let mask = d.get(header + stride * h..header + stride * h + mask_stride * h);
    let has_alpha = bits == 32 && pixels.chunks(4).any(|p| p[3] != 0);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let row = h - 1 - y;
        for x in 0..w {
            let p = &pixels[row * stride + x * bits / 8..];
            let clear = mask.is_some_and(|m| m[row * mask_stride + x / 8] & (0x80 >> (x % 8)) != 0);
            let alpha = if has_alpha {
                p[3]
            } else if clear {
                0
            } else {
                255
            };
            out[(y * w + x) * 4..][..4].copy_from_slice(&[p[2], p[1], p[0], alpha]);
        }
    }
    Some((out, w as u32, h as u32))
}

/// The game's icon, from the install `mashup.local.toml` names.
fn load() -> Option<winit::window::Icon> {
    let install = crate::mount::config::LocalConfig::load()
        .ok()?
        .game_path(crate::games::cs_source::GAME)?;
    let path = icon_file(&install)?;
    let loaded = decode(&path).and_then(|(rgba, w, h)| {
        winit::window::Icon::from_rgba(rgba, w, h).map_err(|e| format!("{}: {e}", path.display()))
    });
    match loaded {
        Ok(icon) => {
            info!("window icon: {}", path.display());
            Some(icon)
        }
        Err(e) => {
            warn!("window icon: {e}");
            None
        }
    }
}

#[derive(Default)]
enum IconState {
    #[default]
    Unloaded,
    /// Loaded, waiting for the window to exist.
    Loaded(winit::window::Icon),
    Done,
}

/// Load the icon once and set it on the windows once they exist (winit's
/// windows live on the main thread).
fn set_icon(_main_thread: NonSendMarker, mut state: Local<IconState>) {
    if matches!(*state, IconState::Unloaded) {
        *state = load().map_or(IconState::Done, IconState::Loaded);
    }
    let IconState::Loaded(icon) = &*state else { return };
    let set = WINIT_WINDOWS.with_borrow(|windows| {
        if windows.windows.is_empty() {
            return false;
        }
        for window in windows.windows.values() {
            window.set_window_icon(Some(icon.clone()));
            #[cfg(target_os = "windows")]
            {
                use winit::platform::windows::WindowExtWindows;
                window.set_taskbar_icon(Some(icon.clone()));
            }
        }
        true
    });
    if set {
        *state = IconState::Done;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 24-bit icon: red, green on the bottom row; blue and a
    /// masked-out pixel on top.
    #[test]
    fn icon_bitmaps_decode_with_their_mask() {
        let mut dib = Vec::new();
        for v in [40i32, 2, 4] {
            dib.extend(v.to_le_bytes());
        }
        dib.extend(1u16.to_le_bytes());
        dib.extend(24u16.to_le_bytes());
        dib.extend([0u8; 24]);
        // Rows bottom-up, BGR, padded to 4 bytes.
        dib.extend([0, 0, 255, 0, 255, 0, 0, 0]);
        dib.extend([255, 0, 0, 9, 9, 9, 0, 0]);
        // Mask rows bottom-up: the top row's second pixel is clear.
        dib.extend([0, 0, 0, 0]);
        dib.extend([0x40, 0, 0, 0]);
        let mut ico = vec![0, 0, 1, 0, 1, 0, 2, 2, 0, 0, 1, 0, 24, 0];
        ico.extend((dib.len() as u32).to_le_bytes());
        ico.extend(22u32.to_le_bytes());
        ico.extend(&dib);
        let (rgba, w, h) = decode_ico(&ico).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(rgba, [0, 0, 255, 255, 9, 9, 9, 0, 255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn the_installs_icon_decodes_when_there_is_one() {
        let Some(install) = crate::mount::config::LocalConfig::load()
            .ok()
            .and_then(|c| c.game_path(crate::games::cs_source::GAME))
        else {
            return;
        };
        let Some(path) = icon_file(&install) else { return };
        let (rgba, w, h) = decode(&path).unwrap();
        assert!(w >= 16 && h >= 16, "{w}x{h}");
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // Not all transparent.
        assert!(rgba.chunks(4).any(|p| p[3] > 0));
        assert!(winit::window::Icon::from_rgba(rgba, w, h).is_ok());
    }
}
