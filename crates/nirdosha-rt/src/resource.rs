//! `resource(kind = "..")` enforcement (issue #69): [`acquire`]/[`release`]
//! are the two recognized mint points a `nirdosha-driver` MIR dataflow
//! pass looks for by resolved `DefId`, mirroring how `Auth::prove` is
//! the one recognized mint point `requires(role = "..")` looks for.
//!
//! Under plain cargo these are ordinary, do-nothing wrappers — no Drop
//! glue, no runtime check, `Resource<T>` derefs straight through to
//! `T`. Under `cargo nirdosha build --deep` (Stage 2), a fn claiming
//! `resource(kind = "..")` gets every path from an `acquire` call to
//! its matching `release` call proven, the same "every path" semantics
//! `requires`/`ensures` (issue #68) already use.

use std::ops::{Deref, DerefMut};

/// A value obtained from [`acquire`]. Transparent at runtime; its only
/// job under plain cargo is being a distinct, greppable type. Under
/// Stage 2 it is the tracked local the dataflow pass follows.
pub struct Resource<T>(T);

impl<T> Deref for Resource<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Resource<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Mint a tracked resource handle. Every local this call's result flows
/// into must reach a matching [`release`] on every path out of a
/// `resource(kind = "..")`-claiming fn — checked by
/// `nirdosha-driver`'s resource dataflow pass, not by this function.
pub fn acquire<T>(value: T) -> Resource<T> {
    Resource(value)
}

/// Consume a tracked resource handle, discharging the acquire/release
/// obligation [`acquire`] created.
pub fn release<T>(resource: Resource<T>) -> T {
    resource.0
}
