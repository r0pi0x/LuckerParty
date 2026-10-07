//! The chat area (radio calls and game messages) and the hint text, laid
//! out like the game's own `HudChat` and `HudHintDisplay` panels when the
//! map has a HUD (`map::hud::ActiveHud`), else at CS:S's places. Anything
//! can add a line with `ChatLine` or a hint with `Hint`; lines are also
//! printed to the console.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::{
    console::{Console, Level},
    map::hud::{ActiveHud, HudCoord, HudPanel},
    map::radio::ChatColor,
};

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChatLine>()
            .add_message::<Hint>()
            .init_resource::<ChatLog>()
            .init_resource::<HintState>()
            .add_systems(Update, ((collect, draw_chat).chain(), draw_hint));
    }
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
