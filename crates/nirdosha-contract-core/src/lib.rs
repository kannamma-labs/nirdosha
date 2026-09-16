//! Shared machinery for the Nirdosha Rust dialect's contract layer.
//!
//! Three surfaces share this code:
//!
//! - [`nirdosha-macros`] parses `#[contract(...)]` attributes and emits the
//!   doc-encoded contract plus enforcement code (role-proof parameter, NFR
//!   guard).
//! - [`cargo-nirdosha`] — the "Nirdosha compiler", Stage 1 — source-scans
//!   crates, verifies contract claims (including hand-written doc
//!   contracts), and refuses to build lying code before delegating to plain
//!   cargo.
//! - the [`model::Contract`] JSON encoding is the portable artifact: it
//!   survives in plain Rust with zero dependencies, flows into rustdoc, and
//!   is what any agent or auditor can extract from a compiled crate's docs.
//!
//! Design principle: a Nirdosha program is a *subset* of Rust. Everything
//! written here must be valid, compilable Rust that runs unchanged under a
//! stock toolchain. The contract attributes and doc strings are inert to
//! plain rustc; under `cargo nirdosha` they stop being comments and become
//! checks.

pub mod certificate;
pub mod docparse;
pub mod declarations;
pub mod evidence;
pub mod model;
pub mod parse;
pub mod role;
pub mod scan;
