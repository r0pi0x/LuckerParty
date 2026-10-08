//! Drawing a game's own VGUI menus (`map::hud::GameMenus`): each control
//! of a panel layout as a Bevy UI node, in the virtual 640x480 screen
//! scaled by the window height and centred across it, in the client
//! scheme's colours (`Button.*`, `Label.*`, `Border.*`, `Frame.BgColor`)
//! and text fonts (`fonts::UiFonts`: the client scheme's, in the system's
//! Verdana where installed, else a stand-in). The buy and team menus use this; while one is open the
//! mouse is free (`VguiOpen`), as in CS:S, and grabbed again when it
//! closes.

use std::collections::HashMap;

use bevy::{
    prelude::*,
    text::{Justify, LineBreak},
    window::CursorOptions,
};

use super::fonts::{Scheme, UiFonts};
use crate::map::hud::{ActiveHud, GameHud, UiAlign, UiControl, UiKind, UiLayout};

pub struct VguiPlugin;

impl Plugin for VguiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VguiOpen>()
            .add_systems(Update, style_buttons)
            .add_systems(PostUpdate, cursor);
    }
}

/// Which game-look menu is drawn open now (set by the menus as they draw):
/// the mouse is free while any is.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct VguiOpen {
    pub buy: bool,
    pub team: bool,
}

impl VguiOpen {
    pub fn any(&self) -> bool {
        self.buy || self.team
    }
}

/// Which menu a button belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VguiMenu {
    Buy,
    Team,
}

/// A drawn button: its control's name and command, and whether it works.
#[derive(Component, Clone, Debug)]
pub(super) struct VguiButton {
    pub menu: VguiMenu,
    pub name: String,
    pub command: Option<String>,
    pub enabled: bool,
}

/// What a menu changes about a control as it draws it.
pub(super) struct Shown {
    pub text: Option<String>,
    pub enabled: bool,
    pub visible: bool,
}

impl Shown {
    pub fn of(c: &UiControl) -> Self {
        Self {
            text: None,
            enabled: c.enabled,
            visible: c.visible,
        }
    }
}

/// The visible, working button `key` presses on a page (its `&` hotkey).
pub(super) fn hotkey_button<'a>(
    layout: &'a UiLayout,
    key: char,
    shown: &mut dyn FnMut(&UiControl) -> Shown,
) -> Option<&'a UiControl> {
    layout.controls.iter().find(|c| {
        c.kind == UiKind::Button && c.hotkey == Some(key) && {
            let s = shown(c);
            s.visible && s.enabled
        }
    })
}

/// The number key pressed this frame, as a character.
pub(super) fn digit(keys: &ButtonInput<KeyCode>) -> Option<char> {
    const DIGITS: [KeyCode; 10] = [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    DIGITS
        .iter()
        .position(|k| keys.just_pressed(*k))
        .and_then(|n| char::from_digit(n as u32, 10))
}

/// The virtual 640x480 screen in a window: scaled by its height, centred
/// across its width.
pub(super) fn area(width: f32, height: f32) -> Rect {
    let scale = height / 480.0;
    let w = 640.0 * scale;
    let x = ((width - w) / 2.0).max(0.0);
    Rect::new(x, 0.0, x + w, height)
}

/// Draws layouts for one window size.
pub(super) struct Painter<'a> {
    pub hud: &'a GameHud,
    pub images: &'a HashMap<usize, Handle<Image>>,
    pub fonts: &'a UiFonts,
    pub height: f32,
    pub scale: f32,
}

impl<'a> Painter<'a> {
    pub fn new(hud: &'a ActiveHud, fonts: &'a UiFonts, height: f32) -> Self {
        Self {
            hud: &hud.0,
            images: &hud.1,
            fonts,
            height,
            scale: height / 480.0,
        }
    }

    /// A scheme colour (`Button.TextColor`, `Orange`), else `fallback`.
    pub fn color(&self, name: &str, fallback: [u8; 4]) -> Color {
        let [r, g, b, a] = self.hud.colors.get(name).copied().unwrap_or(fallback);
        Color::srgba_u8(r, g, b, a)
    }

    /// A client scheme font (`Default` when None or missing) at this
    /// window's size.
    pub fn font(&self, name: Option<&str>) -> TextFont {
        let name = name
            .filter(|n| self.fonts.sizes(Scheme::Client, n).is_some())
            .unwrap_or("Default");
        self.fonts.client(name, self.height, 12.0)
    }

    fn text_color(&self, c: &UiControl, enabled: bool) -> Color {
        const ORANGE: [u8; 4] = [255, 176, 0, 255];
        if let Some([r, g, b, a]) = c.fg {
            return Color::srgba_u8(r, g, b, a);
        }
        if !enabled {
            return self.color("Label.DisabledFgColor2", [188, 112, 0, 128]);
        }
        match c.kind {
            UiKind::Button => self.color("Button.TextColor", ORANGE),
            UiKind::RichText => self.color("RichText.TextColor", ORANGE),
            _ if c.dull => self.color("Label.TextDullColor", ORANGE),
            _ if c.bright => self.color("Label.TextBrightColor", ORANGE),
            _ => self.color("Label.TextColor", ORANGE),
        }
    }

    /// Button colours: background, then armed (hovered) background.
    pub fn button_colors(&self) -> (Color, Color) {
        (
            self.color("Button.BgColor", [0, 0, 0, 64]),
            self.color("Button.ArmedBgColor", [192, 28, 0, 140]),
        )
    }

    /// Spawn `layout`'s controls under `parent` (a node `size` pixels big).
    /// Returns the drawn controls by lower-case name, with their sizes, so
    /// a menu can fill a panel (`ItemInfo`).
    pub fn spawn(
        &self,
        commands: &mut Commands,
        parent: Entity,
        size: Vec2,
        layout: &UiLayout,
        menu: VguiMenu,
        shown: &mut dyn FnMut(&UiControl) -> Shown,
    ) -> HashMap<String, (Entity, Vec2)> {
        let mut out = HashMap::new();
        let bounds = Rect::from_corners(Vec2::ZERO, size);
        for c in &layout.controls {
            let s = shown(c);
            if !s.visible || matches!(c.kind, UiKind::Frame | UiKind::Other(_)) {
                continue;
            }
            let r = c.rect(bounds, self.scale);
            let node = Node {
                position_type: PositionType::Absolute,
                left: px(r.min.x),
                top: px(r.min.y),
                width: px(r.width()),
                height: px(r.height()),
                ..default()
            };
            let text = s.text.clone().unwrap_or_else(|| c.text.clone());
            let id = match &c.kind {
                UiKind::Label | UiKind::RichText => {
                    let rich = c.kind == UiKind::RichText;
                    let mut n = aligned(node, if rich { UiAlign::NorthWest } else { c.align });
                    if rich {
                        n.overflow = Overflow::clip();
                    }
                    let e = commands
                        .spawn((
                            n,
                            ZIndex(c.z),
                            BackgroundColor(c.bg.map_or(Color::NONE, rgba)),
                            ChildOf(parent),
                        ))
                        .id();
                    self.text(commands, e, text, c, s.enabled, rich || c.wrap, r.width());
                    e
                }
                UiKind::Button => {
                    let (bg, _) = self.button_colors();
                    let bright = self.color("Border.Bright", [188, 112, 0, 128]);
                    let dark = self.color("Border.Dark", [188, 112, 0, 128]);
                    let e = commands
                        .spawn((
                            Node {
                                border: UiRect::all(px(1.0)),
                                padding: UiRect::horizontal(px(4.0 * self.scale)),
                                ..aligned(node, c.align)
                            },
                            BorderColor {
                                top: bright,
                                left: bright,
                                right: dark,
                                bottom: dark,
                            },
                            BackgroundColor(c.bg.map_or(bg, rgba)),
                            ZIndex(c.z),
                            Button,
                            Interaction::default(),
                            VguiButton {
                                menu,
                                name: c.name.clone(),
                                command: c.command.clone(),
                                enabled: s.enabled,
                            },
                            ChildOf(parent),
                        ))
                        .id();
                    self.text(commands, e, text, c, s.enabled, false, r.width());
                    e
                }
                UiKind::Image => {
                    let mut e = commands.spawn((
                        node,
                        ZIndex(c.z),
                        BackgroundColor(c.fill.or(c.bg).map_or(Color::NONE, rgba)),
                        ChildOf(parent),
                    ));
                    if let Some(sprite) = c.image.as_ref().and_then(|i| self.hud.sprites.get(i))
                        && let Some(handle) = self.images.get(&sprite.texture)
                    {
                        let [x, y, w, h] = sprite.rect;
                        e.insert(ImageNode {
                            image: handle.clone(),
                            rect: Some(Rect::new(x, y, x + w, y + h)),
                            ..default()
                        });
                    }
                    e.id()
                }
                UiKind::Panel => commands
                    .spawn((
                        node,
                        ZIndex(c.z),
                        BackgroundColor(c.bg.map_or(Color::NONE, rgba)),
                        ChildOf(parent),
                    ))
                    .id(),
                UiKind::Divider => commands
                    .spawn((
                        Node {
                            border: UiRect::all(px(1.0)),
                            ..node
                        },
                        BorderColor::all(self.color("Border.Dark", [188, 112, 0, 128])),
                        ZIndex(c.z),
                        ChildOf(parent),
                    ))
                    .id(),
                UiKind::Frame | UiKind::Other(_) => continue,
            };
            out.insert(c.name.to_lowercase(), (id, r.size()));
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        commands: &mut Commands,
        parent: Entity,
        text: String,
        c: &UiControl,
        enabled: bool,
        wrap: bool,
        width: f32,
    ) {
        let justify = match c.align {
            UiAlign::Center | UiAlign::North | UiAlign::South => Justify::Center,
            UiAlign::East | UiAlign::NorthEast | UiAlign::SouthEast => Justify::Right,
            _ => Justify::Left,
        };
        let mut e = commands.spawn((
            Text::new(text),
            self.font(c.font.as_deref()),
            TextColor(self.text_color(c, enabled)),
            TextLayout::new(
                justify,
                if wrap {
                    LineBreak::WordBoundary
                } else {
                    LineBreak::NoWrap
                },
            ),
            ChildOf(parent),
        ));
        if wrap {
            e.insert(Node {
                max_width: px(width),
                ..default()
            });
        }
    }
}

fn rgba([r, g, b, a]: [u8; 4]) -> Color {
    Color::srgba_u8(r, g, b, a)
}

/// A node laying its text out as VGUI's `textAlignment` places it.
fn aligned(node: Node, align: UiAlign) -> Node {
    use UiAlign::*;
    let horizontal = match align {
        West | NorthWest | SouthWest => JustifyContent::FlexStart,
        Center | North | South => JustifyContent::Center,
        East | NorthEast | SouthEast => JustifyContent::FlexEnd,
    };
    let vertical = match align {
        NorthWest | North | NorthEast => AlignItems::FlexStart,
        West | Center | East => AlignItems::Center,
        SouthWest | South | SouthEast => AlignItems::FlexEnd,
    };
    Node {
        display: Display::Flex,
        justify_content: horizontal,
        align_items: vertical,
        ..node
    }
}

/// Hovered buttons light up in the armed colour, as VGUI's do.
fn style_buttons(
    hud: Option<Res<ActiveHud>>,
    mut buttons: Query<(&Interaction, &VguiButton, &mut BackgroundColor), Changed<Interaction>>,
) {
    let Some(hud) = hud else { return };
    let get = |name: &str, fallback: [u8; 4]| rgba(hud.0.colors.get(name).copied().unwrap_or(fallback));
    let (normal, armed) = (
        get("Button.BgColor", [0, 0, 0, 64]),
        get("Button.ArmedBgColor", [192, 28, 0, 140]),
    );
    for (interaction, button, mut bg) in &mut buttons {
        let lit = button.enabled && *interaction != Interaction::None;
        bg.0 = if lit { armed } else { normal };
    }
}

/// The mouse is free while a game-look menu is open; when it closes, it is
/// grabbed again once the button is up (so the click that closed it isn't
/// a shot), unless the console or the game menu took over.
fn cursor(
    open: Res<VguiOpen>,
    mouse: Res<ButtonInput<MouseButton>>,
    ui: Option<Res<super::console::ConsoleUi>>,
    game_menu: Option<Res<super::game_menu::GameMenu>>,
    cursor: Option<Single<&mut CursorOptions>>,
    mut freed: Local<bool>,
) {
    let Some(mut cursor) = cursor else { return };
    if open.any() {
        if super::input::cursor_grabbed(&cursor) && !ui.as_ref().is_some_and(|u| u.open) {
            super::input::release_cursor(&mut cursor);
        }
        *freed = true;
    } else if *freed && !mouse.pressed(MouseButton::Left) {
        *freed = false;
        let elsewhere = ui.is_some_and(|u| u.open) || game_menu.is_some_and(|m| m.open);
        if !elsewhere {
            super::input::capture_cursor(&mut cursor);
        }
    }
}

/// Whether the menus may read the keyboard: playing (mouse grabbed) or a
/// game-look menu open, and nothing else typing.
pub(super) fn keys_live(
    cursor: &CursorOptions,
    open: &VguiOpen,
    console: Option<&super::console::ConsoleUi>,
    chat: Option<&super::chat::ChatInput>,
    game_menu: Option<&super::game_menu::GameMenu>,
) -> bool {
    let busy =
        console.is_some_and(|c| c.open) || chat.is_some_and(|c| c.open.is_some()) || game_menu.is_some_and(|m| m.open);
    !busy && (super::input::cursor_grabbed(cursor) || open.any())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_virtual_screen_is_centred_and_scaled_by_height() {
        assert_eq!(area(1280.0, 720.0), Rect::new(160.0, 0.0, 1120.0, 720.0));
        assert_eq!(area(1920.0, 1080.0), Rect::new(240.0, 0.0, 1680.0, 1080.0));
        // Narrower than 4:3: from the left edge.
        assert_eq!(area(600.0, 480.0).min.x, 0.0);
    }

    #[test]
    fn hotkeys_find_visible_working_buttons() {
        let mut l = UiLayout::default();
        for (name, key, enabled) in [("a", '1', true), ("b", '2', false), ("c", '1', true)] {
            let mut c = UiControl::new(name, UiKind::Button, 0.0, 0.0, 10.0, 10.0);
            c.hotkey = Some(key);
            c.enabled = enabled;
            l.controls.push(c);
        }
        let mut all = |c: &UiControl| Shown::of(c);
        assert_eq!(hotkey_button(&l, '1', &mut all).map(|c| c.name.as_str()), Some("a"));
        assert!(hotkey_button(&l, '2', &mut all).is_none(), "disabled");
        let mut hide_a = |c: &UiControl| Shown {
            visible: c.name != "a",
            ..Shown::of(c)
        };
        assert_eq!(hotkey_button(&l, '1', &mut hide_a).map(|c| c.name.as_str()), Some("c"));
    }
}
