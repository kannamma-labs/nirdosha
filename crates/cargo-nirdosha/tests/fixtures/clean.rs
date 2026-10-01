// Compliant: every claim is true. Both compilers accept this file.

/// nirdosha:contract {"effects":["pure"]}
pub fn add(a: u64, b: u64) -> u64 {
    a + b
}

/// nirdosha:contract {"effects":["pure"],"nfr":{"latency_ms":100}}
pub fn totals(items: &[u64]) -> u64 {
    items.iter().sum()
}
