//! Weapons in a network game (docs/plans/active/multiplayer.md, slice 4).
//!
//! - **A client's own weapons** are predicted with its movement: the
//!   server's state of what its player carries goes in `OwnState` with
//!   the rest (`weapon::sync`); the client fires, reloads, switches and
//!   zooms at once and shows its own shots' effects the first time a
//!   command runs. It deals no damage: the server traces the same command
//!   with lag compensation (`weapon::lagcomp`, the view tick each
//!   `NetCmd` carries) and applies the hits; health comes back
//!   replicated.
//! - **Others' weapons** (`NetHeld`): what each character holds, so a
//!   client draws it in their hands (`weapon::remote::show_held`); each
//!   round they fire comes as `FireBullets` (origin, angles, spread seed
//!   and spread) and is traced again on the client for its tracers,
//!   impacts and sound (`weapon::remote::RemoteShot`), the same spread
//!   pattern the server traced; reloads, swings and throws as `WeaponFx`
//!   for their bodies. The shooter's own client gets neither (it
//!   predicted them).
//! - **Things in the world** (`NetItem`): loose weapons and grenades in
//!   flight are the server's and drawn from snapshots like props; smoke
//!   clouds (`NetSmoke`) start when heard; a detonation comes as
//!   `Detonation` (the explosion's look and sound), a flash or a blast's
//!   ringing as `Senses` to the player it hit.
//! - **Dropping** (`drop`, G): a request to the server (`DropRequest`);
//!   picking up is the server's (walking over, +use).

use avian3d::prelude::LinearVelocity;
use bevy::{app::RunFixedMainLoopSystems, prelude::*};
use bevy_replicon::prelude::*;

use super::{
    Detonation, DropRequest, FireBullets, HitConfirm, Killed, NetCharacter, NetHeld, NetItem, NetSmoke, Senses,
    WeaponFx,
    interp::{Around, InterpClock, Snapshots},
    server::Player,
    weapon_fx,
};
use crate::{
    core::{Blinded, Damage, DamageKind, Deafened, Died, Hitgroup, LocalPlayer, NetRole},
    map::{
        PlaySound,
        interp::{InterpSystems, NetDrawn, SNAP_SPEED},
        loose::{LooseItem, ShownItem},
    },
    weapon::{
        AltModes, Inventory, SpreadShape, Weapon, WeaponEvent, WeaponEventKind, WeaponRegistry,
        drop::DropRequested,
        grenade::{Detonated, GrenadeEffect, GrenadeKind, Projectile, SmokeCloud, Throwable},
        remote::{RemoteAction, RemoteShot, show_held},
    },
};

pub(super) fn plugin(app: &mut App) {
    let server = || resource_equals(NetRole::Server);
    let client = || resource_equals(NetRole::Client);
    app.init_resource::<Templates>()
        .add_observer(item_arrived)
        .add_systems(FixedLast, (write_held, write_items, write_smokes).run_if(server()))
        .add_systems(
            PreUpdate,
            receive_drops
                .after(ServerSystems::Receive)
                .run_if(in_state(ServerState::Running)),
        )
        .add_systems(
            PostUpdate,
            (send_weapon_events, send_detonations, send_senses, send_deaths)
                .before(ServerSystems::Send)
                .run_if(server()),
        )
        .add_systems(
            PreUpdate,
            (
                apply_held,
                receive_fire,
                receive_fx,
                receive_detonations,
                receive_senses,
                start_smokes,
                receive_deaths,
                receive_hits,
            )
                .chain()
                .after(ClientSystems::Receive)
                .run_if(client()),
        )
        .add_systems(PostUpdate, send_drops.before(ClientSystems::Send).run_if(client()))
        .add_systems(
            RunFixedMainLoop,
            draw_items
                .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop)
                .after(InterpSystems::Ease)
                .after(super::interp::draw_others)
                .run_if(client()),
        );
}

/// A weapon of each kind with no owner, built from the registry, for
/// what a client needs to know of a weapon nobody here carries (a
/// grenade's effect).
#[derive(Resource, Default)]
struct Templates(std::collections::HashMap<&'static str, Entity>);

fn template(world: &mut World, id: &'static str) -> Option<Entity> {
    if let Some(e) = world.resource::<Templates>().0.get(id).copied()
        && world.get_entity(e).is_ok()
    {
        return Some(e);
    }
    let build = world.resource::<WeaponRegistry>().find(id)?.build;
    let mut e = world.spawn(Name::new(format!("Template {id}")));
    build(&mut e);
    let e = e.id();
    world.resource_mut::<Templates>().0.insert(id, e);
    Some(e)
}

/// Left the server: the templates go.
pub(super) fn reset(world: &mut World) {
    let templates: Vec<Entity> = world.resource_mut::<Templates>().0.drain().map(|(_, e)| e).collect();
    for e in templates {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
}

fn weapon_id(world: &World, index: u16) -> Option<&'static str> {
    world.resource::<WeaponRegistry>().0.get(index as usize).map(|d| d.id)
}

// --- Server

/// What each character holds into `NetHeld`.
#[allow(clippy::type_complexity)]
fn write_held(
    mut q: Query<(Entity, Option<&Inventory>, Option<&mut NetHeld>), With<NetCharacter>>,
    weapons: Query<(&Weapon, Option<&AltModes>, Option<&Throwable>)>,
    registry: Res<WeaponRegistry>,
    mut commands: Commands,
) {
    for (e, inv, held) in &mut q {
        let active = inv.and_then(|i| i.active).and_then(|w| weapons.get(w).ok());
        let now = NetHeld {
            weapon: active.and_then(|(w, ..)| registry.index(w.id)).map(|i| i as u16),
            mode: active.and_then(|(_, m, _)| m).map_or(0, |m| m.current),
            primed: active.and_then(|(.., t)| t).is_some_and(|t| t.primed()),
        };
        match held {
            Some(mut h) => {
                h.set_if_neq(now);
            }
            None => {
                commands.entity(e).insert(now);
            }
        }
    }
}

/// Loose weapons and grenades in flight into `NetItem` (replicated from
/// the first one on).
#[allow(clippy::type_complexity)]
fn write_items(
    mut q: Query<
        (
            Entity,
            &Transform,
            Option<&LooseItem>,
            Option<&Projectile>,
            Option<&LinearVelocity>,
            Option<&mut NetItem>,
        ),
        Or<(With<LooseItem>, With<Projectile>)>,
    >,
    mut commands: Commands,
) {
    for (e, t, loose, projectile, v, have) in &mut q {
        let (model, velocity) = match (loose, projectile) {
            (Some(l), _) => (l.0.clone(), v.map_or(Vec3::ZERO, |v| v.0)),
            (None, Some(p)) => (p.weapon.to_string(), p.velocity),
            _ => continue,
        };
        let now = NetItem {
            model,
            origin: t.translation.to_array(),
            rotation: t.rotation.to_array(),
            velocity: velocity.to_array(),
        };
        match have {
            Some(mut have) => {
                have.set_if_neq(now);
            }
            None => {
                commands.entity(e).insert((Replicated, now));
            }
        }
    }
}

/// Smoke clouds into `NetSmoke` (once: a client runs its own copy).
fn write_smokes(
    q: Query<(Entity, &SmokeCloud), Without<NetSmoke>>,
    registry: Res<WeaponRegistry>,
    weapons: Query<&Weapon>,
    projectiles: Query<&Projectile>,
    time: Res<Time>,
    mut commands: Commands,
) {
    for (e, cloud) in &q {
        // The grenade it came from (it lies in the cloud), else the first
        // smoke grenade the registry has.
        let id = cloud
            .grenade
            .and_then(|g| projectiles.get(g).ok().map(|p| p.weapon).or_else(|| weapons.get(g).ok().map(|w| w.id)));
        let index = id.and_then(|id| registry.index(id)).unwrap_or(0);
        commands.entity(e).insert((
            Replicated,
            NetSmoke {
                centre: cloud.centre.to_array(),
                weapon: index as u16,
                age: (time.elapsed_secs_f64() - cloud.started) as f32,
            },
        ));
    }
}

/// A client asked to drop what it holds.
fn receive_drops(mut requests: MessageReader<FromClient<DropRequest>>, players: Query<&Player>, mut commands: Commands) {
    for r in requests.read() {
        let Some(client) = r.client_id.entity() else { continue };
        let Ok(p) = players.get(client) else { continue };
        let character = p.character;
        commands.queue(move |w: &mut World| {
            crate::weapon::drop::drop_weapon(w, character, true);
        });
    }
}

/// What a character's weapon did, to every client but its own.
#[allow(clippy::type_complexity)]
fn send_weapon_events(
    mut events: MessageReader<WeaponEvent>,
    players: Query<(Entity, &Player)>,
    weapons: Query<&Weapon>,
    replicated: Query<(), (With<Replicated>, With<NetCharacter>)>,
    registry: Res<WeaponRegistry>,
    mut fire: MessageWriter<ToClients<FireBullets>>,
    mut fx: MessageWriter<ToClients<WeaponFx>>,
    mut hits: MessageWriter<ToClients<HitConfirm>>,
) {
    for e in events.read().filter(|e| e.shown()) {
        if !replicated.contains(e.owner) {
            continue;
        }
        // A hit, to the shooter's own client (its hit marker).
        if let WeaponEventKind::Hit {
            target,
            amount,
            hitgroup,
        } = e.kind
        {
            for (client, _) in players.iter().filter(|(_, p)| p.character == e.owner) {
                hits.write(ToClients {
                    targets: SendTargets::Single(ClientId::Client(client)),
                    message: HitConfirm {
                        target,
                        amount,
                        hitgroup: hitgroup_number(hitgroup),
                    },
                });
            }
            continue;
        }
        let others = players
            .iter()
            .filter(|(_, p)| p.character != e.owner)
            .map(|(c, _)| SendTargets::Single(ClientId::Client(c)));
        let kind = match &e.kind {
            WeaponEventKind::Fired {
                origin,
                yaw,
                pitch,
                seed,
                spread,
                mode,
            } => {
                let Some(index) = weapons.get(e.weapon).ok().and_then(|w| registry.index(w.id)) else {
                    continue;
                };
                let (disc, inaccuracy, spread) = match *spread {
                    SpreadShape::Disc { inaccuracy, spread } => (true, inaccuracy, spread),
                    SpreadShape::Template(s) => (false, s, 0.0),
                };
                let message = FireBullets {
                    shooter: e.owner,
                    weapon: index as u16,
                    origin: origin.to_array(),
                    yaw: *yaw,
                    pitch: *pitch,
                    seed: *seed,
                    disc,
                    inaccuracy,
                    spread,
                    mode: *mode,
                };
                for targets in others {
                    fire.write(ToClients {
                        targets,
                        message: message.clone(),
                    });
                }
                continue;
            }
            WeaponEventKind::ReloadStarted => weapon_fx::RELOAD,
            WeaponEventKind::ShellInserting => weapon_fx::SHELL,
            WeaponEventKind::Reloaded => weapon_fx::RELOADED,
            WeaponEventKind::Swing { secondary: false, .. } => weapon_fx::SWING,
            WeaponEventKind::Swing { secondary: true, .. } => weapon_fx::SWING2,
            WeaponEventKind::PinPulled => weapon_fx::PIN,
            WeaponEventKind::Thrown => weapon_fx::THROWN,
            _ => continue,
        };
        for targets in others {
            fx.write(ToClients {
                targets,
                message: WeaponFx { owner: e.owner, kind },
            });
        }
    }
}

/// Grenades going off, to every client.
fn send_detonations(
    mut detonated: MessageReader<Detonated>,
    registry: Res<WeaponRegistry>,
    mut out: MessageWriter<ToClients<Detonation>>,
) {
    for d in detonated.read() {
        out.write(ToClients {
            targets: SendTargets::CLIENTS_ONLY,
            message: Detonation {
                kind: match d.kind {
                    GrenadeKind::Blast => 0,
                    GrenadeKind::Flash => 1,
                    GrenadeKind::Smoke => 2,
                },
                weapon: registry.index(d.weapon).unwrap_or(0) as u16,
                at: d.at.to_array(),
                ground: d.ground.map(|(p, n, _)| (p.to_array(), n.to_array())),
            },
        });
    }
}

/// Blindness and hearing effects to the player they hit.
fn send_senses(
    players: Query<(Entity, &Player)>,
    blinded: Query<Ref<Blinded>>,
    mut deafened: MessageReader<Deafened>,
    time: Res<Time>,
    mut out: MessageWriter<ToClients<Senses>>,
) {
    let now = time.elapsed_secs_f64();
    let mut hearing: Vec<(Entity, crate::core::HearingEffect)> = deafened.read().map(|d| (d.target, d.effect)).collect();
    for (client, p) in &players {
        let blind = blinded
            .get(p.character)
            .ok()
            .filter(|b| b.is_changed() && b.end > now)
            .map(|b| (b.alpha, (b.fade_start - now).max(0.0) as f32, (b.end - b.fade_start.max(now)) as f32));
        let heard = hearing
            .iter()
            .position(|(t, _)| *t == p.character)
            .map(|i| hearing.swap_remove(i).1);
        if blind.is_none() && heard.is_none() {
            continue;
        }
        out.write(ToClients {
            targets: SendTargets::Single(ClientId::Client(client)),
            message: Senses { blind, hearing: heard },
        });
    }
}

/// Deaths, to every client (kill feed, ragdolls).
fn send_deaths(
    mut died: MessageReader<Died>,
    replicated: Query<(), With<Replicated>>,
    registry: Res<WeaponRegistry>,
    mut out: MessageWriter<ToClients<Killed>>,
) {
    for d in died.read() {
        if !replicated.contains(d.entity) {
            continue;
        }
        let damage = &d.damage;
        out.write(ToClients {
            targets: SendTargets::CLIENTS_ONLY,
            message: Killed {
                victim: d.entity,
                attacker: d.attacker.filter(|a| replicated.contains(*a)),
                weapon: damage.weapon.and_then(|w| registry.index(w)).map(|i| i as u16),
                hitgroup: hitgroup_number(damage.hitgroup),
                kind: kind_number(damage.kind),
                point: damage.point.to_array(),
                dir: damage.dir.to_array(),
                force: damage.force.to_array(),
            },
        });
    }
}

const HITGROUPS: [Hitgroup; 8] = [
    Hitgroup::Generic,
    Hitgroup::Head,
    Hitgroup::Chest,
    Hitgroup::Stomach,
    Hitgroup::LeftArm,
    Hitgroup::RightArm,
    Hitgroup::LeftLeg,
    Hitgroup::RightLeg,
];

const KINDS: [DamageKind; 7] = [
    DamageKind::Generic,
    DamageKind::Bullet,
    DamageKind::Melee,
    DamageKind::Blast,
    DamageKind::Crush,
    DamageKind::Fall,
    DamageKind::Burn,
];

fn hitgroup_number(g: Hitgroup) -> u8 {
    HITGROUPS.iter().position(|x| *x == g).unwrap_or(0) as u8
}

fn kind_number(k: DamageKind) -> u8 {
    KINDS.iter().position(|x| *x == k).unwrap_or(0) as u8
}

// --- Client

/// Deaths the server saw: the kill feed and the ragdoll here
/// (`core::Died`; nothing is subtracted, health comes replicated).
fn receive_deaths(mut killed: MessageReader<Killed>, mut commands: Commands) {
    for k in killed.read() {
        let k = k.clone();
        commands.queue(move |w: &mut World| {
            if w.get_entity(k.victim).is_err() {
                return;
            }
            let weapon = k.weapon.and_then(|i| weapon_id(w, i));
            w.write_message(Died {
                entity: k.victim,
                attacker: k.attacker,
                damage: Damage {
                    target: k.victim,
                    attacker: k.attacker,
                    amount: 0.0,
                    point: Vec3::from_array(k.point),
                    dir: Vec3::from_array(k.dir),
                    hitgroup: HITGROUPS.get(k.hitgroup as usize).copied().unwrap_or_default(),
                    kind: KINDS.get(k.kind as usize).copied().unwrap_or_default(),
                    weapon,
                    force: Vec3::from_array(k.force),
                },
            });
        });
    }
}

/// Our hits the server confirmed, as weapon events of our player.
fn receive_hits(
    mut hits: MessageReader<HitConfirm>,
    me: Option<Single<(Entity, Option<&Inventory>), With<LocalPlayer>>>,
    mut events: MessageWriter<WeaponEvent>,
) {
    let Some((me, inv)) = me.map(|m| *m) else {
        hits.clear();
        return;
    };
    let weapon = inv.and_then(|i| i.active).unwrap_or(me);
    for h in hits.read() {
        events.write(WeaponEvent {
            owner: me,
            weapon,
            kind: WeaponEventKind::Hit {
                target: h.target,
                amount: h.amount,
                hitgroup: HITGROUPS.get(h.hitgroup as usize).copied().unwrap_or_default(),
            },
            replay: false,
        });
    }
}

/// What others hold, in their hands here.
#[allow(clippy::type_complexity)]
fn apply_held(
    q: Query<(Entity, Ref<NetHeld>, Option<&Inventory>), (With<NetDrawn>, Without<LocalPlayer>)>,
    mut commands: Commands,
) {
    for (e, held, inv) in &q {
        if !held.is_changed() && inv.is_some() {
            continue;
        }
        let h: NetHeld = (*held).clone();
        commands.queue(move |w: &mut World| {
            let id = h.weapon.and_then(|i| weapon_id(w, i));
            show_held(w, e, id, h.mode, h.primed);
        });
    }
}

/// Others' rounds, to trace again here.
fn receive_fire(mut fire: MessageReader<FireBullets>, mut commands: Commands) {
    for f in fire.read() {
        let f = f.clone();
        commands.queue(move |w: &mut World| {
            let Some(id) = weapon_id(w, f.weapon) else { return };
            if w.get_entity(f.shooter).is_err() || w.get::<LocalPlayer>(f.shooter).is_some() {
                return;
            }
            // What it holds may not have arrived yet.
            let primed = w.get::<NetHeld>(f.shooter).is_some_and(|h| h.primed);
            show_held(w, f.shooter, Some(id), f.mode, primed);
            w.write_message(RemoteShot {
                shooter: f.shooter,
                weapon: id,
                origin: Vec3::from_array(f.origin),
                yaw: f.yaw,
                pitch: f.pitch,
                seed: f.seed,
                spread: if f.disc {
                    SpreadShape::Disc {
                        inaccuracy: f.inaccuracy,
                        spread: f.spread,
                    }
                } else {
                    SpreadShape::Template(f.inaccuracy)
                },
                mode: f.mode,
            });
        });
    }
}

/// Others' reloads, swings and throws.
fn receive_fx(mut fx: MessageReader<WeaponFx>, mut out: MessageWriter<RemoteAction>) {
    for f in fx.read() {
        let kind = match f.kind {
            weapon_fx::RELOAD => WeaponEventKind::ReloadStarted,
            weapon_fx::SHELL => WeaponEventKind::ShellInserting,
            weapon_fx::RELOADED => WeaponEventKind::Reloaded,
            weapon_fx::SWING | weapon_fx::SWING2 => WeaponEventKind::Swing {
                hit: false,
                secondary: f.kind == weapon_fx::SWING2,
                at: None,
                line: (Vec3::ZERO, Vec3::ZERO),
            },
            weapon_fx::PIN => WeaponEventKind::PinPulled,
            weapon_fx::THROWN => WeaponEventKind::Thrown,
            _ => continue,
        };
        out.write(RemoteAction { shooter: f.owner, kind });
    }
}

/// Grenades going off: their look (`Detonated`) and sound.
fn receive_detonations(mut detonations: MessageReader<Detonation>, mut commands: Commands) {
    for d in detonations.read() {
        let d = d.clone();
        commands.queue(move |w: &mut World| {
            let Some(id) = weapon_id(w, d.weapon) else { return };
            let effect = template(w, id).and_then(|t| w.get::<Throwable>(t)).map(|t| t.effect.clone());
            let at = Vec3::from_array(d.at);
            let (kind, sound, shake) = match &effect {
                Some(GrenadeEffect::Blast(b)) => (GrenadeKind::Blast, b.sound.clone(), b.shake),
                Some(GrenadeEffect::Flash(f)) => (GrenadeKind::Flash, f.sound.clone(), None),
                Some(GrenadeEffect::Smoke(s)) => (GrenadeKind::Smoke, s.sound.clone(), None),
                None => return,
            };
            w.write_message(Detonated {
                kind,
                weapon: id,
                thrower: None,
                at,
                ground: d
                    .ground
                    .map(|(p, n)| (Vec3::from_array(p), Vec3::from_array(n), Entity::PLACEHOLDER)),
                projectile: Entity::PLACEHOLDER,
                shake,
            });
            if let Some(s) = sound {
                w.write_message(PlaySound::at(s, at));
            }
        });
    }
}

/// Our player blinded or deafened.
fn receive_senses(
    mut senses: MessageReader<Senses>,
    me: Option<Single<Entity, With<LocalPlayer>>>,
    time: Res<Time>,
    mut deafened: MessageWriter<Deafened>,
    mut commands: Commands,
) {
    let Some(me) = me.map(|m| *m) else {
        senses.clear();
        return;
    };
    let now = time.elapsed_secs_f64();
    for s in senses.read() {
        if let Some((alpha, hold, fade)) = s.blind {
            commands.entity(me).insert(Blinded {
                alpha,
                fade_start: now + hold as f64,
                end: now + (hold + fade) as f64,
            });
        }
        if let Some(effect) = s.hearing {
            deafened.write(Deafened { target: me, effect });
        }
    }
}

/// Smoke clouds heard of: run here from the grenade's rule.
fn start_smokes(q: Query<(Entity, &NetSmoke), Without<SmokeCloud>>, mut commands: Commands) {
    for (e, s) in &q {
        let s = s.clone();
        commands.queue(move |w: &mut World| {
            let Some(id) = weapon_id(w, s.weapon) else { return };
            let Some(GrenadeEffect::Smoke(smoke)) = template(w, id)
                .and_then(|t| w.get::<Throwable>(t))
                .map(|t| t.effect.clone())
            else {
                return;
            };
            let now = w.resource::<Time>().elapsed_secs_f64();
            if let Ok(mut ent) = w.get_entity_mut(e) {
                ent.insert(SmokeCloud {
                    centre: Vec3::from_array(s.centre),
                    started: now - s.age as f64,
                    smoke,
                    grenade: None,
                });
            }
        });
    }
}

/// Ask the server to drop what we hold (`drop` on a client).
fn send_drops(mut asked: MessageReader<DropRequested>, mut out: MessageWriter<DropRequest>) {
    for _ in asked.read() {
        out.write(DropRequest);
    }
}

/// A loose weapon or grenade from the server: drawn with its model.
fn item_arrived(add: On<Add, NetItem>, q: Query<&NetItem>, role: Option<Res<NetRole>>, mut commands: Commands) {
    if role.as_deref() != Some(&NetRole::Client) {
        return;
    }
    let Ok(item) = q.get(add.entity) else { return };
    commands.entity(add.entity).insert((
        ShownItem(item.model.clone()),
        Transform::from_translation(Vec3::from_array(item.origin))
            .with_rotation(Quat::from_array(item.rotation).normalize()),
        NetDrawn,
    ));
}

/// Loose weapons and grenades where the server had them at the render
/// time.
fn draw_items(
    clock: Res<InterpClock>,
    command_clock: Res<super::predict::CommandClock>,
    fixed: Res<Time<Fixed>>,
    mut q: Query<(&mut Snapshots<NetItem>, &mut Transform)>,
) {
    if !clock.running {
        return;
    }
    let at = clock.render_tick;
    let step = super::interp::tick_step(&command_clock, &fixed) as f32;
    for (mut buf, mut t) in &mut q {
        buf.prune(at);
        let pose = match buf.around(at) {
            Around::Empty => continue,
            Around::Before((_, p)) | Around::After((_, p)) => pose_of(p),
            Around::Between((ta, a), (tb, b), f) => {
                let (pa, pb) = (pose_of(a), pose_of(b));
                if pa.translation.distance(pb.translation) > SNAP_SPEED * step * (tb - ta) as f32 {
                    pb
                } else {
                    Transform {
                        translation: pa.translation.lerp(pb.translation, f),
                        rotation: pa.rotation.slerp(pb.rotation, f),
                        scale: Vec3::ONE,
                    }
                }
            }
        };
        let pose = Transform { scale: t.scale, ..pose };
        if *t != pose {
            *t = pose;
        }
    }
}

fn pose_of(p: &NetItem) -> Transform {
    Transform {
        translation: Vec3::from_array(p.origin),
        rotation: Quat::from_array(p.rotation).normalize(),
        scale: Vec3::ONE,
    }
}
