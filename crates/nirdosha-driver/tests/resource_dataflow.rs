//! Issue #69: `resource(kind = "..")` -- a real `rustc_mir_dataflow`
//! forward analysis proving every `nirdosha_rt::resource::acquire()` is
//! matched by a `release()` on every path out of the claiming fn. Same
//! `Fixture` harness as `tests/numeric.rs`/`tests/contract_predicates.rs`.
//!
//! Fixtures define their own local `mod nirdosha_rt { pub mod resource
//! { .. } }` shim rather than depending on the real crate -- same shape
//! (`acquire`/`release`), so `nirdosha-driver`'s local-crate-prefix
//! normalization (`dataflow::callee_path`) resolves it identically to
//! the real external `nirdosha_rt` dependency this checks in a real
//! build.
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
            "nir-resource-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn compile(&self, body: &str) -> Output {
        let src = format!(
            r#"
            mod nirdosha_rt {{ pub mod resource {{
                pub struct Resource<T>(pub T);
                pub fn acquire<T>(v: T) -> Resource<T> {{ Resource(v) }}
                pub fn release<T>(r: Resource<T>) -> T {{ r.0 }}
            }} }}
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
fn acquire_matched_by_release_on_every_path_is_accepted() {
    let f = Fixture::new();
    success(&f.compile(
        r#"
        /// nirdosha:contract {"resource":{"kind":"lock"}}
        pub fn straight_line() -> i32 {
            let r = nirdosha_rt::resource::acquire(5);
            nirdosha_rt::resource::release(r)
        }
        /// nirdosha:contract {"resource":{"kind":"lock"}}
        pub fn both_branches_release(flag: bool) -> i32 {
            let r = nirdosha_rt::resource::acquire(5);
            if flag {
                nirdosha_rt::resource::release(r)
            } else {
                nirdosha_rt::resource::release(r)
            }
        }
        "#,
    ));
}

#[test]
fn a_resource_dropped_on_one_path_without_release_is_a_leak() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"resource":{"kind":"lock"}}
        pub fn leaky(flag: bool) -> i32 {
            let r = nirdosha_rt::resource::acquire(5);
            if flag {
                nirdosha_rt::resource::release(r)
            } else {
                0
            }
        }
        "#,
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("acquired but never released"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn acquiring_again_while_still_held_is_a_double_acquire() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"resource":{"kind":"lock"}}
        pub fn double(flag: bool) -> i32 {
            let mut r = nirdosha_rt::resource::acquire(1);
            if flag {
                r = nirdosha_rt::resource::acquire(2);
            }
            nirdosha_rt::resource::release(r);
            0
        }
        "#,
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("still held"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn releasing_a_value_this_fn_never_acquired_is_refused() {
    let f = Fixture::new();
    let out = f.compile(
        r#"
        /// nirdosha:contract {"resource":{"kind":"lock"}}
        pub fn release_only(r: nirdosha_rt::resource::Resource<i32>) -> i32 {
            nirdosha_rt::resource::release(r)
        }
        "#,
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("never saw acquire() produce"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_fn_with_no_resource_clause_is_never_checked() {
    let f = Fixture::new();
    success(&f.compile(
        r#"
        pub fn leaky(flag: bool) -> i32 {
            let r = nirdosha_rt::resource::acquire(5);
            if flag {
                nirdosha_rt::resource::release(r)
            } else {
                0
            }
        }
        "#,
    ));
}
