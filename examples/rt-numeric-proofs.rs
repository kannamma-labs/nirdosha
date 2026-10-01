//! Standalone Rust counterpart of features/58_numeric_smt_proofs.nir.
//! cargo build -p nirdosha-driver
//! target/debug/nirdosha-driver examples/rt-numeric-proofs.rs --crate-type lib --out-dir /tmp/rt-numeric -C overflow-checks=yes
//! The certificate is /tmp/rt-numeric/nirdosha/mir/contract-report-rt_numeric_proofs.json.

/// nirdosha:contract {"effects":["pure"]}
pub fn divide_by_gap(x: u32, y: u32) -> u32 {
    if x > y { 100 / (x - y) } else { 0 }
}

/// nirdosha:contract {"effects":["pure"]}
pub fn bounded_index(values: [i32; 4], index: usize) -> i32 {
    if index < 4 { values[index] } else { 0 }
}
