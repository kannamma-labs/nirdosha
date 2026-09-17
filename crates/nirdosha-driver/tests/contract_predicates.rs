//! Issue #68: `requires(expr)`/`ensures(expr)` -- a numeric pre/post
//! condition over MIR, discharged against the same Z3 VC IR item 1
//! (issue #67) built. Same `Fixture` harness as `tests/numeric.rs`.
use nirdosha_contract_core::certificate::Certificate;
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
            "nir-contract-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn compile(&self, src: &str) -> Output {
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
                "-C",
                "overflow-checks=yes",
                "--out-dir",
            ])
            .arg(&self.0)
            .output()
            .unwrap()
    }
    fn cert(&self) -> Certificate {
        let cert: Certificate = serde_json::from_slice(
            &fs::read(self.0.join("certs/contract-report-fixture.json")).unwrap(),
        )
        .unwrap();
        assert!(cert.binding_valid());
        cert
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
#[cfg(feature = "smt")]
fn requires_assumption_discharges_a_division_by_zero_obligation_intervals_could_not() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"requires":{"expr":"y != 0"}}
        pub fn safe_div(x: u64, y: u64) -> u64 { x / y }
        /// nirdosha:contract {}
        pub fn unguarded_div(x: u64, y: u64) -> u64 { x / y }
        "#,
    );
    success(&out);
    let cert = f.cert();
    assert!(
        cert.proofs
            .iter()
            .any(|p| p["function"] == "safe_div" && p["kind"] == "division_by_zero"),
        "requires(y != 0) should let Z3 discharge safe_div's division-by-zero obligation: {cert:?}"
    );
    assert!(
        !cert
            .proofs
            .iter()
            .any(|p| p["function"] == "unguarded_div" && p["kind"] == "division_by_zero"),
        "unguarded_div has no requires(..) precondition and must not be certified: {cert:?}"
    );
}

#[test]
#[cfg(feature = "smt")]
fn ensures_is_proven_when_it_holds_on_every_return_path() {
    let f = Fixture::new();
    success(&f.compile(
        r#"
        /// nirdosha:contract {"ensures":{"expr":"result >= 0"}}
        pub fn magnitude(x: i32) -> i32 { if x < 0 { -x } else { x } }
        "#,
    ));
    let cert = f.cert();
    let checks = cert.verification["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["function"] == "magnitude")
        .unwrap()["checks"]
        .as_array()
        .unwrap();
    assert!(
        checks
            .iter()
            .any(|c| c["kind"] == "ensures" && c["proven"] == true),
        "{checks:?}"
    );
}

#[test]
#[cfg(feature = "smt")]
fn ensures_that_does_not_hold_on_every_path_is_rejected() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"ensures":{"expr":"result > 0"}}
        pub fn magnitude(x: i32) -> i32 { if x < 0 { -x } else { x } }
        "#,
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("ensures(..)"),
        "{}",
        stderr(&out)
    );
    assert!(!f.0.join("certs/contract-report-fixture.json").exists());
}

#[test]
#[cfg(feature = "smt")]
fn an_ensures_predicate_naming_an_undeclared_identifier_is_refused() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"ensures":{"expr":"result >= floor"}}
        pub fn magnitude(x: i32) -> i32 { if x < 0 { -x } else { x } }
        "#,
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("floor"),
        "should name the unresolved identifier: {}",
        stderr(&out)
    );
}
