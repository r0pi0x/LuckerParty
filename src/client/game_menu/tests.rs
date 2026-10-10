use super::*;
use crate::client::options::setting_index;
use crate::map::hud::{GameUiItem, ServerSetting};

const CVARS: [(&str, &str); 17] = [
    ("sensitivity", "3"),
    ("m_pitch", "0.022"),
    ("zoom_sensitivity_ratio", "1.2"),
    ("volume", "1"),
    ("viewmodel_fov", "54"),
    ("cl_righthand", "1"),
    ("cl_showfps", "0"),
    ("mat_vsync", "1"),
    ("mat_antialias", "4"),
    ("mashup_resolution", ""),
    ("mashup_rounds", "1"),
    ("bot_reaction", "0.35"),
    ("hud_fastswitch", "0"),
    ("con_enable", "1"),
    ("cl_autowepswitch", "1"),
    ("hud_centerid", "1"),
    ("cl_disablefreezecam", "0"),
];

fn get(n: &str) -> Option<String> {
    CVARS
        .iter()
        .chain(&[("hostname", "Mine"), ("mp_friendlyfire", "0"), ("bot_prefix", "")])
        .find(|(k, _)| *k == n)
        .map(|(_, v)| v.to_string())
}

/// The menu opened in a game (Esc).
fn menu() -> GameMenu {
    let mut m = GameMenu {
        in_game: true,
        ..default()
    };
    m.open(Page::Main, vec!["cs_office".into(), "de_dust2".into(), "de_nuke".into()], Some("de_dust2"), get);
    m
}

/// The menu at startup: the main menu, out of a game.
fn main_menu() -> GameMenu {
    let mut m = GameMenu::default();
    m.open(Page::Main, vec!["cs_office".into(), "de_dust2".into(), "de_nuke".into()], None, |_| None);
    m
}

#[test]
fn cancel_stops_a_load_and_nothing_else_works_meanwhile() {
    let mut m = main_menu();
    m.loading = Some("de_dust2".into());
    let before = m.clone();
    assert!(m.handle(Input::Click(Target::Main(0), 0)).lines.is_empty());
    assert_eq!(m, before, "the dialog takes no other input");
    let out = m.handle(Input::Click(Target::Cancel, 0));
    assert_eq!(out.lines, vec!["disconnect".to_string()]);
    assert!(m.loading.is_none() && m.open && !m.in_game);
}

#[test]
fn the_loading_line_follows_the_load() {
    let m = main_menu();
    assert_eq!(loading_text(&m, None, "de_nuke"), "Loading de_nuke ...");
    let p = crate::map::loading::LoadProgress {
        fraction: 0.5,
        stage: "LoadingProgress_LoadResources",
    };
    // Without the game's strings, the token itself.
    assert_eq!(loading_text(&m, Some(p), "de_nuke"), "LoadingProgress_LoadResources");
}

#[test]
fn joining_shows_its_stages_and_the_download() {
    use crate::net::client::{DownloadInfo, JoinProgress, JoinStage};
    let m = main_menu();
    let mut j = JoinProgress {
        stage: JoinStage::Connecting,
        ..default()
    };
    assert_eq!(dialog_state(&m, Some(&j), None, false, "x").0, "Connecting to server...");
    j.stage = JoinStage::ServerInfo;
    assert_eq!(dialog_state(&m, Some(&j), None, false, "x").0, "Retrieving server info...");
    j.stage = JoinStage::ChangingLevel;
    assert_eq!(dialog_state(&m, Some(&j), None, false, "x").0, "Server is changing level...");
    j.stage = JoinStage::Downloading;
    j.download = Some(DownloadInfo {
        file: "maps/a.bsp".into(),
        from: "server".into(),
        done: 250,
        total: 1000,
    });
    assert_eq!(
        dialog_state(&m, Some(&j), None, false, "x"),
        ("Verifying and downloading resources...".to_string(), 0.25)
    );
    // Loading the map: the map's own stages.
    j.stage = JoinStage::LoadingMap;
    let p = crate::map::loading::LoadProgress {
        fraction: 0.5,
        stage: "LoadingProgress_LoadMap",
    };
    assert_eq!(dialog_state(&m, Some(&j), Some(p), false, "x").1, 0.5);
    // A host starting its server, before the map reports.
    assert_eq!(
        dialog_state(&m, None, None, true, "de_dust2").0,
        "Starting local game server..."
    );
    assert_eq!(dialog_state(&m, None, None, false, "de_dust2").0, "Loading de_dust2 ...");
    let (stage, text) = detail_stage(Some(&JoinProgress {
        stage: JoinStage::Downloading,
        download: j.download.clone(),
        ..default()
    }), None);
    assert_eq!(stage, "downloading the map");
    assert_eq!(text, "0 KB of 1 KB (25%) of maps/a.bsp from server");
}

#[test]
fn a_failure_shows_until_closed() {
    use crate::net::client::JoinFailure;
    let mut m = main_menu();
    assert_eq!(net_failure_text(&m, &JoinFailure::Full, "Server is full."), "Server is full.");
    m.failure = Some("Server is full.".into());
    m.loading = Some(String::new());
    assert!(m.handle(Input::Click(Target::Main(0), 0)).lines.is_empty());
    assert!(m.failure.is_some(), "only Close closes it");
    let out = m.handle(Input::Click(Target::Cancel, 0));
    assert!(out.lines.is_empty(), "already disconnected");
    assert!(m.failure.is_none() && m.loading.is_none() && m.open);
}

/// A shown entry's index.
fn at(m: &GameMenu, item: MainItem) -> usize {
    m.entries().iter().position(|e| e.item == item).unwrap()
}

/// Click a shown entry.
fn click(m: &mut GameMenu, item: MainItem) -> Outcome {
    let i = at(m, item);
    press(m, &[Input::Click(Target::Main(i), 0)])
}

fn shown(m: &GameMenu) -> Vec<MainItem> {
    m.entries().iter().map(|e| e.item).collect()
}

fn press(m: &mut GameMenu, inputs: &[Input]) -> Outcome {
    let mut all = Outcome::default();
    for i in inputs {
        let o = m.handle(i.clone());
        all.lines.extend(o.lines);
        all.after_load.extend(o.after_load);
        all.close |= o.close;
        all.clipboard = o.clipboard.or(all.clipboard);
    }
    all
}

/// Click the row `row` finds.
fn click_on(m: &mut GameMenu, row: impl Fn(&GameMenu) -> usize) -> Outcome {
    let i = row(m);
    press(m, &[Input::Click(Target::Row(i), 0)])
}

fn setting(cvar: &str) -> usize {
    SETTINGS.iter().position(|s| s.cvar == cvar).unwrap()
}

/// The row of the open page with this field.
fn row_with(m: &GameMenu, field: Field) -> usize {
    m.rows().iter().position(|r| r.field() == Some(field)).unwrap_or_else(|| panic!("no {field:?}"))
}

fn row_of(m: &GameMenu, cvar: &str) -> usize {
    row_with(m, Field::Setting(setting(cvar)))
}

fn button(m: &GameMenu, action: Action) -> usize {
    m.rows()
        .iter()
        .position(|r| matches!(r, Row::Button { action: a, .. } if *a == action))
        .unwrap()
}

fn control(m: &GameMenu, i: usize) -> Control {
    match &m.rows()[i] {
        Row::Control { control, .. } => control.clone(),
        r => panic!("{r:?}"),
    }
}

#[test]
fn opens_with_the_game_as_it_is() {
    let m = menu();
    assert!(m.open);
    assert_eq!((m.page, m.focus), (Page::Main, 0));
    assert_eq!(m.new_game.map, 1, "the loaded map");
    assert_eq!(m.new_game.difficulty, NORMAL);
    assert!(!m.new_game.bots_on, "no bots in the game now");
    // No crosshair colour cvar given: not available.
    assert_eq!(m.values[setting("cl_crosshaircolor")], None);
    assert_eq!(m.entries().len(), MAIN.len() + OURS.len());
}

#[test]
fn in_game_entries_show_only_in_a_game() {
    use MainItem::*;
    let m = menu();
    assert_eq!(
        shown(&m),
        [Resume, Disconnect, FindServers, NewGame, BugReport, Options, Quit, QuickStart, Greybox, Bots, Team, Extras, Console]
    );
    let e = m.entries();
    assert!(e[2].gap && e[2].enabled, "a gap under Disconnect, then Find Servers");
    assert!(e[7].gap, "ours after a gap");
    let m = main_menu();
    assert_eq!(shown(&m), [FindServers, NewGame, BugReport, Options, Quit, QuickStart, Greybox, Extras, Console]);
    let e = m.entries();
    assert!(!e[0].gap, "no gap over the first entry");
    assert_eq!(m.focus, 0, "the first entry: Find Servers");
    assert_eq!(e[1].label, "CREATE SERVER");
}

#[test]
fn find_servers_opens_the_server_browser() {
    let mut m = main_menu();
    let find = at(&m, MainItem::FindServers);
    let o = press(&mut m, &[Input::Hover(Target::Main(find)), Input::Click(Target::Main(find), 0)]);
    assert_eq!(o.lines, ["openserverbrowser"]);
    assert_eq!(m.focus, find);
    assert!(m.open && m.page == Page::Main, "the browser shows over the menu");
    // Up from it wraps to Console.
    press(&mut m, &[Input::Up]);
    assert_eq!(m.entries()[m.focus].item, MainItem::Console);
}

#[test]
fn the_main_menu_stays_open() {
    let mut m = main_menu();
    let o = press(&mut m, &[Input::Close]);
    assert!(!o.close && m.open && m.page == Page::Main, "Esc does nothing there");
    // Esc cancels a dialog, back to the main menu.
    click(&mut m, MainItem::Options);
    assert_eq!(m.page, Page::Settings);
    let o = press(&mut m, &[Input::Close]);
    assert!(!o.close && m.open && m.page == Page::Main);
    // A bug report from it keeps it open.
    let o = click(&mut m, MainItem::BugReport);
    assert_eq!(o.lines, ["bugreport"]);
    assert!(m.open && !o.close);
    // The console entry opens the console over it.
    let o = click(&mut m, MainItem::Console);
    assert_eq!(o.lines, ["toggleconsole"]);
    assert!(m.open);
}

#[test]
fn create_server_starts_a_map_from_the_main_menu() {
    let mut m = main_menu();
    click(&mut m, MainItem::NewGame);
    assert_eq!((m.page, m.create_tab), (Page::NewGame, 0));
    // Enter: Start, the default button.
    let o = press(&mut m, &[Input::Activate]);
    assert_eq!(o.lines.last().map(String::as_str), Some("map cs_office"));
    // The menu stays up, loading, and takes no input until the map is in.
    assert!(!o.close && m.open);
    assert_eq!(m.loading.as_deref(), Some("cs_office"));
    assert_eq!(press(&mut m, &[Input::Close, Input::Activate]), Outcome::default());
    assert!(m.enter_game(), "the load closes the open menu");
    assert!(m.in_game && !m.open && m.loading.is_none());
    // Esc now opens the in-game menu, with Resume.
    m.open(Page::Main, vec!["cs_office".into()], Some("cs_office"), |_| None);
    assert_eq!(m.entries()[m.focus].item, MainItem::Resume);
}

#[test]
fn quick_start_and_the_greybox() {
    let mut m = main_menu();
    let o = click(&mut m, MainItem::QuickStart);
    assert_eq!(
        o.lines,
        ["bot_kick", "mashup_rounds 1", "bot_reaction 0.35", "bot_aim_error 2.5", "bot_turn_rate 360", "map de_dust2"]
    );
    assert_eq!(o.after_load.len(), QUICK_BOTS as usize);
    assert_eq!(o.after_load.iter().filter(|l| *l == "bot_add 1").count(), 5, "five terrorists, four CTs");
    assert_eq!(m.loading.as_deref(), Some(QUICK_MAP));
    let mut m = main_menu();
    let o = click(&mut m, MainItem::Greybox);
    assert_eq!(o.lines, ["map greybox"]);
    assert_eq!(m.loading.as_deref(), Some("greybox"));
    // From a game, the greybox closes the menu at once.
    let mut m = menu();
    let o = click(&mut m, MainItem::Greybox);
    assert!(o.close && !m.open && o.lines == ["map greybox"]);
}

/// `disconnect` and `map greybox` on a world (headless): the map gives
/// way to the greybox, the menu follows.
#[test]
fn disconnect_and_the_greybox_on_the_world() {
    let mut app = App::new();
    app.add_plugins(crate::console::ConsolePlugin)
        .insert_resource(GameMenu {
            in_game: true,
            ..default()
        })
        .init_resource::<RegrabCursor>()
        .insert_resource(crate::map::LoadedMapName("cs_source:de_dust2".into()));
    let w = app.world_mut();
    super::super::console::load_greybox(w);
    left_game(w);
    let parts = w
        .query_filtered::<(), With<crate::greybox::GreyboxPart>>()
        .iter(w)
        .count();
    assert!(parts > 0, "the greybox is back behind the main menu");
    assert_eq!(w.resource::<crate::map::LoadedMapName>().0, "greybox");
    let m = w.resource::<GameMenu>();
    assert!(m.open && !m.in_game && m.page == Page::Main);
    assert!(!shown(m).contains(&MainItem::Resume));
    // The greybox from the main menu: in a game, the menu closed and
    // the mouse grabbed again.
    w.resource_mut::<GameMenu>().loading = Some("greybox".into());
    entered_game(w);
    let m = w.resource::<GameMenu>();
    assert!(m.in_game && !m.open && m.loading.is_none());
    assert!(w.resource::<RegrabCursor>().0);
    // A failed load gives the main menu back.
    let mut m = w.resource_mut::<GameMenu>();
    m.leave_game();
    m.loading = Some("nope".into());
    map_load_failed(w);
    assert!(w.resource::<GameMenu>().loading.is_none());
}

#[test]
fn disconnect_returns_to_the_main_menu() {
    let mut m = menu();
    let o = click(&mut m, MainItem::Disconnect);
    assert_eq!(o.lines, ["disconnect"]);
    assert!(!o.close && m.open && !m.in_game && m.page == Page::Main);
    assert!(!shown(&m).contains(&MainItem::Resume) && !shown(&m).contains(&MainItem::Disconnect));
    assert!(m.entries()[m.focus].enabled);
    // Esc no longer leaves the menu.
    press(&mut m, &[Input::Close]);
    assert!(m.open);
}

#[test]
fn normal_difficulty_is_the_bots_default() {
    let c = crate::bot::BotConfig::default();
    let (_, r, a, t) = DIFFICULTIES[NORMAL];
    assert_eq!((r, a, t), (c.reaction, c.aim_error, c.turn_rate));
}

#[test]
fn escape_and_resume_close_with_no_lines() {
    let mut m = menu();
    let o = m.handle(Input::Close);
    assert!(o.close && o.lines.is_empty() && !m.open);
    let mut m = menu();
    let o = m.handle(Input::Activate);
    assert!(o.close && o.lines.is_empty() && !m.open);
    // A closed menu ignores input.
    assert_eq!(m.handle(Input::Activate), Outcome::default());
}

#[test]
fn keys_move_through_pages_and_back() {
    let mut m = menu();
    click(&mut m, MainItem::Bots);
    assert_eq!(m.page, Page::Bots);
    // The info line can't be focused.
    assert_eq!(m.focus, 1);
    press(&mut m, &[Input::Up]);
    assert_eq!(m.focus, 4, "wraps past the info line to Close");
    press(&mut m, &[Input::Focus(1)]);
    assert_eq!(m.focus, 1, "Tab wraps too");
    press(&mut m, &[Input::Back]);
    assert_eq!((m.page, m.focus), (Page::Main, at(&m, MainItem::Bots)), "back on its entry");
    // Up past ours to Quit.
    press(&mut m, &[Input::Up, Input::Up, Input::Up]);
    assert_eq!(m.entries()[m.focus].item, MainItem::Quit);
    assert_eq!(press(&mut m, &[Input::Activate]).lines, ["quit"]);
}

#[test]
fn bots_and_team_pages_run_their_commands() {
    let mut m = menu();
    click(&mut m, MainItem::Bots);
    assert_eq!(m.page, Page::Bots);
    let o = press(
        &mut m,
        &[
            Input::Click(Target::Row(1), 0),
            Input::Click(Target::Row(2), 0),
            Input::Click(Target::Row(3), 0),
        ],
    );
    assert_eq!(o.lines, ["bot_add 1", "bot_add 2", "bot_kick"]);
    assert!(!o.close && m.open, "adding bots keeps the menu open");
    // Info rows ignore clicks.
    assert!(press(&mut m, &[Input::Click(Target::Row(0), 0)]).lines.is_empty());

    click(&mut m, MainItem::Team);
    assert_eq!(m.page, Page::Team);
    let o = press(&mut m, &[Input::Down, Input::Activate]);
    assert_eq!(o.lines, ["jointeam 3"]);
    assert!(o.close && !m.open, "joining a team goes back to playing");

    // Auto-assign joins the smaller team.
    let mut m = menu();
    m.set_counts([0, 0], [3, 1]);
    click(&mut m, MainItem::Team);
    assert_eq!(press(&mut m, &[Input::Click(Target::Row(2), 0)]).lines, ["jointeam 3"]);
}

/// Create Server driven as CS:S's: the map from its drop-down, bots
/// included, how many typed, a difficulty button, the Bot page's team,
/// the Game page's options; Start sets what changed, loads the map,
/// then adds the bots.
#[test]
fn create_server_by_its_controls() {
    let mut m = menu();
    let ui = GameUi {
        server_settings: vec![
            ServerSetting {
                cvar: "hostname".into(),
                label: "Server Name".into(),
                kind: ServerSettingKind::Text,
                default: "Counter-Strike Source".into(),
            },
            ServerSetting {
                cvar: "mp_friendlyfire".into(),
                label: "Friendly fire".into(),
                kind: ServerSettingKind::Bool,
                default: "0".into(),
            },
            ServerSetting {
                cvar: "mp_timelimit".into(),
                label: "Time limit".into(),
                kind: ServerSettingKind::Number { min: Some(0.0), max: None },
                default: "20".into(),
            },
        ],
        ..default()
    };
    m.set_ui(Some(Arc::new(ui)));
    m.open(Page::Main, vec!["cs_office".into(), "de_dust2".into(), "de_nuke".into()], Some("de_dust2"), get);
    click(&mut m, MainItem::NewGame);
    // The map: a click opens its list on the map now, Down and Enter
    // pick the next.
    let map = row_with(&m, Field::Map);
    press(&mut m, &[Input::Click(Target::Row(map), 0)]);
    // CS:S's `< Random Map >` first.
    assert_eq!(m.combo.map(|c| (c.owner, c.highlight, c.len)), Some((map, 2, 4)));
    press(&mut m, &[Input::Down, Input::Activate]);
    assert_eq!((m.new_game.map, m.combo), (2, None));
    // Include bots: their count and difficulty show.
    assert!(m.rows().iter().all(|r| r.field() != Some(Field::BotCount)));
    click_on(&mut m, |m| row_with(m, Field::BotsOn));
    let count = row_with(&m, Field::BotCount);
    // Tab to the count (its text selected), type over it; only digits.
    press(&mut m, &[Input::Focus(1)]);
    assert_eq!(m.focus, count);
    press(&mut m, &[Input::Type("3x".into())]);
    assert_eq!(m.new_game.bot_count, "3");
    press(&mut m, &[Input::Type("45".into())]);
    assert_eq!(m.new_game.bot_count, "3", "two digits at most");
    click_on(&mut m, |m| row_with(m, Field::Difficulty(2)));
    assert_eq!(m.new_game.difficulty, 2);
    assert_eq!(control(&m, row_with(&m, Field::Difficulty(2))), Control::Radio(true));
    // The Bot page: counter-terrorists only.
    press(&mut m, &[Input::Click(Target::Tab(2), 0)]);
    assert_eq!(m.create_tab, 2);
    let team = row_with(&m, Field::BotTeam);
    press(&mut m, &[Input::Click(Target::Row(team), 0), Input::Click(Target::ComboItem(2), 0)]);
    assert_eq!(m.new_game.bot_team, 2);
    // The Game page: friendly fire on (Ctrl+Tab back to it).
    press(&mut m, &[Input::NextTab(-1)]);
    assert_eq!(m.create_tab, 1);
    let rows = m.rows();
    let ff = rows
        .iter()
        .position(|r| matches!(r, Row::Control { label, .. } if label == "Friendly fire"))
        .unwrap();
    assert!(!rows.iter().any(|r| matches!(r, Row::Control { label, .. } if label == "Time limit")), "mashup lacks it");
    let o = press(&mut m, &[Input::Click(Target::Row(ff), 0)]);
    assert!(o.lines.is_empty(), "nothing runs before Start: {:?}", o.lines);
    let o = click_on(&mut m, |m| button(m, Action::Start));
    assert_eq!(
        o.lines,
        [
            "bot_kick",
            "mashup_rounds 1",
            "mp_friendlyfire 1",
            "bot_reaction 0.2",
            "bot_aim_error 1.5",
            "bot_turn_rate 540",
            "map de_nuke",
        ]
    );
    assert_eq!(o.after_load, ["bot_add 2", "bot_add 2", "bot_add 2"]);
    assert!(o.close && !m.open);
}

#[test]
fn create_servers_combo_boxes_take_the_wheel_letters_and_escape() {
    let mut m = menu();
    click(&mut m, MainItem::NewGame);
    let map = row_with(&m, Field::Map);
    // Closed and focused: Up/Down and the wheel step it, no wrapping.
    press(&mut m, &[Input::FocusRow(map), Input::Down, Input::Down, Input::Down]);
    assert_eq!(m.new_game.map, 2);
    press(&mut m, &[Input::Wheel(map, 1)]);
    assert_eq!(m.new_game.map, 1);
    // A letter jumps.
    press(&mut m, &[Input::Char('c')]);
    assert_eq!(m.new_game.map, 0);
    // Open: hover highlights, Esc closes without a pick, a click outside
    // too.
    press(&mut m, &[Input::Space, Input::Hover(Target::ComboItem(2))]);
    assert_eq!(m.combo.map(|c| c.highlight), Some(2));
    press(&mut m, &[Input::Close]);
    assert_eq!((m.combo, m.new_game.map, m.page), (None, 0, Page::NewGame), "Esc closed only the list");
    press(&mut m, &[Input::Space, Input::Click(Target::Outside, 0)]);
    assert_eq!((m.combo, m.new_game.map), (None, 0));
    // Its wheel scrolls the list, not the value.
    press(&mut m, &[Input::Space, Input::Scroll(1)]);
    assert_eq!(m.new_game.map, 0);
    press(&mut m, &[Input::Char('d'), Input::Activate]);
    assert_eq!(m.new_game.map, 1, "a letter in the open list, Enter picks");
}

#[test]
fn no_maps_no_start() {
    let mut m = GameMenu::default();
    m.open(Page::NewGame, Vec::new(), None, |_| None);
    let start = button(&m, Action::Start);
    let o = press(&mut m, &[Input::Click(Target::Row(start), 0)]);
    assert!(o.lines.is_empty() && m.open);
    // The empty map box doesn't open a list.
    press(&mut m, &[Input::Click(Target::Row(0), 0)]);
    assert_eq!((m.page, m.combo), (Page::NewGame, None));
}

#[test]
fn options_ok_cancel_and_apply() {
    let mut m = menu();
    click(&mut m, MainItem::Options);
    assert_eq!((m.page, m.tab), (Page::Settings, Tab::Keyboard));
    let apply = button(&m, Action::Apply);
    assert!(matches!(m.rows()[apply], Row::Button { enabled: false, .. }), "nothing to apply yet");
    press(&mut m, &[Input::NextTab(1)]);
    assert_eq!(m.tab, Tab::Mouse);
    // Reverse mouse, then the sensitivity slider pressed at its end and
    // dragged: once per change.
    let reverse = row_of(&m, "m_pitch");
    let o = press(&mut m, &[Input::Click(Target::Row(reverse), 0)]);
    assert_eq!(o.lines, ["m_pitch -0.022"]);
    let slider = row_of(&m, "sensitivity");
    let o = press(&mut m, &[Input::Slide(slider, 1.0), Input::Slide(slider, 1.0), Input::Slide(slider, 0.0)]);
    assert_eq!(o.lines, ["sensitivity 20.0", "sensitivity 0.1"]);
    // Focused, Right steps it.
    assert_eq!(press(&mut m, &[Input::Right]).lines, ["sensitivity 0.2"]);
    // Apply keeps them; Cancel puts back only what changed after.
    let apply = button(&m, Action::Apply);
    assert!(matches!(m.rows()[apply], Row::Button { enabled: true, .. }));
    press(&mut m, &[Input::Click(Target::Row(apply), 0)]);
    assert!(matches!(m.rows()[apply], Row::Button { enabled: false, .. }), "applied");
    assert_eq!(press(&mut m, &[Input::FocusRow(slider), Input::Right]).lines, ["sensitivity 0.3"]);
    let o = click_on(&mut m, |m| button(m, Action::Cancel));
    assert_eq!(o.lines, ["sensitivity 0.2"]);
    assert_eq!(m.page, Page::Main);
    // OK keeps; Esc is Cancel.
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(1), 0)]);
    let reverse = row_of(&m, "m_pitch");
    press(&mut m, &[Input::Click(Target::Row(reverse), 0)]);
    assert!(click_on(&mut m, |m| button(m, Action::Ok)).lines.is_empty());
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(1), 0), Input::Click(Target::Row(reverse), 0)]);
    assert_eq!(press(&mut m, &[Input::Close]).lines, ["m_pitch 0.022"], "back to what OK kept");
    // The close box is Cancel too.
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(1), 0), Input::Click(Target::Row(reverse), 0)]);
    assert_eq!(press(&mut m, &[Input::Click(Target::Close, 0)]).lines, ["m_pitch 0.022"]);
    assert!(m.open && m.in_game, "the menu stays");
}

#[test]
fn video_options_and_its_advanced_dialog() {
    let mut m = menu();
    m.resolutions = vec!["1920x1080".into(), "1280x720".into()];
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(3), 0)]);
    assert_eq!(m.tab, Tab::Video);
    // The resolution's drop-down lists the window sizes.
    let res = row_of(&m, "mashup_resolution");
    press(&mut m, &[Input::Click(Target::Row(res), 0)]);
    assert_eq!(m.combo.map(|c| c.len), Some(2));
    let o = press(&mut m, &[Input::Click(Target::ComboItem(1), 0)]);
    assert_eq!(o.lines, ["mashup_resolution 1280x720"]);
    // Advanced...: vsync as a drop-down, antialiasing; Cancel puts them
    // back, the options stay open.
    let adv = button(&m, Action::VideoAdvanced);
    press(&mut m, &[Input::Click(Target::Row(adv), 0)]);
    assert_eq!(m.page, Page::VideoAdvanced);
    let aa = row_of(&m, "mat_antialias");
    assert!(matches!(control(&m, aa), Control::Combo { selected: Some(2), .. }));
    let o = press(&mut m, &[Input::Click(Target::Row(aa), 0), Input::Up, Input::Up, Input::Activate]);
    assert_eq!(o.lines, ["mat_antialias 0"]);
    let o = click_on(&mut m, |m| button(m, Action::Cancel));
    assert_eq!(o.lines, ["mat_antialias 4"]);
    assert_eq!((m.page, m.focus), (Page::Settings, adv));
    // Show FPS isn't CS:S's: in our own dialog.
    assert!(m.rows().iter().all(|r| r.field() != Some(Field::Setting(setting("cl_showfps")))));
    click(&mut m, MainItem::Extras);
    assert_eq!(m.page, Page::Extras);
    assert!(m.rows().iter().any(|r| r.field() == Some(Field::Setting(setting("cl_showfps")))));
}

#[test]
fn the_sensitivity_text_entry_beside_its_slider() {
    // With the game's mouse layout: the slider and the text entry.
    let mut layout = UiLayout::default();
    for (name, kind) in [("ReverseMouse", "CCvarNegateCheckButton"), ("Slider", "CCvarSlider"), ("SensitivityLabel", "TextEntry")] {
        layout
            .controls
            .push(UiControl::new(name, UiKind::Other(kind.into()), 0.0, 0.0, 100.0, 24.0));
    }
    let ui = GameUi {
        options: HashMap::from([("mouse".to_string(), layout)]),
        ..default()
    };
    let mut m = menu();
    m.set_ui(Some(Arc::new(ui)));
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(1), 0)]);
    let entry = row_with(&m, Field::SettingText(setting("sensitivity")));
    assert!(matches!(control(&m, entry), Control::Text { text, .. } if text == "3.0"));
    // A click places the caret; the text typed sets the cvar once it
    // reads as a number in range.
    press(&mut m, &[Input::Click(Target::Row(entry), 0), Input::Caret(entry, 3), Input::Edit(Edit::Home(true))]);
    assert_eq!(m.caret, Caret { at: 0, anchor: 3 });
    let o = press(&mut m, &[Input::Copy]);
    assert_eq!(o.clipboard.as_deref(), Some("3.0"));
    let o = press(&mut m, &[Input::Type("2".into()), Input::Type("5".into())]);
    assert_eq!(o.lines, ["sensitivity 2"], "25 is out of range");
    assert!(matches!(control(&m, entry), Control::Text { text, .. } if text == "25"), "as typed");
    assert_eq!(m.values[setting("sensitivity")].as_deref(), Some("2"));
    // Without the layout no entry: the slider shows the number.
    let mut m = menu();
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(1), 0)]);
    assert!(m.rows().iter().all(|r| !matches!(r.field(), Some(Field::SettingText(_)))));
}

fn keyboard() -> GameMenu {
    let mut m = menu();
    let mut b = BTreeMap::new();
    binds::bind_defaults(&mut b, true);
    m.set_binds(b, BTreeSet::from(["bugreport".to_string()]));
    click(&mut m, MainItem::Options);
    m
}

fn bind_row(m: &GameMenu, command: &str) -> usize {
    m.rows()
        .iter()
        .position(|r| matches!(r, Row::Bind { command: c, .. } if c == command))
        .unwrap()
}

#[test]
fn the_keyboard_tab_lists_actions_with_their_keys() {
    let m = keyboard();
    let rows = m.rows();
    assert!(matches!(&rows[0], Row::Heading(t) if t == "Movement"));
    assert!(matches!(&rows[bind_row(&m, "+jump")], Row::Bind { keys, known: true, .. } if keys == "MWHEELDOWN, MWHEELUP, SPACE"));
    assert_eq!(m.focus, 1, "the first action, past the heading");
    assert_eq!(m.list(), (rows.len() - 7, KEY_ROWS));
}

/// Keyboard > Advanced...: CS:S's two check boxes (fast weapon switch,
/// developer console) in its order; ticking applies at once, Cancel
/// (or Esc) puts them back, OK keeps them.
#[test]
fn the_keyboard_tabs_advanced_dialog() {
    let mut m = keyboard();
    let at = button(&m, Action::Advanced);
    press(&mut m, &[Input::Click(Target::Row(at), 0)]);
    assert_eq!(m.page, Page::KeyboardAdvanced);
    let rows = m.rows();
    let labels: Vec<String> = rows.iter().map(label_of).collect();
    assert_eq!(labels, ["Fast weapon switch", "Enable developer console", "OK", "Cancel"]);
    assert!(matches!(&rows[0], Row::Control { control: Control::Check(false), .. }), "{:?}", rows[0]);
    assert_eq!(m.focus, 0);
    let o = press(&mut m, &[Input::Space]);
    assert_eq!(o.lines, ["hud_fastswitch 1"]);
    let o = press(&mut m, &[Input::Click(Target::Row(1), 0)]);
    assert_eq!(o.lines, ["con_enable 0"]);
    // Cancel: both back.
    let o = press(&mut m, &[Input::Click(Target::Row(3), 0)]);
    assert_eq!(o.lines, ["hud_fastswitch 0", "con_enable 1"]);
    assert_eq!((m.page, m.tab, m.focus), (Page::Settings, Tab::Keyboard, at));
    // Enter on the check box: OK, the default button, keeps it.
    press(&mut m, &[Input::Activate, Input::Space]);
    let o = press(&mut m, &[Input::Activate]);
    assert!(o.lines.is_empty());
    assert_eq!(m.page, Page::Settings);
    assert_eq!(m.values[setting_index("hud_fastswitch").unwrap()].as_deref(), Some("1"));
    // Esc on the dialog cancels it, leaving the options open.
    press(&mut m, &[Input::Activate, Input::Focus(1), Input::Space]);
    let o = press(&mut m, &[Input::Close]);
    assert_eq!(o.lines, ["con_enable 1"]);
    assert!(m.open && m.page == Page::Settings);
}

#[test]
fn rebinding_through_the_keyboard_tab() {
    let mut m = keyboard();
    let jump = bind_row(&m, "+jump");
    // One click selects, the next waits for a key.
    press(&mut m, &[Input::Click(Target::Row(jump), 0)]);
    assert_eq!((m.key_row, m.capture.as_deref()), (Some(jump), None));
    press(&mut m, &[Input::Click(Target::Row(jump), 0)]);
    assert_eq!(m.capture.as_deref(), Some("+jump"));
    // Keys while waiting go to the bind, not the menu.
    let o = press(&mut m, &[Input::Down, Input::BindKey("f")]);
    assert_eq!(o.lines, ["unbind mwheeldown; unbind mwheelup; unbind space; bind f +jump"]);
    assert_eq!(m.capture, None);
    assert!(matches!(&m.rows()[jump], Row::Bind { keys, .. } if keys == "F"));
    // Esc while waiting gives up, leaving the menu open.
    press(&mut m, &[Input::Activate]);
    assert!(m.capture.is_some());
    let o = press(&mut m, &[Input::Close]);
    assert!(o.lines.is_empty() && m.open && m.capture.is_none());
    // Edit key and Clear key act on the selected action.
    let duck = bind_row(&m, "+duck");
    press(&mut m, &[Input::Click(Target::Row(duck), 0)]);
    let edit = button(&m, Action::EditKey);
    let clear = edit + 1;
    let o = press(&mut m, &[Input::Click(Target::Row(clear), 0)]);
    assert_eq!(o.lines, ["unbind ctrl"]);
    press(&mut m, &[Input::Click(Target::Row(edit), 0)]);
    assert_eq!(m.capture.as_deref(), Some("+duck"));
    let o = press(&mut m, &[Input::BindKey("mouse4")]);
    assert_eq!(o.lines, ["bind mouse4 +duck"]);
    // Use Defaults puts every bind back.
    let o = press(&mut m, &[Input::Click(Target::Row(edit - 1), 0)]);
    assert_eq!(o.lines, ["binddefaults"]);
    assert_eq!(m.binds.get("space").map(String::as_str), Some("+jump"));
    assert_eq!(m.binds.get("mouse4"), None);
}

#[test]
fn the_keyboard_list_scrolls_to_the_focus() {
    let mut m = keyboard();
    for _ in 0..20 {
        press(&mut m, &[Input::Down]);
    }
    let (len, shown) = m.list();
    assert!(m.focus < len);
    assert!(m.scroll <= m.focus && m.focus < m.scroll + shown, "{} in {}+{shown}", m.focus, m.scroll);
}

#[test]
fn the_games_menu_entries_with_ours_added() {
    let item = |label: &str, command: &str| GameUiItem {
        label: label.into(),
        command: command.into(),
        in_game_only: false,
    };
    let ui = GameUi {
        menu: vec![
            item("RESUME GAME", "ResumeGame"),
            item("DISCONNECT", "Disconnect"),
            item("", ""),
            item("FIND SERVERS", "OpenServerBrowser"),
            item("CREATE SERVER", "OpenCreateMultiplayerGameDialog"),
            item("REPORT BUG", "engine bug"),
            item("OPTIONS", "OpenOptionsDialog"),
            item("QUIT", "Quit"),
        ],
        strings: HashMap::from([("gameui_gamemenu_newgame".to_string(), "NEW GAME".to_string())]),
        ..default()
    };
    let e = main_entries(Some(&ui));
    let items: Vec<MainItem> = e.iter().map(|e| e.item).collect();
    use MainItem::*;
    assert_eq!(
        items,
        [Resume, Disconnect, FindServers, NewGame, BugReport, Options, Quit, QuickStart, Greybox, Bots, Team, Extras, Console]
    );
    assert_eq!(e[3].label, "CREATE SERVER", "the game's words");
    assert!(e[2].gap && !e[2].gap_in_game_only, "the file's blank entry");
    assert!(e[0].in_game_only && e[1].in_game_only, "resume and disconnect only in a game");
    assert_eq!(e[4].label, "REPORT BUG");
    // Without the file: CS:S's entries, ours.
    assert_eq!(main_entries(None).len(), MAIN.len() + OURS.len());
    let mut m = menu();
    m.set_ui(Some(Arc::new(ui)));
    press(&mut m, &[Input::Click(Target::Main(5), 0)]);
    assert_eq!(m.page, Page::Settings);
    press(&mut m, &[Input::Back]);
    assert_eq!(m.focus, 5, "back on its entry");
}

#[test]
fn the_install_menu_file_hides_its_gap_out_of_a_game() {
    let item = |label: &str, command: &str, in_game_only: bool| GameUiItem {
        label: label.into(),
        command: command.into(),
        in_game_only,
    };
    // As CS:S's GameMenu.res: the blank entry is only for a game.
    let ui = GameUi {
        menu: vec![
            item("RESUME GAME", "ResumeGame", true),
            item("DISCONNECT", "Disconnect", true),
            item("PLAYERS", "OpenPlayerListDialog", true),
            item("", "", true),
            item("FIND SERVERS", "OpenServerBrowser", false),
            item("CREATE SERVER", "OpenCreateMultiplayerGameDialog", false),
            item("ACHIEVEMENTS", "OpenCSAchievementsDialog", false),
            item("QUIT", "Quit", false),
        ],
        ..default()
    };
    let mut m = main_menu();
    m.set_ui(Some(Arc::new(ui)));
    let e = m.entries();
    assert_eq!(e[0].item, MainItem::FindServers);
    assert!(!e[0].gap);
    m.in_game = true;
    let e = m.entries();
    assert_eq!(e[2].item, MainItem::FindServers);
    assert!(e[2].gap);
    assert!(!e.iter().any(|e| e.label == "ACHIEVEMENTS" || e.label == "PLAYERS"), "what mashup lacks is left out");
}

#[test]
fn hover_moves_focus_only_on_the_main_page() {
    let mut m = menu();
    let options = at(&m, MainItem::Options);
    press(&mut m, &[Input::Hover(Target::Main(options))]);
    assert_eq!(m.focus, options);
    press(&mut m, &[Input::Activate, Input::NextTab(1)]);
    assert_eq!(m.page, Page::Settings);
    let focus = m.focus;
    // Neither the left-hand list nor a hover takes the dialog's focus.
    press(&mut m, &[Input::Hover(Target::Main(1)), Input::Hover(Target::Row(focus + 1))]);
    assert_eq!(m.focus, focus);
}

#[test]
fn escape_opens_and_closes_and_the_mouse_follows() {
    use bevy::window::CursorGrabMode;
    let mut app = App::new();
    app.add_plugins(crate::console::ConsolePlugin)
        .init_resource::<super::super::console::ConsoleUi>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .insert_resource(GameMenu {
            in_game: true,
            ..default()
        })
        .init_resource::<AfterLoad>()
        .init_resource::<RegrabCursor>()
        .add_systems(
            Update,
            (
                keys.before(super::super::console::toggle),
                super::super::console::toggle,
                cursor,
            )
                .chain(),
        );
    app.world_mut()
        .spawn((super::super::console::ConsoleRoot, Visibility::Hidden));
    let window = app
        .world_mut()
        .spawn(CursorOptions {
            grab_mode: CursorGrabMode::Locked,
            ..default()
        })
        .id();
    let press = |app: &mut App, key: KeyCode| {
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(key);
        keys.clear();
    };
    let grabbed = |app: &App| app.world().get::<CursorOptions>(window).unwrap().grab_mode != CursorGrabMode::None;
    let open = |app: &App| app.world().resource::<GameMenu>().open;

    press(&mut app, KeyCode::Escape);
    assert!(open(&app) && !grabbed(&app), "Esc opens the menu and frees the mouse");
    press(&mut app, KeyCode::Escape);
    assert!(!open(&app) && grabbed(&app), "Esc closes it and grabs the mouse");

    // Esc in the console only closes the console.
    press(&mut app, KeyCode::Backquote);
    press(&mut app, KeyCode::Escape);
    assert!(!open(&app) && !app.world().resource::<super::super::console::ConsoleUi>().open);

    // Resume clicked: the mouse is grabbed once the button is up, so
    // the click doesn't fire.
    press(&mut app, KeyCode::Escape);
    assert!(open(&app));
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    press(&mut app, KeyCode::Enter);
    assert!(!open(&app) && !grabbed(&app));
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    app.update();
    assert!(grabbed(&app));
}

/// Rebinding in the open menu with real keys: the console's binds
/// change, and the new key then drives the action (headless).
#[test]
fn a_key_rebound_in_the_options_drives_the_action() {
    let mut app = App::new();
    app.add_plugins(crate::console::ConsolePlugin)
        .init_resource::<super::super::console::ConsoleUi>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(GameMenu {
            in_game: true,
            ..default()
        })
        .init_resource::<AfterLoad>()
        .init_resource::<RegrabCursor>()
        .add_systems(Update, keys.before(super::super::console::toggle))
        .add_systems(Update, super::super::console::toggle);
    super::super::binds::test_binds(&mut app);
    binds::commands(&mut app);
    app.world_mut()
        .spawn((super::super::console::ConsoleRoot, Visibility::Hidden));
    let tap = |app: &mut App, key: KeyCode| {
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(key);
        keys.clear();
        app.update();
    };
    // Open on the options (the keyboard tab), as `menu keyboard` does.
    app.world_mut().commands().queue(|w: &mut World| {
        open_menu(w, Page::Settings);
    });
    app.update();
    assert!(app.world().resource::<GameMenu>().open);
    // To "Jump", Enter to wait for a key, then K.
    let jump = bind_row(app.world().resource::<GameMenu>(), "+jump");
    for _ in 1..jump {
        tap(&mut app, KeyCode::ArrowDown);
    }
    assert_eq!(app.world().resource::<GameMenu>().focus, jump);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(app.world().resource::<GameMenu>().capture.as_deref(), Some("+jump"));
    tap(&mut app, KeyCode::KeyK);
    let console = app.world().resource::<Console>();
    assert_eq!(console.binds.get("k").map(String::as_str), Some("+jump"));
    assert_eq!(console.binds.get("space"), None);
    assert!(console.dirty, "saved to config.cfg on quit");
    // K now holds jump; Space doesn't.
    let mut keys = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    keys.press(KeyCode::KeyK);
    assert!(binds::pressed(&console.binds, &keys, &mouse, "+jump"));
    keys.release(KeyCode::KeyK);
    keys.press(KeyCode::Space);
    assert!(!binds::pressed(&console.binds, &keys, &mouse, "+jump"));
}

#[test]
fn bug_report_closes_first() {
    let mut m = menu();
    let o = click(&mut m, MainItem::BugReport);
    assert_eq!(o.lines, ["bugreport"]);
    assert!(o.close && !m.open);
}

#[test]
fn menuinput_words() {
    let mut m = menu();
    click(&mut m, MainItem::NewGame);
    assert_eq!(menu_input(&m, &["focus".into(), "map".into()]), Ok(Input::FocusRow(0)));
    assert_eq!(menu_input(&m, &["open".into()]), Ok(Input::Space));
    assert_eq!(menu_input(&m, &["pick".into(), "2".into()]), Ok(Input::Click(Target::ComboItem(2), 0)));
    assert!(menu_input(&m, &["focus".into(), "nothing".into()]).is_err());
    assert!(m.describe().starts_with("NewGame, focus 0"));
}

#[test]
fn create_server_offers_a_random_map() {
    let mut m = main_menu();
    m.random_seed = 8;
    click(&mut m, MainItem::NewGame);
    let map = row_with(&m, Field::Map);
    match control(&m, map) {
        Control::Combo { entries, selected, .. } => {
            assert_eq!(entries, ["< Random Map >", "cs_office", "de_dust2", "de_nuke"]);
            assert_eq!(selected, Some(1), "the first map, not random, at first");
        }
        c => panic!("{c:?}"),
    }
    press(&mut m, &[Input::Click(Target::Row(map), 0), Input::Click(Target::ComboItem(0), 0)]);
    assert!(m.new_game.random);
    assert!(matches!(control(&m, map), Control::Combo { text, selected: Some(0), .. } if text == "< Random Map >"));
    // Start plays one of the maps, picked by the seed (8 % 3: de_nuke).
    let o = click_on(&mut m, |m| button(m, Action::Start));
    assert!(o.lines.contains(&"map de_nuke".to_string()), "{:?}", o.lines);
    assert_eq!(m.loading.as_deref(), Some("de_nuke"));
    // Another seed, another map; picking a map leaves random.
    let mut m = main_menu();
    m.random_seed = 3;
    m.new_game.random = true;
    assert_eq!(m.chosen_map(), Some(0));
    m.new_game.set_map_entry(3);
    assert_eq!((m.new_game.random, m.chosen_map()), (false, Some(2)));
}

#[test]
fn water_detail_sets_both_water_cvars() {
    let mut m = GameMenu {
        in_game: true,
        ..default()
    };
    let water = |n: &str| match n {
        "r_waterforceexpensive" => Some("1".to_string()),
        "r_waterforcereflectentities" => Some("0".to_string()),
        n => get(n),
    };
    m.open(Page::Main, vec!["de_dust2".into()], None, water);
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(3), 0)]);
    let adv = button(&m, Action::VideoAdvanced);
    press(&mut m, &[Input::Click(Target::Row(adv), 0)]);
    let w = row_of(&m, "r_waterforceexpensive r_waterforcereflectentities");
    assert!(matches!(control(&m, w), Control::Combo { selected: Some(1), text, .. } if text == "Reflect world"));
    let o = press(&mut m, &[Input::Click(Target::Row(w), 0), Input::Click(Target::ComboItem(2), 0)]);
    assert_eq!(o.lines, ["r_waterforceexpensive 1", "r_waterforcereflectentities 1"], "Reflect all");
    let o = press(&mut m, &[Input::Click(Target::Row(w), 0), Input::Click(Target::ComboItem(0), 0)]);
    assert_eq!(o.lines, ["r_waterforceexpensive 0", "r_waterforcereflectentities 0"], "Simple reflections");
    // Cancel puts both back.
    let o = click_on(&mut m, |m| button(m, Action::Cancel));
    assert_eq!(o.lines, ["r_waterforceexpensive 1", "r_waterforcereflectentities 0"]);
}

#[test]
fn the_aspect_ratio_filters_the_resolutions() {
    let mut m = GameMenu {
        in_game: true,
        ..default()
    };
    let res = |n: &str| if n == "mashup_resolution" { Some("1920x1080".to_string()) } else { get(n) };
    m.resolutions = vec!["1920x1080".into(), "1680x1050".into(), "1280x1024".into(), "1280x720".into(), "1024x768".into()];
    m.open(Page::Main, vec!["de_dust2".into()], None, res);
    assert_eq!(m.aspect, Some(1), "16:9 from the size now");
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(3), 0)]);
    let r = row_of(&m, "mashup_resolution");
    assert!(matches!(control(&m, r), Control::Combo { entries, .. } if entries == ["1920x1080", "1280x720"]));
    // Normal (4:3, 5:4): its sizes, and the size goes to its largest.
    let a = row_with(&m, Field::Aspect);
    let o = press(&mut m, &[Input::Click(Target::Row(a), 0), Input::Click(Target::ComboItem(0), 0)]);
    assert_eq!(o.lines, ["mashup_resolution 1280x1024"]);
    let r = row_of(&m, "mashup_resolution");
    assert!(matches!(control(&m, r), Control::Combo { entries, .. } if entries == ["1280x1024", "1024x768"]));
    assert_eq!(aspect_of("1680x1050"), Some(2));
}

/// Multiplayer > Advanced from a script like the install's
/// `cfg/user.scr`: the cvars mashup has as controls, the rest greyed.
fn advanced_menu() -> GameMenu {
    let list = |pairs: &[(&str, &str)]| ServerSettingKind::List(pairs.iter().map(|(l, v)| (l.to_string(), v.to_string())).collect());
    let ui = GameUi {
        user_settings: vec![
            ServerSetting {
                cvar: "mp_decals".into(),
                label: "Decal limit".into(),
                kind: ServerSettingKind::Number { min: Some(0.0), max: Some(4096.0) },
                default: "512".into(),
            },
            ServerSetting {
                cvar: "cl_righthand".into(),
                label: "Weapon alignment".into(),
                kind: list(&[("Left handed", "0"), ("Right handed", "1")]),
                default: "0".into(),
            },
            ServerSetting {
                cvar: "cl_clanid".into(),
                label: "Clan Tag".into(),
                kind: list(&[("None", "0")]),
                default: "0".into(),
            },
            ServerSetting {
                cvar: "cl_c4progressbar".into(),
                label: "Defuse progress bar".into(),
                kind: ServerSettingKind::Bool,
                default: "1".into(),
            },
            ServerSetting {
                cvar: "cl_autohelp".into(),
                label: "Auto-help".into(),
                kind: ServerSettingKind::Bool,
                default: "1".into(),
            },
        ],
        ..default()
    };
    let mut m = GameMenu {
        in_game: true,
        ..default()
    };
    m.set_ui(Some(Arc::new(ui)));
    let has = |n: &str| match n {
        "mp_decals" => Some("200".to_string()),
        "cl_c4progressbar" => Some("1".to_string()),
        n => get(n),
    };
    m.open(Page::Main, vec!["de_dust2".into()], None, has);
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(5), 0)]);
    assert_eq!(m.tab, Tab::Multiplayer);
    let adv = button(&m, Action::MultiplayerAdvanced);
    press(&mut m, &[Input::Click(Target::Row(adv), 0)]);
    assert_eq!(m.page, Page::MultiplayerAdvanced);
    m
}

#[test]
fn multiplayer_advanced_lists_the_scripts_options() {
    let mut m = advanced_menu();
    let rows = m.rows();
    let labels: Vec<String> = rows.iter().map(crate::client::game_menu::label_of).collect();
    assert_eq!(labels, ["Decal limit", "Weapon alignment", "Clan Tag", "Defuse progress bar", "Auto-help", "OK", "Cancel"]);
    // Ours as controls with their values now; mashup lacks the clan tag
    // and auto-help: greyed, showing the script's defaults, not focused.
    assert_eq!(control(&m, 0), Control::Text { text: "200".into(), numeric: true, max: 8 });
    assert!(matches!(control(&m, 1), Control::Combo { text, .. } if text == "Right handed"));
    assert!(matches!(&rows[2], Row::Greyed { control: Control::Combo { text, .. }, .. } if text == "None"));
    assert!(matches!(&rows[4], Row::Greyed { control: Control::Check(true), .. }));
    assert!(!rows[2].focusable() && !rows[4].focusable());
    assert_eq!(m.list(), (5, ADVANCED_ROWS));
    // Changes wait for OK (CS:S's dialog): left-handed, fewer decals, no bar.
    let o = press(&mut m, &[Input::Click(Target::Row(1), 0), Input::Click(Target::ComboItem(0), 0)]);
    assert!(o.lines.is_empty());
    press(&mut m, &[Input::Click(Target::Row(0), 0), Input::Edit(Edit::SelectAll), Input::Type("50".into())]);
    let o = press(&mut m, &[Input::Click(Target::Row(3), 0)]);
    assert!(o.lines.is_empty());
    let o = click_on(&mut m, |m| button(m, Action::Ok));
    assert_eq!(o.lines, ["mp_decals 50", "cl_righthand 0", "cl_c4progressbar 0"]);
    assert_eq!((m.page, m.focus), (Page::Settings, button(&m, Action::MultiplayerAdvanced)));
    // Opened again: as they are now; Cancel (Esc) runs nothing.
    let adv = button(&m, Action::MultiplayerAdvanced);
    press(&mut m, &[Input::Click(Target::Row(adv), 0)]);
    assert_eq!(control(&m, 0), Control::Text { text: "50".into(), numeric: true, max: 8 });
    let o = press(&mut m, &[Input::Click(Target::Row(3), 0), Input::Close]);
    assert!(o.lines.is_empty());
    assert_eq!(m.page, Page::Settings);
    let o = press(&mut m, &[Input::Click(Target::Row(adv), 0), Input::Activate]);
    assert!(o.lines.is_empty(), "nothing changed: {:?}", o.lines);
}

#[test]
fn multiplayer_advanced_without_the_install_has_the_weapon_hand() {
    let mut m = menu();
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(5), 0)]);
    click_on(&mut m, |m| button(m, Action::MultiplayerAdvanced));
    let hand = row_with(&m, Field::UserCvar(0));
    assert!(matches!(control(&m, hand), Control::Combo { text, .. } if text == "Right handed"));
    press(&mut m, &[Input::Click(Target::Row(hand), 0), Input::Click(Target::ComboItem(0), 0)]);
    let o = click_on(&mut m, |m| button(m, Action::Ok));
    assert_eq!(o.lines, ["cl_righthand 0"]);
}

/// The three options this game's code reads (`cl_autowepswitch`,
/// `hud_centerid`, `cl_disablefreezecam`) are check boxes there, with
/// `user.scr`'s defaults, and OK sets the cvars.
#[test]
fn multiplayer_advanced_has_the_switch_names_and_freeze_cam() {
    let mut m = menu();
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(5), 0)]);
    click_on(&mut m, |m| button(m, Action::MultiplayerAdvanced));
    let rows: Vec<String> = m.rows().iter().map(crate::client::game_menu::label_of).collect();
    assert_eq!(
        rows[1..4],
        [
            "Automatically switch to picked up weapons (if more powerful)",
            "Center player names",
            "Disable freeze cam"
        ]
    );
    let switch = row_with(&m, Field::UserCvar(1));
    let names = row_with(&m, Field::UserCvar(2));
    let freeze = row_with(&m, Field::UserCvar(3));
    assert_eq!(control(&m, switch), Control::Check(true));
    assert_eq!(control(&m, names), Control::Check(true));
    assert_eq!(control(&m, freeze), Control::Check(false));
    for r in [switch, names, freeze] {
        press(&mut m, &[Input::Click(Target::Row(r), 0)]);
    }
    let o = click_on(&mut m, |m| button(m, Action::Ok));
    assert_eq!(o.lines, ["cl_autowepswitch 0", "hud_centerid 0", "cl_disablefreezecam 1"]);
}

#[test]
fn the_voice_tab_is_there_and_greyed() {
    let mut m = menu();
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(4), 0)]);
    assert_eq!(m.tab, Tab::Voice);
    // No voice chat: only the dialog's buttons.
    assert!(m.rows().iter().all(|r| matches!(r, Row::Button { .. })));
}

/// The menu with the cvars of the Audio tab's drop-downs, the texture
/// settings and the brightness.
fn av_menu() -> GameMenu {
    let mut m = GameMenu {
        in_game: true,
        ..default()
    };
    let cvars = |n: &str| match n {
        "snd_surround_speakers" => Some("2".to_string()),
        "snd_pitchquality" => Some("1".to_string()),
        "dsp_slow_cpu" => Some("0".to_string()),
        "closecaption" | "cc_subtitles" => Some("0".to_string()),
        "mashup_spoken_language" => Some("english".to_string()),
        "mat_picmip" => Some("0".to_string()),
        "mat_trilinear" => Some("0".to_string()),
        "mat_forceaniso" => Some("1".to_string()),
        "mat_monitorgamma" => Some("2.2".to_string()),
        n => get(n),
    };
    m.open(Page::Main, vec!["de_dust2".into()], None, cvars);
    m
}

/// The Audio tab's speaker, quality, captioning and language drop-downs
/// list CS:S's entries and set its cvars (quality and captioning two
/// each).
#[test]
fn the_audio_tabs_drop_downs_have_their_entries() {
    let mut m = av_menu();
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(2), 0)]);
    assert_eq!(m.tab, Tab::Audio);
    let entries = |m: &GameMenu, cvar: &str| match control(m, row_of(m, cvar)) {
        Control::Combo { entries, selected, .. } => (entries, selected),
        c => panic!("{c:?}"),
    };
    assert_eq!(
        entries(&m, "snd_surround_speakers"),
        (
            ["Headphones", "2 Speakers", "4 Speakers", "5.1 Speakers", "7.1 Speakers"].map(String::from).to_vec(),
            Some(1)
        )
    );
    assert_eq!(entries(&m, "snd_pitchquality dsp_slow_cpu"), (["Low", "Medium", "High"].map(String::from).to_vec(), Some(2)));
    assert_eq!(
        entries(&m, "closecaption cc_subtitles"),
        (
            ["No captions", "Subtitles (dialog only)", "Closed Captions"].map(String::from).to_vec(),
            Some(0)
        )
    );
    assert_eq!(entries(&m, "mashup_spoken_language"), (vec!["English".to_string()], Some(0)));
    let speakers = row_of(&m, "snd_surround_speakers");
    let o = press(&mut m, &[Input::Click(Target::Row(speakers), 0), Input::Click(Target::ComboItem(0), 0)]);
    assert_eq!(o.lines, ["snd_surround_speakers 0"], "headphones");
    let quality = row_of(&m, "snd_pitchquality dsp_slow_cpu");
    let o = press(&mut m, &[Input::Click(Target::Row(quality), 0), Input::Click(Target::ComboItem(0), 0)]);
    assert_eq!(o.lines, ["snd_pitchquality 0", "dsp_slow_cpu 1"], "low");
    let captions = row_of(&m, "closecaption cc_subtitles");
    let o = press(&mut m, &[Input::Click(Target::Row(captions), 0), Input::Click(Target::ComboItem(1), 0)]);
    assert_eq!(o.lines, ["closecaption 1", "cc_subtitles 1"], "subtitles");
    let o = click_on(&mut m, |m| button(m, Action::Cancel));
    assert_eq!(
        o.lines,
        ["snd_surround_speakers 2", "snd_pitchquality 1", "dsp_slow_cpu 0", "closecaption 0", "cc_subtitles 0"],
        "Cancel puts them back"
    );
}

/// Video > Advanced's texture detail and filtering drop-downs set CS:S's
/// cvars.
#[test]
fn texture_detail_and_filtering_set_their_cvars() {
    let mut m = av_menu();
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(3), 0)]);
    click_on(&mut m, |m| button(m, Action::VideoAdvanced));
    let detail = row_of(&m, "mat_picmip");
    assert!(matches!(control(&m, detail), Control::Combo { selected: Some(2), text, .. } if text == "High"));
    let o = press(&mut m, &[Input::Click(Target::Row(detail), 0), Input::Click(Target::ComboItem(0), 0)]);
    assert_eq!(o.lines, ["mat_picmip 2"], "low");
    let filter = row_of(&m, "mat_trilinear mat_forceaniso");
    assert!(matches!(control(&m, filter), Control::Combo { selected: Some(0), text, .. } if text == "Bilinear"));
    let o = press(&mut m, &[Input::Click(Target::Row(filter), 0), Input::Click(Target::ComboItem(4), 0)]);
    assert_eq!(o.lines, ["mat_trilinear 0", "mat_forceaniso 8"], "anisotropic 8x");
}

/// "Adjust brightness levels..." opens the gamma dialog over the options:
/// its slider and the entry beside it set `mat_monitorgamma`; Cancel
/// puts it back.
#[test]
fn the_brightness_dialog_sets_the_gamma() {
    let mut layout = UiLayout::default();
    for (name, kind) in [("Gamma", "CCvarSlider"), ("GammaEntry", "TextEntry"), ("OKButton", "Button"), ("Button1", "Button")] {
        layout
            .controls
            .push(UiControl::new(name, UiKind::Other(kind.into()), 0.0, 0.0, 100.0, 24.0));
    }
    let ui = GameUi {
        options: HashMap::from([("video_gamma".to_string(), layout)]),
        ..default()
    };
    let mut m = av_menu();
    m.set_ui(Some(Arc::new(ui)));
    click(&mut m, MainItem::Options);
    press(&mut m, &[Input::Click(Target::Tab(3), 0)]);
    let gamma_button = button(&m, Action::Gamma);
    assert_eq!(crate::client::game_menu::label_of(&m.rows()[gamma_button]), "Adjust brightness levels...");
    press(&mut m, &[Input::Click(Target::Row(gamma_button), 0)]);
    assert_eq!(m.page, Page::Gamma);
    let slider = row_of(&m, "mat_monitorgamma");
    assert!(matches!(control(&m, slider), Control::Slider { .. }));
    let entry = row_with(&m, Field::SettingText(setting("mat_monitorgamma")));
    assert!(matches!(control(&m, entry), Control::Text { text, .. } if text == "2.20"));
    // A step to the right: darker.
    let o = press(&mut m, &[Input::Click(Target::Row(slider), 0), Input::Right]);
    assert_eq!(o.lines, ["mat_monitorgamma 2.25"]);
    let o = click_on(&mut m, |m| button(m, Action::Cancel));
    assert_eq!(o.lines, ["mat_monitorgamma 2.2"], "as it was");
    assert_eq!((m.page, m.focus), (Page::Settings, gamma_button));
    // VR mode is left out of the video tab on purpose.
    assert!(crate::client::game_menu::draw::LEFT_OUT.contains(&"VRMode"));
}
