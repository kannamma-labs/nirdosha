//! # rt-payroll-lying — the money demo
//!
//! This program has **zero Nirdosha dependencies**. To the external
//! world it is just another Rust program:
//!
//! ```bash
//! cargo run -p rt-payroll-lying   # builds and runs under plain cargo
//! ```
//!
//! But its doc comments carry `nirdosha:contract` declarations, and
//! when the same source meets the Nirdosha compiler:
//!
//! ```bash
//! cargo nirdosha build            # REFUSED: the claims are lies
//! ```
//!
//! the claims stop being comments and become checks:
//!
//! - `greeting` claims pure and *is* pure — verified.
//! - `snapshot_audit` claims pure but reads a file and a clock —
//!   refused, with the exact lines named.
//! - `mislabeled` has a typo'd contract key — refused, because an
//!   unknown key can never degrade into a silent no-op.

/// nirdosha:contract {"effects":["pure"]}
fn greeting() -> &'static str {
    "payroll auditor"
}

/// nirdosha:contract {"effects":["pure"]}
fn snapshot_audit() -> String {
    let started = std::time::Instant::now();
    let ledger = std::fs::read_to_string("ledger.txt").unwrap_or_default();
    format!("audited {ledger:?} in {:?}", started.elapsed())
}

/// nirdosha:contract {"effekts":["pure"]}
fn mislabeled(x: u64) -> u64 {
    x
}

fn main() {
    println!("{} — {}", greeting(), snapshot_audit());
    let _ = mislabeled(7);
}