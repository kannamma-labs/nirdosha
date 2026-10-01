//! Issue #76: `sequence(before = "..", after = "..")` -- a real
//! `rustc_mir_dataflow` forward analysis proving every call to `after`
//! is guaranteed to be preceded by a call to `before` on every path.
//! Before this fix, there was no mechanism anywhere in this codebase
//! for a cross-call ordering rule like "debit before credit" -- this
//! test fails to even parse the contract clause against the pre-fix
//! model, and after the fix, exercises both the clean and violating
//! shapes end to end through the real compiled driver, the same
//! `Fixture` harness `tests/resource_dataflow.rs` uses for issue #69's
//! sibling `resource(kind = "..")` check.
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
            "nir-sequence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn compile(&self, body: &str) -> Output {
        let src = format!(
            r#"
            pub fn debit(n: i64) -> i64 {{ n }}
            pub fn credit(n: i64) -> i64 {{ n }}
            {body}
            "#
        );
        fs::write(self.0.join("fixture.rs"), src).unwrap();
        Command::new(env!("CARGO_BIN_EXE_nirdosha-driver"))
            .current_dir(&self.0)
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("NIRDOSHA_DRIVER")
            .env_remove("CARGO_MANIFEST_DIR")
            .env("NIRDOSHA_MIR_CERT_DIR", self.0.join("certs"))
            .args([
                "fixture.rs",
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
fn debit_then_credit_on_every_path_is_accepted() {
    let f = Fixture::new();
    success(&f.compile(
        r#"
        /// nirdosha:contract {"sequence":{"before":"debit","after":"credit"}}
        pub fn transfer(amount: i64) -> i64 {
            debit(amount);
            credit(amount)
        }
        /// nirdosha:contract {"sequence":{"before":"debit","after":"credit"}}
        pub fn transfer_both_branches(amount: i64, flag: bool) -> i64 {
            debit(amount);
            if flag { credit(amount) } else { credit(0) }
        }
        "#,
    ));
}

#[test]
fn credit_before_debit_is_refused() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"sequence":{"before":"debit","after":"credit"}}
        pub fn backwards(amount: i64) -> i64 {
            credit(amount);
            debit(amount)
        }
        "#,
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("without `before` guaranteed first"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn credit_reachable_without_debit_on_one_branch_is_refused() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"sequence":{"before":"debit","after":"credit"}}
        pub fn conditionally_skips_debit(amount: i64, flag: bool) -> i64 {
            if flag {
                debit(amount);
            }
            credit(amount)
        }
        "#,
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("without `before` guaranteed first"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn credit_with_no_debit_call_at_all_is_refused() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"sequence":{"before":"debit","after":"credit"}}
        pub fn never_debits(amount: i64) -> i64 {
            credit(amount)
        }
        "#,
    );
    assert!(!out.status.success());
    assert!(stderr(&out).contains("without `before` guaranteed first"));
}

#[test]
fn an_unrelated_fn_with_no_sequence_claim_is_untouched() {
    let f = Fixture::new();
    success(&f.compile(
        r#"
        pub fn whatever_order(amount: i64) -> i64 {
            credit(amount);
            debit(amount)
        }
        "#,
    ));
}
