//! Finding the CS:S install on first run. At startup the log says which
//! install is used and how it was found (`mount::install`: the config,
//! the user's saved choice, a Steam library) or that none was. With none,
//! at the main menu, a dialog drawn as the game menu's GameUI dialogs
//! says so: a box to paste the folder into, why a folder doesn't do,
//! Retry (that folder, or search Steam again when the box is empty) and
//! Continue Without (built-in looks). A folder that checks out is saved
//! in the user's settings file and the menu's look is read from it.
//!
//! `mashup_install [folder|auto]` shows or sets the same from the
//! console; `mashup_firstrun` drives the dialog (tests, screenshots).
//! As the other menus, the model (`FirstRun::handle`) is unit-tested
//! without a window.

use std::path::{Path, PathBuf};

use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    prelude::*,
};

use super::{
    fonts::UiFonts,
    game_menu::{GameMenu, Look, UiText, bevel, frame, label, place},
};
use crate::{
    console::ConsoleAppExt,
    games::cs_source::GAME,
    mount::{
        config::LocalConfig,
        install::{self, Resolved},
    },
};

pub struct FirstRunPlugin;

impl Plugin for FirstRunPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FirstRun>()
            .add_systems(Startup, startup)
            .add_systems(Update, (keys, pointer, draw).chain().after(super::game_menu::MenuSystems));
        app.console_command(
            "mashup_install",
            "mashup_install [folder|auto]: show which CS:S install is used and how it was found; with a folder \
             (the one holding cstrike/ and hl2/), use and remember it; auto: forget it and search Steam again.",
            |w, a| {
                let arg = a.join(" ");
                match arg.trim() {
                    "" => Ok(Some(resolve_now().describe(NAME))),
                    "auto" => {
                        if let Some(settings) = install::settings_path() {
                            install::save_install(&settings, GAME, None)?;
                        }
                        reload(w);
                        Ok(Some(resolve_now().describe(NAME)))
                    }
                    folder => use_folder(w, folder).map(Some),
                }
            },
        )
        .console_command(
            "mashup_firstrun",
            "mashup_firstrun [open | type <text> | clear | retry | continue]: drive the first-run dialog \
             (CS:S not found) as typing and clicks would; prints its state.",
            |w, a| {
                let input = match a.first().map(String::as_str) {
                    None => None,
                    Some("open") => {
                        w.resource_mut::<FirstRun>().open = true;
                        None
                    }
                    Some("type") => Some(Input::Type(a[1..].join(" "))),
                    Some("clear") => Some(Input::Clear),
                    Some("retry") => Some(Input::Click(Target::Retry)),
                    Some("continue") => Some(Input::Click(Target::Continue)),
                    Some(other) => return Err(format!("no input \"{other}\"")),
                };
                if let Some(input) = input {
                    apply(w, input);
                }
                let f = w.resource::<FirstRun>();
                Ok(Some(format!(
                    "open {}, folder \"{}\"{}",
                    f.open,
                    f.text,
                    f.message.as_deref().map(|m| format!(", message: {m}")).unwrap_or_default()
                )))
            },
        );
    }
}

const NAME: &str = "Counter-Strike: Source";

// ---------------------------------------------------------------------------
// The model.

/// The dialog's state.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct FirstRun {
    pub open: bool,
    /// The folder typed or pasted.
    pub text: String,
    /// Why the last try didn't find the game.
    pub message: Option<String>,
    pub focus: Target,
    /// The game's look, when it comes (after a folder checks out).
    pub ui: UiText,
}

/// What a click or key is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    #[default]
    Field,
    Retry,
    Continue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Type(String),
    Backspace,
    Clear,
    /// Enter: presses the focused button (Retry from the box).
    Enter,
    /// Esc: Continue Without.
    Escape,
    /// Tab: the next control.
    Tab,
    Click(Target),
}

/// What the dialog asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Use this folder.
    Use(String),
    /// Search Steam again.
    Search,
}

impl FirstRun {
    pub fn handle(&mut self, input: Input) -> Option<Request> {
        if !self.open {
            return None;
        }
        match input {
            Input::Type(s) => {
                self.focus = Target::Field;
                for c in s.chars().filter(|c| !c.is_control()) {
                    if self.text.len() + c.len_utf8() > 1024 {
                        break;
                    }
                    self.text.push(c);
                }
            }
            Input::Backspace => {
                self.text.pop();
            }
            Input::Clear => self.text.clear(),
            Input::Tab => {
                self.focus = match self.focus {
                    Target::Field => Target::Retry,
                    Target::Retry => Target::Continue,
                    Target::Continue => Target::Field,
                }
            }
            Input::Click(Target::Field) => self.focus = Target::Field,
            Input::Escape | Input::Click(Target::Continue) => self.close(),
            Input::Enter if self.focus == Target::Continue => self.close(),
            Input::Enter | Input::Click(Target::Retry) => {
                let folder = self.text.trim().trim_matches('"').trim();
                return Some(if folder.is_empty() { Request::Search } else { Request::Use(folder.to_string()) });
            }
        }
        None
    }

    /// A request's outcome: found (the dialog closes) or why not.
    pub fn answer(&mut self, outcome: Result<String, String>) {
        match outcome {
            Ok(_) => self.close(),
            Err(why) => self.message = Some(why),
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.message = None;
    }
}

/// The folder the user meant: `path` itself, or the install around or
/// under it (its `cstrike/` or `hl2/` folder pasted, or the Steam
/// library's `common/` folder).
pub fn install_folder(path: &Path) -> Result<PathBuf, String> {
    let game = install::steam_game(GAME).ok_or("no Steam game entry")?;
    let first = install::check(path, game);
    if first.is_ok() {
        return Ok(path.to_path_buf());
    }
    let nearby = [
        path.parent().map(Path::to_path_buf),
        Some(path.join("Counter-Strike Source")),
        Some(path.join("steamapps/common/Counter-Strike Source")),
    ];
    nearby.into_iter().flatten().find(|p| install::check(p, game).is_ok()).ok_or_else(|| first.unwrap_err())
}

// ---------------------------------------------------------------------------
// Running it.

fn resolve_now() -> Resolved {
    let config = LocalConfig::load().unwrap_or_default();
    install::resolve(&config, GAME)
}

/// Check, remember and use `folder`; what is used now.
fn use_folder(w: &mut World, folder: &str) -> Result<String, String> {
    let path = install::expand_home(Path::new(folder));
    let path = std::path::absolute(&path).unwrap_or(path);
    let path = install_folder(&path)?;
    let settings = install::settings_path().ok_or("no folder for the user's settings")?;
    install::save_install(&settings, GAME, Some(&path))?;
    reload(w);
    let now = resolve_now();
    let mut text = now.describe(NAME);
    if now.install.as_ref().is_some_and(|i| i.path != path) {
        text.push_str(&format!(" (saved {}, but the config's path comes first)", path.display()));
    }
    info!("{text}");
    Ok(text)
}

/// Search again (Steam, the saved folder); found or why not.
fn search(w: &mut World) -> Result<String, String> {
    install::forget_steam();
    let now = resolve_now();
    if now.install.is_none() {
        return Err(format!("{NAME} still wasn't found: {}.", now.notes.join("; ")));
    }
    reload(w);
    let text = now.describe(NAME);
    info!("{text}");
    Ok(text)
}

/// The install changed: everything read from it at startup again.
fn reload(w: &mut World) {
    install::forget_steam();
    super::console::forget_map_names();
    super::game_menu::reload_install(w);
}

fn apply(w: &mut World, input: Input) {
    let Some(request) = w.resource_mut::<FirstRun>().handle(input) else {
        return;
    };
    let outcome = match request {
        Request::Use(folder) => use_folder(w, &folder),
        Request::Search => search(w),
    };
    w.resource_mut::<FirstRun>().answer(outcome);
}

/// Say which install is used; with none at the main menu, open the
/// dialog.
fn startup(args: Option<Res<super::ClientArgs>>, mut first_run: ResMut<FirstRun>) {
    let found = resolve_now();
    let line = found.describe(NAME);
    if found.install.is_some() {
        info!("{line}");
        for n in &found.notes {
            warn!("{NAME} install: {n}");
        }
    } else {
        warn!("{line}; built-in looks (set one with mashup_install <folder>)");
        let in_game = args.is_some_and(|a| a.0.starts_in_game());
        first_run.open = !in_game;
    }
}

/// Keys: typing and pasting into the box, Enter, Esc, Tab.
fn keys(
    mut events: MessageReader<KeyboardInput>,
    held: Res<ButtonInput<KeyCode>>,
    console_ui: Res<super::console::ConsoleUi>,
    first_run: Res<FirstRun>,
    mut commands: Commands,
) {
    if !first_run.open || console_ui.open {
        events.clear();
        return;
    }
    let ctrl = held.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight]);
    for e in events.read() {
        if e.state != ButtonState::Pressed {
            continue;
        }
        let input = match (&e.logical_key, e.key_code) {
            (_, KeyCode::KeyV) if ctrl => {
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
                Input::Type(text.lines().next().unwrap_or("").to_string())
            }
            (_, KeyCode::KeyU) if ctrl => Input::Clear,
            (_, KeyCode::Enter | KeyCode::NumpadEnter) => Input::Enter,
            (_, KeyCode::Escape) => Input::Escape,
            (_, KeyCode::Tab) => Input::Tab,
            (Key::Backspace, _) => Input::Backspace,
            (Key::Space, _) => Input::Type(" ".into()),
            (Key::Character(s), _) if !ctrl => Input::Type(s.to_string()),
            _ => continue,
        };
        commands.queue(move |w: &mut World| apply(w, input));
    }
}

#[derive(Component, Clone, Copy)]
struct Hit(Target);

fn pointer(hits: Query<(&Interaction, &Hit), Changed<Interaction>>, first_run: Res<FirstRun>, mut commands: Commands) {
    if !first_run.open {
        return;
    }
    for (i, hit) in &hits {
        if *i == Interaction::Pressed {
            let input = Input::Click(hit.0);
            commands.queue(move |w: &mut World| apply(w, input));
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing.

#[derive(Component)]
struct FirstRunRoot;

/// Dialog size in scheme pixels.
const W: f32 = 560.0;
const H: f32 = 250.0;

#[allow(clippy::too_many_arguments)]
fn draw(
    mut first_run: ResMut<FirstRun>,
    menu: Res<GameMenu>,
    fonts: Res<UiFonts>,
    shown: Query<Entity, With<FirstRunRoot>>,
    windows: Query<&Window>,
    mut last_size: Local<Vec2>,
    mut commands: Commands,
) {
    if first_run.ui != menu.ui {
        first_run.ui = menu.ui.clone();
    }
    let size = windows.iter().next().map_or(Vec2::new(640.0, 480.0), |w| Vec2::new(w.width(), w.height()));
    let resized = (size - *last_size).abs().max_element() > 0.5;
    if !first_run.is_changed() && !resized {
        return;
    }
    *last_size = size;
    for e in &shown {
        commands.entity(e).despawn();
    }
    if !first_run.open {
        return;
    }
    let f = &*first_run;
    let look = Look {
        ui: f.ui.0.as_deref(),
        fonts: &fonts,
        s: (size.y / 720.0).clamp(0.6, 3.0),
        height: size.y,
        accent: Color::srgb_u8(255, 176, 0),
    };
    let root = commands
        .spawn((
            FirstRunRoot,
            Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), ..default() },
            // Modal over the menu (46) and the server browser (47), under
            // the console.
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
            GlobalZIndex(48),
        ))
        .id();
    let dialog = frame(&mut commands, root, &look, size, (W, H), &format!("{NAME} Not Found"));
    // Opaque, as the browser's modal dialogs.
    let bg = look.frame_bg().to_srgba();
    commands.entity(dialog).insert(BackgroundColor(Color::srgb(bg.red * 0.7, bg.green * 0.7, bg.blue * 0.7)));
    let text_font = look.font("Default", (16.0, false));
    let lines = [
        format!("{NAME} wasn't found on this computer. Lucker Party plays with its"),
        "maps, models and sounds, read from your own install.".to_string(),
        "Paste the folder that holds its cstrike and hl2 folders, for example".to_string(),
        if cfg!(windows) {
            "...\\steamapps\\common\\Counter-Strike Source".to_string()
        } else {
            ".../steamapps/common/Counter-Strike Source".to_string()
        },
    ];
    for (i, line) in lines.iter().enumerate() {
        let color = if i == 3 { look.dull() } else { look.text() };
        label(
            &mut commands,
            dialog,
            &look,
            (20.0, 34.0 + 20.0 * i as f32, W - 40.0, 20.0),
            line,
            text_font.clone(),
            color,
            -1,
        );
    }
    // The folder box.
    let (bx, by, bw, bh) = (20.0, 122.0, W - 40.0, 24.0);
    let field = commands
        .spawn((
            Node { border: UiRect::all(px(1.0)), ..place(&look, bx, by, bw, bh) },
            bevel(&look, false),
            BackgroundColor(look.sunken_bg()),
            Hit(Target::Field),
            Button,
            Interaction::default(),
            ChildOf(dialog),
        ))
        .id();
    let shown_text = if f.focus == Target::Field { format!("{}_", f.text) } else { f.text.clone() };
    // The end of a long path shows, as a text entry scrolled to its caret.
    let max_chars = ((bw - 8.0) / 7.5) as usize;
    let count = shown_text.chars().count();
    let shown_text: String =
        if count > max_chars { shown_text.chars().skip(count - max_chars).collect() } else { shown_text };
    label(
        &mut commands,
        field,
        &look,
        (4.0, 0.0, bw - 8.0, bh - 2.0),
        &shown_text,
        text_font.clone(),
        look.color("TextEntry.TextColor", [221, 221, 221, 255]),
        -1,
    );
    if let Some(message) = &f.message {
        // Two lines at most, broken at a space.
        let mut lines = vec![String::new()];
        for word in message.split(' ') {
            let line = lines.last_mut().unwrap();
            if !line.is_empty() && line.chars().count() + word.chars().count() > 72 {
                lines.push(String::new());
            }
            let line = lines.last_mut().unwrap();
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        for (i, line) in lines.iter().take(2).enumerate() {
            label(
                &mut commands,
                dialog,
                &look,
                (20.0, 150.0 + 18.0 * i as f32, W - 40.0, 18.0),
                line,
                text_font.clone(),
                Color::srgb_u8(255, 110, 90),
                -1,
            );
        }
    }
    let buttons = [(Target::Retry, "Retry", 100.0), (Target::Continue, "Continue Without", 150.0)];
    let mut x = W - 20.0;
    for (target, text, w) in buttons.iter().rev() {
        x -= w;
        button(&mut commands, dialog, &look, (x, H - 46.0, *w, 26.0), text, *target, f.focus == *target);
        x -= 10.0;
    }
}

/// A VGUI button: raised, lit when focused.
fn button(
    commands: &mut Commands,
    parent: Entity,
    look: &Look,
    rect: (f32, f32, f32, f32),
    text: &str,
    target: Target,
    focused: bool,
) {
    let (x, y, w, h) = rect;
    let bg = if focused { Color::srgba(1.0, 1.0, 1.0, 0.12) } else { look.color("Button.BgColor", [0, 0, 0, 0]) };
    let e = commands
        .spawn((
            Node { border: UiRect::all(px(1.0)), ..place(look, x, y, w, h) },
            bevel(look, true),
            BackgroundColor(bg),
            Hit(target),
            Button,
            Interaction::default(),
            ChildOf(parent),
        ))
        .id();
    let color = if focused { look.white() } else { look.color("Button.TextColor", [255, 255, 255, 255]) };
    label(commands, e, look, (6.0, 0.0, w - 12.0, h - 2.0), text, look.font("Default", (16.0, false)), color, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> FirstRun {
        FirstRun { open: true, ..default() }
    }

    #[test]
    fn retry_uses_the_typed_folder_or_searches_again() {
        let mut f = open();
        assert_eq!(f.handle(Input::Click(Target::Retry)), Some(Request::Search));
        f.handle(Input::Type("\"C:\\Games\\Counter-Strike Source\"  ".into()));
        assert_eq!(
            f.handle(Input::Enter),
            Some(Request::Use("C:\\Games\\Counter-Strike Source".into())),
            "pasted quotes and spaces go"
        );
        f.answer(Err("no cstrike".into()));
        assert!(f.open);
        assert_eq!(f.message.as_deref(), Some("no cstrike"));
        f.answer(Ok("found".into()));
        assert!(!f.open && f.message.is_none());
        assert_eq!(f.handle(Input::Enter), None, "closed: nothing");
    }

    #[test]
    fn continue_without_and_keys() {
        let mut f = open();
        f.handle(Input::Type("ab".into()));
        f.handle(Input::Backspace);
        assert_eq!(f.text, "a");
        f.handle(Input::Tab);
        f.handle(Input::Tab);
        assert_eq!(f.focus, Target::Continue);
        assert_eq!(f.handle(Input::Enter), None);
        assert!(!f.open);
        let mut f = open();
        f.handle(Input::Escape);
        assert!(!f.open);
    }

    #[test]
    fn a_pasted_subfolder_finds_its_install() {
        let dir = std::env::temp_dir().join(format!("mashup-first-run-{}", std::process::id()));
        let css = dir.join("steamapps/common/Counter-Strike Source");
        for f in install::steam_game(GAME).unwrap().required {
            let p = css.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        assert_eq!(install_folder(&css).unwrap(), css);
        assert_eq!(install_folder(&css.join("cstrike")).unwrap(), css);
        assert_eq!(install_folder(&dir.join("steamapps/common")).unwrap(), css);
        assert_eq!(install_folder(&dir).unwrap(), css, "the library");
        assert!(install_folder(&dir.join("nowhere")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
