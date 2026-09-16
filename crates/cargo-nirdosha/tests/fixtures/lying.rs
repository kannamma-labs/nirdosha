// Lying: claims purity, performs file I/O and reads a clock.
// Plain cargo builds it. cargo nirdosha refuses it.

/// nirdosha:contract {"effects":["pure"]}
pub fn snapshot() -> String {
    let started = std::time::Instant::now();
    let ledger = std::fs::read_to_string("ledger.txt").unwrap_or_default();
    format!("audited {ledger} in {:?}", started.elapsed())
}
