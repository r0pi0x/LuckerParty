//! The options dialog's pages (Keyboard, Mouse, Audio, Video,
//! Multiplayer) and its two Advanced dialogs: their rows, OK / Cancel /
//! Apply, the keyboard list and the crosshair preview; the multiplayer
//! tab's Advanced dialog (its options from the install's `cfg/user.scr`).

use super::*;

/// The video tab's aspect ratios (CS:S's words): normal, 16:9, 16:10.
pub const ASPECTS: [(&str, &str); 3] = [
    ("#GameUI_AspectNormal", "Normal (4:3)"),
    ("#GameUI_AspectWide16x9", "Widescreen 16:9"),
    ("#GameUI_AspectWide16x10", "Widescreen 16:10"),
];

/// The aspect ratio (`ASPECTS`) of a window size (`1920x1080`): normal up
/// to 3:2 (4:3, 5:4), 16:10 near 1.6, else wide (16:9 and wider).
pub fn aspect_of(size: &str) -> Option<usize> {
    let s = crate::client::options::parse_resolution(size)?;
    let r = s.x as f32 / s.y as f32;
    Some(if r < 1.5 {
        0
    } else if (r - 1.6).abs() < 0.05 {
        2
    } else {
        1
    })
}

/// A fallback option's choices: token, ours, value.
type Choices = &'static [(&'static str, &'static str, &'static str)];

/// Multiplayer > Advanced without the install's script: ours, in CS:S's
/// words and `user.scr`'s order and defaults (cvar, label token, our
/// label, its choices (none: a check box), default).
pub(super) const USER_FALLBACK: [(&str, &str, &str, Choices, &str); 4] = [
    (
        "cl_righthand",
        "#Cstrike_Weapon_Alignment",
        "Weapon alignment",
        &[("#Cstrike_Left_Handed", "Left handed", "0"), ("#Cstrike_Right_Handed", "Right handed", "1")],
        "1",
    ),
    (
        "cl_autowepswitch",
        "#Cstrike_Automatic_Weapon_Switch",
        "Automatically switch to picked up weapons (if more powerful)",
        &[],
        "1",
    ),
    ("hud_centerid", "#Valve_Center_Player_Names", "Center player names", &[], "1"),
    ("cl_disablefreezecam", "#Cstrike_Disable_Freeze_Cam", "Disable freeze cam", &[], "0"),
];

/// The keyboard list without the game's `kb_act.lst`: what mashup does.
pub(super) const OUR_ACTIONS: &[(&str, &str)] = &[
    ("", "Movement"),
    ("+forward", "Move forward"),
    ("+back", "Move back"),
    ("+moveleft", "Move left (strafe)"),
    ("+moveright", "Move right (strafe)"),
    ("+speed", "Walk"),
    ("+jump", "Jump"),
    ("+duck", "Duck"),
    ("", "Combat"),
    ("+attack", "Fire"),
    ("+attack2", "Weapon special function"),
    ("+reload", "Reload weapon"),
    ("lastinv", "Last weapon used"),
    ("drop", "Drop weapon"),
    ("", "Communication"),
    ("radio1", "Standard radio messages"),
    ("radio2", "Group radio messages"),
    ("radio3", "Report radio messages"),
    ("messagemode", "Chat message"),
    ("messagemode2", "Team message"),
    ("", "Menus"),
    ("buymenu", "Buy menu"),
    ("chooseteam", "Select team"),
    ("+showscores", "Display multiplayer scores"),
    ("slot1", "Menu item 1"),
    ("slot2", "Menu item 2"),
    ("slot3", "Menu item 3"),
    ("slot4", "Menu item 4"),
    ("slot5", "Menu item 5"),
    ("", "Miscellaneous"),
    ("+use", "Use items"),
    ("bug", "Report a bug"),
    ("+freelook", "Free look"),
];

/// The keyboard tab: the action list (sections, action and key columns,
/// a scroll bar) where the game's layout puts it, and its buttons.
pub(super) fn keyboard_tab(commands: &mut Commands, content: Entity, look: &Look, menu: &GameMenu, rows: &[Row]) {
    let layout = menu.ui.0.as_ref().and_then(|u| u.options.get("keyboard"));
    let rect = |name: &str, fallback: (f32, f32, f32, f32)| {
        layout
            .and_then(|l| l.get(name))
            .map_or(fallback, |c| (coord(c.x), coord(c.y), c.wide, c.tall))
    };
    let (lx, ly, lw, lh) = rect("listpanel_keybindlist", (8.0, 10.0, 480.0, 258.0));
    let (len, shown) = menu.list();
    let list = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, lx, ly, lw, lh)
            },
            bevel(look, false),
            BackgroundColor(look.color("SectionedListPanel.BgColor", [0, 0, 0, 128])),
            RelativeCursorPosition::default(),
            WheelList,
            ChildOf(content),
        ))
        .id();
    let bar_w = look.number("ScrollBar.Wide", 17.0);
    let row_h = ((lh - 22.0) / KEY_ROWS as f32).floor();
    let key_x = (lw - bar_w) * 0.62;
    let header = look.font("DefaultBold", (16.0, true));
    let text_font = look.font("Default", (16.0, false));
    let header_color = look.color("SectionedListPanel.HeaderTextColor", [255, 255, 255, 255]);
    label(commands, list, look, (6.0, 1.0, key_x, 20.0), &menu.text("#GameUI_Action", "Action"), header.clone(), header_color, -1);
    label(
        commands,
        list,
        look,
        (key_x, 1.0, lw - bar_w - key_x, 20.0),
        &menu.text("#GameUI_KeyButton", "Key/Button"),
        header.clone(),
        header_color,
        -1,
    );
    commands.spawn((
        place(look, 2.0, 21.0, lw - bar_w - 6.0, 1.0),
        BackgroundColor(look.color("SectionedListPanel.DividerColor", [0, 0, 0, 255])),
        ChildOf(list),
    ));
    let text = look.color("SectionedListPanel.TextColor", [190, 190, 190, 255]);
    for (k, i) in (menu.scroll..(menu.scroll + shown).min(len)).enumerate() {
        let y = 22.0 + k as f32 * row_h;
        let width = lw - bar_w - 4.0;
        match &rows[i] {
            Row::Heading(t) => {
                label(commands, list, look, (6.0, y, width, row_h), t, header.clone(), header_color, -1);
            }
            Row::Bind { label: name, keys, known, .. } => {
                let selected = menu.key_row == Some(i);
                let waiting = selected && menu.capture.is_some();
                let bg = if selected { look.selected_bg() } else { Color::NONE };
                let fg = if selected {
                    look.selected_text()
                } else if *known {
                    text
                } else {
                    look.disabled()
                };
                let e = commands
                    .spawn((
                        place(look, 1.0, y, width, row_h),
                        BackgroundColor(bg),
                        Hit(Target::Row(i), 0),
                        Button,
                        Interaction::default(),
                        ChildOf(list),
                    ))
                    .id();
                label(commands, e, look, (16.0, 0.0, key_x - 20.0, row_h), name, text_font.clone(), fg, -1);
                let keys = if waiting { "Press a key (Esc: cancel)".to_string() } else { keys.clone() };
                label(commands, e, look, (key_x - 1.0, 0.0, width - key_x, row_h), &keys, text_font.clone(), fg, -1);
            }
            _ => {}
        }
    }
    scroll_bar(commands, list, look, (lw - bar_w - 2.0, 0.0, lh - 2.0), (len, shown, menu.scroll));
    // The buttons, where the game's layout has them.
    let names = ["Defaults", "ChangeKeyButton", "ClearKeyButton", "KeyAdvancedButton"];
    let fallback = [
        (8.0, 276.0, 134.0, 24.0),
        (272.0, 276.0, 106.0, 24.0),
        (384.0, 276.0, 105.0, 24.0),
        (148.0, 276.0, 111.0, 24.0),
    ];
    for (k, name) in names.iter().enumerate() {
        let i = len + k;
        let Row::Button { label: text, enabled, .. } = &rows[i] else { continue };
        let state = Btn::enabled(*enabled).focused(menu.focus == i);
        widgets::button(commands, content, look, rect(name, fallback[k]), text, state, 0, Hit(Target::Row(i), 0));
    }
}

/// The crosshair as the multiplayer tab's settings draw it, in the
/// layout's crosshair box.
pub(super) fn crosshair_preview(commands: &mut Commands, parent: Entity, look: &Look, menu: &GameMenu, (x, y, w, h): (f32, f32, f32, f32)) {
    let get = |cvar: &str| {
        SETTINGS
            .iter()
            .position(|s| s.cvar == cvar)
            .and_then(|i| menu.values.get(i).cloned().flatten())
            .and_then(|v| v.trim().parse::<f32>().ok())
    };
    let defaults = crate::client::hud::CrosshairColor::default();
    let c = crate::client::hud::CrosshairColor {
        color: get("cl_crosshaircolor").map_or(defaults.color, |v| v as u8),
        scale: get("cl_crosshairscale").unwrap_or(defaults.scale),
        alpha: get("cl_crosshairalpha").map_or(defaults.alpha, |v| v as u8),
        use_alpha: get("cl_crosshairusealpha").map_or(defaults.use_alpha, |v| v as u8),
        dynamic: get("cl_dynamiccrosshair").map_or(defaults.dynamic, |v| v as u8),
        size: get("cl_crosshairsize").unwrap_or(defaults.size),
        thickness: get("cl_crosshairthickness").unwrap_or(defaults.thickness),
        dot: get("cl_crosshairdot").map_or(defaults.dot, |v| v as u8),
        r: get("cl_crosshaircolor_r").map_or(defaults.r, |v| v as u8),
        g: get("cl_crosshaircolor_g").map_or(defaults.g, |v| v as u8),
        b: get("cl_crosshaircolor_b").map_or(defaults.b, |v| v as u8),
    };
    let e = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, x, y, w, h)
            },
            bevel(look, false),
            BackgroundColor(Color::srgb(0.25, 0.27, 0.3)),
            ChildOf(parent),
        ))
        .id();
    // As at this window's height, in screen pixels.
    let centre = Vec2::new(w, h) * look.s / 2.0;
    for (offset, size) in c.lines(4.0, look.height) {
        if size.min_element() <= 0.0 {
            continue;
        }
        let at = centre + offset - size / 2.0;
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(at.x.round()),
                top: px(at.y.round()),
                width: px(size.x),
                height: px(size.y),
                ..default()
            },
            BackgroundColor(c.color()),
            ChildOf(e),
        ));
    }
}

impl GameMenu {
    /// Settings' values now (those `which` picks).
    pub(super) fn snapshot(&self, which: impl Fn(&crate::client::options::Setting) -> bool) -> Vec<(usize, Option<String>)> {
        SETTINGS
            .iter()
            .enumerate()
            .filter(|(_, s)| which(s))
            .map(|(i, _)| (i, self.values.get(i).cloned().flatten()))
            .collect()
    }

    /// Show an options tab.
    pub fn set_tab(&mut self, tab: Tab) {
        if self.page != Page::Settings {
            return;
        }
        self.tab = tab;
        self.capture = None;
        self.combo = None;
        self.scroll = 0;
        self.key_row = None;
        self.focus = self.first_focusable_from(0, 1);
        self.select_focused_key();
        self.focus_changed();
        self.keep_visible();
    }

    /// The keyboard list: the game's actions, then ours it lacks.
    pub fn key_actions(&self) -> Vec<KeyAction> {
        let ours = |command: &str, label: &str| KeyAction::Action {
            command: command.to_string(),
            label: label.to_string(),
        };
        let Some(ui) = self.ui.0.as_ref().filter(|u| !u.actions.is_empty()) else {
            return OUR_ACTIONS
                .iter()
                .map(|(c, l)| {
                    if c.is_empty() {
                        KeyAction::Section(l.to_string())
                    } else {
                        ours(c, l)
                    }
                })
                .collect();
        };
        let mut list = ui.actions.clone();
        let has = |c: &str| {
            list.iter()
                .any(|a| matches!(a, KeyAction::Action { command, .. } if command.eq_ignore_ascii_case(c)))
        };
        let missing: Vec<KeyAction> = OUR_ACTIONS
            .iter()
            .filter(|(c, _)| !c.is_empty() && !has(c))
            .map(|(c, l)| ours(c, l))
            .collect();
        if !missing.is_empty() {
            list.push(KeyAction::Section("mashup".into()));
            list.extend(missing);
        }
        list
    }

    /// Whether mashup has a keyboard action's command.
    pub(super) fn known_command(&self, line: &str) -> bool {
        let first = line.split_whitespace().next().unwrap_or_default().to_lowercase();
        binds::is_polled(line) || self.known.contains(&first)
    }

    /// Whether setting `i` shows as a combo box: choices, window sizes,
    /// and a check box setting the game's layout shows as one (VSync).
    pub(super) fn shown_as_combo(&self, i: usize) -> bool {
        let s = &SETTINGS[i];
        match s.kind {
            SettingKind::Choice(_) | SettingKind::Resolution => true,
            SettingKind::Toggle => s
                .field
                .and_then(|f| self.settings_layout(s.place)?.get(f))
                .is_some_and(|c| class_of(c).eq_ignore_ascii_case("ComboBox")),
            _ => false,
        }
    }

    /// The layout a place's settings are on.
    pub(super) fn settings_layout(&self, place: Place) -> Option<&UiLayout> {
        let name = match place {
            Place::Options(tab) => tab.page(),
            Place::KeyboardAdvanced => "keyboard_advanced",
            Place::VideoAdvanced => "video_advanced",
            Place::Extras => return None,
        };
        self.ui.0.as_ref()?.options.get(name)
    }

    /// The settings of a place that mashup has, in its layout's order
    /// (else ours), with the text entries showing slider values.
    pub(super) fn place_rows(&self, place: Place) -> Vec<Row> {
        let mut fields: Vec<(usize, Field)> = Vec::new();
        let layout = self.settings_layout(place);
        let order = |name: &str| {
            layout
                .and_then(|l| l.controls.iter().position(|c| c.name.eq_ignore_ascii_case(name)))
                .unwrap_or(usize::MAX)
        };
        for (i, s) in SETTINGS.iter().enumerate() {
            if s.place != place || self.values.get(i).cloned().flatten().is_none() {
                continue;
            }
            // In a layout without its control: not shown (the game has no
            // place for it).
            if layout.is_some() && s.field.is_some_and(|f| order(f) == usize::MAX) {
                continue;
            }
            fields.push((s.field.map_or(i, order), Field::Setting(i)));
            if let Some((entry, _)) = VALUE_ENTRIES.iter().find(|(_, cvar)| *cvar == s.cvar)
                && (layout.is_none() || order(entry) != usize::MAX)
            {
                fields.push((order(entry), Field::SettingText(i)));
            }
        }
        if layout.is_some() {
            fields.sort_by_key(|(o, _)| *o);
        }
        // Without the layout the value entry is the slider's own number.
        fields
            .into_iter()
            .filter(|(_, f)| layout.is_some() || !matches!(f, Field::SettingText(_)))
            .map(|(_, f)| self.control_row(f))
            .collect()
    }

    /// The options' OK, Cancel and Apply (Apply greyed with nothing to
    /// apply).
    pub(super) fn dialog_buttons(&self) -> Vec<Row> {
        let changed = self.options_before.iter().any(|(i, v)| self.values.get(*i) != Some(v));
        vec![
            Row::Button {
                label: self.text("#GameUI_OK", "OK"),
                action: Action::Ok,
                enabled: true,
            },
            Row::Button {
                label: self.text("#GameUI_Cancel", "Cancel"),
                action: Action::Cancel,
                enabled: true,
            },
            Row::Button {
                label: self.text("#GameUI_Apply", "Apply"),
                action: Action::Apply,
                enabled: changed,
            },
        ]
    }

    /// Put settings back as they were.
    pub(super) fn restore(&mut self, before: Vec<(usize, Option<String>)>, out: &mut Outcome) {
        for (s, before) in before {
            if let Some(v) = before
                && self.values.get(s) != Some(&Some(v.clone()))
            {
                self.set_value(s, v, out);
            }
        }
    }

    /// Set setting `i` to `next` (a line when that changes what it shows).
    pub(super) fn set_value(&mut self, i: usize, next: String, out: &mut Outcome) {
        let (Some(setting), Some(Some(current))) = (SETTINGS.get(i), self.values.get(i)) else {
            return;
        };
        let none = |_: &str| None;
        let changed = match setting.kind {
            SettingKind::Negate => next.trim() != current.trim(),
            _ => setting.show(&next, &none) != setting.show(current, &none),
        };
        if changed {
            out.lines.extend(setting.lines(&next));
            self.values[i] = Some(next);
        }
    }

    /// The keyboard tab's rows: the action list, its buttons, OK / Cancel /
    /// Apply.
    pub(super) fn keyboard_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .key_actions()
            .into_iter()
            .map(|a| match a {
                KeyAction::Section(t) => Row::Heading(t),
                KeyAction::Action { command, label } => Row::Bind {
                    keys: binds::keys_for(&self.binds, &command)
                        .iter()
                        .map(|k| binds::display(k))
                        .collect::<Vec<_>>()
                        .join(", "),
                    known: self.known_command(&command),
                    label,
                    command,
                },
            })
            .collect();
        let selected = self.key_row.is_some_and(|r| matches!(rows.get(r), Some(Row::Bind { .. })));
        rows.push(Self::button_row(&self.text("#GameUI_UseDefaults", "Use Defaults"), Action::DefaultBinds));
        rows.push(Row::Button {
            label: self.text("#GameUI_SetNewKey", "Edit key"),
            action: Action::EditKey,
            enabled: selected,
        });
        rows.push(Row::Button {
            label: self.text("#GameUI_ClearKey", "Clear Key"),
            action: Action::ClearKey,
            enabled: selected,
        });
        rows.push(Self::button_row(&self.text("#GameUI_AdvancedEllipsis", "Advanced..."), Action::Advanced));
        rows.extend(self.dialog_buttons());
        rows
    }

    /// An options tab's rows (not the keyboard's): its settings in the
    /// layout's order, OK / Cancel / Apply.
    pub(super) fn options_rows(&self) -> Vec<Row> {
        let mut rows = self.place_rows(Place::Options(self.tab));
        if self.tab == Tab::Video && !self.resolutions.is_empty() {
            // Its aspect ratio, where the layout has it (else after the
            // resolution).
            let row = self.control_row(Field::Aspect);
            let order = self.row_order(&row);
            let at = if order == usize::MAX {
                rows.iter()
                    .position(|r| matches!(r, Row::Control { field: Field::Setting(i), .. } if SETTINGS[*i].cvar == "mashup_resolution"))
                    .map_or(rows.len(), |p| p + 1)
            } else {
                rows.iter().position(|r| self.row_order(r) > order).unwrap_or(rows.len())
            };
            rows.insert(at, row);
        }
        let advanced = match self.tab {
            Tab::Video => Some(Action::VideoAdvanced),
            Tab::Multiplayer => Some(Action::MultiplayerAdvanced),
            _ => None,
        };
        if let Some(action) = advanced {
            // Its Advanced... button, where the layout has it.
            let name = advanced_button(&action).unwrap_or_default();
            let at = self.layout().and_then(|l| l.controls.iter().position(|c| c.name == name));
            let b = Self::button_row(&self.text("#GameUI_AdvancedEllipsis", "Advanced..."), action);
            let before = at.map_or(rows.len(), |at| {
                rows.iter()
                    .position(|r| self.row_order(r) > at)
                    .unwrap_or(rows.len())
            });
            rows.insert(before, b);
        }
        rows.extend(self.dialog_buttons());
        rows
    }

    /// An Advanced dialog's rows: its settings, OK, Cancel.
    pub(super) fn advanced_rows(&self) -> Vec<Row> {
        let place = if self.page == Page::KeyboardAdvanced {
            Place::KeyboardAdvanced
        } else {
            Place::VideoAdvanced
        };
        let mut rows = self.place_rows(place);
        rows.push(self.ok_row());
        rows.push(self.cancel_row());
        rows
    }

    /// Multiplayer > Advanced's cvars: the install's `cfg/user.scr` (else
    /// ours), each with its value now.
    pub(super) fn user_cvars_now(&self, get: &dyn Fn(&str) -> Option<String>) -> Vec<ServerCvar> {
        let script: Vec<crate::map::hud::ServerSetting> = match self.ui.0.as_ref().filter(|u| !u.user_settings.is_empty()) {
            Some(ui) => ui.user_settings.clone(),
            None => USER_FALLBACK
                .iter()
                .map(|(cvar, token, ours, list, default)| crate::map::hud::ServerSetting {
                    cvar: cvar.to_string(),
                    label: self.text(token, ours),
                    kind: if list.is_empty() {
                        ServerSettingKind::Bool
                    } else {
                        ServerSettingKind::List(list.iter().map(|(t, o, v)| (self.text(t, o), v.to_string())).collect())
                    },
                    default: default.to_string(),
                })
                .collect(),
        };
        script
            .into_iter()
            .map(|s| {
                let before = get(&s.cvar);
                // A list's default is its value (an index into the list
                // when it isn't one of them).
                let default = match &s.kind {
                    ServerSettingKind::List(items) if !items.iter().any(|(_, v)| *v == s.default) => s
                        .default
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|k| items.get(k))
                        .map_or(s.default.clone(), |(_, v)| v.clone()),
                    _ => s.default.clone(),
                };
                ServerCvar {
                    cvar: s.cvar,
                    label: s.label,
                    kind: s.kind,
                    value: before.clone().unwrap_or(default),
                    before,
                    page: 0,
                    field: None,
                }
            })
            .collect()
    }

    /// Multiplayer > Advanced's rows: its options in the script's order
    /// (greyed where mashup lacks the cvar), OK, Cancel.
    pub(super) fn multiplayer_advanced_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .user_cvars
            .iter()
            .enumerate()
            .map(|(i, c)| match c.before {
                Some(_) => self.control_row(Field::UserCvar(i)),
                None => Row::Greyed {
                    label: c.label.clone(),
                    control: self.scr_control(c),
                },
            })
            .collect();
        rows.push(self.ok_row());
        rows.push(self.cancel_row());
        rows
    }

    /// Multiplayer > Advanced's OK: the lines setting what changed.
    pub(super) fn user_cvar_lines(&self) -> Vec<String> {
        self.user_cvars
            .iter()
            .filter(|c| c.before.as_ref().is_some_and(|b| b.trim() != c.value.trim()))
            .map(|c| format!("{} {}", c.cvar, crate::console::quote(&c.value)))
            .collect()
    }

    /// The window sizes the video tab's resolution list shows: those of
    /// its aspect ratio (all without one; all when none has it).
    pub fn shown_resolutions(&self) -> Vec<String> {
        let shown: Vec<String> = self
            .resolutions
            .iter()
            .filter(|r| self.aspect.is_none() || aspect_of(r) == self.aspect)
            .cloned()
            .collect();
        if shown.is_empty() { self.resolutions.clone() } else { shown }
    }

    /// Pick an aspect ratio: the resolution list shows its sizes; a window
    /// size of another goes to its largest (as CS:S's list refills).
    pub(super) fn pick_aspect(&mut self, k: usize, out: &mut Outcome) {
        if k >= ASPECTS.len() {
            return;
        }
        self.aspect = Some(k);
        let Some(i) = crate::client::options::setting_index("mashup_resolution") else { return };
        let shown = self.shown_resolutions();
        let current = self.values.get(i).cloned().flatten().unwrap_or_default();
        if !shown.contains(&current)
            && let Some(first) = shown.first().cloned()
            && self.values.get(i).is_some_and(Option::is_some)
        {
            self.set_value(i, first, out);
        }
    }
}
