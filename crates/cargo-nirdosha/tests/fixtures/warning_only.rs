// Pure claim on a body-less trait declaration: warning, not error.

pub trait Auditor {
    /// nirdosha:contract {"effects":["pure"]}
    fn total(&self, items: &[u64]) -> u64;
}
