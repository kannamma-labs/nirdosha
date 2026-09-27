//! RFC 0029 §7.1 runtime capability + effect gateway.
//!
//! The generation-time admission gate (`cargo-nirdosha`'s
//! `run_service_admission`) proves a screen's `[screen.service]` wiring was
//! authorized once, at build time. It says nothing about what happens per
//! live HTTP request. This crate closes that gap: a [`CapabilityIssuer`]
//! mints a short-lived [`DecisionCapability`] bound to one
//! subject/resource/effect under a governed [`PolicyBundle`]'s validity
//! window, and a concrete gateway implementing [`gateway::EffectGateway`]
//! is the only thing that can consume one -- verifying resource/effect
//! identity, bundle, expiry, and single use (replay) before letting the
//! caller's real mutation run, and writing one evidence record either way
//! via `nirdosha_audit`'s hash-chained log. [`TransferRequestGatewayV1`]
//! and [`FundsReserveGatewayV1`] both compose this same
//! [`gateway::GatewayCore`] contract rather than reimplementing it.
//!
//! A gateway only accepts a capability whose nonce is present in its issuer's
//! own `issued_registry` (checked before the guarded effect ever runs) and
//! whose `bundle_hash` matches the bundle the gateway was bound to at
//! construction -- so a hand-built `DecisionCapability`, or one minted under
//! a bundle the gateway no longer trusts, is rejected outright rather than
//! merely resource/effect/expiry checked like a genuine one.
//!
//! Scope this crate does **not** claim: the capability carries no
//! cryptographic signature (no MAC over its fields) -- the issued-registry
//! check defends against forgery only *within the same process* that holds
//! the `Arc` the issuer and gateway share; it is not proof against a
//! malicious actor with the ability to fabricate arbitrary process state
//! (e.g. via unsafe code or a compromised dependency in the same address
//! space). The policy bundle's `approved_by`/`signed_by` are checked for
//! presence and the validity window against the clock -- not verified as a
//! real Ed25519 signature. The issuer's own mint-provenance registry
//! (`issued_registry`) is still in-process only. And
//! [`funds_reserve::InMemoryFundsReserveBackend`]'s own ledger is
//! in-process, not a durable store.
//!
//! The one exception is single-use *replay* protection: `GatewayCore`'s
//! consumed-nonce tracking is now a pluggable [`replay_store::ReplayStore`],
//! with a real transaction-backed [`replay_store::SqliteReplayStore`] for
//! multi-process deployment (see its module doc for why an in-process
//! `HashSet` can't do this) -- [`replay_store::InMemoryReplayStore`] remains
//! the default for tests and single-process development. Everything else
//! above is disclosed scope boundary, not silent gap: real capability
//! signing, durable mint-provenance, a durable reservation store, and
//! shadow-mode comparison against the existing banking implementation are
//! explicitly later work -- see `funds_reserve`'s own module doc for the
//! `funds.reserve`-specific ones.

mod bundle;
mod capability;
mod funds_reserve;
mod gateway;
mod replay_store;

pub use bundle::{BundleError, PolicyBundle};
pub use capability::{CapabilityIssuer, DecisionCapability, GatewayError};
pub use funds_reserve::{
    AccountStatus, FactEnvelope, FundsReserveBackend, FundsReserveDenial, FundsReserveGatewayV1,
    FundsReserveOutcome, FundsReserveReceipt, FundsReserveRequest, InMemoryFundsReserveBackend, SettlementStatus,
};
pub use gateway::{EffectGateway, GatewayCore, TransferRequestGatewayV1};
pub use replay_store::{InMemoryReplayStore, ReplayStore, ReplayStoreError, SqliteReplayStore};
