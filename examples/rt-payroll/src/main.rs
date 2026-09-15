//! # rt-payroll — the compliant face of the Nirdosha Rust dialect
//!
//! This file is, to the external world, just another Rust program:
//!
//! ```bash
//! cargo run -p rt-payroll          # builds and runs like any Rust code
//! ```
//!
//! Hand the same source to the Nirdosha compiler and it becomes
//! *verified* Nirdosha code:
//!
//! ```bash
//! cargo nirdosha build              # verifies every claim first, then builds
//! cargo nirdosha verify             # just verification + certificate
//! ```
//!
//! Three contract forms are on display:
//!
//! 1. [`compute_payroll`] — the full `#[contract]` macro:
//!    `effects(pure)` (lying is a compile error, even under plain
//!    cargo), `requires(role = "hr_staff")` (the macro injects an
//!    unforgeable proof parameter — uncallable without a minted proof,
//!    under *any* compiler), and `nfr(..)` (latency + concurrency
//!    guards wrap the body).
//! 2. [`audit_total`] — the zero-dependency hand-written doc form:
//!    a plain doc comment, inert to rustc, verified by `cargo nirdosha`.
//! 3. [`executive_total`] — `requires` without `nfr`: capability
//!    enforcement only, zero runtime cost beyond the type.
//!
//! Delete the `prove` call in `main` and this file stops compiling —
//! that's the `uncallable` guarantee doing its job.

use nirdosha_rt::{contract, Auth};

nirdosha_rt::roles! {
    HrStaff = "hr_staff";
    Manager = "manager";
}

struct PayRecord {
    employee: u64,
    gross_cents: u64,
}

/// Net pay: gross minus a flat 10% statutory deduction. Pure
/// arithmetic over a fixed batch — provable now, Z3-certifiable in
/// Stage 2.
#[contract(
    effects(pure),
    requires(role = "hr_staff"),
    nfr(latency_ms = 250, concurrency_max = 4)
)]
fn compute_payroll(records: &[PayRecord]) -> u64 {
    records.iter().map(|r| r.gross_cents - r.gross_cents / 10).sum()
}

/// Hand-written, zero-macro doc form. Inert to plain rustc; `cargo
/// nirdosha` verifies the claim. Use this when a crate must build with
/// no nirdosha dependency at all.
/// nirdosha:contract {"effects":["pure"]}
fn audit_total(records: &[PayRecord]) -> u64 {
    records.iter().map(|r| r.gross_cents).sum()
}

/// A second role gate: the same proof mechanism, no NFR guard.
#[contract(requires(role = "manager"))]
fn executive_total(records: &[PayRecord]) -> u64 {
    records.iter().map(|r| r.gross_cents / 5).sum()
}

fn main() {
    let records = vec![
        PayRecord { employee: 1, gross_cents: 120_000 },
        PayRecord { employee: 2, gross_cents: 85_000 },
        PayRecord { employee: 3, gross_cents: 96_500 },
    ];

    // A session for sita, who holds hr_staff. Try removing hr_staff from
    // this list — the program stops compiling.
    let session = Auth::login("sita", &["hr_staff"]);
    let proof = session
        .prove::<nirdosha_roles::HrStaff>()
        .expect("sita holds hr_staff");

    let net = compute_payroll(&proof, &records);
    let gross = audit_total(&records);
    let ids: Vec<u64> = records.iter().map(|r| r.employee).collect();
    println!("payroll: employees {ids:?}, net {net} cents (gross {gross} cents)");

    // The manager-gated path stays uncallable with this session:
    match session.prove::<nirdosha_roles::Manager>() {
        Err(e) => println!("manager view refused: {e}"),
        Ok(_) => unreachable!("sita does not hold the manager role"),
    }

    // A manager session mints the token the executive view demands:
    let manager = Auth::login("dasharatha", &["manager"]);
    match manager.prove::<nirdosha_roles::Manager>() {
        Ok(manager_proof) => {
            println!("executive view: {} cents", executive_total(&manager_proof, &records));
        }
        Err(e) => println!("executive view refused: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payroll_maths() {
        let records = vec![PayRecord { employee: 1, gross_cents: 100 }];
        let session = Auth::login("sita", &["hr_staff"]);
        let proof = session.prove::<nirdosha_roles::HrStaff>().unwrap();
        assert_eq!(compute_payroll(&proof, &records), 90);
    }

    #[test]
    fn unheld_role_is_uncallable() {
        let outsider = Auth::login("ravana", &["janitor"]);
        assert!(outsider.prove::<nirdosha_roles::HrStaff>().is_err());
    }

    #[test]
    fn nfr_guard_records_latency() {
        nirdosha_rt::reset_events();
        let records = vec![PayRecord { employee: 1, gross_cents: 100 }];
        let session = Auth::login("sita", &["hr_staff"]);
        let proof = session.prove::<nirdosha_roles::HrStaff>().unwrap();
        let _ = compute_payroll(&proof, &records);
        let log = nirdosha_rt::events();
        assert_eq!(log.len(), 1, "compute_payroll is NFR-guarded: exactly one event");
        assert_eq!(log[0].function, "compute_payroll");
        assert_eq!(log[0].limit_ms, Some(250.0));
    }
}