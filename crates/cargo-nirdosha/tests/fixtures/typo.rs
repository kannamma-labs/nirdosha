// A typo in a contract key is a build error, not a silent no-op.

/// nirdosha:contract {"effekts":["pure"]}
pub fn mislabeled(x: u64) -> u64 {
    x
}
