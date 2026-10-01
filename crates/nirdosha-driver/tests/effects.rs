//! Real rustc/driver differential tests. Every rejection fixture is valid
//! Rust; the driver must reject it at both optimization levels without ICEs.
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
            "nir-deep-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn compile(&self, source: &str, deep: bool, optimization: &str) -> Output {
        let path = self.0.join("fixture.rs");
        fs::write(&path, source).unwrap();
        let compiler = if deep {
            env!("CARGO_BIN_EXE_nirdosha-driver")
        } else {
            "rustc"
        };
        Command::new(compiler)
            .env_remove("NIRDOSHA_DRIVER")
            .arg(&path)
            .args([
                "--crate-name",
                "fixture",
                "--crate-type",
                "lib",
                "--edition=2024",
                "--emit=metadata",
                "-A",
                "warnings",
                "-C",
            ])
            .arg(format!("opt-level={optimization}"))
            .arg("--out-dir")
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

fn reject(source: &str, diagnostic: &str) {
    let fixture = Fixture::new();
    for optimization in ["0", "3"] {
        let plain = fixture.compile(source, false, optimization);
        assert!(
            plain.status.success(),
            "invalid fixture: {}",
            String::from_utf8_lossy(&plain.stderr)
        );
        let deep = fixture.compile(source, true, optimization);
        let stderr = String::from_utf8_lossy(&deep.stderr);
        assert!(
            !deep.status.success(),
            "driver accepted fixture at O{optimization}: {source}"
        );
        assert!(!stderr.contains("internal compiler error"), "{stderr}");
        assert!(
            stderr.contains(diagnostic),
            "expected {diagnostic}: {stderr}"
        );
    }
}

#[test]
fn scalar_helpers_and_mutual_recursion_pass() {
    let fixture = Fixture::new();
    let source = r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root(n: u64) -> u64 { even(n) + odd(n) }
        fn even(n: u64) -> u64 { if n == 0 { 0 } else { odd(n - 1) } }
        fn odd(n: u64) -> u64 { if n == 0 { 1 } else { even(n - 1) } }
    "#;
    for optimization in ["0", "3"] {
        let output = fixture.compile(source, true, optimization);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn expanded_macro_cannot_hide_io() {
    reject(
        r#"
        macro_rules! hidden { () => { std::fs::read_to_string("x") }; }
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root() { let _ = hidden!(); }
    "#,
        "file system access",
    );
}

#[test]
fn std_callback_is_not_trusted_by_crate_name() {
    reject(
        r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root() -> usize {
            [1, 2].iter().map(|_| { println!("effect"); 1 }).sum()
        }
    "#,
        "external call lacks a verified effect summary",
    );
}

#[test]
fn drop_is_an_effect_boundary() {
    // Issue #78: a local type's explicit `Drop::drop` gets the same real
    // recursive analysis a direct call does, so the actual effect (I/O,
    // here) is named in the chain rather than a generic "destructor
    // effects lack a verified summary" catch-all.
    reject(
        r#"
        pub struct Noisy;
        impl Drop for Noisy { fn drop(&mut self) { println!("effect"); } }
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root(value: Noisy) { }
    "#,
        "I/O",
    );
}

#[test]
fn owning_a_vec_with_no_custom_drop_is_pure() {
    // Issue #78's own headline example: MIR inserts a `Drop` terminator
    // for `v` (`Vec`'s own explicit `impl Drop` deallocates the buffer),
    // but that destructor is curated-trusted (`std_effects::
    // PURE_DESTRUCTOR_TYPES`) — owning a `Vec` locally must not block a
    // pure claim just because it needs drop glue.
    let fixture = Fixture::new();
    let source = r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn build(mut v: Vec<i64>, x: i64) -> Vec<i64> {
            v.push(x);
            v
        }
    "#;
    for optimization in ["0", "3"] {
        let output = fixture.compile(source, true, optimization);
        assert!(
            output.status.success(),
            "O{optimization}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn foreign_destructor_without_a_verified_summary_is_rejected() {
    // The other side of issue #78: a foreign type's explicit `Drop` that
    // is NOT in the curated table must still fail closed, exactly like
    // an uncovered foreign call does.
    reject(
        r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root() {
            let m = std::sync::Mutex::new(0);
            let _g = m.lock().unwrap();
        }
    "#,
        "destructor effects",
    );
}

#[test]
fn dynamic_and_generic_calls_fail_without_panicking() {
    reject(
        r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root(callback: fn() -> u64) -> u64 { callback() }
    "#,
        "dynamically dispatched call",
    );
    reject(
        r#"
        pub trait Callback { fn call(&self) -> u64; }
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root<T: Callback>(callback: &T) -> u64 { callback.call() }
    "#,
        "unresolved trait dispatch",
    );
    reject(
        r#"
        pub trait Callback { fn call(&self) -> u64; }
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root(callback: &dyn Callback) -> u64 { callback.call() }
    "#,
        "unresolved trait dispatch",
    );
    reject(
        r#"
        pub trait Callback { fn call(&self) -> u64 { 1 } }
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root<T: Callback>(callback: &T) -> u64 { callback.call() }
    "#,
        "unresolved trait dispatch",
    );
}

#[test]
fn reference_writes_and_static_reads_are_not_pure() {
    reject(
        r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root(value: &mut u64) { *value = 1; }
    "#,
        "write through a reference",
    );
    reject(
        r#"
        static VALUE: u64 = 1;
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root() -> u64 { VALUE }
    "#,
        "static state",
    );
}

#[test]
fn macro_generated_unsafe_is_rejected() {
    reject(
        r#"
        macro_rules! hidden { ($p:expr) => { unsafe { *$p } }; }
        /// nirdosha:contract {"effects":["pure"]}
        pub fn root(p: *const u64) -> u64 { hidden!(p) }
    "#,
        "unsafe block",
    );
}

#[test]
fn recursive_component_is_checked_for_every_claiming_root() {
    let fixture = Fixture::new();
    let source = r#"
        /// nirdosha:contract {"effects":["pure"]}
        pub fn first(n: u64) { if n > 0 { second(n - 1); } println!("effect"); }
        /// nirdosha:contract {"effects":["pure"]}
        pub fn second(n: u64) { if n > 0 { first(n - 1); } }
    "#;
    for optimization in ["0", "3"] {
        let output = fixture.compile(source, true, optimization);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(
            stderr.contains("fn `first` claims effects(pure)"),
            "{stderr}"
        );
        assert!(
            stderr.contains("fn `second` claims effects(pure)"),
            "{stderr}"
        );
        assert!(!stderr.contains("internal compiler error"), "{stderr}");
    }
}

#[test]
fn ordinary_rust_without_claims_passes_through() {
    let fixture = Fixture::new();
    let output = fixture.compile("pub fn ordinary() { println!(\"allowed\"); }", true, "3");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
