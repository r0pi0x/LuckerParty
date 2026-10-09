//! The one place UI text gets its font: a scheme font by name (the GameUI
//! scheme's `Default`, `ConsoleText`, the client scheme's `ChatFont`,
//! `HudHintText`) resolved to a face and a size, as CS:S draws it.
//!
//! The schemes name Windows system faces (Tahoma, Verdana, Lucida Console,
//! Courier New, Trebuchet MS, Arial), which the game doesn't ship:
//! they are read from the system's font folders at startup (Windows'
//! `%WINDIR%\Fonts` and the per-user fonts folder; on Linux and elsewhere
//! the fontconfig folders), never bundled. A face the system lacks falls
//! back to the nearest installed sans or mono (DejaVu, Liberation, Noto);
//! without any, Bevy's built-in face. The game's own font files (`cs.ttf`,
//! `cstrike.ttf`: HUD numbers and icon glyphs) come with the map's HUD
//! (`map::hud::GameHud::fonts`) and are kept here by scheme name too.
//!
//! Scheme heights are a line's (ascent plus descent, as Windows sizes a
//! font); Bevy sizes the em square, so each face's own line-to-em ratio
//! (its OS/2 Windows metrics) converts one to the other, and a fallback
//! face keeps the line height the scheme asks for. Weights from 600 take
//! the bold face; `antialias 1` fonts are smoothed and others drawn hard
//! edged when the real face is present (fallback faces stay smoothed:
//! they aren't hinted for it).
//!
//! The faces load as the app is built. The scheme's default text face
//! (the GameUI `Default`: Tahoma) also replaces Bevy's default font, so text that names no font
//! isn't drawn in Bevy's built-in face. Without a window (headless tests)
//! nothing is loaded and lookups give default fonts.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use bevy::{prelude::*, text::FontSmoothing};

use crate::map::hud::{ActiveHud, GameUi, UiFontSize, font_for_height};

pub struct FontsPlugin;

impl Plugin for FontsPlugin {
    fn build(&self, app: &mut App) {
        // Loaded as the app is built (Bevy's text plugin comes first), so
        // every startup system finds the faces and the default font is
        // replaced before any text is laid out.
        let mut ui = UiFonts::default();
        if let Some(mut fonts) = app.world_mut().get_resource_mut::<Assets<Font>>() {
            load_system_faces(&mut fonts, &mut ui);
        }
        app.insert_resource(ui).add_systems(PreUpdate, take_hud_fonts);
    }
}

/// Which scheme a font name is looked up in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// GameUI's `SourceScheme.res`: menus, dialogs, the console. Sizes in
    /// screen pixels.
    Source,
    /// The game's `ClientScheme.res`: HUD, chat, hints, scoreboard, buy and
    /// team menus. Sizes proportional (at 480 lines).
    Client,
}

/// What kind of face a family is, for falling back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Sans,
    Mono,
}

/// A system family the schemes use: its Windows file names (regular,
/// bold) and the installed faces that stand in for it elsewhere.
struct Family {
    name: &'static str,
    kind: Kind,
    files: [&'static [&'static str]; 2],
}

/// Stand-ins end with Windows' own faces, for families Windows lacks.
const SANS: [&[&str]; 2] = [
    &[
        "DejaVuSans.ttf",
        "LiberationSans-Regular.ttf",
        "NotoSans-Regular.ttf",
        "tahoma.ttf",
        "arial.ttf",
    ],
    &[
        "DejaVuSans-Bold.ttf",
        "LiberationSans-Bold.ttf",
        "NotoSans-Bold.ttf",
        "tahomabd.ttf",
        "arialbd.ttf",
    ],
];
/// Narrower than Verdana: Arial-metric first.
const NARROW_SANS: [&[&str]; 2] = [
    &["LiberationSans-Regular.ttf", "DejaVuSans.ttf", "NotoSans-Regular.ttf"],
    &["LiberationSans-Bold.ttf", "DejaVuSans-Bold.ttf", "NotoSans-Bold.ttf"],
];
const MONO: [&[&str]; 2] = [
    &[
        "DejaVuSansMono.ttf",
        "LiberationMono-Regular.ttf",
        "NotoSansMono-Regular.ttf",
        "lucon.ttf",
        "cour.ttf",
    ],
    &[
        "DejaVuSansMono-Bold.ttf",
        "LiberationMono-Bold.ttf",
        "NotoSansMono-Bold.ttf",
        "courbd.ttf",
    ],
];
/// Courier New's metric twin first.
const COURIER: [&[&str]; 2] = [
    &[
        "LiberationMono-Regular.ttf",
        "DejaVuSansMono.ttf",
        "NotoSansMono-Regular.ttf",
    ],
    &[
        "LiberationMono-Bold.ttf",
        "DejaVuSansMono-Bold.ttf",
        "NotoSansMono-Bold.ttf",
    ],
];

/// The faces CS:S's schemes name, with their Windows file names, and the
/// fallbacks for each.
const FAMILIES: [(Family, [&[&str]; 2]); 6] = [
    (
        Family {
            name: "tahoma",
            kind: Kind::Sans,
            files: [&["tahoma.ttf"], &["tahomabd.ttf"]],
        },
        NARROW_SANS,
    ),
    (
        Family {
            name: "verdana",
            kind: Kind::Sans,
            files: [&["verdana.ttf"], &["verdanab.ttf"]],
        },
        SANS,
    ),
    (
        Family {
            name: "lucida console",
            kind: Kind::Mono,
            files: [&["lucon.ttf"], &[]],
        },
        MONO,
    ),
    (
        Family {
            name: "courier new",
            kind: Kind::Mono,
            files: [&["cour.ttf"], &["courbd.ttf"]],
        },
        COURIER,
    ),
    (
        Family {
            name: "trebuchet ms",
            kind: Kind::Sans,
            files: [&["trebuc.ttf"], &["trebucbd.ttf"]],
        },
        SANS,
    ),
    (
        Family {
            name: "arial",
            kind: Kind::Sans,
            files: [&["arial.ttf"], &["arialbd.ttf"]],
        },
        NARROW_SANS,
    ),
];

/// The family the schemes' default text uses (GameUI `Default`).
const DEFAULT_FAMILY: &str = "tahoma";
/// The client scheme's usual face, for client fonts it lacks.
const CLIENT_FAMILY: &str = "verdana";
/// Debug overlays' face.
const DEBUG_FAMILY: &str = "lucida console";

/// A loaded face: its font, the family it really is, its line height in
/// ems, and whether it is the family asked for (not a stand-in).
#[derive(Clone, Debug)]
pub struct Face {
    pub handle: Handle<Font>,
    pub family: String,
    pub line_per_em: f32,
    pub exact: bool,
}

/// Scheme fonts, the faces they resolve to, and the game's font files.
#[derive(Resource, Default)]
pub struct UiFonts {
    /// By lower-case family and boldness.
    faces: HashMap<(String, bool), Face>,
    /// Each kind's stand-in, regular and bold.
    generic: HashMap<(bool, bool), Face>,
    source: HashMap<String, Vec<UiFontSize>>,
    client: HashMap<String, Vec<UiFontSize>>,
    /// The game's own font files by scheme name, with their height (480
    /// lines).
    game: HashMap<String, (Handle<Font>, f32)>,
    /// The same as glyph fonts for `hud_text` (with their height at 480
    /// lines).
    game_glyphs: HashMap<String, (super::hud_text::GlyphFont, f32)>,
    /// The system faces' file bytes, for `hud_text`.
    face_bytes: HashMap<AssetId<Font>, std::sync::Arc<Vec<u8>>>,
}

impl UiFonts {
    /// The face for a family name (any case; `Verdana Bold` is Verdana's
    /// bold face), else its kind's stand-in.
    pub fn face(&self, family: &str, bold: bool) -> Option<&Face> {
        let mut name = family.trim().to_lowercase();
        let mut bold = bold;
        if let Some(n) = name.strip_suffix(" bold") {
            name = n.to_string();
            bold = true;
        }
        self.faces
            .get(&(name.clone(), bold))
            .or_else(|| self.faces.get(&(name.clone(), false)))
            .or_else(|| {
                let mono = kind_of(&name) == Kind::Mono;
                self.generic
                    .get(&(mono, bold))
                    .or_else(|| self.generic.get(&(mono, false)))
            })
    }

    /// A scheme font's sizes, when the scheme has it.
    pub fn sizes(&self, scheme: Scheme, name: &str) -> Option<&[UiFontSize]> {
        match scheme {
            Scheme::Source => &self.source,
            Scheme::Client => &self.client,
        }
        .get(name)
        .map(Vec::as_slice)
    }

    /// Text in `family` whose line is `line_px` tall.
    pub fn line(&self, family: &str, bold: bool, line_px: f32, antialias: bool) -> TextFont {
        let face = self.face(family, bold);
        let line_per_em = face.map_or(1.2, |f| f.line_per_em);
        TextFont {
            font: face.map(|f| f.handle.clone()).unwrap_or_default().into(),
            font_size: FontSize::Px((line_px / line_per_em).max(1.0)),
            // Hard edges only for the real face: stand-ins aren't hinted
            // for it.
            font_smoothing: if antialias || !face.is_some_and(|f| f.exact) {
                FontSmoothing::AntiAliased
            } else {
                FontSmoothing::None
            },
            ..default()
        }
    }

    /// A client scheme font (proportional) in a window `height` pixels
    /// tall; without it, `fallback` scheme pixels of the client face.
    pub fn client(&self, name: &str, height: f32, fallback: f32) -> TextFont {
        let sizes = self.sizes(Scheme::Client, name).unwrap_or_default();
        match font_for_height(sizes, height) {
            Some(s) => {
                let px = crate::map::hud::font_pixels(sizes, height).unwrap_or(s.tall);
                self.line(&s.family, s.bold(), px, s.antialias)
            }
            None => self.line(CLIENT_FAMILY, false, fallback * height / 480.0, true),
        }
    }

    /// A GameUI scheme font, its height times `scale` (GameUI sizes are
    /// screen pixels; menus drawn larger pass their scale); without it,
    /// `fallback` (height, bold) in the default face.
    pub fn source(&self, name: &str, height: f32, scale: f32, fallback: (f32, bool)) -> TextFont {
        let sizes = self.sizes(Scheme::Source, name).unwrap_or_default();
        match font_for_height(sizes, height) {
            Some(s) => self.line(&s.family, s.bold(), s.tall * scale, s.antialias),
            None => self.line(DEFAULT_FAMILY, fallback.1, fallback.0 * scale, true),
        }
    }

    /// A GameUI scheme font's face at a size of our own (em pixels): for
    /// text laid out by us in a game's face (the console); `fallback`
    /// until the scheme is read or when it lacks the font.
    pub fn source_face(&self, name: &str, fallback: &str, em_px: f32) -> TextFont {
        let s = self
            .sizes(Scheme::Source, name)
            .and_then(|s| s.first())
            .map(|s| (s.family.as_str(), s.bold()))
            .unwrap_or((fallback, false));
        self.em(s.0, s.1, em_px)
    }

    /// `family` at `em_px` em pixels, smoothed.
    pub fn em(&self, family: &str, bold: bool, em_px: f32) -> TextFont {
        let face = self.face(family, bold);
        TextFont {
            font: face.map(|f| f.handle.clone()).unwrap_or_default().into(),
            font_size: FontSize::Px(em_px),
            ..default()
        }
    }

    /// Debug overlays: a readable mono face (Lucida Console where
    /// installed) at `em_px`.
    pub fn debug(&self, em_px: f32) -> TextFont {
        self.em(DEBUG_FAMILY, false, em_px)
    }

    /// One of the game's font files by scheme name (`HudNumbers`,
    /// `WeaponIcons`) and its height at 480 lines.
    pub fn game(&self, name: &str) -> Option<(Handle<Font>, f32)> {
        self.game.get(name).cloned()
    }

    /// A scheme font as `hud_text` draws it, in a window `height` pixels
    /// tall: the game's font file (its size proportional to 480 lines)
    /// or a client scheme font in its system face (sized as `client`
    /// sizes it), and its cell height in pixels.
    pub fn hud_font(&self, name: &str, height: f32) -> Option<(super::hud_text::GlyphFont, f32)> {
        if let Some((font, tall)) = self.game_glyphs.get(name) {
            return Some((font.clone(), super::hud_text::proportional_tall(*tall, height)));
        }
        let sizes = self.sizes(Scheme::Client, name)?;
        let s = font_for_height(sizes, height)?;
        let face = self.face(&s.family, s.bold())?;
        let data = self.face_bytes.get(&face.handle.id())?.clone();
        let px = crate::map::hud::font_pixels(sizes, height)?;
        Some((
            super::hud_text::GlyphFont {
                data,
                additive: s.additive,
            },
            px,
        ))
    }

    /// Where each char boundary of `text` falls in `font`, window pixels
    /// from its start (one more than the chars): text entries place their
    /// caret and selection by it. A face without its bytes (Bevy's
    /// default) guesses half an em a char.
    pub fn char_offsets(&self, font: &TextFont, text: &str) -> Vec<f32> {
        use ab_glyph::{Font as _, FontRef, ScaleFont as _};
        let em = match font.font_size {
            FontSize::Px(px) => px,
            _ => 16.0,
        };
        let face = match &font.font {
            bevy::text::FontSource::Handle(h) => self.face_bytes.get(&h.id()),
            _ => None,
        }
        .and_then(|bytes| FontRef::try_from_slice(bytes).ok());
        let mut out = Vec::with_capacity(text.chars().count() + 1);
        let mut x = 0.0;
        out.push(0.0);
        match face {
            Some(f) => {
                let upem = f.units_per_em().unwrap_or(1000.0);
                let scaled = f.as_scaled(ab_glyph::PxScale::from(em * f.height_unscaled() / upem));
                let mut last = None;
                for c in text.chars() {
                    let id = scaled.glyph_id(c);
                    if let Some(prev) = last {
                        x += scaled.kern(prev, id);
                    }
                    x += scaled.h_advance(id);
                    last = Some(id);
                    out.push(x);
                }
            }
            None => {
                for _ in text.chars() {
                    x += em * 0.5;
                    out.push(x);
                }
            }
        }
        out
    }

    /// Whether the map's HUD fonts are loaded.
    pub fn has_game_fonts(&self) -> bool {
        !self.game.is_empty()
    }

    /// The GameUI scheme's fonts (once read).
    pub fn set_source(&mut self, ui: &GameUi) {
        self.source = ui.fonts.clone();
    }

    /// Which face each family resolved to, for the log.
    fn report(&self) -> String {
        let mut v: Vec<String> = self
            .faces
            .iter()
            .filter(|((_, bold), _)| !bold)
            .map(|((name, _), f)| format!("{name} -> {}{}", f.family, if f.exact { "" } else { " (stand-in)" }))
            .collect();
        v.sort();
        v.join(", ")
    }
}

fn kind_of(family: &str) -> Kind {
    let f = family.to_lowercase();
    FAMILIES
        .iter()
        .find(|(fam, _)| fam.name == f)
        .map(|(fam, _)| fam.kind)
        .unwrap_or(
            if ["mono", "console", "courier", "fixed"].iter().any(|k| f.contains(k)) {
                Kind::Mono
            } else {
                Kind::Sans
            },
        )
}

/// The folders system fonts live in: Windows' and the user's on Windows,
/// fontconfig's usual ones elsewhere.
fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(w) = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")) {
        dirs.push(PathBuf::from(w).join("Fonts"));
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts"));
    }
    if !cfg!(windows) {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        dirs.extend(data.map(|d| d.join("fonts")));
        dirs.extend(home.map(|h| h.join(".fonts")));
        dirs.extend(
            [
                "/usr/share/fonts",
                "/usr/local/share/fonts",
                "/Library/Fonts",
                "/System/Library/Fonts",
            ]
            .map(PathBuf::from),
        );
    }
    dirs
}

/// Every font file under `dirs` by lower-case file name (first found
/// wins; subfolders a few levels down).
fn index_fonts(dirs: &[PathBuf]) -> HashMap<String, PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut HashMap<String, PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                if depth < 4 {
                    walk(&path, depth + 1, out);
                }
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let lower = name.to_lowercase();
                if lower.ends_with(".ttf") || lower.ends_with(".otf") {
                    out.entry(lower).or_insert(path);
                }
            }
        }
    }
    let mut out = HashMap::new();
    for d in dirs {
        walk(d, 0, &mut out);
    }
    out
}

/// A TrueType file's line height in ems as Windows measures it (OS/2
/// `usWinAscent + usWinDescent` over `head.unitsPerEm`; `hhea` without
/// an OS/2 table).
pub(crate) fn line_per_em(ttf: &[u8]) -> Option<f32> {
    let u16_at = |o: usize| ttf.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let i16_at = |o: usize| u16_at(o).map(|v| v as i16);
    let u32_at = |o: usize| ttf.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let tables = u16_at(4)? as usize;
    let table = |tag: &[u8; 4]| {
        (0..tables)
            .map(|i| 12 + i * 16)
            .find(|&o| ttf.get(o..o + 4) == Some(tag.as_slice()))
            .and_then(|o| u32_at(o + 8))
            .map(|o| o as usize)
    };
    let em = u16_at(table(b"head")? + 18)? as f32;
    if em <= 0.0 {
        return None;
    }
    let line = match table(b"OS/2") {
        Some(os2) => u16_at(os2 + 74)? as f32 + u16_at(os2 + 76)? as f32,
        None => {
            let hhea = table(b"hhea")?;
            i16_at(hhea + 4)? as f32 - i16_at(hhea + 6)? as f32
        }
    };
    (line > 0.0).then_some(line / em)
}

/// The faces of every family the schemes use, and the stand-ins; the
/// default face put in Bevy's default font.
fn load_system_faces(fonts: &mut Assets<Font>, ui: &mut UiFonts) {
    let index = index_fonts(&font_dirs());
    let mut loaded: HashMap<PathBuf, (Handle<Font>, f32, String)> = HashMap::new();
    let mut face_bytes = HashMap::new();
    let mut load = |file: &str, fonts: &mut Assets<Font>| -> Option<(Handle<Font>, f32, String)> {
        let path = index.get(&file.to_lowercase())?;
        if let Some(l) = loaded.get(path) {
            return Some(l.clone());
        }
        let bytes = std::fs::read(path).ok()?;
        let line = line_per_em(&bytes).unwrap_or(1.2);
        let name = path.file_name()?.to_string_lossy().into_owned();
        let handle = fonts.add(Font::from_bytes(bytes.clone()));
        face_bytes.insert(handle.id(), std::sync::Arc::new(bytes));
        let l = (handle, line, name);
        loaded.insert(path.clone(), l.clone());
        Some(l)
    };
    for (fam, fallback) in &FAMILIES {
        for bold in [false, true] {
            let i = bold as usize;
            let exact = fam.files[i].iter().find_map(|f| load(f, fonts));
            let face = match exact {
                Some(f) => Some((f, true)),
                None if bold && !fam.files[1].is_empty() || !bold => {
                    fallback[i].iter().find_map(|f| load(f, fonts)).map(|f| (f, false))
                }
                // No bold face of its own (Lucida Console): the regular.
                None => None,
            };
            if let Some(((handle, line_per_em, family), exact)) = face {
                ui.faces.insert(
                    (fam.name.to_string(), bold),
                    Face {
                        handle,
                        family,
                        line_per_em,
                        exact,
                    },
                );
            }
        }
    }
    for (mono, files) in [(false, SANS), (true, MONO)] {
        for bold in [false, true] {
            if let Some((handle, line_per_em, family)) = files[bold as usize].iter().find_map(|f| load(f, fonts)) {
                ui.generic.insert(
                    (mono, bold),
                    Face {
                        handle,
                        family,
                        line_per_em,
                        exact: false,
                    },
                );
            }
        }
    }
    ui.face_bytes = face_bytes;
    // Bevy's default font: the default text face, before any text is
    // laid out (its font collection reads each asset once, on first use).
    if let Some(face) = ui.face(DEFAULT_FAMILY, false)
        && let Some(font) = fonts.get(&face.handle).cloned()
    {
        let _ = fonts.insert(AssetId::default(), font);
    }
    info!("fonts: {}", ui.report());
}

/// The map's client scheme fonts and the game's font files, when its HUD
/// arrives.
fn take_hud_fonts(hud: Option<Res<ActiveHud>>, mut fonts: Option<ResMut<Assets<Font>>>, mut ui: ResMut<UiFonts>) {
    let Some(hud) = hud else {
        if !ui.game.is_empty() {
            ui.game.clear();
            ui.game_glyphs.clear();
        }
        return;
    };
    if !hud.is_changed() {
        return;
    }
    ui.client = hud.0.text_fonts.clone();
    ui.game.clear();
    ui.game_glyphs = hud
        .0
        .fonts
        .iter()
        .map(|(name, f)| {
            let glyph = super::hud_text::GlyphFont {
                data: f.data.clone(),
                additive: f.additive,
            };
            (name.clone(), (glyph, f.tall))
        })
        .collect();
    if let Some(fonts) = fonts.as_mut() {
        for (name, f) in &hud.0.fonts {
            ui.game
                .insert(name.clone(), (fonts.add(Font::from_bytes(f.data.to_vec())), f.tall));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_fall_back_by_kind() {
        assert_eq!(kind_of("Lucida Console"), Kind::Mono);
        assert_eq!(kind_of("Courier New"), Kind::Mono);
        assert_eq!(kind_of("Some Fixed Face"), Kind::Mono);
        assert_eq!(kind_of("Verdana"), Kind::Sans);
        assert_eq!(kind_of("Unknown"), Kind::Sans);
    }

    #[test]
    fn windows_file_names() {
        let f = |n: &str| FAMILIES.iter().find(|(f, _)| f.name == n).unwrap().0.files;
        assert_eq!(f("tahoma"), [&["tahoma.ttf"][..], &["tahomabd.ttf"][..]]);
        assert_eq!(f("verdana")[0], &["verdana.ttf"]);
        assert_eq!(f("lucida console")[0], &["lucon.ttf"]);
        assert_eq!(f("courier new")[0], &["cour.ttf"]);
        assert_eq!(f("trebuchet ms")[0], &["trebuc.ttf"]);
    }

    /// A minimal font file: head, OS/2 and hhea tables.
    fn ttf(em: u16, win: (u16, u16), hhea: (i16, i16), with_os2: bool) -> Vec<u8> {
        let mut tables: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&em.to_be_bytes());
        tables.push((b"head", head));
        let mut hh = vec![0u8; 36];
        hh[4..6].copy_from_slice(&hhea.0.to_be_bytes());
        hh[6..8].copy_from_slice(&hhea.1.to_be_bytes());
        tables.push((b"hhea", hh));
        if with_os2 {
            let mut os2 = vec![0u8; 96];
            os2[74..76].copy_from_slice(&win.0.to_be_bytes());
            os2[76..78].copy_from_slice(&win.1.to_be_bytes());
            tables.push((b"OS/2", os2));
        }
        let mut out = vec![0, 1, 0, 0];
        out.extend((tables.len() as u16).to_be_bytes());
        out.extend([0u8; 6]);
        let mut offset = 12 + tables.len() * 16;
        let mut body: Vec<u8> = Vec::new();
        for (tag, data) in &tables {
            out.extend(tag.as_slice());
            out.extend([0u8; 4]);
            out.extend((offset as u32).to_be_bytes());
            out.extend((data.len() as u32).to_be_bytes());
            offset += data.len();
            body.extend(data);
        }
        out.extend(body);
        out
    }

    #[test]
    fn line_height_from_windows_metrics() {
        // Tahoma's: 2049 + 423 over 2048.
        let t = line_per_em(&ttf(2048, (2049, 423), (0, 0), true)).unwrap();
        assert!((t - 1.207).abs() < 0.001, "{t}");
        // No OS/2: hhea ascender minus descender.
        let h = line_per_em(&ttf(1000, (0, 0), (900, -300), false)).unwrap();
        assert!((h - 1.2).abs() < 0.001, "{h}");
        assert!(line_per_em(b"not a font").is_none());
    }

    #[test]
    fn without_faces_lookups_give_default_fonts() {
        let mut ui = UiFonts::default();
        assert!(ui.face("Verdana", false).is_none());
        let px = |f: TextFont| match f.font_size {
            FontSize::Px(p) => p,
            other => panic!("{other:?}"),
        };
        // 12 at 480 lines is 24 at 960; a 1.2 line per em.
        assert!((px(ui.client("ChatFont", 960.0, 12.0)) - 20.0).abs() < 1e-3);
        ui.client.insert(
            "ChatFont".into(),
            vec![UiFontSize {
                family: "Verdana".into(),
                tall: 9.0,
                weight: 700,
                yres: None,
                antialias: true,
                additive: false,
            }],
        );
        assert!((px(ui.client("ChatFont", 960.0, 12.0)) - 15.0).abs() < 1e-3);
        assert!((px(ui.source("Missing", 720.0, 1.0, (12.0, false))) - 10.0).abs() < 1e-3);
    }

    #[test]
    fn bold_names_and_stand_ins() {
        let mut ui = UiFonts::default();
        let face = |family: &str, exact| Face {
            handle: Handle::default(),
            family: family.into(),
            line_per_em: 1.0,
            exact,
        };
        ui.faces.insert(("verdana".into(), true), face("verdanab.ttf", true));
        ui.faces.insert(("verdana".into(), false), face("verdana.ttf", true));
        ui.generic.insert((true, false), face("mono.ttf", false));
        assert_eq!(ui.face("Verdana Bold", false).unwrap().family, "verdanab.ttf");
        assert_eq!(ui.face("VERDANA", false).unwrap().family, "verdana.ttf");
        assert_eq!(ui.face("Lucida Console", true).unwrap().family, "mono.ttf");
        assert!(ui.face("Tahoma", false).is_none(), "no sans stand-in loaded");
        // Hard edges for the real face only.
        assert_eq!(
            ui.line("Verdana", false, 12.0, false).font_smoothing,
            FontSmoothing::None
        );
        assert_eq!(
            ui.line("Lucida Console", false, 12.0, false).font_smoothing,
            FontSmoothing::AntiAliased
        );
    }
}
