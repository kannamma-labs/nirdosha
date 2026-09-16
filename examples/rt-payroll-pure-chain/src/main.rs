//! # rt-payroll-pure-chain — the Stage 2 demo
//!
//! This program's lies are **indirect**: every pure-claiming function's
//! *own body* is locally clean, and one of them reaches file I/O only
//! through the call graph.
//!
//! ```bash
//! cargo build                          # plain cargo: builds and runs fine
//! cargo nirdosha build                 # Stage 1: body-local scan — PASSES (the lie is out of sight)
//! cargo nirdosha build --deep          # Stage 2: MIR call graph — REFUSED, chains named
//! ```
//!
//! The gap between those last two lines is exactly what Stage 2 exists
//! for: `effects(pure)` verified against the *transitive closure* of
//! what a function calls, with real name resolution.
//!
//! `risk_weight` stays in the file on purpose: recursion plus checked
//! arithmetic must survive the effect lattice (back-edges contribute
//! nothing, overflow checks are the documented dialect guard) — a
//! checker that flags factorial is a checker nobody trusts.

/// nirdosha:contract {"effects":["pure"]}
fn net_total(records: &[u64]) -> u64 {
    let overrides = ledger_overrides();
    records.iter().sum::<u64>() + overrides
}

/// nirdosha:contract {"effects":["pure"]}
fn ledger_overrides() -> u64 {
    let raw = read_ledger();
    raw.len() as u64
}

/// No claim, no contract: a plain Rust helper. The lie lives here.
fn read_ledger() -> String {
    std::fs::read_to_string("ledger.txt").unwrap_or_default()
}

/// nirdosha:contract {"effects":["pure"]}
fn risk_weight(n: u64) -> u64 {
    // Recursion + checked arithmetic: allowed by construction — a
    // back-edge in the effect lattice contributes nothing, and overflow
    // checks are the dialect's documented runtime guard.
    if n <= 1 {
        1
    } else {
        n * risk_weight(n - 1)
    }
}

fn main() {
    let records = [120_000u64, 85_000, 96_500];
    println!("net total: {} cents", net_total(&records));
    println!("risk weight: {}", risk_weight(5));
}