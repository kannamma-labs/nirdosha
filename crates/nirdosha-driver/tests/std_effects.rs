//! Issue #71: a curated std/core/alloc effect-summary table so Stage 2
//! doesn't reject every external call, including `Vec::push`,
//! `.iter().map(..)`, and the dialect's own injected NFR guard
//! (`docs/V2_GUARANTEES.md`'s documented gap). Same `Fixture` harness
//! as `tests/effects.rs`.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nir-std-effects-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn compile(&self, source: &str) -> Output {
        let path = self.0.join("fixture.rs");
        fs::write(&path, source).unwrap();
        Command::new(env!("CARGO_BIN_EXE_nirdosha-driver"))
            .env_remove("NIRDOSHA_DRIVER")
            .arg(&path)
            .args([
                "--crate-name",
                "fixture",
                "--crate-type",
                "lib",
                "--edition=2024",
                "--emit=metadata",
                "-Awarnings",
                "-C",
                "opt-level=0",
                "--out-dir",
            ])
            .arg(&self.0)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn success(o: &Output) {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn vec_push_and_an_iterator_map_chain_are_accepted() {
    // `v: &mut Vec<i64>` throughout -- a reference, never owned, so
    // nothing is locally dropped. Deliberate, to isolate this issue's
    // actual scope (the external-call summary table) from the
    // separate, pre-existing "any value needing drop glue is an
    // impurity regardless of whether its `Drop` is custom or purely
    // compiler-generated deallocation" gap `TerminatorKind::Drop`'s
    // unconditional rejection has today (`main.rs`'s own `"destructor
    // effects lack a verified summary"` case) -- filed separately.
    let f = Fixture::new();
    success(&f.compile(
        r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn build(v: &mut Vec<i64>, x: i64) {
            v.push(x);
        }
        /// nirdosha:contract {"effects":["pure"]}
        pub fn total(v: &[i64]) -> i64 {
            v.iter().map(|n| *n * 2).filter(|n| *n > 0).sum()
        }
        "#,
    ));
}

#[test]
fn option_and_result_combinators_are_accepted() {
    // `i64` error/ok payloads throughout -- no owned heap type, same
    // "isolate this issue's own scope from the separate Drop gap"
    // reasoning as the test above.
    let f = Fixture::new();
    success(&f.compile(
        r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn combined(o: Option<i64>, r: Result<i64, i64>) -> i64 {
            o.map(|x| x + 1).unwrap_or(0) + r.map(|x| x * 2).unwrap_or(0)
        }
        "#,
    ));
}

// The dialect's own injected NFR guard (`nirdosha_rt::nfr::enter`) is
// covered by `std_effects.rs`'s own unit test
// (`classifies_nirdosha_rts_own_injected_nfr_guard`), not here: this
// harness compiles self-contained fixtures with no external crate
// dependency, so a real `#[nirdosha_rt::contract(..)]` expansion (which
// needs `nirdosha_rt` linked as an actual dependency, making the call a
// genuinely *foreign* `DefId` rather than a local same-crate shim) isn't
// reachable from a bare `nirdosha-driver <file>.rs` invocation the way
// the other tests here are.

#[test]
fn a_side_effect_hidden_inside_a_trusted_higher_order_call_is_still_rejected() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root(v: &[i64]) -> i64 {
            v.iter().map(|x| { println!("effect"); *x }).sum()
        }
        "#,
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("external call lacks a verified effect summary"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_call_not_in_the_curated_table_is_still_rejected() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root() -> i64 { std::process::id() as i64 }
        "#,
    );
    assert!(!out.status.success());
    assert!(stderr(&out).contains("process spawn"), "{}", stderr(&out));
}
