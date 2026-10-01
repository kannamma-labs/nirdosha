//! Issue #76's disclosed follow-on: MIR-level (Stage 2) enforcement of
//! a signed domain pack's `mandatory_fns`/`protected_structs`
//! invariants -- the same properties `cargo-nirdosha`'s Stage 1
//! syntactic checker already enforces, now checked against real,
//! resolved MIR instead of `syn` source text. Before this fix, no
//! such MIR-level check existed at all -- `nirdosha-driver` never
//! consulted an active pack. This test fails against that prior
//! behavior (a real, active pack's `protected_structs` rule is
//! silently ignored, and a macro-generated construction site that a
//! syntactic scan can't see through slips past entirely) and passes
//! once `pack_check` is wired into `analyze()`.
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
            "nir-pack-check-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// Installs an active pack exactly the way `nirdosha-hi::hi_plugin`
    /// does: a manifest under `.nir/plugins/<id>/pack.json`, plus a
    /// non-revoked row in `.nir/hi.db`'s `plugins` table.
    fn install_pack(&self, id: &str, mandatory_fns: &[&str], protected_structs: &[&str]) {
        let plugins_dir = self.0.join(".nir").join("plugins").join(id);
        fs::create_dir_all(&plugins_dir).unwrap();
        fs::write(
            plugins_dir.join("pack.json"),
            serde_json::json!({
                "id": id,
                "name": id,
                "mandatory_fns": mandatory_fns,
                "protected_structs": protected_structs,
            })
            .to_string(),
        )
        .unwrap();
        let conn = rusqlite::Connection::open(self.0.join(".nir").join("hi.db")).unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS plugins (id TEXT PRIMARY KEY, name TEXT, sha256 TEXT, manifest_path TEXT, installed_at TEXT, revoked_at TEXT)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO plugins (id, name, sha256) VALUES (?1, ?1, 'x')", [id]).unwrap();
    }

    fn compile(&self, body: &str) -> Output {
        let src = format!(
            r#"
            pub struct Account {{ pub balance_cents: i64 }}
            pub fn debit_ledger(n: i64) -> Account {{ Account {{ balance_cents: n }} }}
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
            .env("NIRDOSHA_PACKAGE_ROOT", &self.0)
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
fn no_active_pack_is_a_silent_no_op() {
    let f = Fixture::new();
    success(&f.compile("pub fn bad_charge(n: i64) -> Account { Account { balance_cents: n } }"));
}

#[test]
fn constructing_a_protected_struct_outside_the_certified_primitive_is_refused() {
    let f = Fixture::new();
    f.install_pack("banking-v0-test", &["debit_ledger"], &["Account"]);
    let out = f.compile("pub fn bad_charge(n: i64) -> Account { Account { balance_cents: n } }");
    assert!(!out.status.success());
    let stderr = stderr(&out);
    assert!(stderr.contains("bad_charge") && stderr.contains("Account"), "{stderr}");
}

#[test]
fn a_macro_generated_construction_site_is_still_caught() {
    // The exact evasion the syntactic (Stage 1) checker can't see
    // through -- `nirdosha-contract-core::pack_check` walks `syn`
    // source text, so a construction site hidden inside a macro
    // expansion is invisible to it. The MIR-level version resolves the
    // real, expanded `Account` construction regardless.
    let f = Fixture::new();
    f.install_pack("banking-v0-test", &["debit_ledger"], &["Account"]);
    let out = f.compile(
        r#"
        macro_rules! sneaky { ($n:expr) => { Account { balance_cents: $n } }; }
        pub fn bad_charge(n: i64) -> Account { sneaky!(n) }
        "#,
    );
    assert!(!out.status.success());
    assert!(stderr(&out).contains("bad_charge"));
}

#[test]
fn the_certified_primitive_itself_is_exempt() {
    let f = Fixture::new();
    f.install_pack("banking-v0-test", &["debit_ledger"], &["Account"]);
    // Only `debit_ledger` (already in the fixture prelude, and named
    // mandatory) constructs `Account` here -- nothing else does. It
    // also needs a real call site of its own, or the separate
    // mandatory-call-site check would fail this test for an unrelated
    // reason.
    success(&f.compile("pub fn main() { debit_ledger(1); }"));
}

#[test]
fn a_mandatory_primitive_with_no_call_site_anywhere_is_refused() {
    let f = Fixture::new();
    f.install_pack("banking-v0-test", &["debit_ledger"], &[]);
    let out = f.compile("pub fn main() {}");
    assert!(!out.status.success());
    let stderr = stderr(&out);
    assert!(stderr.contains("debit_ledger") && stderr.contains("mandatory"), "{stderr}");
}

#[test]
fn a_mandatory_primitive_called_from_a_macro_expansion_is_accepted() {
    let f = Fixture::new();
    f.install_pack("banking-v0-test", &["debit_ledger"], &[]);
    let out = f.compile(
        r#"
        macro_rules! wire { () => { debit_ledger(1); }; }
        pub fn main() { wire!(); }
        "#,
    );
    success(&out);
}

#[test]
fn a_revoked_pack_no_longer_governs_the_build() {
    let f = Fixture::new();
    f.install_pack("banking-v0-test", &[], &["Account"]);
    let conn = rusqlite::Connection::open(f.0.join(".nir").join("hi.db")).unwrap();
    conn.execute("UPDATE plugins SET revoked_at = '2026-01-01' WHERE id = 'banking-v0-test'", []).unwrap();
    success(&f.compile("pub fn bad_charge(n: i64) -> Account { Account { balance_cents: n } }"));
}
