//! Replicated values applied in tick order, held-back ones too.
//!
//! replicon (0.44.3) holds a client's mutations back until the (reliable)
//! update they follow has arrived, acknowledging them on arrival; when
//! the update comes, it applies each entity's newest held-back mutation
//! and, by default, drops the older ones as outdated. The server, having
//! their acknowledgements, leaves the values they carried out of the
//! mutations it builds afterwards, so a value changed once on an entity
//! whose body changes every tick (a dead player's last health, a score)
//! was lost for good on a lossy link (found by the soak,
//! tests/it/heavy/net_soak.rs; test `net::a_value_survives_held_back_
//! mutations`).
//!
//! Here the client keeps them: a receive marker on every replicated
//! entity (replicon's `ConfirmHistory`) asks replicon for the older
//! mutations too (`need_history`), and each component's write takes a
//! value only if it is no older than the one it holds (`newest`: the
//! message tick each component was last written from, per entity). A
//! value an older mutation carries that no newer one did is applied, as
//! if it had arrived in order; one a newer message replaced is not.
//!
//! Limit: replicon passes older mutations within 64 ticks of the
//! entity's newest (its confirmation mask), so an update held up longer
//! than a second (three resends in a row lost at 300 ms) still loses a
//! value changed once before it (docs/tech-debt.md, net).

use std::marker::PhantomData;

use bevy::{ecs::component::Mutable, prelude::*};
use bevy_replicon::{
    bytes::Bytes,
    client::confirm_history::ConfirmHistory,
    prelude::*,
    shared::{
        replication::{
            deferred_entity::DeferredEntity,
            receive_markers::MarkerConfig,
            registry::{
                ctx::{RemoveCtx, WriteCtx},
                receive_fns::{MutWrite, RemoveFn, WriteFn},
            },
            storage::EntityStorageCtx,
        },
        replicon_tick::RepliconTick,
    },
};

/// The message tick a component's value was last written from (replicon's
/// per-entity storage, dropped with the entity's replication).
struct Written<C>(RepliconTick, PhantomData<fn() -> C>);

/// Whether a value of `C` from this message is to be written: no older
/// than the value written last; for a component the entity doesn't have,
/// not from a message older than the entity's newest (an older mutation
/// would put back a component a later update removed). Notes the tick.
pub fn newest<C: Component>(ctx: &mut WriteCtx, entity: &DeferredEntity) -> bool {
    let tick = ctx.message_tick;
    if !entity.contains::<C>()
        && entity
            .get::<ConfirmHistory>()
            .is_some_and(|h| tick.is_older(h.last_tick()))
    {
        return false;
    }
    match ctx.get_mut::<Written<C>>() {
        Some(w) if tick.is_older(w.0) => false,
        Some(w) => {
            w.0 = tick;
            true
        }
        None => {
            ctx.insert(Written::<C>(tick, PhantomData));
            true
        }
    }
}

/// Replicon's write for a plain component, in tick order (`newest`).
pub fn write<C: Component<Mutability = Mutable>>(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<C>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    let value: C = rule_fns.deserialize(ctx, message)?;
    if !newest::<C>(ctx, entity) {
        return Ok(());
    }
    if let Some(mut c) = entity.get_mut::<C>() {
        *c = value;
    } else {
        entity.insert(value);
    }
    Ok(())
}

fn remove<C: Component>(_ctx: &mut RemoveCtx, entity: &mut DeferredEntity) {
    entity.remove::<C>();
}

/// The marker asking for older mutations, on every replicated entity.
pub(super) fn plugin(app: &mut App) {
    app.register_marker_with::<ConfirmHistory>(MarkerConfig {
        need_history: true,
        ..default()
    });
}

/// Extension: replicate `C` with its values applied in tick order.
pub trait OrderedAppExt {
    /// `C`'s values in tick order, written with `write` (and `remove`):
    /// `ordered::write` for a plain component.
    fn ordered_with<C: Component<Mutability: MutWrite<C>>>(&mut self, write: WriteFn<C>, remove: RemoveFn)
    -> &mut Self;

    /// A plain component's values in tick order.
    fn ordered<C: Component<Mutability = Mutable>>(&mut self) -> &mut Self;
}

impl OrderedAppExt for App {
    fn ordered_with<C: Component<Mutability: MutWrite<C>>>(
        &mut self,
        write: WriteFn<C>,
        remove: RemoveFn,
    ) -> &mut Self {
        self.set_marker_fns::<ConfirmHistory, C>(write, remove)
    }

    fn ordered<C: Component<Mutability = Mutable>>(&mut self) -> &mut Self {
        self.set_marker_fns::<ConfirmHistory, C>(write::<C>, remove::<C>)
    }
}
