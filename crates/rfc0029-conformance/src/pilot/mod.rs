//! Narrow executable pilot required by
//! `rfcs/0029-phase0-readiness-and-domain-matrix.md` §4 ("Smallest
//! executable pilot"): a falsification experiment against the versioned
//! candidate IR, exactly scoped to `funds.reserve`. It is not a production
//! gateway and is not wired to any store, network, or live effect path.

pub mod assertion_lifecycle;
pub mod funds_reserve;
pub mod report;
