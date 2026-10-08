//! The debug UI (F2, `debugui [tab]`): a tabbed egui window for
//! playtesting and development. Every control runs a console line (a cvar
//! or a command), so what it does lives in one place and the console,
//! binds and configs can do the same; the window only reads state to show
//! it. While it is open the mouse is free (the game takes no input), and
//! typing in one of its fields goes to the field, not to binds.
//!
//! Tabs: Player (position, state, health/armour/money, god, noclip,
//! spawns and bookmarks, weapons), World (map, time scale, gravity, tick),
//! Movement (the movement and mouse cvars), Bots (add, kick, skill, each
//! bot's state), Rendering (overlays and culling), Perf (frame-time graph,
//! the `mashup_perf` readout), Audio, Logic (map entities, fire inputs,
//! outputs fired), Rounds, Cvars (every cvar, searchable and editable).

use std::collections::{HashMap, VecDeque};

use bevy::{prelude::*, window::CursorOptions};
use bevy_inspector_egui::{
    bevy_egui::{EguiContext, EguiPrimaryContextPass, PrimaryEguiContext},
    egui,
};

use crate::{
    console::{Console, ConsoleAppExt, quote},
    core::{Health, LocalPlayer, MovementState, SpawnPoint, Team, Velocity},
};

/// The debug window's state (client only; nothing here is simulated).
#[derive(Resource, Default)]
pub struct DebugUi {
    pub open: bool,
    pub tab: Tab,
    /// Text typed into fields, by field (kept while the field is edited).
    edits: HashMap<String, String>,
    /// The Cvars tab's search and "changed only".
    cvar_filter: String,
    changed_only: bool,
    /// The Logic tab: the entity filter, the picked entity's name, the
    /// input and value to fire.
    logic_filter: String,
    logic_pick: Option<(String, String)>,
    fire_input: String,
    fire_value: String,
    /// The World tab's map choice.
    map_pick: String,
    /// The Player tab's weapon choice and bookmark name.
    weapon_pick: String,
    bookmark_name: String,
    /// Saved places: name and a `setpos ...;setang ...` line.
    bookmarks: Option<Vec<(String, String)>>,
    /// Recent frames for the Perf graph: frame and main-world CPU ms.
    frames: VecDeque<(f32, f32)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Player,
    World,
    Movement,
    Bots,
    Rendering,
    Perf,
    Audio,
    Logic,
    Rounds,
    Cvars,
}

impl Tab {
    pub const ALL: [Tab; 10] = [
        Tab::Player,
        Tab::World,
        Tab::Movement,
        Tab::Bots,
        Tab::Rendering,
        Tab::Perf,
        Tab::Audio,
        Tab::Logic,
        Tab::Rounds,
        Tab::Cvars,
    ];

    /// The name `debugui` takes.
    pub fn name(self) -> &'static str {
        match self {
            Tab::Player => "player",
            Tab::World => "world",
            Tab::Movement => "movement",
            Tab::Bots => "bots",
            Tab::Rendering => "rendering",
            Tab::Perf => "perf",
            Tab::Audio => "audio",
            Tab::Logic => "logic",
            Tab::Rounds => "rounds",
            Tab::Cvars => "cvars",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::Player => "Player",
            Tab::World => "World",
            Tab::Movement => "Movement",
            Tab::Bots => "Bots",
            Tab::Rendering => "Rendering",
            Tab::Perf => "Perf",
            Tab::Audio => "Audio",
            Tab::Logic => "Logic",
            Tab::Rounds => "Rounds",
            Tab::Cvars => "Cvars",
        }
    }

    pub fn from_name(name: &str) -> Option<Tab> {
        let n = name.to_lowercase();
        Tab::ALL.into_iter().find(|t| t.name() == n || t.name().starts_with(&n) && n.len() >= 2)
    }
}

/// Ranges for cvars' sliders and the console's argument help (as
/// Source's FCVAR min/max would give; here only hints, the cvars take
/// any value): name, lowest, highest.
pub const CVAR_RANGES: &[(&str, f32, f32)] = &[
    ("sv_accelerate", 0.0, 20.0),
    ("sv_airaccelerate", 0.0, 1000.0),
    ("sv_friction", 0.0, 10.0),
    ("sv_stopspeed", 0.0, 300.0),
    ("sv_gravity", 0.0, 2000.0),
    ("sv_maxspeed", 0.0, 1000.0),
    ("sv_stepsize", 0.0, 36.0),
    ("sv_maxvelocity", 0.0, 10000.0),
    ("sv_bounce", 0.0, 2.0),
    ("cl_forwardspeed", 0.0, 1000.0),
    ("sv_ladder_dampen", 0.0, 1.0),
    ("sv_ladder_angle", -1.0, 1.0),
    ("host_timescale", 0.0, 4.0),
    ("sensitivity", 0.1, 20.0),
    ("m_yaw", -0.1, 0.1),
    ("m_pitch", -0.1, 0.1),
    ("zoom_sensitivity_ratio", 0.0, 4.0),
    ("volume", 0.0, 1.0),
    ("dsp_volume", 0.0, 2.0),
    ("viewmodel_fov", 30.0, 120.0),
    ("cam_idealdist", 0.0, 400.0),
    ("cam_idealyaw", -180.0, 180.0),
    ("mp_freezetime", 0.0, 60.0),
    ("mp_roundtime", 0.0, 30.0),
    ("mp_buytime", 0.0, 10.0),
    ("mp_startmoney", 0.0, 16000.0),
    ("mp_respawn_delay", 0.0, 30.0),
    ("mp_c4timer", 10.0, 90.0),
    ("mp_forcecamera", 0.0, 2.0),
    ("bot_reaction", 0.0, 2.0),
    ("bot_aim_error", 0.0, 20.0),
    ("bot_turn_rate", 30.0, 1500.0),
    ("bot_grenades", 0.0, 2.0),
    ("mashup_drawnav", 0.0, 2.0),
    ("mashup_perf", 0.0, 3.0),
    ("mashup_freecam", 0.0, 2.0),
    ("mashup_watch", 0.0, 32.0),
    ("cl_showfps", 0.0, 2.0),
    ("developer", 0.0, 2.0),
    ("mat_hdr_level", 0.0, 2.0),
    ("cl_crosshaircolor", 0.0, 4.0),
    ("cl_crosshairalpha", 0.0, 255.0),
    ("cl_bobcycle", 0.0, 2.0),
    ("cl_bobup", 0.0, 1.0),
    ("cl_wpn_sway_scale", 0.0, 20.0),
    ("cl_wpn_sway_interp", 0.0, 1.0),
    ("ragdoll_sleepaftertime", 0.0, 30.0),
];

/// The debug UI's state and `debugui` command, without drawing (headless
/// apps and tests can open and switch it).
pub struct DebugUiStatePlugin;

impl Plugin for DebugUiStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugUi>()
            .add_systems(Startup, apply_ranges)
            .console_command(
                "debugui",
                "debugui [tab|close]: toggle the debug window (F2), or open it on a tab (player, world, movement, \
                 bots, rendering, perf, audio, logic, rounds, cvars).",
                |w, a| {
                    let mut ui = w.resource_mut::<DebugUi>();
                    match a.first().map(|s| s.to_lowercase()) {
                        None => ui.open = !ui.open,
                        Some(c) if c == "close" || c == "0" => ui.open = false,
                        Some(t) => {
                            ui.tab = Tab::from_name(&t).ok_or_else(|| {
                                let names: Vec<&str> = Tab::ALL.iter().map(|t| t.name()).collect();
                                format!("no tab \"{t}\" ({})", names.join(", "))
                            })?;
                            ui.open = true;
                        }
                    }
                    Ok(None)
                },
            );
        player_commands(app);
    }
}

fn local_player(w: &mut World) -> Result<Entity, String> {
    w.query_filtered::<Entity, With<LocalPlayer>>()
        .single(w)
        .map_err(|_| "no local player".to_string())
}

/// The Player tab's setters: health, armour and money, in CS:S points.
fn player_commands(app: &mut App) {
    app.console_command(
        "mashup_sethealth",
        "mashup_sethealth <1-100>: set your health (CS:S points).",
        |w, a| {
            let hp = a.first().and_then(|v| v.parse::<f32>().ok()).ok_or("mashup_sethealth <1-100>")?;
            let p = local_player(w)?;
            let mut h = w.get_mut::<Health>(p).ok_or("no health")?;
            h.current = (hp / 100.0).clamp(0.01, h.max.max(0.01));
            Ok(None)
        },
    )
    .console_command(
        "mashup_setarmor",
        "mashup_setarmor <0-100> [helmet 0|1]: set your armour (CS:S points) and helmet.",
        |w, a| {
            let amount = a
                .first()
                .and_then(|v| v.parse::<f32>().ok())
                .ok_or("mashup_setarmor <0-100> [helmet]")?;
            let p = local_player(w)?;
            let helmet = match a.get(1) {
                Some(h) => h != "0",
                None => w.get::<crate::weapon::Armor>(p).is_some_and(|a| a.helmet),
            };
            w.entity_mut(p).insert(crate::weapon::Armor {
                amount: (amount / 100.0).clamp(0.0, 1.0),
                helmet,
            });
            Ok(None)
        },
    )
    .console_command(
        "mashup_setmoney",
        "mashup_setmoney <amount>: set your money.",
        |w, a| {
            let amount = a.first().and_then(|v| v.parse::<f64>().ok()).ok_or("mashup_setmoney <amount>")?;
            let p = local_player(w)?;
            w.entity_mut(p)
                .insert(crate::weapon::economy::Money(amount.max(0.0).round() as u32));
            Ok(None)
        },
    );
}

/// Give the cvars in `CVAR_RANGES` their ranges (after every plugin
/// registered its cvars).
fn apply_ranges(mut console: ResMut<Console>) {
    for (name, min, max) in CVAR_RANGES {
        console.set_range(name, *min, *max);
    }
}

/// The window itself (needs egui, so only with a window).
pub struct DebugUiPlugin;

impl Plugin for DebugUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(DebugUiStatePlugin)
            .add_systems(Update, follow_open)
            .add_systems(EguiPrimaryContextPass, draw.run_if(|ui: Res<DebugUi>| ui.open));
    }
}

/// Free the mouse when the window opens; take it back for playing when
/// it closes (unless the console or the game menu is open).
fn follow_open(
    ui: Res<DebugUi>,
    console: Option<Res<super::console::ConsoleUi>>,
    menu: Option<Res<super::game_menu::GameMenu>>,
    mut cursor: Single<&mut CursorOptions>,
    mut was: Local<bool>,
) {
    if ui.open == *was {
        return;
    }
    *was = ui.open;
    if ui.open {
        super::input::release_cursor(&mut cursor);
    } else if !console.is_some_and(|c| c.open) && !menu.is_some_and(|m| m.open) {
        super::input::capture_cursor(&mut cursor);
    }
}

/// Queue a console line (the controls' only way to change anything).
fn run(world: &mut World, line: impl Into<String>) {
    world.resource_mut::<Console>().submit(line);
}

/// A number as a cvar takes it: whole for integer cvars, else short.
pub fn format_number(v: f64, integer: bool) -> String {
    if integer {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// The widget a cvar gets: a checkbox for on/off ones, a slider when it
/// has a range, a drag number for other numbers, else a text field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CvarWidget {
    Check,
    Slider(f32, f32),
    Drag,
    Text,
}

pub fn widget_for(cvar: &crate::console::Cvar, value: &str) -> CvarWidget {
    let number = value.trim().parse::<f64>().is_ok();
    match cvar.range {
        Some((a, b)) if number && !(a == 0.0 && b == 1.0 && cvar.integer) => CvarWidget::Slider(a, b),
        _ if cvar.values == ["0", "1"] && (value == "0" || value == "1") => CvarWidget::Check,
        _ if number => CvarWidget::Drag,
        _ => CvarWidget::Text,
    }
}

/// A grid row for a cvar: its name (help on hover), its control, and a
/// reset button when it differs from its default.
fn cvar_row(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi, name: &str, label: Option<&str>) {
    let Some(cvar) = world.resource::<Console>().cvar(name).cloned() else {
        ui.weak(name);
        ui.weak("(not registered)");
        ui.end_row();
        return;
    };
    let now = (cvar.get)(world).unwrap_or_default();
    ui.label(label.unwrap_or(cvar.name.as_str()))
        .on_hover_text(format!("{}\n{}\ndefault \"{}\"", cvar.name, cvar.help, cvar.default));
    let mut line = None;
    match widget_for(&cvar, &now) {
        CvarWidget::Check => {
            let mut on = now != "0";
            if ui.checkbox(&mut on, "").changed() {
                line = Some(format!("{} {}", cvar.name, on as u8));
            }
        }
        CvarWidget::Slider(a, b) => {
            let mut v: f64 = now.trim().parse().unwrap_or(0.0);
            let mut slider = egui::Slider::new(&mut v, a as f64..=b as f64).clamping(egui::SliderClamping::Never);
            if cvar.integer {
                slider = slider.integer();
            }
            if ui.add(slider).changed() {
                line = Some(format!("{} {}", cvar.name, format_number(v, cvar.integer)));
            }
        }
        CvarWidget::Drag => {
            let mut v: f64 = now.trim().parse().unwrap_or(0.0);
            let speed = if cvar.integer { 0.2 } else { (v.abs() * 0.01).max(0.001) };
            let mut drag = egui::DragValue::new(&mut v).speed(speed);
            if cvar.integer {
                drag = drag.fixed_decimals(0);
            }
            if ui.add(drag).changed() {
                line = Some(format!("{} {}", cvar.name, format_number(v, cvar.integer)));
            }
        }
        CvarWidget::Text => {
            if let Some(text) = text_field(ui, state, &format!("cvar:{}", cvar.name), &now, 140.0) {
                line = Some(format!("{} {}", cvar.name, quote(&text)));
            }
        }
    }
    if now != cvar.default {
        if ui
            .small_button("reset")
            .on_hover_text(format!("back to \"{}\"", cvar.default))
            .clicked()
        {
            line = Some(format!("reset {}", cvar.name));
        }
    } else {
        ui.label("");
    }
    ui.end_row();
    if let Some(l) = line {
        run(world, l);
    }
}

/// A single-line field showing `value` until edited; Enter returns the
/// text typed.
fn text_field(ui: &mut egui::Ui, state: &mut DebugUi, key: &str, value: &str, width: f32) -> Option<String> {
    let mut buf = state.edits.get(key).cloned().unwrap_or_else(|| value.to_string());
    let r = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(width));
    let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if entered {
        state.edits.remove(key);
        return Some(buf);
    }
    if r.has_focus() {
        state.edits.insert(key.to_string(), buf);
    } else {
        state.edits.remove(key);
    }
    None
}

/// A grid of cvar rows.
fn cvar_grid(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi, id: &str, names: &[&str]) {
    egui::Grid::new(id).num_columns(3).striped(true).show(ui, |ui| {
        for n in names {
            cvar_row(ui, world, state, n, None);
        }
    });
}

/// A button running a console line, the line on hover.
fn command_button(ui: &mut egui::Ui, world: &mut World, label: &str, line: &str) {
    if ui.button(label).on_hover_text(line).clicked() {
        run(world, line.to_string());
    }
}

/// Engine meters to CS:S units (x, y, z with z up), as getpos prints.
fn units(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y) / 0.0254
}

fn draw(world: &mut World) {
    let Ok(ctx) = world
        .query_filtered::<&mut EguiContext, With<PrimaryEguiContext>>()
        .single_mut(world)
        .map(|mut c| c.get_mut().clone())
    else {
        return;
    };
    world.resource_scope::<DebugUi, _>(|world, mut state| {
        // The frame history for the graph.
        let dt = world.resource::<Time<Real>>().delta_secs() * 1e3;
        let cpu = world
            .get_resource::<super::perf::FrameTimes>()
            .and_then(|f| f.frames.back().map(|f| f.1 * 1e3))
            .unwrap_or(0.0);
        state.frames.push_back((dt, cpu));
        while state.frames.len() > 300 {
            state.frames.pop_front();
        }
        let mut open = true;
        egui::Window::new("Debug")
            .open(&mut open)
            .default_size([560.0, 520.0])
            .default_pos([60.0, 60.0])
            .vscroll(false)
            .show(&ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for t in Tab::ALL {
                        if ui.selectable_label(state.tab == t, t.title()).clicked() {
                            state.tab = t;
                        }
                    }
                });
                ui.separator();
                let tab = state.tab;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match tab {
                    Tab::Player => player_tab(ui, world, &mut state),
                    Tab::World => world_tab(ui, world, &mut state),
                    Tab::Movement => movement_tab(ui, world, &mut state),
                    Tab::Bots => bots_tab(ui, world, &mut state),
                    Tab::Rendering => rendering_tab(ui, world, &mut state),
                    Tab::Perf => perf_tab(ui, world, &mut state),
                    Tab::Audio => audio_tab(ui, world, &mut state),
                    Tab::Logic => logic_tab(ui, world, &mut state),
                    Tab::Rounds => rounds_tab(ui, world, &mut state),
                    Tab::Cvars => cvars_tab(ui, world, &mut state),
                });
            });
        if !open {
            world.resource_mut::<Console>().submit("debugui close");
        }
    });
}

// ------------------------------------------------------------- tabs

fn player_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    let Ok(p) = world.query_filtered::<Entity, With<LocalPlayer>>().single(world) else {
        ui.label("No local player.");
        return;
    };
    let t = world.get::<Transform>(p).map_or(Vec3::ZERO, |t| t.translation);
    let v = world.get::<Velocity>(p).map_or(Vec3::ZERO, |v| v.0);
    let state_c = world.get::<MovementState>(p).cloned();
    let intent = world.get::<crate::core::Intent>(p).map(|i| (i.pitch, i.yaw));
    let health = world.get::<Health>(p).map(|h| h.current * 100.0);
    let armor = world.get::<crate::weapon::Armor>(p).map(|a| (a.amount * 100.0, a.helmet));
    let money = world.get::<crate::weapon::economy::Money>(p).map(|m| m.0);
    let team = world.get::<Team>(p).map(|t| t.0);
    let god = world.get::<crate::core::God>(p).is_some();
    let movement = world.get::<crate::slots::MovementSlot>(p).map(|m| m.0).unwrap_or("?");
    let noclip = movement == crate::movement::noclip::ID;
    let dead = world.get::<crate::rules::Dead>(p).is_some();
    let pos = units(t);
    let vel = units(v);
    ui.heading("State");
    egui::Grid::new("player_state").num_columns(2).show(ui, |ui| {
        ui.label("position");
        ui.monospace(format!("{:.1} {:.1} {:.1}", pos.x, pos.y, pos.z));
        ui.end_row();
        if let Some((pitch, yaw)) = intent {
            ui.label("angles");
            ui.monospace(format!(
                "{:.1} {:.1}",
                -pitch.to_degrees() + 0.0,
                (yaw.to_degrees() + 90.0).rem_euclid(360.0)
            ));
            ui.end_row();
        }
        ui.label("velocity");
        ui.monospace(format!(
            "{:.1} (horizontal {:.1}, vertical {:.1})",
            vel.length(),
            vel.truncate().length(),
            vel.z
        ));
        ui.end_row();
        if let Some(s) = &state_c {
            ui.label("movement");
            ui.monospace(format!(
                "{movement}{}{}{}{}",
                if s.on_ground { "  on ground" } else { "  in air" },
                if s.crouching { ", crouched" } else { "" },
                if s.on_ladder { ", on ladder" } else { "" },
                if dead { ", dead" } else { "" },
            ));
            ui.end_row();
        }
        ui.label("team");
        ui.monospace(match team {
            Some(1) => "terrorists",
            Some(2) => "counter-terrorists",
            _ => "-",
        });
        ui.end_row();
    });
    ui.horizontal(|ui| {
        let mut g = god;
        if ui.checkbox(&mut g, "god").on_hover_text("god").changed() {
            run(world, "god");
        }
        let mut n = noclip;
        if ui.checkbox(&mut n, "noclip").on_hover_text("noclip").changed() {
            run(world, "noclip");
        }
        command_button(ui, world, "Respawn", "kill");
        command_button(ui, world, "All weapons", "impulse 101");
        command_button(ui, world, "Hurt (chest)", "mashup_hurtme chest 25");
    });
    ui.horizontal(|ui| {
        command_button(ui, world, "Join T", "jointeam 2");
        command_button(ui, world, "Join CT", "jointeam 3");
        command_button(ui, world, "Third person", "thirdperson");
        command_button(ui, world, "First person", "firstperson");
    });
    ui.separator();
    ui.heading("Health, armour, money");
    egui::Grid::new("player_stats").num_columns(2).show(ui, |ui| {
        if let Some(mut h) = health {
            ui.label("health");
            if ui.add(egui::Slider::new(&mut h, 1.0..=100.0).integer()).changed() {
                run(world, format!("mashup_sethealth {}", h.round()));
            }
            ui.end_row();
        }
        let (mut a, helmet) = armor.unwrap_or((0.0, false));
        ui.label("armour");
        ui.horizontal(|ui| {
            let mut hm = helmet;
            let changed = ui.add(egui::Slider::new(&mut a, 0.0..=100.0).integer()).changed();
            let helmet_changed = ui.checkbox(&mut hm, "helmet").changed();
            if changed || helmet_changed {
                run(world, format!("mashup_setarmor {} {}", a.round(), hm as u8));
            }
        });
        ui.end_row();
        let mut m = money.unwrap_or(0) as f64;
        ui.label("money");
        ui.horizontal(|ui| {
            if ui.add(egui::Slider::new(&mut m, 0.0..=16000.0).integer().step_by(50.0)).changed() {
                run(world, format!("mashup_setmoney {}", m.round()));
            }
            if money.is_none() {
                ui.weak("(no money: rounds off)");
            }
        });
        ui.end_row();
    });
    ui.separator();
    ui.heading("Weapons");
    let held: Vec<String> = world
        .get::<crate::weapon::Inventory>(p)
        .map(|inv| {
            inv.weapons
                .iter()
                .filter_map(|e| {
                    let w = world.get::<crate::weapon::Weapon>(*e)?;
                    let mag = world
                        .get::<crate::weapon::Magazine>(*e)
                        .map_or(String::new(), |m| format!(" {}/{}", m.clip, m.reserve));
                    let active = if inv.active == Some(*e) { " (in hand)" } else { "" };
                    Some(format!("{}{mag}{active}", w.id))
                })
                .collect()
        })
        .unwrap_or_default();
    ui.label(if held.is_empty() { "none".to_string() } else { held.join(", ") });
    let ids: Vec<&'static str> = world
        .get_resource::<crate::weapon::WeaponRegistry>()
        .map(|r| r.0.iter().map(|d| d.id).collect())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        if state.weapon_pick.is_empty() {
            state.weapon_pick = ids.first().map(|s| s.to_string()).unwrap_or_default();
        }
        egui::ComboBox::from_id_salt("give_weapon")
            .selected_text(state.weapon_pick.clone())
            .width(240.0)
            .show_ui(ui, |ui| {
                for id in &ids {
                    ui.selectable_value(&mut state.weapon_pick, id.to_string(), *id);
                }
            });
        if ui.button("Give").clicked() && !state.weapon_pick.is_empty() {
            run(world, format!("give {}", state.weapon_pick));
        }
        command_button(ui, world, "Drop", "drop");
    });
    ui.separator();
    ui.heading("Teleport");
    let mut spawns: Vec<(Option<Team>, Vec3, f32)> = world
        .query::<(&SpawnPoint, &Transform)>()
        .iter(world)
        .map(|(s, t)| (s.team, t.translation, t.rotation.to_euler(EulerRot::YXZ).0))
        .collect();
    spawns.sort_by_key(|s| s.0.map_or(0, |t| t.0));
    ui.label(format!("{} spawn points", spawns.len()));
    ui.horizontal_wrapped(|ui| {
        let mut n = [0u32; 3];
        for (team, at, yaw) in &spawns {
            let k = team.map_or(0, |t| (t.0 as usize).min(2));
            n[k] += 1;
            let label = match team.map(|t| t.0) {
                Some(1) => format!("T {}", n[k]),
                Some(2) => format!("CT {}", n[k]),
                _ => format!("spawn {}", n[k]),
            };
            let u = units(*at + Vec3::Y);
            let line = format!(
                "setpos {:.1} {:.1} {:.1}; setang 0 {:.1}",
                u.x,
                u.y,
                u.z,
                (yaw.to_degrees() + 90.0).rem_euclid(360.0)
            );
            if ui.small_button(label).on_hover_text(&line).clicked() {
                run(world, line);
            }
        }
    });
    // Bookmarks: places saved here (cfg folder, bookmarks.txt).
    if state.bookmarks.is_none() {
        state.bookmarks = Some(load_bookmarks());
    }
    ui.horizontal(|ui| {
        ui.label("bookmark");
        ui.add(egui::TextEdit::singleline(&mut state.bookmark_name).desired_width(140.0).hint_text("name"));
        if ui.button("Save here").clicked()
            && let Some((pitch, yaw)) = intent
        {
            let name = if state.bookmark_name.trim().is_empty() {
                format!("place {}", state.bookmarks.as_ref().map_or(0, |b| b.len()) + 1)
            } else {
                state.bookmark_name.trim().to_string()
            };
            let line = format!(
                "setpos {:.1} {:.1} {:.1}; setang {:.1} {:.1}",
                pos.x,
                pos.y,
                pos.z,
                -pitch.to_degrees(),
                (yaw.to_degrees() + 90.0).rem_euclid(360.0)
            );
            let marks = state.bookmarks.get_or_insert_default();
            marks.push((name, line));
            save_bookmarks(marks);
            state.bookmark_name.clear();
        }
    });
    let mut remove = None;
    for (i, (name, line)) in state.bookmarks.clone().unwrap_or_default().iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.button(name).on_hover_text(line).clicked() {
                run(world, line.clone());
            }
            if ui.small_button("x").on_hover_text("forget it").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove
        && let Some(marks) = state.bookmarks.as_mut()
    {
        marks.remove(i);
        save_bookmarks(marks);
    }
}

fn bookmarks_path() -> Option<std::path::PathBuf> {
    crate::console::cfg_dir().map(|d| d.join("bookmarks.txt"))
}

/// Bookmarks as saved: one `name<TAB>line` per line.
fn load_bookmarks() -> Vec<(String, String)> {
    bookmarks_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| {
            t.lines()
                .filter_map(|l| l.split_once('\t'))
                .map(|(n, l)| (n.to_string(), l.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn save_bookmarks(marks: &[(String, String)]) {
    if let Some(p) = bookmarks_path() {
        let text: Vec<String> = marks.iter().map(|(n, l)| format!("{}\t{l}", n.replace('\t', " "))).collect();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, text.join("\n"));
    }
}

fn world_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    ui.heading("Map");
    let name = world
        .get_resource::<crate::map::LoadedMapName>()
        .map_or("greybox".to_string(), |m| m.0.clone());
    let loading = super::console::map_loading(world);
    ui.label(format!("{name}{}", if loading { " (loading...)" } else { "" }));
    let maps = super::console::map_names();
    ui.horizontal(|ui| {
        if state.map_pick.is_empty() {
            state.map_pick = name.rsplit(':').next().unwrap_or("").to_string();
        }
        egui::ComboBox::from_id_salt("map_pick")
            .selected_text(state.map_pick.clone())
            .width(200.0)
            .show_ui(ui, |ui| {
                for m in &maps {
                    ui.selectable_value(&mut state.map_pick, m.clone(), m);
                }
            });
        if ui.add_enabled(!loading && !maps.is_empty(), egui::Button::new("Load")).clicked() {
            run(world, format!("map {}", state.map_pick));
        }
        if maps.is_empty() {
            ui.weak("(no CS:S install configured)");
        }
    });
    let logic = world
        .get_resource::<crate::logic::Logic>()
        .map(|l| (l.world.ids().len(), l.world.round));
    let restarts = world.get_resource::<crate::core::RoundRestarts>().map_or(0, |r| r.0);
    let tick = world.get_resource::<crate::core::SimTick>().map_or(0, |t| t.0);
    let rate = 1.0 / world.resource::<Time<Fixed>>().timestep().as_secs_f64();
    let entities = world.entities().count_spawned();
    egui::Grid::new("world_info").num_columns(2).show(ui, |ui| {
        ui.label("tick");
        ui.monospace(format!("{tick} at {rate:.1}/s"));
        ui.end_row();
        ui.label("game time");
        ui.monospace(format!("{:.1} s", world.resource::<Time<Virtual>>().elapsed_secs()));
        ui.end_row();
        ui.label("ECS entities");
        ui.monospace(entities.to_string());
        ui.end_row();
        ui.label("map logic");
        ui.monospace(match logic {
            Some((n, round)) => format!("{n} entities, round {round}"),
            None => "none".into(),
        });
        ui.end_row();
        ui.label("round restarts");
        ui.monospace(restarts.to_string());
        ui.end_row();
    });
    ui.separator();
    ui.heading("Time and physics");
    ui.horizontal(|ui| {
        for (label, v) in [("pause", "0"), ("0.25x", "0.25"), ("0.5x", "0.5"), ("1x", "1"), ("2x", "2")] {
            command_button(ui, world, label, &format!("host_timescale {v}"));
        }
    });
    cvar_grid(ui, world, state, "world_cvars", &["host_timescale", "sv_gravity", "mp_friendlyfire"]);
}

fn movement_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    let speed = world
        .query_filtered::<&Velocity, With<LocalPlayer>>()
        .single(world)
        .map_or(0.0, |v| units(v.0).truncate().length());
    ui.label(format!("speed {speed:.1} units/s"));
    ui.heading("CS:S movement");
    let names: Vec<&str> = crate::games::cs_source::movement::CVARS.iter().map(|(n, _)| *n).collect();
    cvar_grid(ui, world, state, "movement_cvars", &names);
    ui.horizontal(|ui| {
        command_button(
            ui,
            world,
            "All to defaults",
            &names.iter().map(|n| format!("reset {n}")).collect::<Vec<_>>().join("; "),
        );
        command_button(ui, world, "Surf (airaccelerate 150)", "sv_airaccelerate 150");
        command_button(ui, world, "Noclip", "noclip");
    });
    ui.separator();
    ui.heading("Mouse");
    cvar_grid(
        ui,
        world,
        state,
        "mouse_cvars",
        &["sensitivity", "m_yaw", "m_pitch", "zoom_sensitivity_ratio"],
    );
}

fn bots_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    ui.horizontal(|ui| {
        command_button(ui, world, "Add T", "bot_add 1");
        command_button(ui, world, "Add CT", "bot_add 2");
        command_button(ui, world, "Kick all", "bot_kick");
        command_button(ui, world, "Back to orders", "bot_goto");
    });
    ui.horizontal(|ui| {
        ui.label("skill");
        for (name, reaction, aim, turn) in super::game_menu::DIFFICULTIES {
            command_button(
                ui,
                world,
                name,
                &format!("bot_reaction {reaction}; bot_aim_error {aim}; bot_turn_rate {turn}"),
            );
        }
    });
    cvar_grid(
        ui,
        world,
        state,
        "bot_cvars",
        &[
            "bot_stop",
            "bot_dont_shoot",
            "bot_grenades",
            "bot_radio",
            "bot_reaction",
            "bot_aim_error",
            "bot_turn_rate",
            "bot_debug",
            "mashup_drawbots",
            "mashup_watch",
        ],
    );
    ui.separator();
    let mut rows: Vec<_> = world
        .query::<(&crate::bot::Bot, &Team, &Health, Option<&Name>, &Transform)>()
        .iter(world)
        .map(|(b, t, h, n, at)| {
            (
                n.map_or("?".to_string(), |n| n.to_string()),
                t.0,
                h.current * 100.0,
                format!("{:?}", b.role()),
                format!("{:?}", b.activity()),
                at.translation,
            )
        })
        .collect();
    rows.sort_by_key(|r| (r.1, r.0.strip_prefix("Bot ").and_then(|n| n.parse::<u32>().ok())));
    ui.heading(format!("{} bots", rows.len()));
    egui::Grid::new("bot_rows").num_columns(6).striped(true).show(ui, |ui| {
        for h in ["name", "team", "health", "role", "activity", ""] {
            ui.strong(h);
        }
        ui.end_row();
        for (name, team, health, role, activity, at) in rows {
            ui.label(&name);
            ui.label(if team == 1 { "T" } else { "CT" });
            ui.label(format!("{health:.0}"));
            ui.label(role);
            ui.label(activity);
            ui.horizontal(|ui| {
                if let Some(n) = name.strip_prefix("Bot ") {
                    command_button(ui, world, "watch", &format!("mashup_watch {n}"));
                }
                let u = units(at + Vec3::Y);
                command_button(ui, world, "go to", &format!("setpos {:.1} {:.1} {:.1}", u.x, u.y, u.z));
            });
            ui.end_row();
        }
    });
}

fn rendering_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    ui.heading("Overlays");
    cvar_grid(
        ui,
        world,
        state,
        "overlay_cvars",
        &[
            "cl_showpos",
            "cl_showfps",
            "net_graph",
            "mashup_drawhitboxes",
            "mashup_drawcollision",
            "mashup_drawphys",
            "mashup_drawnav",
            "mashup_drawbots",
            "mashup_healthbars",
            "mashup_ragdoll_debug",
            "mashup_objectives",
            "snd_show",
            "developer",
        ],
    );
    ui.separator();
    ui.heading("Culling");
    cvar_grid(ui, world, state, "vis_cvars", &["r_novis", "r_portalsopenall", "r_occlusion"]);
    if let Some(r) = world.get_resource::<super::perf::PerfReport>() {
        for l in r.lines.iter().filter(|l| l.starts_with("vis:") || l.starts_with("areas:") || l.starts_with("occluders:")) {
            ui.monospace(l);
        }
    }
    ui.separator();
    ui.heading("Look");
    cvar_grid(
        ui,
        world,
        state,
        "look_cvars",
        &[
            "mat_hdr_level",
            "mat_drawwater",
            "r_WaterDrawReflection",
            "r_WaterDrawRefraction",
            "r_drawviewmodel",
            "viewmodel_fov",
            "cl_righthand",
            "muzzleflash_light",
            "cl_ejectbrass",
            "r_drawflecks",
            "violence_hblood",
            "cl_ragdoll_physics_enable",
        ],
    );
    ui.separator();
    ui.heading("Camera");
    ui.horizontal(|ui| {
        command_button(ui, world, "Third person", "thirdperson");
        command_button(ui, world, "First person", "firstperson");
    });
    cvar_grid(ui, world, state, "camera_cvars", &["cam_idealdist", "cam_idealyaw", "mashup_freecam"]);
}

fn perf_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    let frames: Vec<(f32, f32)> = state.frames.iter().copied().collect();
    let n = frames.len().max(1) as f32;
    let avg = frames.iter().map(|f| f.0).sum::<f32>() / n;
    let max = frames.iter().map(|f| f.0).fold(0.0f32, f32::max);
    ui.label(format!(
        "{:.0} fps, frame {avg:.2} ms (max {max:.2}) over the last {} frames; white: frame, green: main-world CPU",
        1e3 / avg.max(1e-3),
        frames.len()
    ));
    frame_graph(ui, &frames);
    if let Some(r) = world.get_resource::<super::perf::PerfReport>() {
        for l in &r.lines {
            ui.monospace(l);
        }
    }
    ui.separator();
    cvar_grid(
        ui,
        world,
        state,
        "perf_cvars",
        &["mashup_perf", "mashup_perf_log", "host_timescale", "mat_vsync", "r_novis"],
    );
    ui.horizontal(|ui| {
        command_button(ui, world, "Log particles", "mashup_particles");
        command_button(ui, world, "Bug report (F9)", "bugreport");
    });
    ui.collapsing("Traces", |ui| {
        ui.label(
            "A Chrome trace of every system and render pass needs a build with the profile feature: \
             cargo build --profile playtest --features profile, then run with TRACE_CHROME=<file> \
             and a short --frames count; tracesum <file> sums it up (docs/OBSERVABILITY.md, section 3c).",
        );
    });
}

/// Frame times as lines over the last frames, with 16.7 and 33.3 ms
/// marks.
fn frame_graph(ui: &mut egui::Ui, frames: &[(f32, f32)]) {
    let width = ui.available_width().max(100.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 120.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(160));
    let top = frames.iter().map(|f| f.0).fold(40.0f32, f32::max).min(200.0);
    let y = |ms: f32| rect.bottom() - (ms / top).min(1.0) * rect.height();
    for (ms, label) in [(1000.0 / 60.0, "60 fps"), (1000.0 / 30.0, "30 fps")] {
        let at = y(ms);
        painter.line_segment(
            [egui::pos2(rect.left(), at), egui::pos2(rect.right(), at)],
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(70)),
        );
        painter.text(
            egui::pos2(rect.left() + 4.0, at - 2.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(11.0),
            egui::Color32::from_gray(140),
        );
    }
    if frames.len() < 2 {
        return;
    }
    let step = rect.width() / 299.0;
    let x0 = rect.right() - step * (frames.len() - 1) as f32;
    let line = |pick: fn(&(f32, f32)) -> f32| -> Vec<egui::Pos2> {
        frames
            .iter()
            .enumerate()
            .map(|(i, f)| egui::pos2(x0 + step * i as f32, y(pick(f))))
            .collect()
    };
    painter.line(line(|f| f.1), egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 220, 120)));
    painter.line(line(|f| f.0), egui::Stroke::new(1.5_f32, egui::Color32::WHITE));
}

fn audio_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    cvar_grid(ui, world, state, "audio_cvars", &["volume", "dsp_off", "dsp_volume", "snd_show", "ignorerad"]);
    ui.separator();
    ui.heading("Soundscape and room");
    match (
        world.get_resource::<crate::map::soundscape::ScapeState>(),
        world.get_resource::<crate::map::room::RoomDsp>(),
    ) {
        (Some(s), Some(r)) => {
            ui.monospace(crate::map::soundscape::readout(s, r));
        }
        _ => {
            ui.weak("no soundscape (the map has none, or it hasn't loaded)");
        }
    }
}

fn logic_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    let Some(logic) = world.get_resource::<crate::logic::Logic>() else {
        ui.label("No map logic (the greybox map has none).");
        return;
    };
    // The entities: name, class, origin, keyvalues, outputs.
    struct Row {
        name: String,
        class: String,
        origin: Vec3,
        keys: Vec<(String, String)>,
        outputs: Vec<String>,
    }
    let mut rows: Vec<Row> = logic
        .world
        .ids()
        .into_iter()
        .filter_map(|id| logic.world.get(id))
        .map(|e| Row {
            name: e.targetname.clone(),
            class: e.classname.clone(),
            origin: e.origin,
            keys: e.keyvalues.clone(),
            outputs: e
                .outputs
                .iter()
                .flat_map(|(o, cs)| {
                    cs.iter().map(move |c| {
                        format!(
                            "{o} -> {} {}{} after {} s{}",
                            c.target,
                            c.input,
                            c.param.as_ref().map_or(String::new(), |p| format!(" ({p})")),
                            c.delay,
                            if c.times > 0 { format!(", {} more times", c.times) } else { String::new() }
                        )
                    })
                })
                .collect(),
        })
        .collect();
    let fired: Vec<String> = logic
        .world
        .fired
        .iter()
        .rev()
        .take(40)
        .map(|(tick, id, out)| {
            let who = logic.world.get(*id).map_or("?".to_string(), |e| {
                if e.targetname.is_empty() { e.classname.clone() } else { e.targetname.clone() }
            });
            format!("tick {tick}: {who} {out}")
        })
        .collect();
    let messages: Vec<String> = logic.world.log.iter().rev().take(20).cloned().collect();
    let total = rows.len();
    let filter = state.logic_filter.to_lowercase();
    rows.retain(|r| filter.is_empty() || r.name.to_lowercase().contains(&filter) || r.class.to_lowercase().contains(&filter));
    ui.horizontal(|ui| {
        ui.label("filter");
        ui.add(egui::TextEdit::singleline(&mut state.logic_filter).desired_width(160.0));
        ui.label(format!("{} of {total} entities", rows.len()));
    });
    egui::ScrollArea::vertical()
        .id_salt("logic_list")
        .max_height(180.0)
        .show(ui, |ui| {
            for r in &rows {
                let label = if r.name.is_empty() {
                    r.class.clone()
                } else {
                    format!("{}  ({})", r.name, r.class)
                };
                let picked = state.logic_pick.as_ref().is_some_and(|(n, c)| *n == r.name && *c == r.class);
                if ui.selectable_label(picked, label).clicked() {
                    state.logic_pick = Some((r.name.clone(), r.class.clone()));
                }
            }
        });
    ui.separator();
    if let Some((name, class)) = state.logic_pick.clone()
        && let Some(r) = rows.iter().find(|r| r.name == name && r.class == class)
    {
        ui.strong(format!("{} {}", r.class, r.name));
        ui.monospace(format!("origin {:.0} {:.0} {:.0}", r.origin.x, r.origin.y, r.origin.z));
        let target = if r.name.is_empty() { r.class.clone() } else { r.name.clone() };
        ui.horizontal(|ui| {
            ui.label("input");
            ui.add(egui::TextEdit::singleline(&mut state.fire_input).desired_width(110.0).hint_text("Open"));
            ui.label("value");
            ui.add(egui::TextEdit::singleline(&mut state.fire_value).desired_width(80.0));
            let line = format!("ent_fire {} {} {}", quote(&target), state.fire_input.trim(), state.fire_value.trim());
            if ui
                .add_enabled(!state.fire_input.trim().is_empty(), egui::Button::new("Fire"))
                .on_hover_text(line.trim_end())
                .clicked()
            {
                run(world, line.trim_end().to_string());
            }
        });
        ui.collapsing(format!("keyvalues ({})", r.keys.len()), |ui| {
            for (k, v) in &r.keys {
                ui.monospace(format!("{k} = {v}"));
            }
        });
        ui.collapsing(format!("outputs ({})", r.outputs.len()), |ui| {
            for o in &r.outputs {
                ui.monospace(o);
            }
        });
    } else {
        ui.weak("Pick an entity to see its keyvalues and outputs and fire inputs at it.");
    }
    ui.separator();
    cvar_grid(ui, world, state, "logic_cvars", &["mashup_logic_record"]);
    ui.collapsing(format!("outputs fired ({} latest)", fired.len()), |ui| {
        if fired.is_empty() {
            ui.weak("none (turn on mashup_logic_record)");
        }
        for f in &fired {
            ui.monospace(f);
        }
    });
    ui.collapsing("logic messages", |ui| {
        for m in &messages {
            ui.monospace(m);
        }
    });
}

fn rounds_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    if let Some(r) = world.get_resource::<crate::rules::rounds::RoundState>().cloned() {
        let now = world.resource::<Time>().elapsed_secs_f64();
        let phase = match r.phase {
            crate::rules::rounds::Phase::Off => "off (deathmatch)".to_string(),
            crate::rules::rounds::Phase::Freeze { until } => format!("freeze time, {:.0} s left", until - now),
            crate::rules::rounds::Phase::Live { ends, .. } => format!("live, {:.0} s left", ends - now),
            crate::rules::rounds::Phase::Over { until, winner } => format!(
                "over ({}), next in {:.0} s",
                match winner.map(|t| t.0) {
                    Some(1) => "terrorists won",
                    Some(2) => "counter-terrorists won",
                    _ => "draw",
                },
                until - now
            ),
        };
        ui.label(format!(
            "round {}: {phase}; wins T {} CT {}",
            r.number, r.wins[0], r.wins[1]
        ));
    }
    ui.horizontal(|ui| {
        command_button(ui, world, "Restart game", "mp_restartgame 1");
        command_button(ui, world, "Rounds on", "mashup_rounds 1");
        command_button(ui, world, "Deathmatch", "mashup_rounds 0");
        command_button(ui, world, "No freeze time", "mp_freezetime 0");
    });
    cvar_grid(
        ui,
        world,
        state,
        "round_cvars",
        &[
            "mashup_rounds",
            "mp_freezetime",
            "mp_roundtime",
            "mp_buytime",
            "mp_startmoney",
            "mp_c4timer",
            "mp_friendlyfire",
            "mp_respawn_delay",
            "mp_forcecamera",
        ],
    );
}

fn cvars_tab(ui: &mut egui::Ui, world: &mut World, state: &mut DebugUi) {
    ui.horizontal(|ui| {
        ui.label("search");
        ui.add(egui::TextEdit::singleline(&mut state.cvar_filter).desired_width(180.0));
        ui.checkbox(&mut state.changed_only, "changed only");
        command_button(ui, world, "Save config", "host_writeconfig");
    });
    let filter = state.cvar_filter.to_lowercase();
    let cvars: Vec<crate::console::Cvar> = world
        .resource::<Console>()
        .cvars()
        .filter(|c| filter.is_empty() || c.name.to_lowercase().contains(&filter) || c.help.to_lowercase().contains(&filter))
        .cloned()
        .collect();
    let mut shown = Vec::new();
    for c in cvars {
        let now = (c.get)(world).unwrap_or_default();
        if !state.changed_only || now != c.default {
            shown.push(c.name.clone());
        }
    }
    ui.label(format!("{} cvars", shown.len()));
    egui::Grid::new("all_cvars").num_columns(3).striped(true).show(ui, |ui| {
        for n in &shown {
            cvar_row(ui, world, state, n, None);
        }
    });
    let commands: Vec<(String, String)> = world
        .resource::<Console>()
        .commands()
        .filter(|c| !filter.is_empty() && (c.name.to_lowercase().contains(&filter) || c.help.to_lowercase().contains(&filter)))
        .map(|c| (c.name.clone(), c.help.clone()))
        .collect();
    if !commands.is_empty() {
        ui.separator();
        ui.label(format!("{} commands", commands.len()));
        for (n, h) in commands {
            ui.horizontal_wrapped(|ui| {
                ui.monospace(&n);
                ui.weak(h);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_by_name() {
        for t in Tab::ALL {
            assert_eq!(Tab::from_name(t.name()), Some(t));
        }
        assert_eq!(Tab::from_name("Perf"), Some(Tab::Perf));
        assert_eq!(Tab::from_name("cv"), Some(Tab::Cvars));
        assert_eq!(Tab::from_name("x"), None);
    }

    #[test]
    fn numbers_as_cvars_take_them() {
        assert_eq!(format_number(812.6, true), "813");
        assert_eq!(format_number(0.0220, false), "0.022");
        assert_eq!(format_number(150.0, false), "150");
        assert_eq!(format_number(-0.00001, false), "0");
    }

    #[test]
    fn widgets_follow_the_cvar() {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin);
        #[derive(Resource)]
        struct S(u8, f32, u8);
        app.insert_resource(S(0, 800.0, 0));
        crate::console::resource_cvar::<S, u8>(&mut app, "flag", "", |s| &mut s.0);
        crate::console::resource_cvar::<S, f32>(&mut app, "grav", "", |s| &mut s.1);
        crate::console::resource_cvar::<S, u8>(&mut app, "levels", "", |s| &mut s.2);
        let mut c = app.world_mut().resource_mut::<Console>();
        c.set_range("grav", 0.0, 2000.0);
        c.set_range("levels", 0.0, 2.0);
        let c = app.world().resource::<Console>();
        assert_eq!(widget_for(c.cvar("flag").unwrap(), "0"), CvarWidget::Check);
        assert_eq!(widget_for(c.cvar("grav").unwrap(), "800"), CvarWidget::Slider(0.0, 2000.0));
        assert_eq!(widget_for(c.cvar("levels").unwrap(), "1"), CvarWidget::Slider(0.0, 2.0));
        assert!(c.cvar("levels").unwrap().integer && !c.cvar("grav").unwrap().integer);
    }

    #[test]
    fn the_command_opens_closes_and_picks_tabs() {
        let mut app = App::new();
        app.add_plugins((crate::console::ConsolePlugin, DebugUiStatePlugin));
        let run = |app: &mut App, line: &str| {
            app.world_mut().resource_mut::<Console>().submit(line);
            app.update();
            let ui = app.world().resource::<DebugUi>();
            (ui.open, ui.tab)
        };
        assert_eq!(run(&mut app, "debugui"), (true, Tab::Player));
        assert_eq!(run(&mut app, "debugui"), (false, Tab::Player));
        assert_eq!(run(&mut app, "debugui perf"), (true, Tab::Perf));
        assert_eq!(run(&mut app, "debugui close"), (false, Tab::Perf));
        run(&mut app, "debugui nope");
        let out = &app.world().resource::<Console>().output;
        assert!(out.last().unwrap().text.contains("no tab"), "{:?}", out.last());
    }
}
