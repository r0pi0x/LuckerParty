//! The menus' dialogs driven by scripted input as a player would drive
//! them (VGUI's controls, `client::widgets`): Create Server through its
//! drop-down, check box, text entry and radio buttons; Find Servers'
//! filters through the latency drop-down, the map text entry (caret,
//! selection, copy) and Tab; frames dragged, stacked and resized. The
//! models run without a window; their console lines are what the game
//! runs.

use std::net::SocketAddr;

use bevy::prelude::*;
use mashup::{
    client::{
        game_menu::{Action, Control, Field, GameMenu, Input, MainItem, Page, Row, Target},
        server_browser::{self, ButtonId, Check, Focus, ServerBrowser},
        widgets::{Caret, Edit, Windows, edges_at},
    },
    net::{
        NET_VERSION,
        query::{QueryResult, QueryState, ServerInfo},
    },
};

fn run(m: &mut GameMenu, inputs: &[Input]) -> (Vec<String>, Vec<String>) {
    let (mut lines, mut after) = (Vec::new(), Vec::new());
    for i in inputs {
        let o = m.handle(i.clone());
        lines.extend(o.lines);
        after.extend(o.after_load);
    }
    (lines, after)
}

fn row(m: &GameMenu, f: Field) -> usize {
    m.rows()
        .iter()
        .position(|r| matches!(r, Row::Control { field, .. } if *field == f))
        .unwrap_or_else(|| panic!("no {f:?} on {:?}", m.page))
}

#[test]
fn create_server_by_keyboard_and_mouse() {
    let mut m = GameMenu::default();
    let maps = ["cs_italy", "de_dust2", "de_inferno", "de_nuke"]
        .map(String::from)
        .to_vec();
    m.open(Page::Main, maps, None, |cvar| {
        (cvar == "mashup_rounds").then(|| "0".into())
    });
    let create = m.entries().iter().position(|e| e.item == MainItem::NewGame).unwrap();
    run(&mut m, &[Input::Click(Target::Main(create), 0)]);
    assert_eq!(m.page, Page::NewGame);

    // The map: focused, Space opens its list, Down twice, Enter picks.
    let map = row(&m, Field::Map);
    run(&mut m, &[Input::FocusRow(map), Input::Space]);
    assert!(m.combo.is_some(), "the drop-down is open");
    run(&mut m, &[Input::Down, Input::Down, Input::Activate]);
    assert!(m.combo.is_none());
    assert_eq!(m.new_game.map, 2, "de_inferno");
    // Typed letters jump in a closed one too.
    run(&mut m, &[Input::Char('d')]);
    assert_eq!(m.new_game.map, 3);
    run(&mut m, &[Input::Up]);
    assert_eq!(m.new_game.map, 2, "Up steps it back");

    // Bots: the check box, Tab to the count, typed; a difficulty clicked.
    let bots = row(&m, Field::BotsOn);
    run(
        &mut m,
        &[
            Input::Click(Target::Row(bots), 0),
            Input::Focus(1),
            Input::Type("6".into()),
        ],
    );
    assert_eq!(m.new_game.bot_count, "6");
    let expert = row(&m, Field::Difficulty(3));
    run(&mut m, &[Input::Click(Target::Row(expert), 0)]);
    assert!(matches!(
        &m.rows()[expert],
        Row::Control {
            control: Control::Radio(true),
            ..
        }
    ));

    // Enter anywhere but a button: Start, the default button.
    run(&mut m, &[Input::FocusRow(map)]);
    let (lines, after) = run(&mut m, &[Input::Activate]);
    assert_eq!(lines.last().map(String::as_str), Some("map de_inferno"));
    assert!(lines.contains(&"bot_reaction 0.12".to_string()), "expert: {lines:?}");
    assert_eq!(after.iter().filter(|l| *l == "bot_add 1").count(), 3);
    assert_eq!(after.iter().filter(|l| *l == "bot_add 2").count(), 3);
    assert_eq!(m.loading.as_deref(), Some("de_inferno"));
}

#[test]
fn create_server_cancel_and_escape() {
    let mut m = GameMenu::default();
    m.open(Page::NewGame, vec!["de_dust2".into()], None, |_| None);
    // Esc with a list open closes only the list; again, the dialog.
    let map = row(&m, Field::Map);
    run(&mut m, &[Input::FocusRow(map), Input::Space, Input::Close]);
    assert_eq!((m.page, m.combo), (Page::NewGame, None));
    run(&mut m, &[Input::Close]);
    assert_eq!(m.page, Page::Main);
    // Its close box too.
    m.open(Page::NewGame, vec!["de_dust2".into()], None, |_| None);
    let (lines, _) = run(&mut m, &[Input::Click(Target::Close, 0)]);
    assert!(lines.is_empty());
    assert_eq!(m.page, Page::Main);
    // Cancel is a button in the tab order.
    m.open(Page::NewGame, vec!["de_dust2".into()], None, |_| None);
    let cancel = m
        .rows()
        .iter()
        .position(|r| {
            matches!(
                r,
                Row::Button {
                    action: Action::Cancel,
                    ..
                }
            )
        })
        .unwrap();
    for _ in 0..10 {
        if m.focus == cancel {
            break;
        }
        run(&mut m, &[Input::Focus(1)]);
    }
    assert_eq!(m.focus, cancel);
    run(&mut m, &[Input::Space]);
    assert_eq!(m.page, Page::Main);
}

fn lan_server(port: u16, name: &str, map: &str, ping: u32) -> QueryResult {
    QueryResult {
        addr: SocketAddr::from(([10, 0, 0, 2], port)),
        state: QueryState::Answered {
            info: ServerInfo {
                name: name.into(),
                map: map.into(),
                players: 2,
                max_players: 8,
                version: NET_VERSION.into(),
                ..default()
            },
            ping_ms: ping,
        },
        lan: true,
    }
}

#[test]
fn server_browser_filters_by_their_controls() {
    let mut b = ServerBrowser::default();
    b.open(Some(server_browser::Tab::Lan));
    let results = vec![
        lan_server(27071, "near dust", "de_dust2", 30),
        lan_server(27072, "far dust", "de_dust", 300),
        lan_server(27073, "office", "cs_office", 20),
    ];
    b.set_results(&results, false);
    assert_eq!(b.rows.len(), 3);
    b.handle(server_browser::Input::Click(server_browser::Target::Button(
        ButtonId::Filters,
    )));
    assert!(b.filters.shown);

    // Latency: the drop-down opens on <All>, the pointer highlights, a
    // click picks "< 100".
    b.handle(server_browser::Input::Click(server_browser::Target::Latency));
    assert_eq!(b.combo.map(|c| c.highlight), Some(0));
    b.handle(server_browser::Input::Hover(server_browser::Target::ComboItem(1)));
    b.handle(server_browser::Input::Down);
    b.handle(server_browser::Input::Enter);
    assert_eq!((b.filters.latency, b.combo), (2, None));
    // Focused, Up steps it back without opening it.
    b.handle(server_browser::Input::Up);
    assert_eq!(b.filters.latency, 1);
    b.handle(server_browser::Input::Down);

    // The map entry: Tab reaches it from the buttons; typed, a word
    // selected and copied.
    b.focus = Some(Focus::Button(ButtonId::Filters));
    b.handle(server_browser::Input::Tab(1));
    assert_eq!(b.typing, Some(server_browser::Field::MapFilter));
    b.handle(server_browser::Input::Type("dust".into()));
    let out = b.handle(server_browser::Input::Edit(Edit::Home(true)));
    assert!(out.lines.is_empty());
    assert_eq!(b.caret, Caret { at: 0, anchor: 4 });
    let out = b.handle(server_browser::Input::Copy);
    assert_eq!(out.clipboard.as_deref(), Some("dust"));
    // A check box by Space once focused by Tab.
    for _ in 0..6 {
        if b.focus == Some(Focus::Check(Check::NotFull)) {
            break;
        }
        b.handle(server_browser::Input::Tab(1));
    }
    assert_eq!(b.focus, Some(Focus::Check(Check::NotFull)));
    b.handle(server_browser::Input::Space);
    assert!(b.filters.not_full);

    b.set_results(&results, false);
    let names: Vec<&str> = b.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["near dust"], "dust maps under 100 ms");
    assert_eq!(b.filter_text(), "map dust; latency < 100; is not full");
}

#[test]
fn frames_drag_stack_and_resize() {
    let screen = Vec2::new(1280.0, 720.0);
    let mut w = Windows::default();
    let browser = w.place(server_browser::WINDOW, Vec2::new(640.0, 384.0), screen);
    assert_eq!(browser, Vec2::new(320.0, 168.0), "centred the first time");
    w.place("options", Vec2::new(512.0, 406.0), screen);
    assert_eq!(w.front(&[server_browser::WINDOW, "options"]), Some("options"));
    // A drag by the browser's title bar brings it in front and moves it,
    // never off the screen.
    w.begin(server_browser::WINDOW, None, Vec2::new(400.0, 180.0), 1.0, Vec2::ZERO);
    assert_eq!(
        w.front(&[server_browser::WINDOW, "options"]),
        Some(server_browser::WINDOW)
    );
    w.drag_to(Vec2::new(-500.0, 180.0), screen);
    w.end_drag();
    assert_eq!(w.placed(server_browser::WINDOW).unwrap().pos, Vec2::new(0.0, 168.0));
    // Its corner: resized, not under its minimum.
    let p = w.placed(server_browser::WINDOW).unwrap();
    let corner = p.pos + p.size - Vec2::splat(3.0);
    let edges = edges_at(p, corner, 5.0, 18.0);
    w.begin(
        server_browser::WINDOW,
        Some(edges),
        corner,
        1.0,
        server_browser::MIN_SIZE,
    );
    w.drag_to(corner + Vec2::new(120.0, 100.0), screen);
    w.end_drag();
    assert_eq!(w.size(server_browser::WINDOW), Some(Vec2::new(760.0, 484.0)));
    // Closed and opened again in the session: where it was left.
    assert_eq!(
        w.place(server_browser::WINDOW, Vec2::new(760.0, 484.0), screen),
        Vec2::new(0.0, 168.0)
    );
}
