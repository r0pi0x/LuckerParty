//! The bomb (specs/cs_source/objectives.md 1-6): one carrier per round,
//! arming it inside a bomb target by holding primary fire, the planted
//! bomb's timer and beeps, defusing with +use (faster with a kit), the
//! explosion (a `core::Explosion` with the map's bomb radius) and the bomb
//! target's outputs (`map::entities::FireEntityOutput`).

use avian3d::prelude::*;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    MapKind, MapObjectives, ObjectiveEvent, RoundOpen, UNIT,
    use_search::{UseTarget, User, find_use},
};
use crate::{
    console::resource_cvar,
    core::{
        Died, Explosion, FirstTimePredicted, Health, Intent, MovementState, NetRole, Predict, PredictedAppExt,
        RoundRestarts, SimClock, SimSet, Team,
    },
    map::{
        GameSound, PlaySound, PredictedSound,
        interp::NetDrawn,
        entities::FireEntityOutput,
        loose::{LooseItem, ShownItem, SitOnOrigin},
    },
    weapon::{Inventory, Weapon, WeaponEvent, WeaponEventKind, WeaponFrame, drop, economy::DefuseKit},
};

/// Timers due exactly on a tick fire on it (as the weapon frame's).
const TIME_SLACK: f64 = 1e-5;

/// Weapon part: this weapon is the bomb. Its primary attack arms it
/// (no shots); it can't be bought.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct C4;

/// Sound entries the bomb uses (the game's names; None: silent).
#[derive(Clone, Debug, Default)]
pub struct BombSounds {
    /// Each key press while arming (`clicks`).
    pub click: Option<String>,
    pub plant: Option<String>,
    pub beep: Option<String>,
    pub defuse_start: Option<String>,
    pub defuse_finish: Option<String>,
    pub explode: Option<String>,
    /// Buying or picking up a defusal kit.
    pub kit: Option<String>,
    /// The announcer, heard by everyone (no position).
    pub announce_planted: Option<String>,
    pub announce_defused: Option<String>,
}

/// The bomb's rules: CS:S's numbers by default (spec Constants), the
/// weapon, models and sounds from the game.
#[derive(Resource, Clone, Debug)]
pub struct BombRules {
    /// The weapon handed out (None: no bomb, e.g. no game registered one).
    pub weapon: Option<&'static str>,
    pub carrier_team: Team,
    pub defuser_team: Team,
    /// Seconds primary is held in a target until it's planted.
    pub plant_time: f32,
    /// `mp_c4timer`: seconds from planting to the explosion (read at the
    /// plant).
    pub timer: f32,
    pub defuse_time: f32,
    pub defuse_time_kit: f32,
    /// Bomb radius when the map gives none, units (blast damage at the
    /// centre).
    pub radius: f32,
    /// The blast reaches this many times the radius (*hyp.* H1).
    pub range_factor: f32,
    /// Seconds into arming of each key press (`sounds.click`).
    pub clicks: Vec<f32>,
    /// What the key presses type on the bomb's little screen, a character
    /// each (spec Constants, `plant_vm_code`), and when the screen masks
    /// it with `*`s, seconds into arming.
    pub code: String,
    pub code_masked: f32,
    pub sounds: BombSounds,
    /// Held-model keys: the planted bomb, a dropped defusal kit.
    pub planted_model: Option<String>,
    pub kit_model: Option<String>,
    /// Turns the planted model to lie as it should (its model's own up).
    pub planted_turn: Quat,
    /// The planted bomb's box for the use search (relative to where it
    /// sits, meters).
    pub use_box: (Vec3, Vec3),
}

impl Default for BombRules {
    fn default() -> Self {
        Self {
            weapon: None,
            carrier_team: Team(1),
            defuser_team: Team(2),
            plant_time: 3.0,
            timer: 45.0,
            defuse_time: 10.0,
            defuse_time_kit: 5.0,
            radius: 500.0,
            range_factor: 3.5,
            clicks: vec![0.9, 1.2333, 1.5, 1.7, 1.9, 2.1, 2.2333],
            code: "7355608".into(),
            code_masked: 2.6,
            sounds: BombSounds::default(),
            planted_model: None,
            kit_model: None,
            planted_turn: Quat::IDENTITY,
            use_box: (Vec3::new(-8.0, 0.0, -8.0) * UNIT, Vec3::new(8.0, 8.0, 8.0) * UNIT),
        }
    }
}

impl BombRules {
    /// The bomb's screen `elapsed` seconds into arming: the code typed so
    /// far (a character per key press), then all `*`s (spec Constants:
    /// "7", "73", ... "7355608", "*******" at 2.6 s).
    pub fn screen_text(&self, elapsed: f32) -> String {
        if elapsed >= self.code_masked {
            return "*".repeat(self.code.chars().count());
        }
        let typed = self.clicks.iter().filter(|t| **t <= elapsed).count();
        self.code.chars().take(typed).collect()
    }
}

/// On a character arming the bomb: since when (`SimClock` time), the
/// bomb (this process's entity: not sent) and key presses heard.
/// Predicted (`arm`): a network client runs it for its own player and the
/// server's copy comes with that player's state.
#[derive(Component, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Arming {
    pub since: f64,
    #[serde(skip, default = "no_entity")]
    pub weapon: Entity,
    clicks: u32,
}

fn no_entity() -> Entity {
    Entity::PLACEHOLDER
}

/// On a character defusing the planted bomb (it holds still): when it
/// started and ends (`SimClock` time), with a kit. The bomb's `Defuse` is
/// the server's record; this is the defuser's, which its network client
/// predicts (`predict_defuse`).
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Defusing {
    pub started: f64,
    pub ends: f64,
    pub kit: bool,
}

/// Arming and defusing holding a character still (`hold_still`): a
/// network client's commands are recorded after it (they replay held).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BombHold;

/// A defuse in progress: who, when it started and ends, with a kit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Defuse {
    pub who: Entity,
    pub started: f64,
    pub ends: f64,
    pub kit: bool,
}

/// The planted bomb (spec 4).
#[derive(Component, Clone, Debug)]
pub struct PlantedBomb {
    pub planter: Option<Entity>,
    /// The bomb target (index into `MapObjectives::bomb_targets`).
    pub site: Option<usize>,
    pub planted_at: f64,
    pub explode_at: f64,
    /// `mp_c4timer` when it was planted.
    pub timer: f32,
    pub next_beep: f64,
    pub defuse: Option<Defuse>,
    pub defused: bool,
}

/// The beep interval with `remaining` of `timer` seconds left (*hyp.*,
/// spec 4's provisional rule: 1 s after the plant, 0.1 s at the end).
pub fn beep_interval(remaining: f32, timer: f32) -> f32 {
    0.1 + 0.9 * (remaining / timer.max(0.001)).clamp(0.0, 1.0)
}

/// How the round's bomb ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BombOutcome {
    Exploded,
    Defused,
}

/// This round's bomb: the planted one, and how it ended.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct BombState {
    pub planted: Option<Entity>,
    pub planter: Option<Entity>,
    pub outcome: Option<BombOutcome>,
}

/// A defusal kit lying where its owner died.
#[derive(Component, Clone, Copy, Debug)]
pub struct LooseKit;

/// Last tick's fire and use buttons (press edges).
#[derive(Clone, Copy, Debug, Default)]
pub struct Buttons {
    pub fire: bool,
    pub use_key: bool,
}

/// Each character's buttons last tick (a resource, not a component: a
/// component added to every character would reorder their archetypes and
/// so the order other systems see them in).
#[derive(Resource, Default)]
pub struct LastButtons(pub std::collections::HashMap<Entity, Buttons>);

impl LastButtons {
    pub fn get(&self, e: Entity) -> Buttons {
        self.0.get(&e).copied().unwrap_or_default()
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<BombRules>()
        .init_resource::<BombState>()
        .init_resource::<LastButtons>()
        .add_message::<PredictedSound>()
        // Arming (and on a client its defusing) runs with the weapon frame,
        // so a client predicts it like its shots (`core::Predict`).
        .add_systems(
            Predict::Weapons,
            (arm, predict_defuse.run_if(resource_equals(NetRole::Client)))
                .chain()
                .after(crate::weapon::WeaponFrame),
        )
        .predicted_net::<Arming>()
        .predicted_net::<Defusing>()
        // Everywhere: a client holds its own player while it predicts it
        // arming or defusing.
        .add_systems(
            FixedUpdate,
            hold_still
                .in_set(BombHold)
                .after(SimSet::Rules)
                .after(crate::weapon::SelectWeapons)
                .before(SimSet::Movement),
        )
        .add_systems(
            FixedUpdate,
            (
                (
                    defuse,
                    tick_bomb,
                    kits,
                    carrier_events,
                    super::hostages::use_hostages,
                    remember_buttons,
                )
                    .chain()
                    .after(WeaponFrame)
                    .in_set(SimSet::Weapons),
                on_death.after(SimSet::Weapons),
            )
                // Objectives are the server's.
                .run_if(crate::core::authoritative),
        );
    resource_cvar::<BombRules, f32>(
        app,
        "mp_c4timer",
        "Seconds from when the C4 is armed until it blows.",
        |r| &mut r.timer,
    );
}

/// Arming and defusing hold the character still (*hyp.*, Q1, Q7).
fn hold_still(mut intents: Query<&mut Intent, Or<(With<Arming>, With<Defusing>)>>) {
    for mut i in &mut intents {
        i.move_axis = Vec2::ZERO;
        i.jump = false;
    }
}

/// A character's hull, engine space.
pub fn hull(t: &Transform, state: Option<&MovementState>) -> (Vec3, Vec3) {
    match state.filter(|s| s.hull_min != s.hull_max) {
        Some(s) => (t.translation + s.hull_min, t.translation + s.hull_max),
        None => {
            let h = Vec3::new(
                crate::character::CAPSULE_RADIUS,
                crate::character::CAPSULE_HEIGHT / 2.0,
                crate::character::CAPSULE_RADIUS,
            );
            (t.translation - h, t.translation + h)
        }
    }
}

/// The weapon entity of the bomb `owner` carries, if any.
pub fn carried_bomb(world: &World, owner: Entity) -> Option<Entity> {
    let inv = world.get::<Inventory>(owner)?;
    inv.weapons.iter().copied().find(|w| world.get::<C4>(*w).is_some())
}

enum ArmStep {
    Start(Entity),
    Continue,
    Abort { left_zone: bool },
    Refuse(&'static str),
    Plant(Entity, Option<usize>),
}

/// Primary fire with the bomb in hand (spec 3): arming, its key presses,
/// stopping short, the plant. With the weapon frame (`core::Predict::
/// Weapons`): a network client predicts it for its own player (the
/// progress, the hold, the view model's key presses and their sounds, the
/// bomb leaving its hands) live and in replays; the server's state of
/// that player corrects it. What only the server does (the planted bomb,
/// the HUD's words, the rules' sounds) happens where `authoritative`.
pub fn arm(world: &mut World) {
    let now = world.resource::<SimClock>().now;
    let authority = world.get_resource::<NetRole>() != Some(&NetRole::Client);
    let first = world.get_resource::<FirstTimePredicted>().is_none_or(|f| f.0);
    let rules = world.resource::<BombRules>().clone();
    let open = world.get_resource::<RoundOpen>().is_none_or(|o| o.0);
    let mut steps: Vec<(Entity, ArmStep)> = Vec::new();
    {
        let mut q = world.query_filtered::<(
            Entity,
            &Intent,
            &Inventory,
            &Transform,
            Option<&MovementState>,
            Option<&Health>,
            Option<&Arming>,
        ), Without<NetDrawn>>();
        let objectives = world.resource::<MapObjectives>();
        let last = world.resource::<LastButtons>();
        for (e, intent, inv, t, state, health, arming) in q.iter(world) {
            let bomb = inv.active.filter(|w| world.get::<C4>(*w).is_some());
            let alive = health.is_none_or(|h| h.current > 0.0);
            let Some(bomb) = bomb.filter(|_| alive) else {
                if arming.is_some() {
                    steps.push((e, ArmStep::Abort { left_zone: false }));
                }
                continue;
            };
            let (lo, hi) = hull(t, state);
            let site = objectives.bomb_target_at(lo, hi);
            let ground = state.is_none_or(|s| s.on_ground);
            let pressed = intent.fire && !last.get(e).fire;
            match arming {
                Some(_) if !intent.fire || !open || !ground => steps.push((e, ArmStep::Abort { left_zone: false })),
                Some(_) if site.is_none() => steps.push((e, ArmStep::Abort { left_zone: true })),
                Some(a) if now - a.since + TIME_SLACK >= rules.plant_time as f64 => {
                    steps.push((e, ArmStep::Plant(bomb, site)))
                }
                Some(_) => steps.push((e, ArmStep::Continue)),
                None if !intent.fire || !open || now + TIME_SLACK < inv.next_attack => {}
                None if site.is_none() => {
                    if pressed {
                        steps.push((e, ArmStep::Refuse("C4 must be planted at a bomb site.")));
                    }
                }
                None if !ground => {
                    if pressed {
                        steps.push((
                            e,
                            ArmStep::Refuse("You must be standing on the ground to plant the C4."),
                        ));
                    }
                }
                None => steps.push((e, ArmStep::Start(bomb))),
            }
        }
    }
    for (e, step) in steps {
        match step {
            ArmStep::Start(weapon) => {
                world.entity_mut(e).insert(Arming {
                    since: now,
                    weapon,
                    clicks: 0,
                });
                if authority {
                    world.write_message(ObjectiveEvent::BeginPlant { who: e });
                }
                world.write_message(WeaponEvent {
                    owner: e,
                    weapon,
                    kind: WeaponEventKind::ArmingStarted,
                    replay: !first,
                });
            }
            ArmStep::Continue => {
                let Some(mut a) = world.get_mut::<Arming>(e) else {
                    continue;
                };
                let elapsed = (now - a.since) as f32;
                let due = rules
                    .clicks
                    .iter()
                    .filter(|t| **t <= elapsed + TIME_SLACK as f32)
                    .count() as u32;
                let new = due.saturating_sub(a.clicks);
                a.clicks = due;
                if new > 0
                    && first
                    && let Some(s) = &rules.sounds.click
                {
                    let at = world.get::<Transform>(e).map(|t| t.translation);
                    let sound = PlaySound {
                        pitch: None,
                        entry: s.clone(),
                        at,
                        volume: None,
                        source: Some(e),
                        channel: Some(crate::weapon::CHAN_WEAPON),
                    };
                    own_sound(world, authority, e, sound);
                }
            }
            ArmStep::Abort { left_zone } => {
                if let Some(a) = world.entity_mut(e).take::<Arming>() {
                    if authority {
                        world.write_message(ObjectiveEvent::AbortPlant { who: e, left_zone });
                    }
                    let weapon = carried_bomb(world, e).unwrap_or(a.weapon);
                    world.write_message(WeaponEvent {
                        owner: e,
                        weapon,
                        kind: WeaponEventKind::ArmingStopped,
                        replay: !first,
                    });
                }
            }
            ArmStep::Refuse(reason) => {
                if authority {
                    world.write_message(ObjectiveEvent::PlantRefused { who: e, reason });
                }
            }
            ArmStep::Plant(weapon, site) => {
                take_bomb(world, e, weapon);
                if authority {
                    plant(world, e, site, now, &rules);
                }
            }
        }
    }
}

/// A sound of a character's own predicted action (a key press, starting
/// a defuse): the authority plays it and sends it to every client but the
/// actor's (`map::PredictedSound`); a client plays its own at once.
fn own_sound(world: &mut World, authority: bool, by: Entity, sound: PlaySound) {
    if authority {
        world.write_message(PredictedSound { by, sound });
    } else {
        world.write_message(sound);
    }
}

/// The bomb leaves `who`'s hands (planted): out of its inventory, the
/// last weapon (else the best) wanted. Predicted with `arm`.
fn take_bomb(world: &mut World, who: Entity, weapon: Entity) {
    world.entity_mut(who).remove::<Arming>();
    // The bomb goes; the last weapon (else the best) comes back up.
    if let Some(mut inv) = world.get_mut::<Inventory>(who) {
        inv.weapons.retain(|w| *w != weapon);
        if inv.active == Some(weapon) {
            inv.active = None;
        }
        let last = inv.last.filter(|l| inv.weapons.contains(l));
        inv.last = None;
        inv.wanted = last;
    }
    let fallback = {
        let inv = world.get::<Inventory>(who);
        inv.and_then(|inv| {
            inv.weapons
                .iter()
                .copied()
                .min_by_key(|w| world.get::<Weapon>(*w).map_or(u8::MAX, |w| w.slot))
        })
    };
    if let Some(mut inv) = world.get_mut::<Inventory>(who)
        && inv.wanted.is_none()
    {
        inv.wanted = fallback;
    }
    if let Ok(w) = world.get_entity_mut(weapon) {
        w.despawn();
    }
}

/// The bomb sits at `who`'s feet, ticking (the server's; `take_bomb` took
/// it out of its hands).
fn plant(world: &mut World, who: Entity, site: Option<usize>, now: f64, rules: &BombRules) {
    let Some(t) = world.get::<Transform>(who).copied() else {
        return;
    };
    let (lo, _) = hull(&t, world.get::<MovementState>(who));
    let feet = t.translation.with_y(lo.y);
    let yaw = world.get::<Intent>(who).map_or(0.0, |i| i.yaw);
    let timer = rules.timer;
    let mut bomb = world.spawn((
        Name::new("Planted bomb"),
        PlantedBomb {
            planter: Some(who),
            site,
            planted_at: now,
            explode_at: now + timer as f64,
            timer,
            next_beep: now,
            defuse: None,
            defused: false,
        },
        Transform::from_translation(feet).with_rotation(Quat::from_rotation_y(yaw)),
    ));
    if let Some(model) = &rules.planted_model {
        bomb.insert((ShownItem(model.clone()), SitOnOrigin(rules.planted_turn)));
    }
    let bomb = bomb.id();
    {
        let mut state = world.resource_mut::<BombState>();
        state.planted = Some(bomb);
        state.planter = Some(who);
    }
    if let Some(s) = &rules.sounds.plant {
        world.write_message(GameSound(PlaySound::at(s.clone(), feet)));
    }
    if let Some(s) = &rules.sounds.announce_planted {
        world.write_message(GameSound(PlaySound::ui(s.clone())));
    }
    world.write_message(ObjectiveEvent::Planted { who, at: feet, site });
    fire_site(world, site, "BombPlanted", Some(who));
}

/// The bomb target's output, if the map gave it one.
fn fire_site(world: &mut World, site: Option<usize>, output: &str, activator: Option<Entity>) {
    let index = site.and_then(|s| world.resource::<MapObjectives>().bomb_targets.get(s)?.map_index);
    if let Some(map_index) = index {
        world.write_message(FireEntityOutput {
            map_index,
            output: output.into(),
            activator,
        });
    }
}

/// The use search for the planted bomb from a character.
pub fn user_of(t: &Transform, intent: &Intent, state: Option<&MovementState>) -> User {
    let (lo, hi) = hull(t, state);
    let eye = t.translation + state.map_or(Vec3::Y * 0.7, |s| s.eye_offset);
    let look = intent.look_rotation();
    User {
        eye,
        forward: look * Vec3::NEG_Z,
        up: look * Vec3::Y,
        feet: lo.y,
        head: hi.y,
    }
}

/// +use on the planted bomb by the defending team (spec 5).
#[allow(clippy::type_complexity)]
fn defuse(
    users: Query<(
        Entity,
        &Intent,
        &Transform,
        Option<&MovementState>,
        Option<&Health>,
        Option<&Team>,
        Has<DefuseKit>,
    )>,
    last: Res<LastButtons>,
    characters: Query<(), With<Intent>>,
    mut bombs: Query<(Entity, &mut PlantedBomb, &Transform), Without<Intent>>,
    spatial: SpatialQuery,
    rules: Res<BombRules>,
    mut events: MessageWriter<ObjectiveEvent>,
    mut play: MessageWriter<PredictedSound>,
    mut commands: Commands,
    clock: Res<SimClock>,
) {
    let now = clock.now;
    let filter = SpatialQueryFilter::default().with_mask(crate::core::SOLID_LAYERS);
    let clear = |a: Vec3, b: Vec3| -> bool {
        let d = b - a;
        let Ok(dir) = Dir3::new(d) else { return true };
        spatial
            .cast_ray_predicate(a, dir, d.length(), true, &filter, &|e| !characters.contains(e))
            .is_none()
    };
    for (bomb_e, mut bomb, bt) in &mut bombs {
        if bomb.defused {
            continue;
        }
        let target = UseTarget {
            entity: bomb_e,
            lo: bt.translation + rules.use_box.0,
            hi: bt.translation + rules.use_box.1,
        };
        let finds = |t: &Transform, i: &Intent, s: Option<&MovementState>| {
            find_use(&user_of(t, i, s), &[target], &clear) == Some(bomb_e)
        };
        // The one defusing keeps at it, or stops (no progress kept).
        if let Some(d) = bomb.defuse {
            let going = users.get(d.who).is_ok_and(|(_, i, t, s, h, ..)| {
                h.is_none_or(|h| h.current > 0.0) && i.use_key && s.is_none_or(|s| s.on_ground) && finds(t, i, s)
            });
            if !going {
                bomb.defuse = None;
                commands.entity(d.who).try_remove::<Defusing>();
                events.write(ObjectiveEvent::AbortDefuse { who: d.who });
            }
        }
        for (e, intent, t, state, health, team, kit) in &users {
            if !intent.use_key || health.is_some_and(|h| h.current <= 0.0) || team != Some(&rules.defuser_team) {
                continue;
            }
            if bomb.defuse.is_some_and(|d| d.who == e) {
                continue;
            }
            let pressed = !last.get(e).use_key;
            if !finds(t, intent, state) {
                continue;
            }
            if bomb.defuse.is_some() {
                if pressed {
                    events.write(ObjectiveEvent::DefuseRefused {
                        who: e,
                        reason: "The bomb is already being defused.",
                    });
                }
                continue;
            }
            if !state.is_none_or(|s| s.on_ground) {
                if pressed {
                    events.write(ObjectiveEvent::DefuseRefused {
                        who: e,
                        reason: "You must be on the ground to defuse the bomb.",
                    });
                }
                continue;
            }
            let length = if kit { rules.defuse_time_kit } else { rules.defuse_time };
            let d = Defuse {
                who: e,
                started: now,
                ends: now + length as f64,
                kit,
            };
            bomb.defuse = Some(d);
            commands.entity(e).insert(Defusing {
                started: d.started,
                ends: d.ends,
                kit,
            });
            // The defuser's client played it when it predicted the start.
            if let Some(s) = &rules.sounds.defuse_start {
                play.write(PredictedSound {
                    by: e,
                    sound: defuse_sound(s, bt.translation, bomb_e),
                });
            }
            events.write(ObjectiveEvent::BeginDefuse { who: e, kit });
        }
    }
}

/// On a network client's copy of the planted bomb: it predicted its own
/// player finishing the defuse (until the server says so, no new one
/// starts).
#[derive(Component, Clone, Copy, Debug)]
pub struct DefusedHere;

/// The sound of a defuse starting, from the bomb.
fn defuse_sound(entry: &str, at: Vec3, bomb: Entity) -> PlaySound {
    PlaySound {
        pitch: None,
        entry: entry.to_string(),
        at: Some(at),
        volume: None,
        source: Some(bomb),
        channel: None,
    }
}

/// A network client's own player defusing the planted bomb, predicted as
/// the server's `defuse` and `tick_bomb` decide it for that player: it
/// starts with +use on the bomb (alive, a defender, on the ground, nobody
/// else at it), holds while it keeps at it, and stops when it lets go,
/// leaves the bomb or the time is up (the defuse itself, the bomb's state
/// and the HUD's words are the server's). In the weapon frame's place, so
/// replays run it too.
#[allow(clippy::type_complexity)]
fn predict_defuse(
    users: Query<
        (
            Entity,
            &Intent,
            &Transform,
            Option<&MovementState>,
            Option<&Health>,
            Option<&Team>,
            Has<DefuseKit>,
            Option<&Defusing>,
        ),
        Without<NetDrawn>,
    >,
    characters: Query<(), With<Intent>>,
    bombs: Query<(Entity, &PlantedBomb, &Transform, Has<DefusedHere>), Without<Intent>>,
    spatial: SpatialQuery,
    rules: Res<BombRules>,
    clock: Res<SimClock>,
    first: Res<FirstTimePredicted>,
    mut play: MessageWriter<PlaySound>,
    mut commands: Commands,
) {
    let now = clock.now;
    let filter = SpatialQueryFilter::default().with_mask(crate::core::SOLID_LAYERS);
    let clear = |a: Vec3, b: Vec3| -> bool {
        let d = b - a;
        let Ok(dir) = Dir3::new(d) else { return true };
        spatial
            .cast_ray_predicate(a, dir, d.length(), true, &filter, &|e| !characters.contains(e))
            .is_none()
    };
    let bomb = bombs.iter().find(|(_, b, _, done)| !b.defused && !done).map(|(e, b, t, _)| (e, b, t));
    for (e, intent, t, state, health, team, kit, defusing) in &users {
        let alive = health.is_none_or(|h| h.current > 0.0);
        let finds = bomb.is_some_and(|(bomb_e, _, bt)| {
            let target = UseTarget {
                entity: bomb_e,
                lo: bt.translation + rules.use_box.0,
                hi: bt.translation + rules.use_box.1,
            };
            find_use(&user_of(t, intent, state), &[target], &clear) == Some(bomb_e)
        });
        let ground = state.is_none_or(|s| s.on_ground);
        if let Some(d) = defusing {
            let going = alive && intent.use_key && ground && finds;
            let done = now + TIME_SLACK >= d.ends;
            if done && going && let Some((bomb_e, ..)) = bomb {
                // Defused (the server's word comes later): not again.
                commands.entity(bomb_e).insert(DefusedHere);
            }
            if !going || done {
                commands.entity(e).remove::<Defusing>();
            }
            continue;
        }
        let Some((bomb_e, b, bt)) = bomb else { continue };
        let free = b.defuse.is_none_or(|d| d.who == e);
        if intent.use_key && alive && team == Some(&rules.defuser_team) && finds && ground && free {
            let length = if kit { rules.defuse_time_kit } else { rules.defuse_time };
            commands.entity(e).insert(Defusing {
                started: now,
                ends: now + length as f64,
                kit,
            });
            if first.0
                && let Some(s) = &rules.sounds.defuse_start
            {
                play.write(defuse_sound(s, bt.translation, bomb_e));
            }
        }
    }
}

/// The planted bomb: a finished defuse (checked first, spec Q14), beeps,
/// the explosion.
#[allow(clippy::too_many_arguments)]
fn tick_bomb(
    mut bombs: Query<(Entity, &mut PlantedBomb, &Transform)>,
    rules: Res<BombRules>,
    objectives: Res<MapObjectives>,
    mut state: ResMut<BombState>,
    mut events: MessageWriter<ObjectiveEvent>,
    mut play: MessageWriter<GameSound>,
    mut explosions: MessageWriter<Explosion>,
    mut outputs: MessageWriter<FireEntityOutput>,
    mut commands: Commands,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let due = now + TIME_SLACK;
    for (e, mut bomb, t) in &mut bombs {
        if bomb.defused {
            continue;
        }
        let at = t.translation;
        let site_output = |output: &str| {
            bomb.site
                .and_then(|s| objectives.bomb_targets.get(s)?.map_index)
                .map(|map_index| FireEntityOutput {
                    map_index,
                    output: output.into(),
                    activator: bomb.planter,
                })
        };
        if let Some(d) = bomb.defuse
            && due >= d.ends
        {
            let out = site_output("BombDefused");
            bomb.defused = true;
            bomb.defuse = None;
            commands.entity(d.who).try_remove::<Defusing>();
            state.outcome = Some(BombOutcome::Defused);
            if let Some(s) = &rules.sounds.defuse_finish {
                play.write(GameSound(PlaySound::at(s.clone(), at)));
            }
            if let Some(s) = &rules.sounds.announce_defused {
                play.write(GameSound(PlaySound::ui(s.clone())));
            }
            events.write(ObjectiveEvent::Defused { who: d.who });
            outputs.write_batch(out);
            continue;
        }
        if due >= bomb.explode_at {
            let out = site_output("BombExplode");
            if let Some(d) = bomb.defuse {
                commands.entity(d.who).try_remove::<Defusing>();
            }
            let radius = objectives.bomb_radius.unwrap_or(rules.radius);
            explosions.write(Explosion {
                origin: at + Vec3::Y * 8.0 * UNIT,
                damage: radius * 0.01,
                radius: radius * rules.range_factor * UNIT,
                attacker: bomb.planter,
                inflictor: Some(e),
                sound: rules.sounds.explode.clone(),
                weapon: rules.weapon,
            });
            outputs.write_batch(out);
            state.outcome = Some(BombOutcome::Exploded);
            state.planted = None;
            events.write(ObjectiveEvent::Exploded { at });
            commands.entity(e).despawn();
            continue;
        }
        if due >= bomb.next_beep {
            let remaining = (bomb.explode_at - now) as f32;
            bomb.next_beep = now + beep_interval(remaining, bomb.timer) as f64;
            if let Some(s) = &rules.sounds.beep {
                play.write(GameSound(PlaySound {
                    pitch: None,
                    entry: s.clone(),
                    at: Some(at),
                    volume: None,
                    source: Some(e),
                    channel: None,
                }));
            }
            events.write(ObjectiveEvent::Beep { at });
        }
    }
}

/// Defusal kits: the sound when one is bought, picking up a dropped one
/// (defenders without one).
#[allow(clippy::type_complexity)]
fn kits(
    added: Query<(Entity, &Transform), Added<DefuseKit>>,
    loose: Query<(Entity, &Transform), With<LooseKit>>,
    takers: Query<(Entity, &Transform, &Health, Option<&Team>), (With<Intent>, Without<DefuseKit>)>,
    rules: Res<BombRules>,
    mut events: MessageWriter<ObjectiveEvent>,
    mut play: MessageWriter<GameSound>,
    mut commands: Commands,
) {
    for (_, t) in &added {
        if let Some(s) = &rules.sounds.kit {
            play.write(GameSound(PlaySound::at(s.clone(), t.translation)));
        }
    }
    let mut taken = Vec::new();
    for (kit, kt) in &loose {
        for (e, t, h, team) in &takers {
            if h.current <= 0.0 || team != Some(&rules.defuser_team) || taken.contains(&e) {
                continue;
            }
            let d = kt.translation - t.translation;
            if d.xz().length() > 0.6 || d.y.abs() > 1.1 {
                continue;
            }
            taken.push(e);
            commands.entity(kit).despawn();
            commands.entity(e).insert(DefuseKit);
            events.write(ObjectiveEvent::PickedUpKit { who: e });
            break;
        }
    }
}

/// The bomb changing hands: dropped, picked up.
fn carrier_events(
    bombs: Query<(Entity, &Weapon), With<C4>>,
    mut owners: Local<std::collections::HashMap<Entity, Option<Entity>>>,
    mut events: MessageWriter<ObjectiveEvent>,
) {
    let mut now = std::collections::HashMap::new();
    for (e, w) in &bombs {
        match (owners.get(&e).copied(), w.owner) {
            (Some(Some(who)), None) => {
                events.write(ObjectiveEvent::DroppedBomb { who });
            }
            (Some(None), Some(who)) => {
                events.write(ObjectiveEvent::PickedUpBomb { who });
            }
            _ => {}
        }
        now.insert(e, w.owner);
    }
    *owners = now;
}

/// Remember this tick's fire and use buttons (press edges next tick).
pub(super) fn remember_buttons(q: Query<(Entity, &Intent)>, mut last: ResMut<LastButtons>) {
    last.0.clear();
    for (e, i) in &q {
        last.0.insert(
            e,
            Buttons {
                fire: i.fire,
                use_key: i.use_key,
            },
        );
    }
}

/// The dead drop the bomb (whatever they held) and their kit, and stop
/// arming.
fn on_death(mut died: MessageReader<Died>, mut commands: Commands) {
    for d in died.read() {
        let who = d.entity;
        commands.queue(move |w: &mut World| drop_objective_items(w, who));
    }
}

/// `who` leaves play (killed, or off to the spectators): it stops arming
/// and drops the bomb and its defusal kit where it stands.
pub fn drop_objective_items(w: &mut World, who: Entity) {
    if let Some(a) = w.entity_mut(who).take::<Arming>() {
        w.write_message(WeaponEvent {
            owner: who,
            weapon: a.weapon,
            kind: WeaponEventKind::ArmingStopped,
            replay: false,
        });
    }
    if let Some(bomb) = carried_bomb(w, who) {
        drop::drop_this(w, who, bomb, false);
    }
    if w.entity_mut(who).take::<DefuseKit>().is_some()
        && let Some(t) = w.get::<Transform>(who).copied()
    {
        let model = w.resource::<BombRules>().kit_model.clone();
        let mut kit = w.spawn((
            Name::new("Defusal kit"),
            LooseKit,
            Transform::from_translation(t.translation),
        ));
        if let Some(m) = model {
            kit.insert(LooseItem(m));
        }
    }
}

/// A new round's bomb (spec 1): last round's bombs and kits go, one
/// random living carrier-team member gets the bomb on a bomb map.
pub(super) fn round_start(world: &mut World) {
    let gone: Vec<Entity> = world
        .query_filtered::<Entity, Or<(With<PlantedBomb>, With<LooseKit>, With<C4>)>>()
        .iter(world)
        .collect();
    for e in gone {
        world.despawn(e);
    }
    let holders: Vec<Entity> = world
        .query_filtered::<Entity, Or<(With<Arming>, With<Defusing>)>>()
        .iter(world)
        .collect();
    for e in holders {
        world.entity_mut(e).remove::<(Arming, Defusing)>();
    }
    // Inventories forget the bombs just removed.
    let owners: Vec<Entity> = world.query_filtered::<Entity, With<Inventory>>().iter(world).collect();
    for o in owners {
        let Some(inv) = world.get::<Inventory>(o) else { continue };
        let exists = |w: &Entity| world.get_entity(*w).is_ok();
        let keep: Vec<Entity> = inv.weapons.iter().copied().filter(exists).collect();
        let [a, l, w] = [inv.active, inv.last, inv.wanted].map(|s| s.filter(exists));
        let mut inv = world.get_mut::<Inventory>(o).unwrap();
        inv.weapons = keep;
        (inv.active, inv.last, inv.wanted) = (a, l, w);
    }
    *world.resource_mut::<BombState>() = BombState::default();
    let rules = world.resource::<BombRules>().clone();
    let Some(id) = rules.weapon else { return };
    if world.resource::<MapObjectives>().kind() != MapKind::Bomb {
        return;
    }
    let mut candidates: Vec<Entity> = world
        .query_filtered::<(Entity, &Team, &Health), With<Intent>>()
        .iter(world)
        .filter(|(_, t, h)| **t == rules.carrier_team && h.current > 0.0)
        .map(|(e, ..)| e)
        .collect();
    if candidates.is_empty() {
        return;
    }
    candidates.sort();
    let round = world.get_resource::<RoundRestarts>().map_or(0, |r| r.0 as u64);
    let mut x = round.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ candidates.len() as u64 ^ 0xD1B5_4A32_D192_ED03;
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    let who = candidates[(x % candidates.len() as u64) as usize];
    // Into its slot, not drawn.
    let wanted = world.get::<Inventory>(who).and_then(|i| i.wanted);
    let had_active = world
        .get::<Inventory>(who)
        .is_some_and(|i| i.active.is_some() || i.wanted.is_some());
    if crate::weapon::give(world, who, id).is_some() {
        if had_active && let Some(mut inv) = world.get_mut::<Inventory>(who) {
            inv.wanted = wanted;
        }
        world.write_message(ObjectiveEvent::GotBomb { who });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_types_the_code_then_masks_it() {
        let r = BombRules::default();
        assert_eq!(r.screen_text(0.0), "");
        assert_eq!(r.screen_text(0.9), "7");
        assert_eq!(r.screen_text(1.3), "73");
        assert_eq!(r.screen_text(2.0), "73556");
        assert_eq!(r.screen_text(2.3), "7355608");
        assert_eq!(r.screen_text(2.6), "*******");
        assert_eq!(r.screen_text(3.0), "*******");
    }

    #[test]
    fn beeps_speed_up_from_one_second_to_a_tenth() {
        // O28.
        assert!((beep_interval(45.0, 45.0) - 1.0).abs() < 1e-6);
        assert!((beep_interval(22.5, 45.0) - 0.55).abs() < 1e-6);
        assert!((beep_interval(0.0, 45.0) - 0.1).abs() < 1e-6);
        let (mut t, mut n) = (0.0f32, 0);
        while t < 45.0 {
            n += 1;
            t += beep_interval(45.0 - t, 45.0);
        }
        assert!((110..=120).contains(&n), "{n} beeps");
    }

    #[test]
    fn blast_falls_off_over_three_and_a_half_radii() {
        use crate::weapon::grenade::blast_damage;
        // O24, O26, O27 (H1).
        let h1 = |d: f32, r: f32| blast_damage(d, d * 3.5, r);
        for (dist, want) in [
            (0.0, 500.0),
            (500.0, 357.14),
            (1000.0, 214.29),
            (1500.0, 71.43),
            (1750.0, 0.0),
        ] {
            assert!((h1(500.0, dist) - want).abs() < 0.01, "{dist}");
        }
        for (dist, want) in [(500.0, 257.14), (1000.0, 114.29), (1500.0, 0.0)] {
            assert!((h1(400.0, dist) - want).abs() < 0.01, "{dist}");
        }
        for (dist, want) in [(250.0, 228.57), (500.0, 157.14), (1000.0, 14.29)] {
            assert!((h1(300.0, dist) - want).abs() < 0.01, "{dist}");
        }
    }
}
