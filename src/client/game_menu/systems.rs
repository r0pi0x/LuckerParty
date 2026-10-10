//! The menu's systems: reading the install's GameUI look, opening the
//! menu, keys, the pointer, the cursor, lines run after a map load, the
//! interface sounds.

use super::*;

/// Lines waiting for a map load the menu started (`map` loads in the
/// background, `console::finish_map_load` swaps it in).
#[derive(Resource, Default)]
pub(super) struct AfterLoad {
    pub(super) lines: Vec<String>,
    /// Frames waited for the load to start; it started once seen.
    pub(super) waited: u32,
    pub(super) started: bool,
}

/// The menu closed: grab the mouse again once the button that closed it is
/// up (else the click would fire the gun).
#[derive(Resource, Default)]
pub(super) struct RegrabCursor(pub(super) bool);

/// The install's GameUI look, read in the background at startup.
#[derive(Resource, Default)]
pub(super) struct MenuUi {
    pub(super) loading: Option<Arc<Mutex<Option<Option<GameUi>>>>>,
    pub(super) ui: Option<Arc<GameUi>>,
    /// The main menu's backgrounds: 4:3, widescreen.
    pub(super) backgrounds: [Option<Handle<Image>>; 2],
    /// The title's font, its height in scheme pixels and its line height
    /// in ems.
    pub(super) title_font: Option<(Handle<Font>, f32, f32)>,
    /// The options' pictures by name (`GameUi::option_images`).
    pub(super) pictures: HashMap<String, Handle<Image>>,
}

impl MenuUi {
    /// The main menu background for a screen of this size.
    pub(super) fn background(&self, size: Vec2) -> Option<Handle<Image>> {
        let ui = self.ui.as_ref()?;
        let pic = ui.background_for(size.x / size.y.max(1.0))?;
        let i = ui.background_wide.as_ref().is_some_and(|w| std::ptr::eq(w, pic)) as usize;
        self.backgrounds[i].clone()
    }

    /// As `background`, with the picture's size in pixels.
    pub(super) fn background_sized(&self, size: Vec2) -> Option<(Handle<Image>, Vec2)> {
        let pic = self.ui.as_ref()?.background_for(size.x / size.y.max(1.0))?;
        Some((self.background(size)?, Vec2::new(pic.width as f32, pic.height as f32)))
    }
}

/// A decoded picture as a UI image.
pub(in crate::client) fn ui_image(pic: &crate::map::hud::UiImage, images: &mut Assets<Image>) -> Handle<Image> {
    use bevy::{
        asset::RenderAssetUsages,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    };
    images.add(Image::new(
        Extent3d {
            width: pic.width,
            height: pic.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pic.rgba8.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

/// Read the GameUI files on a thread (the install's archives).
pub(super) fn start_loading_ui(mut ui: ResMut<MenuUi>) {
    let slot = Arc::new(Mutex::new(None));
    ui.loading = Some(slot.clone());
    std::thread::spawn(move || {
        let loaded = crate::mount::config::LocalConfig::load()
            .ok()
            .and_then(|c| c.game_path(crate::games::cs_source::GAME))
            .and_then(|p| crate::games::cs_source::mount::open(&p).ok())
            .and_then(|m| crate::games::cs_source::gameui::load(&m));
        if let Ok(mut s) = slot.lock() {
            *s = Some(loaded);
        }
    });
}

/// The install changed (the first-run dialog, `mashup_install`): read
/// its GameUI look again, and the open page's maps.
pub(in crate::client) fn reload_install(w: &mut World) {
    if let Err(e) = w.run_system_cached(start_loading_ui) {
        warn!("game menu: reading the GameUI look again: {e}");
    }
    let menu = w.resource::<GameMenu>();
    if menu.open && menu.loading.is_none() {
        let page = menu.page;
        open_menu(w, page);
    }
}

/// Take the GameUI look once read.
pub(super) fn ui_loaded(
    mut ui: ResMut<MenuUi>,
    mut menu: ResMut<GameMenu>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
    mut ui_fonts: ResMut<UiFonts>,
    mut commands: Commands,
) {
    let Some(slot) = ui.loading.clone() else { return };
    let Some(loaded) = slot.lock().ok().and_then(|mut s| s.take()) else {
        return;
    };
    ui.loading = None;
    let Some(game_ui) = loaded else {
        info!("game menu: no GameUI files in the install, built-in look");
        return;
    };
    ui.backgrounds = [&game_ui.background, &game_ui.background_wide].map(|p| p.as_ref().map(|p| ui_image(p, &mut images)));
    ui.pictures = game_ui
        .option_images
        .iter()
        .map(|(name, p)| (name.clone(), ui_image(p, &mut images)))
        .collect();
    ui.title_font = game_ui
        .title_font
        .as_ref()
        .map(|(bytes, tall)| {
            let line_per_em = crate::client::fonts::line_per_em(bytes).unwrap_or(1.2);
            (fonts.add(Font::from_bytes(bytes.to_vec())), *tall, line_per_em)
        });
    ui_fonts.set_source(&game_ui);
    info!(
        "game menu: GameUI look ({} entries, {} keyboard actions, {} option and Create Server pages, {} server \
         options, {} main menu backgrounds, title {:?}{})",
        game_ui.menu.len(),
        game_ui.actions.len(),
        game_ui.options.len(),
        game_ui.server_settings.len(),
        ui.backgrounds.iter().flatten().count(),
        game_ui.title,
        if ui.title_font.is_some() { " in its font" } else { "" },
    );
    let game_ui = Arc::new(game_ui);
    ui.ui = Some(game_ui.clone());
    menu.set_ui(Some(game_ui));
    // A dialog already open reads its page again: Create Server's Game
    // page options come with the look.
    if menu.open && menu.loading.is_none() && menu.page != Page::Main {
        let (page, create_tab) = (menu.page, menu.create_tab);
        commands.queue(move |w: &mut World| {
            open_menu(w, page);
            w.resource_mut::<GameMenu>().set_create_tab(create_tab);
        });
    }
}

/// Open the menu on a page, reading the settings from the console.
pub(super) fn open_menu(w: &mut World, page: Page) {
    let (get, binds, known) = {
        let mut names: Vec<String> = SETTINGS
            .iter()
            .flat_map(|s| s.cvars().map(str::to_string))
            .chain([ROUNDS_CVAR.to_string(), "bot_reaction".into()])
            .chain(BOT_CVARS.iter().map(|(_, c, _)| c.to_string()))
            .chain(USER_FALLBACK.iter().map(|(c, ..)| c.to_string()))
            .collect();
        if let Some(ui) = w.get_resource::<MenuUi>().and_then(|u| u.ui.clone()) {
            names.extend(ui.server_settings.iter().map(|s| s.cvar.clone()));
            names.extend(ui.user_settings.iter().map(|s| s.cvar.clone()));
        }
        let cvars: Vec<_> = {
            let console = w.resource::<Console>();
            names.iter().filter_map(|n| console.cvar(n).cloned()).collect()
        };
        let console = w.resource::<Console>();
        let binds = console.binds.clone();
        let known: BTreeSet<String> = console.names().into_iter().map(|n| n.to_lowercase()).collect();
        let values = cvars
            .into_iter()
            .filter_map(|c| (c.get)(w).map(|v| (c.name.clone(), v)))
            .collect::<HashMap<_, _>>();
        (values, binds, known)
    };
    let current = w
        .get_resource::<crate::map::LoadedMapName>()
        .map(|m| m.0.rsplit(':').next().unwrap_or(&m.0).to_string());
    let maps = crate::client::console::map_names();
    let (mut bots, mut players) = ([0; 2], [0; 2]);
    let mut q = w.query_filtered::<(&Team, Has<crate::bot::Bot>), With<Intent>>();
    for (team, bot) in q.iter(w) {
        if let Some(i) = [1, 2].iter().position(|t| *t == team.0) {
            players[i] += 1;
            bots[i] += bot as usize;
        }
    }
    let mut monitors = w.query_filtered::<&bevy::window::Monitor, With<bevy::window::PrimaryMonitor>>();
    let modes: Vec<(UVec2, Vec<UVec2>)> = monitors
        .iter(w)
        .map(|m| {
            (
                UVec2::new(m.physical_width, m.physical_height),
                m.video_modes.iter().map(|v| v.physical_size).collect(),
            )
        })
        .collect();
    let mut windows = w.query_filtered::<&Window, With<bevy::window::PrimaryWindow>>();
    let window = windows.iter(w).next().map(|w| w.resolution.physical_size());
    let no_spectators = !crate::client::team_menu::spectators_allowed(w.get_resource::<crate::rules::Deathmatch>());
    let mut menu = w.resource_mut::<GameMenu>();
    menu.set_counts(bots, players);
    menu.no_spectators = no_spectators;
    menu.set_binds(binds, known);
    menu.resolutions = crate::client::options::resolutions(&modes);
    menu.open(page, maps, current.as_deref(), |n| get.get(n).cloned());
    // The aspect ratio of the window as it is when the size is as started.
    if menu.aspect.is_none()
        && let Some(size) = window
    {
        menu.aspect = aspect_of(&format!("{}x{}", size.x, size.y));
    }
    menu.random_seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64 / 1000);
    // Menus close each other.
    if let Some(mut b) = w.get_resource_mut::<crate::client::buy_menu::BuyMenu>() {
        *b = default();
    }
    if let Some(mut t) = w.get_resource_mut::<crate::client::team_menu::TeamMenu>() {
        t.0 = false;
    }
    if let Some(mut r) = w.get_resource_mut::<crate::client::radio::RadioMenu>() {
        r.0 = None;
    }
    if page != Page::Main
        && let Some(mut windows) = w.get_resource_mut::<Windows>()
    {
        windows.raise(page.window());
    }
    w.resource_mut::<RegrabCursor>().0 = false;
}

/// The menu open (on its main page unless it is open already): for the
/// server browser, which shows over it.
pub(in crate::client) fn open_main(w: &mut World) {
    if !w.resource::<GameMenu>().open {
        open_menu(w, Page::Main);
    }
}

/// A map is in (`map`, `map greybox`): playing it, the menu closed.
pub(in crate::client) fn entered_game(w: &mut World) {
    let Some(mut menu) = w.get_resource_mut::<GameMenu>() else { return };
    if menu.enter_game()
        && let Some(mut regrab) = w.get_resource_mut::<RegrabCursor>()
    {
        regrab.0 = true;
    }
}

/// Out of the game (`disconnect`): the main menu.
pub(in crate::client) fn left_game(w: &mut World) {
    let Some(mut menu) = w.get_resource_mut::<GameMenu>() else { return };
    menu.in_game = false;
    menu.loading = None;
    menu.failure = None;
    open_menu(w, Page::Main);
}

/// At startup: playing when the command line gives a map or places the
/// player (`Args::starts_in_game`), else the main menu.
pub(super) fn start_session(args: Option<Res<crate::client::ClientArgs>>, mut menu: ResMut<GameMenu>, mut commands: Commands) {
    menu.in_game = args.is_some_and(|a| a.0.starts_in_game());
    if !menu.in_game {
        commands.queue(|w: &mut World| open_menu(w, Page::Main));
    }
}

/// Run an input through the model; apply what it asks for.
pub(super) fn apply(input: Input, menu: &mut ResMut<GameMenu>, console: &mut Console, after: &mut AfterLoad, regrab: &mut RegrabCursor) {
    let mut next = (**menu).clone();
    let out = next.handle(input);
    if next != **menu {
        **menu = next;
    }
    for line in out.lines {
        console.submit(line);
    }
    if !out.after_load.is_empty() {
        *after = AfterLoad {
            lines: out.after_load,
            ..default()
        };
    }
    if out.close {
        regrab.0 = true;
    }
    if let Some(text) = out.clipboard
        && let Err(e) = arboard::Clipboard::new().and_then(|mut c| c.set_text(text))
    {
        warn!("clipboard: {e}");
    }
}

/// Whether the menu's dialog takes the keys: no other window over it
/// (the server browser in front, the first-run dialog).
pub(super) fn menu_has_keys(
    menu: &GameMenu,
    browser: Option<&crate::client::server_browser::ServerBrowser>,
    first_run: Option<&crate::client::first_run::FirstRun>,
    windows: Option<&Windows>,
) -> bool {
    if first_run.is_some_and(|f| f.open) {
        return false;
    }
    if !browser.is_some_and(|b| b.open) {
        return true;
    }
    // The browser and a dialog of the menu both open: the front one.
    menu.page != Page::Main
        && windows.is_some_and(|w| w.front(&[crate::client::server_browser::WINDOW, menu.page.window()]) == Some(menu.page.window()))
}

/// The focused row is a text entry (typing goes to it: `text_keys`).
pub(super) fn typing(menu: &GameMenu) -> bool {
    menu.combo.is_none()
        && matches!(
            menu.rows().get(menu.focus),
            Some(Row::Control {
                control: Control::Text { .. },
                ..
            })
        )
}

pub(super) const LETTERS: [(KeyCode, char); 36] = [
    (KeyCode::KeyA, 'a'),
    (KeyCode::KeyB, 'b'),
    (KeyCode::KeyC, 'c'),
    (KeyCode::KeyD, 'd'),
    (KeyCode::KeyE, 'e'),
    (KeyCode::KeyF, 'f'),
    (KeyCode::KeyG, 'g'),
    (KeyCode::KeyH, 'h'),
    (KeyCode::KeyI, 'i'),
    (KeyCode::KeyJ, 'j'),
    (KeyCode::KeyK, 'k'),
    (KeyCode::KeyL, 'l'),
    (KeyCode::KeyM, 'm'),
    (KeyCode::KeyN, 'n'),
    (KeyCode::KeyO, 'o'),
    (KeyCode::KeyP, 'p'),
    (KeyCode::KeyQ, 'q'),
    (KeyCode::KeyR, 'r'),
    (KeyCode::KeyS, 's'),
    (KeyCode::KeyT, 't'),
    (KeyCode::KeyU, 'u'),
    (KeyCode::KeyV, 'v'),
    (KeyCode::KeyW, 'w'),
    (KeyCode::KeyX, 'x'),
    (KeyCode::KeyY, 'y'),
    (KeyCode::KeyZ, 'z'),
    (KeyCode::Digit0, '0'),
    (KeyCode::Digit1, '1'),
    (KeyCode::Digit2, '2'),
    (KeyCode::Digit3, '3'),
    (KeyCode::Digit4, '4'),
    (KeyCode::Digit5, '5'),
    (KeyCode::Digit6, '6'),
    (KeyCode::Digit7, '7'),
    (KeyCode::Digit8, '8'),
    (KeyCode::Digit9, '9'),
];

/// Esc opens and closes the menu; arrows, Enter, Space, Backspace, Tab
/// (Ctrl+Tab: the property sheet's tabs) and letters drive it; while a
/// keyboard action waits for a key, the next key, button or wheel notch
/// is its new key. Typing into a text entry is `text_keys`'. Runs before
/// the console's toggle, so the Esc that closes the console doesn't open
/// the menu.
#[allow(clippy::too_many_arguments)]
pub(super) fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    (mouse, scroll): (Option<Res<ButtonInput<MouseButton>>>, Option<Res<AccumulatedMouseScroll>>),
    ui: Res<crate::client::console::ConsoleUi>,
    mut menu: ResMut<GameMenu>,
    mut console: ResMut<Console>,
    mut after: ResMut<AfterLoad>,
    mut regrab: ResMut<RegrabCursor>,
    mut commands: Commands,
    (browser, first_run, windows): (
        Option<Res<crate::client::server_browser::ServerBrowser>>,
        Option<Res<crate::client::first_run::FirstRun>>,
        Option<Res<Windows>>,
    ),
) {
    if ui.open || !menu_has_keys(&menu, browser.as_deref(), first_run.as_deref(), windows.as_deref()) {
        return;
    }
    if !menu.open {
        if keys.just_pressed(KeyCode::Escape) && menu.loading.is_none() {
            commands.queue(|w: &mut World| open_menu(w, Page::Main));
        }
        return;
    }
    if menu.capture.is_some() {
        let input = if keys.just_pressed(KeyCode::Escape) {
            Some(Input::Close)
        } else {
            let none = ButtonInput::<MouseButton>::default();
            binds::first_pressed(&keys, mouse.as_deref().unwrap_or(&none), scroll.as_deref()).map(Input::BindKey)
        };
        if let Some(input) = input {
            apply(input, &mut menu, &mut console, &mut after, &mut regrab);
        }
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let tab = if shift { -1 } else { 1 };
    let text = typing(&menu);
    let mut inputs = Vec::new();
    for (key, input) in [
        (KeyCode::Escape, Some(Input::Close)),
        (KeyCode::ArrowUp, Some(Input::Up)),
        (KeyCode::ArrowDown, Some(Input::Down)),
        (KeyCode::ArrowLeft, (!text).then_some(Input::Left)),
        (KeyCode::ArrowRight, (!text).then_some(Input::Right)),
        (KeyCode::Enter, Some(Input::Activate)),
        (KeyCode::NumpadEnter, Some(Input::Activate)),
        (KeyCode::Space, (!text).then_some(Input::Space)),
        (KeyCode::Backspace, (!text).then_some(Input::Back)),
        (KeyCode::Tab, Some(if ctrl { Input::NextTab(tab) } else { Input::Focus(tab) })),
        (KeyCode::PageUp, Some(Input::Scroll(-(KEY_ROWS as i32)))),
        (KeyCode::PageDown, Some(Input::Scroll(KEY_ROWS as i32))),
    ] {
        if keys.just_pressed(key)
            && let Some(input) = input
        {
            inputs.push(input);
        }
    }
    if !text && !ctrl {
        inputs.extend(LETTERS.iter().filter(|(k, _)| keys.just_pressed(*k)).map(|(_, c)| Input::Char(*c)));
    }
    for input in inputs {
        apply(input, &mut menu, &mut console, &mut after, &mut regrab);
    }
}

/// Typing into the focused text entry: characters, Backspace, Delete,
/// the arrows, Home and End (Shift selects), Ctrl+A, Ctrl+C, Ctrl+X,
/// Ctrl+V.
#[allow(clippy::too_many_arguments)]
pub(super) fn text_keys(
    mut events: MessageReader<KeyboardInput>,
    held: Res<ButtonInput<KeyCode>>,
    ui: Res<crate::client::console::ConsoleUi>,
    mut menu: ResMut<GameMenu>,
    mut console: ResMut<Console>,
    mut after: ResMut<AfterLoad>,
    mut regrab: ResMut<RegrabCursor>,
    (browser, first_run, windows): (
        Option<Res<crate::client::server_browser::ServerBrowser>>,
        Option<Res<crate::client::first_run::FirstRun>>,
        Option<Res<Windows>>,
    ),
) {
    let live = menu.open
        && !ui.open
        && menu.capture.is_none()
        && typing(&menu)
        && menu_has_keys(&menu, browser.as_deref(), first_run.as_deref(), windows.as_deref());
    if !live {
        events.clear();
        return;
    }
    let shift = held.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = held.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight]);
    for e in events.read() {
        if e.state != ButtonState::Pressed {
            continue;
        }
        let input = match (&e.logical_key, e.key_code) {
            (_, KeyCode::KeyA) if ctrl => Input::Edit(Edit::SelectAll),
            (_, KeyCode::KeyC) if ctrl => Input::Copy,
            (_, KeyCode::KeyX) if ctrl => Input::Cut,
            (_, KeyCode::KeyV) if ctrl => {
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
                Input::Type(text.lines().next().unwrap_or("").to_string())
            }
            (_, KeyCode::ArrowLeft) => Input::Edit(Edit::Left(shift)),
            (_, KeyCode::ArrowRight) => Input::Edit(Edit::Right(shift)),
            (_, KeyCode::Home) => Input::Edit(Edit::Home(shift)),
            (_, KeyCode::End) => Input::Edit(Edit::End(shift)),
            (_, KeyCode::Delete) => Input::Edit(Edit::Delete),
            (Key::Backspace, _) => Input::Edit(Edit::Backspace),
            (Key::Space, _) => Input::Type(" ".into()),
            (Key::Character(s), _) if !ctrl => Input::Type(s.to_string()),
            _ => continue,
        };
        apply(input, &mut menu, &mut console, &mut after, &mut regrab);
    }
}

/// What a UI node stands for: a row (step 0) or one of its parts.
#[derive(Component, Clone, Copy)]
pub(super) struct Hit(pub(super) Target, pub(super) i32);

/// A combo box row the wheel steps.
#[derive(Component, Clone, Copy)]
pub(super) struct WheelCombo(pub(super) usize);

/// A text entry row: a click places its caret.
#[derive(Component, Clone)]
pub(super) struct TextHit {
    pub(super) row: usize,
    pub(super) shown: String,
    pub(super) width: f32,
}

/// A list the wheel scrolls.
#[derive(Component)]
pub(super) struct WheelList;

/// Hovering focuses, clicking presses, the wheel scrolls a list under the
/// mouse (or an open combo box's list, or steps a combo box), a held
/// slider follows the mouse.
#[allow(clippy::too_many_arguments)]
pub(super) fn pointer(
    hits: Query<(&Interaction, &Hit), Changed<Interaction>>,
    texts: Query<(&Interaction, &TextHit, &RelativeCursorPosition), Changed<Interaction>>,
    lists: Query<&RelativeCursorPosition, With<WheelList>>,
    combos: Query<(&WheelCombo, &RelativeCursorPosition)>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    slider: Res<SliderDrag>,
    fonts: Res<UiFonts>,
    windows_q: Query<&Window>,
    mut menu: ResMut<GameMenu>,
    mut console: ResMut<Console>,
    mut after: ResMut<AfterLoad>,
    mut regrab: ResMut<RegrabCursor>,
    (browser, first_run): (Option<Res<crate::client::server_browser::ServerBrowser>>, Option<Res<crate::client::first_run::FirstRun>>),
) {
    if !menu.open || menu.capture.is_some() || first_run.is_some_and(|f| f.open) {
        return;
    }
    let mut inputs: Vec<Input> = hits
        .iter()
        .filter_map(|(i, h)| match i {
            Interaction::Hovered => Some(Input::Hover(h.0)),
            Interaction::Pressed => Some(Input::Click(h.0, h.1)),
            Interaction::None => None,
        })
        .collect();
    let height = windows_q.iter().next().map_or(720.0, Window::height);
    for (i, hit, at) in &texts {
        if *i == Interaction::Pressed
            && let Some(p) = at.normalized
        {
            let look = Look {
                ui: menu.ui.0.as_deref(),
                fonts: &fonts,
                s: (height / 720.0).clamp(0.6, 3.0),
                height,
                accent: Color::WHITE,
            };
            let caret = (menu.focus == hit.row).then_some(menu.caret);
            let at = widgets::caret_from_click(&fonts, &look.default_font(), look.s, &hit.shown, hit.width, p.x + 0.5, caret);
            inputs.push(Input::Click(Target::Row(hit.row), 0));
            inputs.push(Input::Caret(hit.row, at));
        }
    }
    if let Some((SliderOwner::Menu, row, f)) = slider.at
        && slider.is_changed()
    {
        inputs.push(Input::Slide(row, f));
    }
    if let Some(scroll) = scroll
        && scroll.delta.y != 0.0
    {
        let notches = widgets::wheel_notches(&scroll);
        if menu.combo.is_some() {
            inputs.push(Input::Scroll(-notches));
        } else if lists.iter().any(|l| l.cursor_over()) {
            inputs.push(Input::Scroll(-notches * 3));
        } else if let Some((c, _)) = combos.iter().find(|(_, r)| r.cursor_over()) {
            inputs.push(Input::Wheel(c.0, notches));
        }
    }
    // The browser in front of the dialog: its own (Bevy's picking already
    // stops at the frame in front).
    let _ = browser;
    for input in inputs {
        apply(input, &mut menu, &mut console, &mut after, &mut regrab);
    }
}

/// Bots and players per team (bots and team pages), and the binds (the
/// keyboard tab), while open.
pub(super) fn sync(
    mut menu: ResMut<GameMenu>,
    console: Res<Console>,
    characters: Query<(&Team, Has<crate::bot::Bot>), With<Intent>>,
    rules: Option<Res<crate::rules::Deathmatch>>,
) {
    if !menu.open {
        return;
    }
    let no_spectators = !crate::client::team_menu::spectators_allowed(rules.as_deref());
    if menu.no_spectators != no_spectators {
        menu.no_spectators = no_spectators;
    }
    let (mut bots, mut players) = ([0; 2], [0; 2]);
    for (team, bot) in &characters {
        let Some(i) = [1, 2].iter().position(|t| *t == team.0) else {
            continue;
        };
        players[i] += 1;
        if bot {
            bots[i] += 1;
        }
    }
    if menu.bots != bots || menu.players != players {
        menu.set_counts(bots, players);
    }
    if menu.binds != console.binds {
        menu.binds = console.binds.clone();
    }
}

/// The mouse is free while the menu is open (whoever grabbed it); once
/// closed, grabbed again when the mouse button is up.
pub(super) fn cursor(
    menu: Res<GameMenu>,
    mut regrab: ResMut<RegrabCursor>,
    mouse: Res<ButtonInput<MouseButton>>,
    ui: Res<crate::client::console::ConsoleUi>,
    cursor: Option<Single<&mut CursorOptions>>,
) {
    let Some(mut cursor) = cursor else { return };
    if menu.open {
        if crate::client::input::cursor_grabbed(&cursor) {
            crate::client::input::release_cursor(&mut cursor);
        }
    } else if regrab.0 && !mouse.pressed(MouseButton::Left) {
        regrab.0 = false;
        if !ui.open {
            crate::client::input::capture_cursor(&mut cursor);
        }
    }
}

/// Run a new game's bot lines once its map is in (or if the load never
/// started: the `map` line failed).
pub(super) fn after_load(w: &mut World) {
    // A load started at the main menu (a menu, the console): its dialog.
    // A server's own map change too (its players load it as well): the
    // dialog over the game, as theirs (single player keeps playing the
    // old map until the new one is in).
    let serving = w.get_resource::<crate::core::NetRole>() == Some(&crate::core::NetRole::Server);
    if let Some(map) = crate::client::console::loading_map(w)
        && let Some(mut menu) = w.get_resource_mut::<GameMenu>()
        && (!menu.in_game || serving)
        && menu.loading.is_none()
    {
        if menu.in_game {
            menu.in_game = false;
            menu.open = true;
            menu.page = Page::Main;
        }
        menu.loading = Some(map);
    }
    let loading = crate::client::console::map_loading(w);
    let mut after = w.resource_mut::<AfterLoad>();
    if after.lines.is_empty() {
        return;
    }
    if loading {
        after.started = true;
        return;
    }
    after.waited += 1;
    // The `map` line runs in the next frame's queue.
    if !after.started && after.waited < 10 {
        return;
    }
    let lines = std::mem::take(&mut after.lines);
    let mut console = w.resource_mut::<Console>();
    for line in lines {
        console.submit(line);
    }
}

/// The interface's sounds (`GameUi::sounds`): the rollover as the
/// pointer comes onto an entry or button, the click as one is pressed,
/// the release as it is let go.
pub(super) fn menu_sounds(
    menu: Res<GameMenu>,
    menu_ui: Option<Res<MenuUi>>,
    hits: Query<(&Interaction, &Hit), Changed<Interaction>>,
    clips: Option<ResMut<Assets<crate::map::live_sound::LiveClip>>>,
    mut last: Local<(Option<Target>, Option<Target>)>,
    mut commands: Commands,
) {
    let (Some(ui), Some(mut clips)) = (menu_ui.and_then(|u| u.ui.clone()), clips) else {
        return;
    };
    if !menu.open {
        *last = (None, None);
        return;
    }
    use crate::map::hud::UiSound;
    let mut play = |sound: UiSound| {
        if let Some(clip) = ui.sounds.get(&sound) {
            let gains = std::sync::Arc::new(crate::map::live_sound::Gains::new(1.0, 1.0));
            let handle = clips.add(crate::map::live_sound::LiveClip::new(clip.clone(), gains, default()));
            commands.spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN));
        }
    };
    let (hovered, pressed) = &mut *last;
    for (interaction, hit) in &hits {
        // The outside of an open list isn't a button.
        if matches!(hit.0, Target::Outside) {
            continue;
        }
        match interaction {
            Interaction::Hovered => {
                if *pressed == Some(hit.0) {
                    play(UiSound::Release);
                } else if *hovered != Some(hit.0) {
                    play(UiSound::Rollover);
                }
                *hovered = Some(hit.0);
                *pressed = None;
            }
            Interaction::Pressed => {
                play(UiSound::Click);
                *pressed = Some(hit.0);
            }
            // Off it: coming back rolls over again (a redraw replaces the
            // node without this, so the entry under the pointer is quiet).
            Interaction::None if *hovered == Some(hit.0) => *hovered = None,
            Interaction::None => {}
        }
    }
}

/// The world drawn under the window's UI (while `world_behind_dialogs`
/// draws it into a picture).
#[derive(Component)]
pub(super) struct WorldUnderUi;

/// While a dialog is open over the in-game menu, the 3D view is drawn
/// into a picture the window's size (`widgets::WorldPicture`) instead of
/// the window: shown whole under all the UI (the game looks as before)
/// and again, dimmed, under each frame (`widgets::Backing::World`), so a
/// dialog hides the menu's entries under it and still shows the world.
/// Back to the window when no dialog is open. A camera drawing into a
/// picture of its own (`--views` captures) is left alone.
#[allow(clippy::type_complexity)]
pub(super) fn world_behind_dialogs(
    backing: Res<widgets::FrameBacking>,
    frames: Query<(), With<widgets::VguiFrame>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: Query<(Entity, &bevy::camera::RenderTarget), With<crate::client::FirstPersonCamera>>,
    mut picture: ResMut<widgets::WorldPicture>,
    mut images: ResMut<Assets<Image>>,
    mut shown: Query<(Entity, &mut ImageNode), With<WorldUnderUi>>,
    mut commands: Commands,
) {
    use bevy::camera::RenderTarget;
    let ours = |t: &RenderTarget| match (t, &picture.0) {
        (RenderTarget::Image(i), Some((h, _))) => i.handle == *h,
        _ => false,
    };
    let camera = cameras
        .iter()
        .find(|(_, t)| matches!(t, RenderTarget::Window(_)) || ours(t))
        .map(|(e, t)| (e, matches!(t, RenderTarget::Image(i) if picture.0.as_ref().is_some_and(|(h, s)| i.handle == *h && size_now(&windows) == Some(*s)))));
    let size = windows.iter().next().map(|w| w.resolution.physical_size());
    let wanted = matches!(backing.0, Some(widgets::Backing::World(_))) && !frames.is_empty();
    let (Some((camera, drawing)), Some(size), true) = (camera, size.filter(|s| s.x > 0 && s.y > 0), wanted) else {
        if picture.0.is_some() {
            for (e, t) in &cameras {
                if ours(t) {
                    commands
                        .entity(e)
                        .insert(RenderTarget::Window(bevy::window::WindowRef::Primary));
                }
            }
            for (e, _) in &shown {
                commands.entity(e).despawn();
            }
            picture.0 = None;
        }
        return;
    };
    let handle = match &picture.0 {
        Some((h, s)) if *s == size.as_vec2() => h.clone(),
        _ => {
            if let Some((old, _)) = picture.0.take() {
                images.remove(&old);
            }
            let h = images.add(Image::new_target_texture(
                size.x,
                size.y,
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                None,
            ));
            picture.0 = Some((h.clone(), size.as_vec2()));
            h
        }
    };
    if !drawing {
        commands.entity(camera).insert(RenderTarget::Image(handle.clone().into()));
    }
    match shown.single_mut() {
        Ok((_, mut node)) => {
            if node.image != handle {
                node.image = handle;
            }
        }
        Err(_) => {
            commands.spawn((
                WorldUnderUi,
                Name::new("World under the UI"),
                ImageNode::new(handle),
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100.0),
                    height: percent(100.0),
                    ..default()
                },
                // Under everything the UI draws.
                GlobalZIndex(i32::MIN / 2),
                bevy::ui::FocusPolicy::Pass,
            ));
        }
    }
}

/// The primary window's size in physical pixels.
fn size_now(windows: &Query<&Window, With<bevy::window::PrimaryWindow>>) -> Option<Vec2> {
    windows.iter().next().map(|w| w.resolution.physical_size().as_vec2())
}
