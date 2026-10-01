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
//! **Capability and bundle signing, real but opt-in** (`authority`
//! module): a [`CapabilityIssuer`] built with
//! [`CapabilityIssuer::with_signing_key`] Ed25519-signs every capability
//! it mints, and any `GatewayCore` built from that issuer then requires
//! and verifies that signature before ever running the guarded effect --
//! closing "the capability carries no cryptographic signature" for
//! deployments that opt in. Likewise [`SignedPolicyBundle`] pairs a
//! [`PolicyBundle`] with a real, verified detached Ed25519 signature
//! against an [`AuthorityRegistry`] of trusted keys, closing "not verified
//! as a real Ed25519 signature" for `approved_by`/`signed_by`. Every
//! *existing* call site (`CapabilityIssuer::new`, `GatewayCore::new`,
//! `PolicyBundle::from_toml_str` alone) is completely unaffected --
//! signing is additive, not a breaking requirement.
//!
//! Scope this crate still does **not** claim, even with signing enabled:
//! *production key custody*. `AuthorityRegistry` says which public keys
//! are trusted, not how those keys were minted, rotated, or protected (a
//! real HSM/KMS-backed signing ceremony, multi-party authorization to add
//! a new trusted authority) -- see `authority`'s own module doc. The
//! issuer's own mint-provenance registry (`issued_registry`) is still
//! in-process only. And [`funds_reserve::InMemoryFundsReserveBackend`]'s
//! own ledger is in-process, not a durable store.
//!
//! The one exception is single-use *replay* protection: `GatewayCore`'s
//! consumed-nonce tracking is now a pluggable [`replay_store::ReplayStore`],
//! with a real transaction-backed [`replay_store::SqliteReplayStore`] for
//! multi-process deployment (see its module doc for why an in-process
//! `HashSet` can't do this) -- [`replay_store::InMemoryReplayStore`] remains
//! the default for tests and single-process development. Everything else
//! above is disclosed scope boundary, not silent gap: durable
//! mint-provenance, a durable reservation store, and shadow-mode
//! comparison against the existing banking implementation are explicitly
//! later work -- see `funds_reserve`'s own module doc for the
//! `funds.reserve`-specific ones.

mod authority;
mod bundle;
mod capability;
mod funds_reserve;
mod gateway;
mod replay_store;

pub use authority::{generate_ed25519_keypair, AuthorityRegistry, AuthorityRegistryError, SignedBundleError, SignedPolicyBundle};
pub use bundle::{BundleError, PolicyBundle};
pub use capability::{CapabilityIssuer, DecisionCapability, GatewayError};
pub use funds_reserve::{
    AccountStatus, FactEnvelope, FundsReserveBackend, FundsReserveDenial, FundsReserveGatewayV1,
    FundsReserveOutcome, FundsReserveReceipt, FundsReserveRequest, InMemoryFundsReserveBackend, SettlementStatus,
};
pub use gateway::{EffectGateway, GatewayCore, TransferRequestGatewayV1};
pub use replay_store::{InMemoryReplayStore, ReplayStore, ReplayStoreError, SqliteReplayStore};
