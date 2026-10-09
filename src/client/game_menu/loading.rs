//! The loading dialog (a map loading, a server being joined) and the
//! Disconnected dialog (why joining failed).

use super::*;

/// Joining a server, or following its map change (`client::net`): the
/// loading dialog over the main menu's background (`label`: the address,
/// then the map), as the game shows it, until the map is in
/// (`entered_game`) or it fails (`show_failure`).
pub(in crate::client) fn joining(w: &mut World, label: &str) {
    let Some(mut menu) = w.get_resource_mut::<GameMenu>() else { return };
    if menu.loading.as_deref() == Some(label) && menu.open && menu.failure.is_none() {
        return;
    }
    menu.loading = Some(label.to_string());
    menu.failure = None;
    menu.in_game = false;
    menu.open = true;
    menu.page = Page::Main;
    menu.capture = None;
    menu.combo = None;
}

/// Joining (or the game) failed: the loading dialog says why, in the
/// game's words, until it's closed.
pub(in crate::client) fn show_failure(w: &mut World) {
    let Some(failure) = w
        .get_resource::<crate::net::client::JoinProgress>()
        .and_then(|p| p.failure.clone())
    else {
        return;
    };
    let Some(mut menu) = w.get_resource_mut::<GameMenu>() else { return };
    let text = net_failure_text(&menu, &failure.0, &failure.1);
    menu.failure = Some(text);
    menu.loading = Some(String::new());
    menu.open = true;
    menu.page = Page::Main;
}

/// Why joining failed, as the game words it (its GameUI strings, `%s1`
/// filled in), else the server's or our own words.
pub fn net_failure_text(menu: &GameMenu, kind: &crate::net::client::JoinFailure, reason: &str) -> String {
    use crate::net::client::JoinFailure as F;
    let game = |token: &str| menu.game_text(token).map(|t| t.trim_end().to_string());
    let text = match kind {
        F::Full => game("#GameUI_ServerRejectServerFull"),
        F::OldServer => game("#GameUI_ServerRejectOldVersion"),
        F::NewServer => game("#GameUI_ServerRejectNewVersion"),
        F::Timeout => game("#GameUI_ServerConnectionTimeout"),
        F::Download { file, error } => {
            let token = match error.as_str() {
                "File does not exist" => "#GameUI_DownloadFailedFileNotFound",
                "Connection closed by remote host" => "#GameUI_DownloadFailedConClosed",
                "Invalid URL" => "#GameUI_DownloadFailedBadURL",
                "Only HTTP is supported" => "#GameUI_DownloadFailedBadProtocol",
                "Cannot get file info from server" => "#GameUI_DownloadFailedNoHeaders",
                "File has no data" => "#GameUI_DownloadFailedZeroLen",
                e if e.starts_with("Cannot connect to server") => "#GameUI_DownloadFailedCantConnect",
                _ => "",
            };
            let generic = game("#GameUI_DownloadFailed").map(|t| format!("{}:\n{error}", t.replace("%s1", file)));
            if token.is_empty() {
                generic
            } else {
                game(token).map(|t| t.replace("%s1", file)).or(generic)
            }
        }
        F::Dropped => game("#GameUI_DisconnectedFromServerExtended").map(|t| t.replace("%s1", reason)),
        _ => None,
    };
    text.unwrap_or_else(|| reason.to_string())
}

/// A `map` load failed: the main menu takes input again.
pub(in crate::client) fn map_load_failed(w: &mut World) {
    if let Some(mut menu) = w.get_resource_mut::<GameMenu>() {
        menu.loading = None;
    }
}

/// A map the main menu started is loading: GameUI's loading dialog (its
/// layout from the install, `GameUi::loading`: the frame, the stage line,
/// the progress bar, Cancel), the menu's entries hidden. The stage and
/// the bar follow the load (`loading_progress`).
pub(super) fn loading_frame(commands: &mut Commands, root: Entity, menu: &GameMenu, look: &Look, map: &str) {
    let layout = look.ui.and_then(|u| u.loading.as_ref());
    let at = |name: &str, fallback: (f32, f32, f32, f32)| {
        layout
            .and_then(|l| l.get(name))
            .map_or(fallback, |c| (coord(c.x), coord(c.y), c.wide, c.tall))
    };
    let (_, _, w, h) = at("LoadingDialog", (0.0, 0.0, 380.0, 112.0));
    let title = menu.text("#GameUI_Loading", "Loading...");
    let f = widgets::frame(commands, root, look, FrameSpec::new("loading").no_close(), (w, h), &title).frame;
    let info = label(
        commands,
        f,
        look,
        at("InfoLabel", (20.0, 34.0, 340.0, 24.0)),
        &loading_text(menu, crate::map::loading::current(), map),
        look.font("Default", (16.0, false)),
        look.text(),
        -1,
    );
    commands.entity(info).insert(LoadingInfo(map.to_string()));
    // The bar: sunken, filled with the scheme's progress colour.
    let (x, y, bw, bh) = at("Progress", (20.0, 64.0, 260.0, 24.0));
    let bar = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                padding: UiRect::all(look.px(2.0)),
                ..place(look, x, y, bw, bh)
            },
            bevel(look, false),
            BackgroundColor(look.color("ProgressBar.BgColor", [0, 0, 0, 128])),
            ChildOf(f),
        ))
        .id();
    let fraction = crate::map::loading::current().map_or(0.0, |p| p.fraction);
    commands.spawn((
        LoadingBar,
        Node {
            width: percent(100.0 * fraction),
            height: percent(100.0),
            ..default()
        },
        BackgroundColor(look.color("ProgressBar.FgColor", [216, 222, 211, 255])),
        ChildOf(bar),
    ));
    let cancel = layout.and_then(|l| l.get("CancelButton"));
    let text = cancel.map_or_else(|| menu.text("#GameUI_Cancel", "Cancel"), |c| c.text.clone());
    widgets::button(
        commands,
        f,
        look,
        at("CancelButton", (288.0, 64.0, 72.0, 24.0)),
        &text,
        Btn::enabled(true).default_button(true),
        0,
        Hit(Target::Cancel, 0),
    );
}

/// The loading dialog's stage line: the stage the load reported, as the
/// game words it (`LoadingProgress_LoadMap`: "Loading world..."), else
/// "Loading <map> ...".
pub(super) fn loading_text(menu: &GameMenu, progress: Option<crate::map::loading::LoadProgress>, map: &str) -> String {
    match progress {
        Some(p) => menu.text(&format!("#{}", p.stage), p.stage),
        None => menu.text("#GameUI_LoadingFilename", "Loading %s1 ...").replace("%s1", map),
    }
}

/// What the loading dialog shows now: its stage line and the bar's
/// fraction. Joining a server: its stages (connecting, retrieving server
/// info, downloading the map with its progress, the map's own load), as
/// the game words them; a map loading: its stages; a server starting:
/// "Starting local game server..." until the map reports.
pub fn dialog_state(
    menu: &GameMenu,
    join: Option<&crate::net::client::JoinProgress>,
    progress: Option<crate::map::loading::LoadProgress>,
    hosting: bool,
    map: &str,
) -> (String, f32) {
    use crate::net::client::JoinStage as S;
    // Joined too: the frame before the dialog closes.
    if let Some(j) = join.filter(|j| j.stage != crate::net::client::JoinStage::Idle && j.failure.is_none()) {
        let (token, ours) = j.stage.token();
        let stage = menu.text(&format!("#{token}"), ours);
        return match j.stage {
            S::LoadingMap => match progress {
                Some(p) => (menu.text(&format!("#{}", p.stage), p.stage), p.fraction),
                None => (stage, 0.0),
            },
            S::Downloading => {
                let f = j
                    .download
                    .as_ref()
                    .filter(|d| d.total > 0)
                    .map_or(0.0, |d| (d.done as f64 / d.total as f64) as f32);
                (stage, f.clamp(0.0, 1.0))
            }
            S::Connecting => (stage, 0.02),
            S::ServerInfo | S::ChangingLevel => (stage, 0.05),
            _ => (stage, 0.08),
        };
    }
    if hosting && progress.is_none() {
        return (
            menu.text("#LoadingProgress_SpawningServer", "Starting local game server..."),
            0.0,
        );
    }
    (
        loading_text(menu, progress, map),
        progress.map_or(0.0, |p| p.fraction),
    )
}

/// `mashup_loading_details`: the loading dialog's detailed view (each
/// stage's time, bytes and percentage; the server's name, map, players
/// and ping). Off by default (the game's dialog alone).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct LoadingDetails(pub u8);

/// The stages the loading dialog went through this time, for the
/// detailed view: each one's name, when it started (real s) and its
/// latest progress text.
#[derive(Resource, Clone, Debug, Default)]
pub struct LoadTimeline {
    pub stages: Vec<(String, f64, String)>,
}

impl LoadTimeline {
    /// The detailed view's lines at `now`: a stage's time runs to the
    /// next one's start (the last to now).
    pub fn lines(&self, now: f64) -> Vec<String> {
        let mut out = Vec::new();
        for (i, (name, start, progress)) in self.stages.iter().enumerate() {
            let end = self.stages.get(i + 1).map_or(now, |s| s.1);
            out.push(format!("{name:<32} {:>7.2} s   {progress}", (end - start).max(0.0)));
        }
        out
    }
}

/// Bytes as the detailed view shows them.
pub(super) fn bytes(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", n as f64 / 1024.0)
    }
}

/// The detailed view's stage now: its name and progress text.
pub(super) fn detail_stage(
    join: Option<&crate::net::client::JoinProgress>,
    progress: Option<crate::map::loading::LoadProgress>,
) -> (String, String) {
    use crate::net::client::JoinStage as S;
    let map_stage = |p: crate::map::loading::LoadProgress| {
        (
            format!("loading the map: {}", p.stage.trim_start_matches("LoadingProgress_")),
            format!("{:.0}%", p.fraction * 100.0),
        )
    };
    match join.filter(|j| j.stage != S::Idle && j.failure.is_none()) {
        Some(j) if j.stage == S::Downloading => {
            let text = j.download.as_ref().map_or(String::new(), |d| {
                if d.total > 0 {
                    format!(
                        "{} of {} ({:.0}%) of {} from {}",
                        bytes(d.done),
                        bytes(d.total),
                        d.done as f64 * 100.0 / d.total as f64,
                        d.file,
                        d.from
                    )
                } else {
                    format!("{} of {} from {}", bytes(d.done), d.file, d.from)
                }
            });
            (j.stage.name().to_string(), text)
        }
        Some(j) if j.stage == S::LoadingMap => progress.map_or_else(|| (j.stage.name().into(), String::new()), map_stage),
        Some(j) => (j.stage.name().to_string(), String::new()),
        None => progress.map_or_else(|| ("starting".into(), String::new()), map_stage),
    }
}

/// The detailed view's text: the server, then the stages.
#[derive(Component)]
pub(super) struct LoadingDetail;

/// The detailed view under the dialog (`mashup_loading_details 1`).
pub(super) fn details_panel(commands: &mut Commands, root: Entity, look: &Look, size: Vec2) {
    let (w, h) = (560.0, 190.0);
    let x = ((size.x / look.s - w) / 2.0).max(0.0);
    let y = ((size.y / look.s) / 2.0 + 70.0).max(0.0);
    let panel = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                padding: UiRect::all(look.px(8.0)),
                ..place(look, x, y, w, h)
            },
            bevel(look, false),
            BackgroundColor(look.sunken_bg()),
            ChildOf(root),
        ))
        .id();
    commands.spawn((
        LoadingDetail,
        Text::new(""),
        look.font("DefaultFixed", (13.0, false)),
        TextColor(look.text()),
        TextLayout::new(Justify::Left, LineBreak::NoWrap),
        ChildOf(panel),
    ));
}

/// Why joining failed, in the loading dialog: "Disconnected", the reason
/// (wrapped), Close.
pub(super) fn failure_frame(commands: &mut Commands, root: Entity, menu: &GameMenu, look: &Look, text: &str) {
    let (w, h) = (380.0, 150.0);
    let title = menu.text("#GameUI_Disconnected", "Disconnected");
    let parts = widgets::frame(commands, root, look, FrameSpec::new("loading"), (w, h), &title);
    if let Some(close) = parts.close {
        commands.entity(close).insert(Hit(Target::Close, 0));
    }
    let body = commands
        .spawn((
            Node {
                overflow: Overflow::clip(),
                ..place(look, 20.0, 34.0, w - 40.0, h - 34.0 - 40.0)
            },
            ChildOf(parts.frame),
        ))
        .id();
    commands.spawn((
        Text::new(text),
        look.font("Default", (16.0, false)),
        TextColor(look.text()),
        TextLayout::new(Justify::Left, LineBreak::WordBoundary),
        ChildOf(body),
    ));
    let close = menu.text("#GameUI_Close", "Close");
    widgets::button(
        commands,
        parts.frame,
        look,
        (w - 20.0 - 72.0, h - 34.0, 72.0, 24.0),
        &close,
        Btn::enabled(true).default_button(true),
        0,
        Hit(Target::Cancel, 0),
    );
}

/// The loading dialog's stage line (the map's name).
#[derive(Component)]
pub(super) struct LoadingInfo(pub(super) String);

/// The loading dialog's bar fill.
#[derive(Component)]
pub(super) struct LoadingBar;

/// The loading dialog follows the load's progress (a map's, joining a
/// server's), and the detailed view times its stages.
#[allow(clippy::too_many_arguments)]
pub(super) fn loading_progress(
    menu: Res<GameMenu>,
    join: Option<Res<crate::net::client::JoinProgress>>,
    settings: Option<Res<crate::net::NetSettings>>,
    role: Option<Res<crate::core::NetRole>>,
    client: Option<Res<bevy_replicon_renet::RenetClient>>,
    time: Res<Time<Real>>,
    mut timeline: ResMut<LoadTimeline>,
    infos: Query<(&LoadingInfo, &Children)>,
    mut texts: Query<&mut Text, Without<LoadingDetail>>,
    mut details: Query<&mut Text, With<LoadingDetail>>,
    mut bars: Query<&mut Node, With<LoadingBar>>,
) {
    let now = time.elapsed_secs_f64();
    if menu.loading.is_none() || menu.failure.is_some() {
        timeline.stages.clear();
        return;
    }
    let progress = crate::map::loading::current();
    let hosting = settings.as_ref().is_some_and(|s| s.maxplayers > 1)
        && role.as_deref().is_none_or(|r| *r != crate::core::NetRole::Client)
        && join.as_ref().is_none_or(|j| !j.showing());
    let label = infos.iter().next().map_or(String::new(), |(i, _)| i.0.clone());
    let (line, fraction) = dialog_state(&menu, join.as_deref(), progress, hosting, &label);
    for mut node in &mut bars {
        let w = percent(100.0 * fraction);
        if node.width != w {
            node.width = w;
        }
    }
    for (_, children) in &infos {
        for c in children.iter() {
            if let Ok(mut t) = texts.get_mut(c)
                && t.0 != line
            {
                t.0 = line.clone();
            }
        }
    }
    // The timeline, for the detailed view.
    let (stage, detail) = detail_stage(join.as_deref(), progress);
    match timeline.stages.last_mut() {
        Some(last) if last.0 == stage => last.2 = detail,
        _ => timeline.stages.push((stage, now, detail)),
    }
    if details.is_empty() {
        return;
    }
    let mut lines = Vec::new();
    if let Some(j) = join.as_deref().filter(|j| j.showing()) {
        let server = j.server.clone().unwrap_or_default();
        let ping = client.as_ref().map_or(0.0, |c| c.rtt() * 1000.0);
        lines.push(format!(
            "server  {}  {}",
            if server.name.is_empty() { "(asking)" } else { &server.name },
            j.address.map_or(String::new(), |a| a.to_string())
        ));
        lines.push(format!(
            "map     {}   players {}/{}   ping {ping:.0} ms",
            j.map.as_deref().unwrap_or("?"),
            server.players,
            server.max_players
        ));
    } else {
        lines.push(format!("map     {label}"));
    }
    lines.push(String::new());
    lines.extend(timeline.lines(now));
    let total = timeline.stages.first().map_or(0.0, |s| now - s.1);
    lines.push(format!("{:<32} {total:>7.2} s", "total"));
    let text = lines.join("\n");
    for mut t in &mut details {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}
