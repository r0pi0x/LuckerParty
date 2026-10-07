//! Physics props' motion for the logic layer (specs/source/prop_damage.md
//! 4 step 13, 8; specs/cs_source/physics_props.md 3.1, 3.5): a frozen prop
//! ("motion disabled") stays put like a static prop until its motion is
//! enabled, then becomes a simulated body; bodies can be put to sleep and
//! woken; a prop that started asleep reports its first wake
//! (`PropAwakened`, Source OnAwakened). A round restart puts each back as
//! it spawned.

use avian3d::prelude::*;
use bevy::{ecs::system::SystemParam, prelude::*};

use super::{MapBrushCollider, MapPhysics, MapPropCollider, PhysicsProp};
use crate::core::MovingSolid;

/// The simulated body of a physics prop with these settings.
pub fn dynamic_body(p: &MapPhysics, bounds: (Vec3, Vec3)) -> impl Bundle {
    (
        RigidBody::Dynamic,
        Mass(p.mass),
        Friction::new(p.friction),
        Restitution::new(p.elasticity),
        LinearDamping(p.damping),
        AngularDamping(p.rotdamping),
        // Source clamps every body to 2000 units/s and 3600 deg/s.
        MaxLinearSpeed(2000.0 * 0.0254),
        MaxAngularSpeed(3600f32.to_radians()),
        PhysicsProp {
            push: p.push,
            mass: p.mass,
            bounds,
        },
        MapPropCollider,
        // Impact sounds and damage listen for its contacts.
        CollisionEventsEnabled,
    )
}

/// A physics prop placed frozen: the body it becomes when its motion is
/// enabled (`enabled` once it has), and what it was while frozen.
#[derive(Component, Clone, Debug)]
pub struct FrozenBody {
    pub physics: MapPhysics,
    pub bounds: (Vec3, Vec3),
}

/// A frozen prop now moving: its still brushes, kept for a round restart.
#[derive(Component, Clone, Debug)]
pub struct Unfrozen {
    solid: Option<MovingSolid>,
    brush_collider: bool,
}

/// A prop that spawned asleep: held still (a static body) until something
/// wakes it (the Wake input, a push from damage, a moving body running
/// into it), which reports `PropAwakened`. (Avian's own sleep can't be
/// set before a body joins the simulation, and props placed touching
/// each other would wake each other on the first step.)
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct StartAsleep;

/// A prop that started asleep woke up (its node).
#[derive(Message, Clone, Copy, Debug)]
pub struct PropAwakened(pub Entity);

/// Set a frozen prop moving; false if it isn't one (or already moves).
pub fn enable_motion(world: &mut World, node: Entity) -> bool {
    let Ok(mut e) = world.get_entity_mut(node) else {
        return false;
    };
    if e.contains::<Unfrozen>() {
        // Disabled again by DisableMotion: just let it move.
        if e.get::<RigidBody>().is_some_and(|b| b.is_static()) {
            e.insert(RigidBody::Dynamic);
            return true;
        }
        return false;
    }
    let Some(frozen) = e.get::<FrozenBody>().cloned() else {
        if e.get::<RigidBody>().is_some_and(|b| b.is_static()) && e.contains::<PhysicsProp>() {
            e.insert(RigidBody::Dynamic);
            return true;
        }
        return false;
    };
    let solid = e.take::<MovingSolid>();
    let brush_collider = e.take::<MapBrushCollider>().is_some();
    // Shown wherever it goes: no fixed visibility clusters.
    e.remove::<super::vis::VisClusters>();
    e.insert((
        Unfrozen { solid, brush_collider },
        dynamic_body(&frozen.physics, frozen.bounds),
        Visibility::Inherited,
    ));
    true
}

/// Freeze a moving prop where it is.
pub fn disable_motion(world: &mut World, node: Entity) {
    if let Ok(mut e) = world.get_entity_mut(node)
        && e.get::<RigidBody>().is_some_and(|b| b.is_dynamic())
    {
        e.insert(RigidBody::Static);
    }
}

/// Wake a sleeping body (one that started asleep reports it).
pub fn wake(world: &mut World, node: Entity) {
    let Ok(mut e) = world.get_entity_mut(node) else { return };
    if e.take::<StartAsleep>().is_some() {
        e.insert(RigidBody::Dynamic);
        world.write_message(PropAwakened(node));
    } else if e.contains::<Sleeping>() {
        e.remove::<Sleeping>();
    }
}

/// Whether a prop node is still asleep since it spawned.
pub fn asleep(world: &World, node: Entity) -> bool {
    world.get::<StartAsleep>(node).is_some()
}

/// Put a body to sleep.
pub fn sleep(world: &mut World, node: Entity) {
    if let Ok(mut e) = world.get_entity_mut(node)
        && e.get::<RigidBody>().is_some_and(|b| b.is_dynamic())
        && !e.contains::<Sleeping>()
    {
        e.insert(Sleeping);
    }
}

/// A round restart: a prop that was frozen is frozen again (its still
/// brushes back), one that started asleep sleeps again and will report
/// its next wake.
pub fn reset(world: &mut World, node: Entity, asleep: bool) {
    let Ok(mut e) = world.get_entity_mut(node) else { return };
    if let Some(u) = e.take::<Unfrozen>() {
        e.remove::<PhysicsProp>().insert(RigidBody::Static);
        if let Some(s) = u.solid {
            e.insert(s);
        }
        if u.brush_collider {
            e.insert(MapBrushCollider);
        }
    } else if e.contains::<FrozenBody>() && e.get::<RigidBody>().is_some_and(|b| b.is_dynamic()) {
        e.insert(RigidBody::Static);
    }
    if asleep && e.contains::<PhysicsProp>() {
        e.insert((StartAsleep, RigidBody::Static));
    }
}

/// A moving body running into a prop that sleeps since it spawned wakes
/// it.
pub(super) fn wake_on_contact(
    mut started: MessageReader<CollisionStart>,
    asleep: Query<(), With<StartAsleep>>,
    bodies: Query<(&RigidBody, &LinearVelocity)>,
    mut commands: Commands,
    mut awakened: MessageWriter<PropAwakened>,
) {
    for s in started.read() {
        let (b1, b2) = (s.body1.unwrap_or(s.collider1), s.body2.unwrap_or(s.collider2));
        for (me, other) in [(b1, b2), (b2, b1)] {
            let moving = bodies
                .get(other)
                .is_ok_and(|(rb, v)| rb.is_dynamic() && v.0.length_squared() > WAKE_SPEED * WAKE_SPEED);
            if moving && asleep.contains(me) {
                commands.entity(me).remove::<StartAsleep>().insert(RigidBody::Dynamic);
                awakened.write(PropAwakened(me));
            }
        }
    }
}

/// A body slower than this (m/s) doesn't wake what it touches.
const WAKE_SPEED: f32 = 0.05;

/// A player's physics shadow (specs/cs_source/physics_props.md 4.1 step 3):
/// an invisible 85 kg body the size of the player's box that the game
/// drives to the player each tick. It is what touches physics props for
/// the player: it pushes the light ones and holds up the ones resting on
/// or against the player. It is on its own layer (`core::SHADOW_LAYER`),
/// which no spatial query sees. Inactive (a dead player's), it touches
/// nothing.
#[derive(Component, Clone, Copy, Debug)]
pub struct PhysicsShadow {
    pub owner: Entity,
    pub active: bool,
}

/// A character whose contacts with physics props go through its
/// `PhysicsShadow` (this entity): its own collider never touches them.
#[derive(Component, Clone, Copy, Debug)]
pub struct Shadowed(pub Entity);

/// The shadow's mass, kg (spec 4.1 step 3).
pub const SHADOW_MASS: f32 = 85.0;
/// The heaviest prop a shadow pushes, kg (spec: the player push mass
/// limit). Heavier props never touch the shadow: the player's movement
/// treats them as walls, and the character's own collider (kinematic)
/// stops them when they move into the player.
pub const SHADOW_PUSH_MASS: f32 = 350.0;

/// Which pairs players' shadows and shadowed characters keep.
#[derive(SystemParam)]
pub struct ShadowContacts<'w, 's> {
    shadows: Query<'w, 's, &'static PhysicsShadow>,
    shadowed: Query<'w, 's, (), With<Shadowed>>,
    of: Query<'w, 's, &'static ColliderOf>,
    props: Query<'w, 's, &'static PhysicsProp>,
}

impl ShadowContacts<'_, '_> {
    /// Some(keep) when collider `a` or `b` is a shadow or a shadowed
    /// character: an active shadow touches only the `prop_physics` bodies
    /// it can push (multiplayer props are in a group players never touch,
    /// spec 4.2.1); a shadowed character touches only the heavier ones.
    /// None: neither is one.
    pub fn filter(&self, a: Entity, b: Entity) -> Option<bool> {
        let body = |e: Entity| self.of.get(e).map_or(e, |o| o.body);
        let (a, b) = (body(a), body(b));
        for (me, other) in [(a, b), (b, a)] {
            if let Ok(s) = self.shadows.get(me) {
                return Some(s.active && self.props.get(other).is_ok_and(pushable));
            }
        }
        for (me, other) in [(a, b), (b, a)] {
            if self.shadowed.contains(me)
                && let Ok(p) = self.props.get(other)
            {
                return Some(p.push == super::PushAway::Collide && !pushable(p));
            }
        }
        None
    }
}

/// Whether players' shadows push this prop: a `prop_physics` (not a
/// multiplayer one) up to `SHADOW_PUSH_MASS`.
pub fn pushable(p: &PhysicsProp) -> bool {
    p.push == super::PushAway::Collide && p.mass <= SHADOW_PUSH_MASS
}
