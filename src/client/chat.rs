//! The chat area (radio calls and game messages) and the hint text, laid
//! out like the game's own `HudChat` and `HudHintDisplay` panels when the
//! map has a HUD (`map::hud::ActiveHud`), else at CS:S's places. Anything
//! can add a line with `ChatLine` or a hint with `Hint`; lines are also
//! printed to the console.
//!
//! Text chat: Y (`messagemode`) and U (`messagemode2`) open a typing line
//! ("Say :", "Say (TEAM) :"), Enter sends it as `say` / `say_team` would,
//! Escape drops it; while it's open the keyboard types and plays nothing
//! else. Lines take the game's formats (`map::radio::SayFormats`) and
//! play its chat sound. Computer players don't chat.

use std::collections::VecDeque;

use bevy::{
    input::{
        ButtonState, InputSystems,
        keyboard::{Key, KeyboardInput},
    },
    prelude::*,
    window::CursorOptions,
};

use crate::{
    console::{Console, ConsoleAppExt, Level},
    core::{Health, LocalPlayer, Team},
    map::hud::{ActiveHud, HudCoord, HudPanel},
    map::radio::ChatColor,
};

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(GameMessagesPlugin)
            .add_message::<Hint>()
            .init_resource::<ChatLog>()
            .init_resource::<HintState>()
            .init_resource::<ChatInput>()
            .add_systems(PreUpdate, typing.after(InputSystems))
            .add_systems(Update, ((collect, draw_chat, draw_input).chain(), draw_hint));
        for (name, team, help) in [
            ("say", false, "Say something to everyone."),
            ("say_team", true, "Say something to your team."),
        ] {
            app.console_command(name, help, move |w, a| {
                say(w, &a.join(" "), team);
                Ok(None)
            });
        }
        for (name, team, help) in [
            ("messagemode", false, "Type a chat line to everyone (Y)."),
            ("messagemode2", true, "Type a chat line to your team (U)."),
        ] {
            app.console_command(name, help, move |w, _| {
                *w.resource_mut::<ChatInput>() = ChatInput {
                    open: Some(team),
                    text: String::new(),
                };
                Ok(None)
            });
        }
    }
}

/// Game messages in the chat, without a window (tests): players joining a
/// team ("Bot 2 is joining the Terrorist force", the game's own strings,
/// `map::radio::SayFormats::joins`).
pub struct GameMessagesPlugin;

impl Plugin for GameMessagesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChatLine>().add_systems(Update, team_joins);
    }
}

/// A line when a character first has a team or changes it (once the
/// map's strings are loaded).
fn team_joins(
    radio: Option<Res<crate::map::radio::RadioCommands>>,
    who: Query<(Entity, &Team, Option<&Name>, Has<LocalPlayer>), With<crate::core::Intent>>,
    mut known: Local<std::collections::HashMap<Entity, u8>>,
    mut chat: MessageWriter<ChatLine>,
) {
    let Some(radio) = radio else { return };
    let mut now = std::collections::HashMap::with_capacity(known.len());
    for (e, team, name, local) in &who {
        now.insert(e, team.0);
        if known.get(&e) == Some(&team.0) {
            continue;
        }
        let name = match (local, name) {
            (true, _) => "Player".to_string(),
            (false, Some(n)) => n.to_string(),
            (false, None) => e.to_string(),
        };
        if let Some(runs) = radio.say.join(&name, team.0) {
            chat.write(ChatLine(runs, Some(*team)));
        }
    }
    *known = now;
}

/// The chat typing line: open (to the team or not) and its text.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct ChatInput {
    /// Some(true): to the team.
    pub open: Option<bool>,
    pub text: String,
}

/// Most characters a chat line takes (Source's say limit, 127 bytes).
const MAX_SAY: usize = 127;

/// Say `text` as the local player to everyone or (`team_only`) their
/// team: a chat line in the game's format, and its sound.
pub fn say(w: &mut World, text: &str, team_only: bool) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    type Me<'a> = (
        Option<&'a Team>,
        Option<&'a Health>,
        Has<crate::rules::Dead>,
        Option<&'a GlobalTransform>,
    );
    let Some((team, alive, at)) = w
        .query_filtered::<Me, With<LocalPlayer>>()
        .iter(w)
        .next()
        .map(|(t, h, dead, at)| (t.copied(), !dead && h.is_none_or(|h| h.current > 0.0), at.map(|g| g.translation())))
    else {
        return;
    };
    let formats = w
        .get_resource::<crate::map::radio::RadioCommands>()
        .map(|r| r.say.clone())
        .unwrap_or_default();
    let place = w.get_resource::<crate::map::nav::NavMesh>().zip(at).and_then(|(nav, at)| {
        let area = nav.area_at(at)?;
        nav.places.get(nav.areas[area].place?).cloned()
    });
    let runs = formats.line("Player", text, team.map(|t| t.0), alive, team_only, place.as_deref());
    w.write_message(ChatLine(runs, team));
    if let Some(s) = formats.sound {
        w.write_message(crate::map::PlaySound::ui(s));
    }
}

/// The typing line: Y / U open it while playing; then the keyboard types
/// into it, Enter sends, Escape drops it, and the game sees no keys.
#[allow(clippy::too_many_arguments)]
fn typing(
    mut input: ResMut<ChatInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut events: MessageReader<KeyboardInput>,
    cursor: Query<&CursorOptions>,
    console: Option<Res<super::console::ConsoleUi>>,
    menu: Option<Res<super::game_menu::GameMenu>>,
    mut commands: Commands,
) {
    let busy = console.is_some_and(|c| c.open) || menu.is_some_and(|m| m.open);
    if input.open.is_none() {
        let playing = cursor.iter().next().is_some_and(super::input::cursor_grabbed);
        if !busy && playing && (keys.just_pressed(KeyCode::KeyY) || keys.just_pressed(KeyCode::KeyU)) {
            input.open = Some(keys.just_pressed(KeyCode::KeyU));
            input.text.clear();
            keys.reset_all();
        }
        // This frame's keys aren't typed (the Y or U itself).
        events.clear();
        return;
    }
    if busy {
        events.clear();
        return;
    }
    for e in events.read() {
        if e.state != ButtonState::Pressed {
            continue;
        }
        match (&e.logical_key, e.key_code) {
            (_, KeyCode::Enter | KeyCode::NumpadEnter) => {
                let text = std::mem::take(&mut input.text);
                let team = input.open.take() == Some(true);
                commands.queue(move |w: &mut World| say(w, &text, team));
                break;
            }
            (_, KeyCode::Escape) => {
                *input = ChatInput::default();
                break;
            }
            (Key::Backspace, _) => {
                input.text.pop();
            }
            (Key::Space, _) => type_text(&mut input.text, " "),
            (Key::Character(s), _) => type_text(&mut input.text, s),
            _ => {}
        }
    }
    keys.reset_all();
}

/// Add typed text, without control characters, up to `MAX_SAY` bytes.
fn type_text(line: &mut String, s: &str) {
    for c in s.chars().filter(|c| !c.is_control()) {
        if line.len() + c.len_utf8() > MAX_SAY {
            return;
        }
        line.push(c);
    }
}

#[derive(Component)]
struct InputText;

/// The typing line under the chat lines: "Say :" or "Say (TEAM) :" (the
/// engine's `chat_say` strings) and the text with a cursor.
fn draw_input(
    input: Res<ChatInput>,
    hud: Option<Res<ActiveHud>>,
    windows: Query<&Window>,
    shown: Query<Entity, With<InputText>>,
    mut built: Local<Option<(ChatInput, f32)>>,
    mut commands: Commands,
) {
    let Some(window) = windows.iter().next() else { return };
    let (w, h) = (window.width(), window.height());
    let key = (input.clone(), h);
    if built.as_ref() == Some(&key) {
        return;
    }
    *built = Some(key);
    for e in &shown {
        commands.entity(e).despawn();
    }
    let Some(team) = input.open else { return };
    let scale = h / 480.0;
    let p = panel(
        hud.as_deref(),
        "HudChat",
        (HudCoord::Start(10.0), HudCoord::Start(275.0), 320.0, 120.0),
    );
    let size = FontSize::Px(12.0 * scale * 0.85);
    let root = commands
        .spawn((
            InputText,
            Text::default(),
            TextFont {
                font_size: size,
                ..default()
            },
            TextShadow::default(),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            Node {
                position_type: PositionType::Absolute,
                left: px(p.x.resolve(w, scale)),
                top: px(p.y.resolve(h, scale) + p.tall * scale + 2.0 * scale),
                width: px(p.wide * scale),
                padding: UiRect::axes(px(3.0 * scale), px(1.0 * scale)),
                ..default()
            },
            GlobalZIndex(42),
        ))
        .id();
    let prompt = if team { "Say (TEAM) :" } else { "Say :" };
    commands.spawn((
        TextSpan::new(format!("{prompt} ")),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(Color::srgb_u8(255, 176, 0)),
        ChildOf(root),
    ));
    commands.spawn((
        TextSpan::new(format!("{}_", input.text)),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(Color::WHITE),
        ChildOf(root),
    ));
}

/// A chat line: coloured runs of text.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct ChatLine(pub Vec<(ChatColor, String)>, pub Option<crate::core::Team>);

impl ChatLine {
    /// A line in the normal colour.
    pub fn plain(text: impl Into<String>) -> Self {
        Self(vec![(ChatColor::Normal, text.into())], None)
    }
}

/// A hint across the screen (`HudHintDisplay`), replacing the one shown.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct Hint(pub String);

/// Seconds a chat line stays (Source's `hud_saytext_time` default).
pub const CHAT_SECONDS: f32 = 12.0;
/// Seconds a hint stays (a guess; docs/tech-debt.md).
pub const HINT_SECONDS: f32 = 4.0;
/// Lines kept at most.
const MAX_LINES: usize = 8;

/// Chat lines shown: runs, the sender's team, when they go.
#[derive(Resource, Default)]
pub struct ChatLog {
    pub lines: VecDeque<(ChatLine, f32)>,
    version: u64,
}

impl ChatLog {
    /// Add a line at `now`; the oldest goes past `MAX_LINES`.
    pub fn push(&mut self, line: ChatLine, now: f32) {
        self.lines.push_back((line, now + CHAT_SECONDS));
        while self.lines.len() > MAX_LINES {
            self.lines.pop_front();
        }
        self.version += 1;
    }

    pub fn expire(&mut self, now: f32) {
        let before = self.lines.len();
        self.lines.retain(|(_, until)| *until > now);
        if self.lines.len() != before {
            self.version += 1;
        }
    }
}

#[derive(Resource, Default)]
struct HintState {
    text: Option<(String, f32)>,
    version: u64,
}

fn collect(
    mut lines: MessageReader<ChatLine>,
    mut log: ResMut<ChatLog>,
    console: Option<ResMut<Console>>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    let mut console = console;
    for l in lines.read() {
        if let Some(c) = console.as_mut() {
            c.print(Level::Info, l.0.iter().map(|(_, s)| s.as_str()).collect::<String>());
        }
        log.push(l.clone(), now);
    }
    log.expire(now);
}

/// A panel from the HUD, or CS:S's numbers for it.
fn panel(hud: Option<&ActiveHud>, name: &str, fallback: (HudCoord, HudCoord, f32, f32)) -> HudPanel {
    hud.and_then(|h| h.0.panels.get(name).cloned()).unwrap_or(HudPanel {
        x: fallback.0,
        y: fallback.1,
        wide: fallback.2,
        tall: fallback.3,
        background: None,
        icon: Vec2::ZERO,
        digit: Vec2::ZERO,
        digit2: Vec2::ZERO,
        keys: Default::default(),
    })
}

/// A run's colour: team colours from the HUD's scheme; the normal and
/// location colours are guesses (docs/tech-debt.md).
pub fn run_color(hud: Option<&ActiveHud>, c: ChatColor, team: Option<crate::core::Team>) -> Color {
    let named = |n: &str| hud.and_then(|h| h.0.color(n));
    match c {
        ChatColor::Normal => Color::srgb_u8(255, 255, 255),
        ChatColor::Location => Color::srgb_u8(153, 255, 153),
        ChatColor::Team => match team.map(|t| t.0) {
            Some(1) => named("T_Red").unwrap_or(Color::srgb_u8(255, 64, 64)),
            Some(2) => named("CT_Blue").unwrap_or(Color::srgb_u8(153, 204, 255)),
            _ => named("team0").unwrap_or(Color::srgb_u8(204, 204, 204)),
        },
    }
}

#[derive(Component)]
struct ChatRoot;

fn draw_chat(
    log: Res<ChatLog>,
    hud: Option<Res<ActiveHud>>,
    windows: Query<&Window>,
    root: Query<Entity, With<ChatRoot>>,
    mut built: Local<(u64, f32)>,
    mut commands: Commands,
) {
    let Some(window) = windows.iter().next() else { return };
    let (w, h) = (window.width(), window.height());
    let scale = h / 480.0;
    if *built == (log.version, h) && (!root.is_empty() || log.lines.is_empty()) {
        return;
    }
    *built = (log.version, h);
    for e in &root {
        commands.entity(e).despawn();
    }
    if log.lines.is_empty() {
        return;
    }
    let hud = hud.as_deref();
    let p = panel(
        hud,
        "HudChat",
        (HudCoord::Start(10.0), HudCoord::Start(275.0), 320.0, 120.0),
    );
    let root = commands
        .spawn((
            ChatRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(p.x.resolve(w, scale)),
                top: px(p.y.resolve(h, scale)),
                width: px(p.wide * scale),
                height: px(p.tall * scale),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexEnd,
                overflow: Overflow::clip(),
                ..default()
            },
            GlobalZIndex(41),
        ))
        .id();
    // ChatFont: 12 at 480 lines.
    let size = 12.0 * scale * 0.85;
    for (line, _) in &log.lines {
        let text = commands
            .spawn((
                Text::default(),
                TextFont {
                    font_size: FontSize::Px(size),
                    ..default()
                },
                TextShadow::default(),
                ChildOf(root),
            ))
            .id();
        for (c, s) in &line.0 {
            commands.spawn((
                TextSpan::new(s.clone()),
                TextFont {
                    font_size: FontSize::Px(size),
                    ..default()
                },
                TextColor(run_color(hud, *c, line.1)),
                ChildOf(text),
            ));
        }
    }
}

#[derive(Component)]
struct HintText;

fn draw_hint(
    mut hints: MessageReader<Hint>,
    mut state: ResMut<HintState>,
    hud: Option<Res<ActiveHud>>,
    windows: Query<&Window>,
    shown: Query<Entity, With<HintText>>,
    time: Res<Time>,
    mut built: Local<(u64, f32)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for h in hints.read() {
        state.text = Some((h.0.clone(), now + HINT_SECONDS));
        state.version += 1;
    }
    if state.text.as_ref().is_some_and(|(_, until)| now > *until) {
        state.text = None;
        state.version += 1;
    }
    let Some(window) = windows.iter().next() else { return };
    let (w, h) = (window.width(), window.height());
    if *built == (state.version, h) {
        return;
    }
    *built = (state.version, h);
    for e in &shown {
        commands.entity(e).despawn();
    }
    let Some((text, _)) = &state.text else { return };
    let scale = h / 480.0;
    let hud = hud.as_deref();
    let p = panel(
        hud,
        "HudHintDisplay",
        (HudCoord::Centre(-240.0), HudCoord::Centre(60.0), 480.0, 100.0),
    );
    let fg = hud.and_then(|h| h.0.color("HintMessageFg")).unwrap_or(Color::WHITE);
    let bg = hud
        .and_then(|h| h.0.color("HintMessageBg"))
        .unwrap_or(Color::srgba_u8(0, 0, 0, 60));
    let inset = p.num("text_xpos").unwrap_or(8.0) * scale;
    // The panel's box; the text centred, on its bottom ("center_y" -1).
    let root = commands
        .spawn((
            HintText,
            Node {
                position_type: PositionType::Absolute,
                left: px(p.x.resolve(w, scale)),
                top: px(p.y.resolve(h, scale)),
                width: px(p.wide * scale),
                height: px(p.tall * scale),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexEnd,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(45),
        ))
        .id();
    commands.spawn((
        Text::new(text.clone()),
        // HudHintText: 12 at 480 lines.
        TextFont {
            font_size: FontSize::Px(12.0 * scale * 0.85),
            ..default()
        },
        TextColor(fg),
        TextLayout::justify(Justify::Center),
        BackgroundColor(bg),
        Node {
            padding: UiRect::all(px(inset)),
            border_radius: BorderRadius::all(px(4.0 * scale)),
            ..default()
        },
        ChildOf(root),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_expire_and_overflow() {
        let mut log = ChatLog::default();
        for i in 0..10 {
            log.push(ChatLine::plain(format!("{i}")), i as f32);
        }
        assert_eq!(log.lines.len(), MAX_LINES);
        assert_eq!(log.lines[0].0, ChatLine::plain("2"));
        log.expire(2.0 + CHAT_SECONDS + 0.5);
        assert_eq!(log.lines.front().map(|l| &l.0), Some(&ChatLine::plain("3")));
    }

    #[test]
    fn typed_text_has_no_control_characters_and_a_limit() {
        let mut line = String::new();
        type_text(&mut line, "hi\u{8}\t!");
        assert_eq!(line, "hi!");
        type_text(&mut line, &"x".repeat(200));
        assert_eq!(line.len(), MAX_SAY);
    }

    #[test]
    fn team_runs_take_the_scheme_colours() {
        use crate::core::Team;
        assert_eq!(
            run_color(None, ChatColor::Team, Some(Team(1))),
            Color::srgb_u8(255, 64, 64)
        );
        assert_eq!(
            run_color(None, ChatColor::Team, Some(Team(2))),
            Color::srgb_u8(153, 204, 255)
        );
    }
}
