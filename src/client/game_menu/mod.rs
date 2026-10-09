//! The main menu and the in-game menu (Esc, or `menu [page]`), one menu
//! drawn as CS:S's GameUI draws it. Out of a game (startup without a map,
//! after `disconnect`) it is the main menu: always open, over the game's
//! background picture, the game's title over its entries, in-game entries
//! hidden. In a game: the entries at the left over the darkened game. Each
//! dialog is a VGUI frame (`widgets`: moved by its title bar, brought to
//! the front by a click, closed from its X) laid out from the install's
//! layouts: the options (`OptionsSub*.res` per tab, OK / Cancel / Apply,
//! the keyboard, video and multiplayer tabs' Advanced dialogs, the last
//! listing `cfg/user.scr`), Create Server (its
//! Server, Game and Bot pages: `CreateMultiplayerGame*Page.res`, the Game
//! page's options from `cfg/settings.scr`); controls the game has and
//! mashup lacks are drawn greyed. Ours (settings CS:S's options don't
//! have) are in our own dialog, Lucker Party Options, from our entries.
//! In the install's GameUI scheme (`map::hud::GameUi`); without the
//! install, in built-in colours and words, controls in a column. The game
//! keeps running while it is open; the mouse is free, so the local
//! player's input is ignored (`input::write_local_intent`). Every choice
//! is a console line (`map`, `bot_add`, `bind`, cvars ...), so the menu
//! holds no game logic: the model (`GameMenu::handle`) turns keys and
//! clicks into those lines, and is unit-tested without a window.

mod create_server;
mod draw;
mod extras;
mod loading;
mod main_menu;
mod model;
mod options_pages;
mod systems;
#[cfg(test)]
mod tests;

pub use self::{create_server::*, loading::*, main_menu::*, model::*};
pub(in crate::client) use self::systems::*;
use self::{draw::*, options_pages::*};

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::AccumulatedMouseScroll,
    },
    prelude::*,
    text::LineBreak,
    ui::RelativeCursorPosition,
    window::CursorOptions,
};

pub(in crate::client) use crate::client::widgets::{Look, bevel, label, place};
use crate::client::{
    binds,
    fonts::UiFonts,
    options::{Place, SETTINGS, SettingKind, TABS, Tab, VALUE_ENTRIES},
    widgets::{
        self, ButtonState as Btn, Caret, ComboEvent, ComboKey, ComboList, Edit, FrameSpec, SliderDrag, SliderOwner,
        SliderTrack, WindowId, Windows,
    },
};
use crate::{
    console::{Console, ConsoleAppExt},
    core::{Intent, Team},
    map::hud::{GameUi, KeyAction, ServerSettingKind, UiControl, UiKind, UiLayout},
};

pub struct GameMenuPlugin;

impl Plugin for GameMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameMenu>()
            .init_resource::<LoadingDetails>()
            .init_resource::<LoadTimeline>()
            .init_resource::<AfterLoad>()
            .init_resource::<RegrabCursor>()
            .init_resource::<MenuUi>()
            .add_systems(Startup, (start_loading_ui, start_session))
            .add_systems(
                Update,
                (
                    ui_loaded,
                    keys.before(super::console::toggle),
                    text_keys,
                    pointer,
                    sync,
                    cursor,
                    after_load,
                    menu_sounds,
                    draw,
                    loading_progress,
                )
                    .chain()
                    .in_set(MenuSystems),
            );
        crate::console::resource_cvar::<LoadingDetails, u8>(
            app,
            "mashup_loading_details",
            "1: the loading dialog adds a detailed view (each stage's time, bytes and percentage; the server's name, \
             map, players and ping).",
            |d| &mut d.0,
        );
        app.console_command(
            "menu",
            "menu [main|newgame|game|bot|bots|team|options|keyboard|mouse|audio|video|voice|multiplayer|advanced|\
             videoadvanced|mpadvanced|extras]: open the game menu (Esc) on a page (newgame: Create Server, game and \
             bot its other pages; options: on a tab; advanced: the keyboard tab's Advanced dialog, videoadvanced the \
             video tab's, mpadvanced the multiplayer tab's; extras: Lucker Party Options).",
            |w, a| {
                let arg = a.first().map(|s| s.to_lowercase());
                let tab = TABS.iter().find(|(t, ..)| Some(t.page()) == arg.as_deref()).map(|(t, ..)| *t);
                let page = match arg.as_deref() {
                    _ if tab.is_some() => Page::Settings,
                    None | Some("main") => Page::Main,
                    Some("newgame" | "createserver" | "game" | "bot") => Page::NewGame,
                    Some("bots") => Page::Bots,
                    Some("team") => Page::Team,
                    Some("options" | "settings" | "advanced" | "videoadvanced" | "mpadvanced") => Page::Settings,
                    Some("extras") => Page::Extras,
                    Some(p) => return Err(format!("no menu page \"{p}\"")),
                };
                open_menu(w, page);
                let mut menu = w.resource_mut::<GameMenu>();
                if let Some(tab) = tab {
                    menu.set_tab(tab);
                }
                match arg.as_deref() {
                    Some("game") => menu.set_create_tab(1),
                    Some("bot") => menu.set_create_tab(2),
                    Some("advanced") => {
                        menu.set_tab(Tab::Keyboard);
                        menu.press_action(&Action::Advanced);
                    }
                    Some("videoadvanced") => {
                        menu.set_tab(Tab::Video);
                        menu.press_action(&Action::VideoAdvanced);
                    }
                    Some("mpadvanced") => {
                        menu.set_tab(Tab::Multiplayer);
                        menu.press_action(&Action::MultiplayerAdvanced);
                    }
                    _ => {}
                }
                Ok(None)
            },
        )
        .console_command(
            "menuinput",
            "menuinput <input>: drive the open game menu as keys and clicks would (screenshots, tests): \
             focus <cvar|map|bots|botcount|botteam|difficulty|aspect> | open (the focused combo box's list) | \
             down | up | enter | space | escape | tab | backtab | type <text> | pick <n> | sheet <n>; prints \
             the focused control.",
            |w, a| {
                let input = menu_input(&w.resource::<GameMenu>(), a)?;
                let mut menu = w.resource_mut::<GameMenu>();
                let mut next = menu.clone();
                let out = next.handle(input);
                *menu = next;
                let state = menu.describe();
                let mut console = w.resource_mut::<Console>();
                for line in out.lines {
                    console.submit(line);
                }
                Ok(Some(state))
            },
        )
        .console_command("gameui_activate", "Open the game menu (Esc).", |w, _| {
            open_menu(w, Page::Main);
            Ok(None)
        })
        .console_command("gameui_hide", "Close the game menu (in a game).", |w, _| {
            let mut menu = w.resource_mut::<GameMenu>();
            if !menu.in_game {
                return Err("the main menu stays open out of a game".into());
            }
            menu.open = false;
            w.resource_mut::<RegrabCursor>().0 = true;
            Ok(None)
        });
    }
}

/// The menu's systems (the server browser runs after them).
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MenuSystems;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Main,
    /// Create Server (its page: `GameMenu::create_tab`).
    NewGame,
    Bots,
    Team,
    /// The options dialog (its tab: `GameMenu::tab`).
    Settings,
    /// The keyboard tab's Advanced dialog (over the options).
    KeyboardAdvanced,
    /// The video tab's Advanced dialog (over the options).
    VideoAdvanced,
    /// The multiplayer tab's Advanced dialog (over the options; its list
    /// from the install's `cfg/user.scr`).
    MultiplayerAdvanced,
    /// Ours: Lucker Party Options (what CS:S's options don't have).
    Extras,
}

impl Page {
    /// Its frame (`widgets::Windows` keeps where it was dragged).
    pub fn window(self) -> WindowId {
        match self {
            Page::Main => "main",
            Page::NewGame => "createserver",
            Page::Bots => "bots",
            Page::Team => "team",
            Page::Settings => "options",
            Page::KeyboardAdvanced => "keyboardadvanced",
            Page::VideoAdvanced => "videoadvanced",
            Page::MultiplayerAdvanced => "multiplayeradvanced",
            Page::Extras => "extras",
        }
    }

    /// A dialog over the options (modal to them).
    fn over_options(self) -> bool {
        matches!(self, Page::KeyboardAdvanced | Page::VideoAdvanced | Page::MultiplayerAdvanced)
    }
}
