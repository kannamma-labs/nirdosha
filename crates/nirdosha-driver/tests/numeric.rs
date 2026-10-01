//! Invoke the real driver at O0/O3, inspect hash-bound proof evidence, and
//! compare Z3 with the explicitly weaker no-default-features interval build.
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
            "nir-numeric-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn compile(&self, src: &str, opt: &str, overflow: &str) -> Output {
        fs::write(self.0.join("fixture.rs"), src).unwrap();
        Command::new(env!("CARGO_BIN_EXE_nirdosha-driver"))
            .current_dir(&self.0)
            // Exercise standalone loader behavior, not Cargo's injected path.
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
            ])
            .arg(format!("opt-level={opt}"))
            .arg("-C")
            .arg(format!("overflow-checks={overflow}"))
            .arg("--out-dir")
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
        assert!(cert.check_sources(&self.0).ok());
        cert
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn success(o: Output) {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
const RELATIONAL: &str = r#"
    /// nirdosha:contract {"effects":["pure"]}
    pub fn gap(x:u32,y:u32)->u32 { if x>y { 100/(x-y) } else { 0 } }
"#;

#[test]
fn z3_is_invoked_for_a_relational_mir_proof() {
    for opt in ["0", "3"] {
        let f = Fixture::new();
        let out = f.compile(RELATIONAL, opt, "yes");
        if cfg!(feature = "smt") {
            success(out);
            let cert = f.cert();
            assert_eq!(cert.verification["backend"], "z3");
            let function = cert.verification["functions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["function"] == "gap")
                .unwrap();
            // This counter is incremented at Solver::check, not at frontend traversal.
            assert!(function["solver_queries"].as_u64().unwrap() >= 2);
            for kind in ["overflow", "division_by_zero"] {
                assert!(
                    cert.proofs.iter().any(|p| p["function"] == "gap"
                        && p["kind"] == kind
                        && p["method"] == "z3"
                        && p["elidable"] == true),
                    "{cert:?}"
                );
            }
            let mut tampered = cert.clone();
            tampered.proofs.clear();
            assert!(!tampered.binding_valid());
            success(f.compile(RELATIONAL, opt, "yes"));
            assert_eq!(f.cert().binding, cert.binding);
        } else {
            assert!(
                !out.status.success(),
                "intervals must not claim this relational proof"
            );
            assert!(String::from_utf8_lossy(&out.stderr).contains("division by zero"));
        }
    }
}

#[test]
fn safe_constant_division_works_with_either_backend() {
    let f = Fixture::new();
    success(f.compile(
        "/// nirdosha:contract {\"effects\":[\"pure\"]}\npub fn root(x:u64)->u64 { x/2 }",
        "0",
        "yes",
    ));
    let cert = f.cert();
    assert!(!cert.proofs.is_empty());
    assert_eq!(
        cert.verification["backend"],
        if cfg!(feature = "smt") {
            "z3"
        } else {
            "interval"
        }
    );
}

#[test]
fn unguarded_division_still_rejects_purity() {
    let f = Fixture::new();
    let o = f.compile(
        "/// nirdosha:contract {\"effects\":[\"pure\"]}\npub fn root(x:i64,y:i64)->i64 { x/y }",
        "3",
        "yes",
    );
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("division by zero"));
    assert!(!f.0.join("certs/contract-report-fixture.json").exists());
}

#[test]
fn unsafe_paths_are_never_certified() {
    for opt in ["0", "3"] {
        for (body, kind) in [
            ("pub fn root(x:u8)->u8 { x+1 }", "overflow"),
            ("pub fn root(x:i64)->i64 { x / -1 }", "overflow"),
            ("pub fn root(x:i64)->i64 { x % -1 }", "overflow"),
            ("pub fn root(x:i64)->i64 { -x }", "overflow_neg"),
            ("pub fn root(x:u8,n:u8)->u8 { x << n }", "overflow"),
            (
                "pub fn root(x:u32,flag:bool)->u32 { let d=if flag {x+1} else {0}; 10/d }",
                "division_by_zero",
            ),
            (
                "pub fn root(mut x:u32)->u32 { while x>0 { x-=1; } 10/x }",
                "division_by_zero",
            ),
            (
                "fn change(x:&mut u32){*x=0;} pub fn root()->u32 {let mut x=2; change(&mut x); 10/x}",
                "division_by_zero",
            ),
            (
                "pub fn root(mut x:u32)->u32 {if x>0 {let r=&mut x;*r=0;10/x}else{0}}",
                "division_by_zero",
            ),
            ("pub fn root(x:u32,i:usize)->u32 { [x,x][i] }", "bounds"),
            (
                "pub fn root(x:u32)->u32 {if x>0 {10 / (x as u8) as u32} else {0}}",
                "division_by_zero",
            ),
        ] {
            let f = Fixture::new();
            success(f.compile(&format!("/// nirdosha:contract {{}}\n{body}"), opt, "yes"));
            let cert = f.cert();
            assert!(
                !cert
                    .proofs
                    .iter()
                    .any(|p| p["function"] == "root" && p["kind"] == kind),
                "{body}: {cert:?}"
            );
        }
    }
}

#[test]
fn disabled_overflow_guards_cannot_be_assumed() {
    for opt in ["0", "3"] {
        let f = Fixture::new();
        success(f.compile(
            "/// nirdosha:contract {}\npub fn root(x:u8)->u8 { let y=x+1; 10/y }",
            opt,
            "no",
        ));
        assert!(
            !f.cert()
                .proofs
                .iter()
                .any(|p| p["kind"] == "division_by_zero")
        );
    }
}

#[test]
#[cfg(feature = "smt")]
fn guarded_array_indexing_and_signed_division_are_proven() {
    for opt in ["0", "3"] {
        let f = Fixture::new();
        success(f.compile(
            r#"
            /// nirdosha:contract {"effects":["pure"]}
            pub fn index(a:[i32;4],i:usize)->i32 { if i<4 {a[i]} else {0} }
            /// nirdosha:contract {}
            pub fn signed(x:i64)->i64 { if x == -7 { let q=x/2; 100/(q+4) } else { 0 } }
            /// nirdosha:contract {}
            pub fn wide(x:u128)->u128 { if x < u128::MAX {x+1} else {x} }
        "#,
            opt,
            "yes",
        ));
        let cert = f.cert();
        for (name, kind) in [
            ("index", "bounds"),
            ("signed", "division_by_zero"),
            ("wide", "overflow"),
        ] {
            assert!(
                cert.proofs
                    .iter()
                    .any(|p| p["function"] == name && p["kind"] == kind),
                "{cert:?}"
            );
            let checks = cert.verification["functions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["function"] == name)
                .unwrap()["checks"]
                .as_array()
                .unwrap();
            assert!(
                checks.iter().all(|c| c["proven"] == true),
                "{name}: {checks:?}"
            );
        }
    }
}
