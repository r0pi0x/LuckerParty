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
    /// Every key as written (lower-case keys), for panel-specific values.
    pub keys: HashMap<String, String>,
}

impl HudPanel {
    /// A numeric key (e.g. `SmallBoxSize`).
    pub fn num(&self, key: &str) -> Option<f32> {
        self.keys.get(&key.to_lowercase())?.trim().parse().ok()
    }
}

/// A HUD font: its file's bytes and the size the game asks for (virtual
/// pixels at 480 lines).
#[derive(Clone, Debug)]
pub struct HudFont {
    pub data: Arc<Vec<u8>>,
    pub tall: f32,
    /// Added to the screen (`additive 1`), else blended.
    pub additive: bool,
}

/// An icon cut from a texture.
#[derive(Clone, Debug, PartialEq)]
pub struct HudSprite {
    /// Index into `MapData::textures`.
    pub texture: usize,
    /// Pixels: x, y, width, height.
    pub rect: [f32; 4],
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
    /// The client scheme's text fonts by name (`ChatFont`, `Default`):
    /// sizes in order of preference, drawn in system faces.
    pub text_fonts: HashMap<String, Vec<UiFontSize>>,
    /// Icon name -> a rectangle of a texture (pixels).
    pub sprites: HashMap<String, HudSprite>,
    /// The game's own menus, when it describes them (see `GameMenus`).
    pub menus: Option<GameMenus>,
    /// Screens on models (Source's VGUI screens: the C4's keypad display),
    /// by name (`c4_view_panel`).
    pub screens: HashMap<String, ModelScreen>,
}

/// A screen on a model: its panel's size in pixels, its text font and
/// named colours (`C4Panel_Armed`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelScreen {
    pub pixels: Vec2,
    pub font: Option<UiFontSize>,
    pub colors: HashMap<String, [u8; 4]>,
}

impl GameHud {
    pub fn color(&self, name: &str) -> Option<Color> {
        self.colors
            .get(name)
            .map(|[r, g, b, a]| Color::srgba_u8(*r, *g, *b, *a))
    }
}

/// What a panel control is (VGUI's `ControlName`, grouped by how it
/// draws).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiKind {
    /// A full-screen or framing panel (`Frame`, `WizardSubPanel`, the
    /// game's own menu classes): only a box.
    Frame,
    Label,
    /// `Button`, `MouseOverPanelButton`: text in a bordered box that runs
    /// `command` when pressed.
    Button,
    /// A picture (`image`: a key of `GameHud::sprites`) and/or a fill.
    Image,
    /// Wrapped text (the team menu's map description).
    RichText,
    /// A plain box (`Panel`): where something else goes (`ItemInfo`).
    Panel,
    /// A line box (`Divider`).
    Divider,
    /// Anything else, by its class name: not drawn.
    Other(String),
}

/// How a label's text sits in its box (VGUI's `textAlignment`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiAlign {
    #[default]
    West,
    Center,
    East,
    NorthWest,
    North,
    NorthEast,
    SouthWest,
    South,
    SouthEast,
}

impl UiAlign {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "center" => UiAlign::Center,
            "east" => UiAlign::East,
            "north-west" => UiAlign::NorthWest,
            "north" => UiAlign::North,
            "north-east" => UiAlign::NorthEast,
            "south-west" => UiAlign::SouthWest,
            "south" => UiAlign::South,
            "south-east" => UiAlign::SouthEast,
            _ => UiAlign::West,
        }
    }
}

/// One control of a panel layout, positioned inside its parent in the
/// virtual 480-line screen (`HudCoord`s resolve against the parent box).
#[derive(Clone, Debug, PartialEq)]
pub struct UiControl {
    /// `fieldName` as written.
    pub name: String,
    pub kind: UiKind,
    pub x: HudCoord,
    pub y: HudCoord,
    pub wide: f32,
    pub tall: f32,
    /// Draw order (`zpos`), higher on top.
    pub z: i32,
    pub visible: bool,
    pub enabled: bool,
    /// Localised text with the hotkey marker taken out.
    pub text: String,
    /// The key that presses it (`&1` in the text), lower case.
    pub hotkey: Option<char>,
    /// What pressing it does: a console line, or another layout's file.
    pub command: Option<String>,
    /// The scheme font's name (`font`), when not the default.
    pub font: Option<String>,
    pub align: UiAlign,
    /// `fgcolor_override` / `bgcolor_override` (RGBA).
    pub fg: Option<[u8; 4]>,
    pub bg: Option<[u8; 4]>,
    /// `fillColor` of an image panel.
    pub fill: Option<[u8; 4]>,
    /// The picture: a key of `GameHud::sprites`.
    pub image: Option<String>,
    /// `dulltext` / `brighttext`.
    pub dull: bool,
    pub bright: bool,
    pub wrap: bool,
    /// The layout shown while the pointer is over it (a buy item's
    /// description panel): a key of `GameMenus::layouts`.
    pub info: Option<String>,
    /// Every key as written (lower-case keys), for control-specific values
    /// (e.g. `cost`).
    pub keys: HashMap<String, String>,
}

impl UiControl {
    /// A plain control of `kind` at `(x, y)` sized `wide` x `tall` (tests,
    /// and games building layouts by hand).
    pub fn new(name: &str, kind: UiKind, x: f32, y: f32, wide: f32, tall: f32) -> Self {
        Self {
            name: name.to_string(),
            kind,
            x: HudCoord::Start(x),
            y: HudCoord::Start(y),
            wide,
            tall,
            z: 0,
            visible: true,
            enabled: true,
            text: String::new(),
            hotkey: None,
            command: None,
            font: None,
            align: UiAlign::West,
            fg: None,
            bg: None,
            fill: None,
            image: None,
            dull: false,
            bright: false,
            wrap: false,
            info: None,
            keys: HashMap::new(),
        }
    }

    /// Its box in pixels inside a parent box (`parent`, pixels), at
    /// `scale` pixels per virtual unit.
    pub fn rect(&self, parent: Rect, scale: f32) -> Rect {
        let min = parent.min
            + Vec2::new(
                self.x.resolve(parent.width(), scale),
                self.y.resolve(parent.height(), scale),
            );
        Rect::from_corners(min, min + Vec2::new(self.wide, self.tall) * scale)
    }
}

/// A panel layout: its controls in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiLayout {
    pub controls: Vec<UiControl>,
}

impl UiLayout {
    /// A control by name (any case).
    pub fn get(&self, name: &str) -> Option<&UiControl> {
        self.controls.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Every control's box in pixels inside `parent` (as `UiControl::rect`),
    /// in `controls` order; a control pinned to a sibling (VGUI's
    /// `pin_to_sibling`) is placed with its corner `pin_corner_to_sibling`
    /// on the sibling's corner `pin_to_sibling_corner`, moved by its
    /// `xpos`/`ypos`.
    pub fn rects(&self, parent: Rect, scale: f32) -> Vec<Rect> {
        fn corner(r: Rect, n: u32) -> Vec2 {
            let c = r.center();
            match n {
                1 => Vec2::new(r.max.x, r.min.y),
                2 => Vec2::new(r.min.x, r.max.y),
                3 => r.max,
                4 => Vec2::new(c.x, r.min.y),
                5 => Vec2::new(r.max.x, c.y),
                6 => Vec2::new(c.x, r.max.y),
                7 => Vec2::new(r.min.x, c.y),
                _ => r.min,
            }
        }
        fn place(layout: &UiLayout, i: usize, parent: Rect, scale: f32, depth: usize) -> Rect {
            let c = &layout.controls[i];
            let own = c.rect(parent, scale);
            let sibling = c
                .keys
                .get("pin_to_sibling")
                .and_then(|s| layout.controls.iter().position(|o| o.name.eq_ignore_ascii_case(s.trim())))
                .filter(|&s| s != i && depth < 8);
            let Some(s) = sibling else { return own };
            let num = |k: &str| c.keys.get(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
            let offset = |coord: HudCoord| match coord {
                HudCoord::Start(v) | HudCoord::Centre(v) | HudCoord::End(v) => v * scale,
            };
            // The offset points inward from the pinned corner: away from
            // the right or bottom edge when that is where it is pinned.
            let pin = num("pin_corner_to_sibling");
            let sign = Vec2::new(
                if matches!(pin, 1 | 3 | 5) { -1.0 } else { 1.0 },
                if matches!(pin, 2 | 3 | 6) { -1.0 } else { 1.0 },
            );
            let anchor = corner(place(layout, s, parent, scale, depth + 1), num("pin_to_sibling_corner"))
                + Vec2::new(offset(c.x), offset(c.y)) * sign;
            let size = own.size();
            let mine = corner(Rect::from_corners(Vec2::ZERO, size), pin);
            Rect::from_corners(anchor - mine, anchor - mine + size)
        }
        (0..self.controls.len()).map(|i| place(self, i, parent, scale, 0)).collect()
    }
}

/// One size of a scheme font: its family, height (pixels at 480 lines,
/// or, when `yres` names the screen heights it is for, pixels; a line's
/// height, ascent plus descent), weight and smoothing.
#[derive(Clone, Debug, PartialEq)]
pub struct UiFontSize {
    pub family: String,
    pub tall: f32,
    pub weight: u32,
    /// Screen heights (inclusive) this size is for.
    pub yres: Option<(u32, u32)>,
    /// Drawn smoothed (`antialias 1`); else hard-edged.
    pub antialias: bool,
    /// Added to the screen (`additive 1`; HUD text drawn by
    /// `client::hud_text`), else blended.
    pub additive: bool,
}

impl UiFontSize {
    /// Drawn with the family's bold face: weight 600 and up.
    pub fn bold(&self) -> bool {
        self.weight >= 600
    }
}

/// The size of `sizes` meant for a window `height` pixels tall: the one
/// whose `yres` holds it, else the first without one, else the first.
pub fn font_for_height(sizes: &[UiFontSize], height: f32) -> Option<&UiFontSize> {
    let h = height.round() as u32;
    sizes
        .iter()
        .find(|s| s.yres.is_some_and(|(lo, hi)| (lo..=hi).contains(&h)))
        .or_else(|| sizes.iter().find(|s| s.yres.is_none()))
        .or(sizes.first())
}

/// The text height (pixels) in a window `height` pixels tall: the size
/// meant for that height, else the first without one scaled from 480
/// lines (Source's proportional fonts), else the first, scaled.
pub fn font_pixels(sizes: &[UiFontSize], height: f32) -> Option<f32> {
    let s = font_for_height(sizes, height)?;
    Some(if s.yres.is_some_and(|(lo, hi)| (lo..=hi).contains(&(height.round() as u32))) {
        s.tall
    } else {
        s.tall * height / 480.0
    })
}

/// A layout file's path as a `GameMenus::layouts` key: lower case, forward
/// slashes.
pub fn layout_key(path: &str) -> String {
    path.trim().replace('\\', "/").to_lowercase()
}

/// A game's own menus (buy, team) as panel layouts (their text in
/// `GameHud::text_fonts`); without them the client draws plain menus.
#[derive(Clone, Debug, Default)]
pub struct GameMenus {
    /// Layouts by `layout_key` of their file.
    pub layouts: HashMap<String, UiLayout>,
    /// The buy menu's first page per team (`core::Team` number).
    pub buy: HashMap<u8, String>,
    /// The team menu's layout.
    pub team: Option<String>,
    /// The scoreboard's layout: its background, headings and the first
    /// row of each team's cells (`CTPlayerName0`, `TPlayerStatus0`, ...).
    pub scoreboard: Option<String>,
    /// The spectator bars' layout (top and bottom bar, target name, map,
    /// clock, team scores) and the spectator menu's bottom bar (the mode).
    pub spectator: Option<String>,
    pub spectator_menu: Option<String>,
    /// The freeze cam's panel: its frame's layout and what is inside the
    /// frame (`FreezePanelBG`'s controls, placed in it).
    pub freeze_panel: Option<(String, String)>,
    /// The loaded map's description (the team menu's `MapInfo`).
    pub map_info: Option<String>,
    /// The game's localised strings by lower-case token (no `#`), for
    /// text the client fills in (`Cstrike_ScoreBoard_CT`).
    pub strings: HashMap<String, String>,
}

impl GameMenus {
    /// A `#token`'s text (any case; the `#` optional), else `fallback`.
    pub fn string<'a>(&'a self, token: &str, fallback: &'a str) -> &'a str {
        let t = token.trim().trim_start_matches('#').to_lowercase();
        self.strings.get(&t).map_or(fallback, String::as_str)
    }

    /// The buy menu's first page for `team` (any team's when it has none).
    pub fn buy_page(&self, team: Option<u8>) -> Option<&String> {
        team.and_then(|t| self.buy.get(&t))
            .or_else(|| self.buy.get(&2))
            .or_else(|| self.buy.values().next())
    }
}

/// A top-down picture of the map for radars and overviews: which texture,
/// and where it lies in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct MapOverview {
    /// Index into `MapData::textures`.
    pub texture: usize,
    /// Engine-space x and z (meters) of the image's top-left corner.
    pub origin: Vec2,
    /// Meters per image pixel.
    pub meters_per_pixel: f32,
    /// Source overviews' `rotate` flag: how the spectator overview turns
    /// the picture; it doesn't change where world points fall on it (checked
    /// against dust2's nav areas).
    pub rotate: bool,
    /// Image size in pixels.
    pub size: Vec2,
}

impl MapOverview {
    /// Where an engine-space point falls on the image, in pixels.
    pub fn pixel(&self, p: Vec3) -> Vec2 {
        (Vec2::new(p.x, p.z) - self.origin) / self.meters_per_pixel
    }
}

/// The loaded map's overview image, for radars.
#[derive(Resource, Clone, Debug)]
pub struct ActiveOverview(pub MapOverview, pub Handle<Image>);

/// The loaded map's game HUD (see `GameHud`).
#[derive(Resource, Clone, Debug)]
pub struct ActiveHud(pub Arc<GameHud>, pub HashMap<usize, Handle<Image>>);

/// A game's own game menu and options dialog look (Source's GameUI), read
/// from the install once, independent of the map: the scheme's colours,
/// numbers and fonts, the menu's entries, the options pages' layouts and
/// the keyboard page's action list, localised strings, Create Server's pages and options, and
/// the main menu's background pictures and title.
#[derive(Clone, Debug, Default)]
pub struct GameUi {
    /// Named colours and the base settings that name them (`Frame.BgColor`).
    pub colors: HashMap<String, [u8; 4]>,
    /// Base settings that are numbers (`Frame.ClientInsetX`).
    pub numbers: HashMap<String, f32>,
    /// The scheme's fonts by name, sizes in order of preference.
    pub fonts: HashMap<String, Vec<UiFontSize>>,
    /// The game menu's entries, in the game's order.
    pub menu: Vec<GameUiItem>,
    /// Localised strings by lower-case token (no `#`).
    pub strings: HashMap<String, String>,
    /// Options pages by name (`keyboard`, `mouse`, `audio`, `video`,
    /// `multiplayer`), and the keyboard tab's Advanced dialog
    /// (`keyboard_advanced`).
    pub options: HashMap<String, UiLayout>,
    /// The keyboard page's list: sections and actions, labels localised.
    pub actions: Vec<KeyAction>,
    /// Create Server's Game page options (`cfg/settings.scr`), in order.
    pub server_settings: Vec<ServerSetting>,
    /// Multiplayer > Advanced's options (`cfg/user.scr`), in order.
    pub user_settings: Vec<ServerSetting>,
    /// The main menu's background for 4:3 screens and for wider ones,
    /// drawn stretched over the whole screen.
    pub background: Option<UiImage>,
    pub background_wide: Option<UiImage>,
    /// The game's title over the main menu's entries, one line each.
    pub title: Vec<String>,
    /// The title's font: a TrueType file and its height in scheme pixels.
    pub title_font: Option<(Arc<Vec<u8>>, f32)>,
    /// The loading dialog's layout (its frame `LoadingDialog`, `InfoLabel`,
    /// `Progress`, `CancelButton`).
    pub loading: Option<UiLayout>,
    /// The interface's sounds by what they are for (`UiSound`).
    pub sounds: HashMap<UiSound, super::sound::MapSoundClip>,
    /// The server browser's layouts by name (`dialog`: the frame, its
    /// tabs and status line; `page`, `page_filters`: a tab's list and
    /// buttons, without and with the filters shown; `add`: the add server
    /// dialog; `password`: the password dialog).
    pub servers: HashMap<String, UiLayout>,
    /// The server browser's icons by name (`password`, `bots`, and their
    /// `_column` header versions).
    pub server_icons: HashMap<String, UiImage>,
    /// Pictures the options' layouts show (an ImagePanel's `image`, lower
    /// case: the brightness dialog's `gamma`).
    pub option_images: HashMap<String, UiImage>,
}

/// An option of a GameUI options script (Create Server's Game page,
/// Multiplayer > Advanced): the cvar it sets, its label, its control,
/// its default.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerSetting {
    pub cvar: String,
    pub label: String,
    pub kind: ServerSettingKind,
    pub default: String,
}

/// How a server option is set.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerSettingKind {
    /// Typed text.
    Text,
    /// A typed number (its limits, when it has them).
    Number { min: Option<f32>, max: Option<f32> },
    /// A check box: 0 or 1.
    Bool,
    /// A combo box: each entry's label (a `#token` or words) and value.
    List(Vec<(String, String)>),
}

/// What an interface sound is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiSound {
    /// The pointer comes onto a button (VGUI's `sound_armed`).
    Rollover,
    /// A button pressed (`sound_depressed`).
    Click,
    /// A button let go (`sound_released`).
    Release,
    /// The freeze cam's picture taken.
    FreezeCam,
}

impl GameUi {
    /// A `#token`'s text (any case; the `#` optional), else None.
    pub fn string(&self, token: &str) -> Option<&str> {
        let t = token.trim().trim_start_matches('#').to_lowercase();
        self.strings.get(&t).map(String::as_str)
    }

    /// A scheme colour by name.
    pub fn color(&self, name: &str) -> Option<[u8; 4]> {
        self.colors.get(name).copied()
    }

    /// The main menu background for a screen this wide over its height:
    /// the widescreen one past 4:3, else the 4:3 one (either when the
    /// other is missing).
    pub fn background_for(&self, aspect: f32) -> Option<&UiImage> {
        let (four_three, wide) = (self.background.as_ref(), self.background_wide.as_ref());
        if aspect > 4.0 / 3.0 + 0.01 {
            wide.or(four_three)
        } else {
            four_three.or(wide)
        }
    }
}

/// A game menu entry: its text and the GameUI command it runs
/// (`ResumeGame`, `OpenOptionsDialog`, `Quit`, `engine <line>` ...).
#[derive(Clone, Debug, PartialEq)]
pub struct GameUiItem {
    pub label: String,
    pub command: String,
    /// Shown only while a game runs (`OnlyInGame`).
    pub in_game_only: bool,
}

/// A row of the keyboard page's list.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyAction {
    /// A section heading.
    Section(String),
    /// A bindable console line and its description.
    Action { command: String, label: String },
}

/// An RGBA8 picture (sRGB).
#[derive(Clone, Debug, PartialEq)]
pub struct UiImage {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

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

    #[test]
    fn controls_resolve_inside_their_parent() {
        // 720 lines: 1.5 pixels per unit; a 640x480 area centred in 1280.
        let s = 720.0 / 480.0;
        let area = Rect::new(160.0, 0.0, 160.0 + 640.0 * s, 720.0);
        let b = UiControl::new("pistols", UiKind::Button, 52.0, 116.0, 170.0, 20.0);
        let r = b.rect(area, s);
        assert_eq!(r.min, Vec2::new(160.0 + 78.0, 174.0));
        assert_eq!(r.size(), Vec2::new(255.0, 30.0));
        // Right- and centre-anchored, and a child inside a child.
        let mut c = UiControl::new("x", UiKind::Label, 0.0, 0.0, 10.0, 10.0);
        c.x = HudCoord::End(20.0);
        c.y = HudCoord::Centre(-5.0);
        let r = c.rect(Rect::new(0.0, 0.0, 300.0, 200.0), 2.0);
        assert_eq!(r.min, Vec2::new(260.0, 90.0));
        let info = UiControl::new("price", UiKind::Label, 140.0, 134.0, 150.0, 24.0);
        assert_eq!(info.rect(r, 2.0).min, Vec2::new(260.0 + 280.0, 90.0 + 268.0));
    }

    #[test]
    fn pinned_controls_sit_on_their_sibling() {
        // scoreboard.res: "Players Alive" pinned with its bottom-left corner
        // (2) to the count's bottom-right (3), 4 units to the right; and
        // one pinned with its right-centre (5) to a box's left-centre (7).
        let mut l = UiLayout::default();
        l.controls.push(UiControl::new("count", UiKind::Label, 10.0, 20.0, 45.0, 12.0));
        let mut suffix = UiControl::new("suffix", UiKind::Label, 4.0, 0.0, 80.0, 10.0);
        suffix.keys = HashMap::from([
            ("pin_to_sibling".to_string(), "COUNT".to_string()),
            ("pin_corner_to_sibling".to_string(), "2".to_string()),
            ("pin_to_sibling_corner".to_string(), "3".to_string()),
        ]);
        l.controls.push(suffix);
        let mut left = UiControl::new("left", UiKind::Label, 0.0, 0.0, 30.0, 10.0);
        left.keys = HashMap::from([
            ("pin_to_sibling".to_string(), "count".to_string()),
            ("pin_corner_to_sibling".to_string(), "5".to_string()),
            ("pin_to_sibling_corner".to_string(), "7".to_string()),
        ]);
        l.controls.push(left);
        // The terrorists' side: its bottom-right (3) on the count's
        // bottom-left (2), the 4 units now to the left.
        let mut t = UiControl::new("t_suffix", UiKind::Label, 4.0, 0.0, 80.0, 10.0);
        t.keys = HashMap::from([
            ("pin_to_sibling".to_string(), "count".to_string()),
            ("pin_corner_to_sibling".to_string(), "3".to_string()),
            ("pin_to_sibling_corner".to_string(), "2".to_string()),
        ]);
        l.controls.push(t);
        let r = l.rects(Rect::new(0.0, 0.0, 640.0, 480.0), 2.0);
        assert_eq!(r[0], Rect::new(20.0, 40.0, 110.0, 64.0));
        assert_eq!(r[1], Rect::new(118.0, 44.0, 278.0, 64.0));
        assert_eq!(r[2], Rect::new(-40.0, 42.0, 20.0, 62.0));
        assert_eq!(r[3], Rect::new(-148.0, 44.0, 12.0, 64.0));
    }

    #[test]
    fn widescreen_background_past_four_three() {
        let pic = |w| UiImage {
            width: w,
            height: 1,
            rgba8: vec![0; w as usize * 4],
        };
        let mut ui = GameUi {
            background: Some(pic(4)),
            background_wide: Some(pic(16)),
            ..Default::default()
        };
        assert_eq!(ui.background_for(16.0 / 9.0).map(|p| p.width), Some(16));
        assert_eq!(ui.background_for(16.0 / 10.0).map(|p| p.width), Some(16));
        assert_eq!(ui.background_for(4.0 / 3.0).map(|p| p.width), Some(4));
        assert_eq!(ui.background_for(5.0 / 4.0).map(|p| p.width), Some(4));
        ui.background_wide = None;
        assert_eq!(ui.background_for(16.0 / 9.0).map(|p| p.width), Some(4));
        ui.background = None;
        assert!(ui.background_for(1.0).is_none());
    }

    #[test]
    fn font_size_for_the_window() {
        let size = |tall: f32, yres: Option<(u32, u32)>| UiFontSize {
            family: "Verdana".into(),
            tall,
            weight: 900,
            yres,
            antialias: false,
            additive: false,
        };
        let sizes = [
            size(12.0, Some((480, 599))),
            size(20.0, Some((1024, 1199))),
            size(9.0, None),
        ];
        assert_eq!(font_pixels(&sizes, 1080.0), Some(20.0));
        assert_eq!(font_pixels(&sizes, 720.0), Some(13.5));
        assert_eq!(font_pixels(&sizes[..1], 960.0), Some(24.0));
        assert_eq!(font_pixels(&[], 960.0), None);
        assert_eq!(font_for_height(&sizes, 1080.0).map(|s| s.tall), Some(20.0));
        assert_eq!(font_for_height(&sizes, 720.0).map(|s| s.tall), Some(9.0));
        assert!(sizes[0].bold());
        assert_eq!(layout_key(r"Resource\UI/BuyPistols_TER.res "), "resource/ui/buypistols_ter.res");
    }
}
