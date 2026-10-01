//! Issue #76: "a signed pack that claims 'certified'/'trusted' but is
//! never checked by the compiler is a soundness bug." Before this fix,
//! an active, installed domain pack's `mandatory_fns`/
//! `protected_structs` invariants were readable on disk
//! (`.nir/plugins/<id>/pack.json` + `.nir/hi.db`, written by
//! `nirdosha-hi`'s real, tested, signature-verifying install flow) but
//! `cargo nirdosha verify`/`build` never consulted them at all — only
//! `nirdosha-hi`'s own LLM-generation guidance loop did. This test
//! fails against that prior behavior (violating code passes clean) and
//! passes once `cargo-nirdosha::verify_sources` calls
//! `nirdosha-contract-core::pack_check` for real.

use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(source: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "nir-pack-invariants-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/main.rs"), source).unwrap();
        Self(dir)
    }

    /// Installs an active pack exactly the way `nirdosha-hi::hi_plugin`
    /// does: a manifest file under `.nir/plugins/<id>/pack.json`, plus
    /// a non-revoked row in `.nir/hi.db`'s `plugins` table.
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
        let conn = Connection::open(self.0.join(".nir").join("hi.db")).unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS plugins (id TEXT PRIMARY KEY, name TEXT, sha256 TEXT, manifest_path TEXT, installed_at TEXT, revoked_at TEXT)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO plugins (id, name, sha256) VALUES (?1, ?1, 'x')", [id]).unwrap();
    }

    fn verify(&self) -> cargo_nirdosha::ScanSummary {
        cargo_nirdosha::verify_sources(
            "pack-fixture",
            &self.0,
            &[self.0.join("src/main.rs")],
            false,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn no_active_pack_is_a_silent_no_op() {
    let fixture = Fixture::new("struct Account { balance_cents: i64 } fn main() { Account { balance_cents: 5 }; }");
    assert!(fixture.verify().violations().is_empty());
}

#[test]
fn constructing_a_protected_struct_outside_the_certified_primitive_is_refused() {
    let fixture = Fixture::new(
        r#"
        struct Account { balance_cents: i64 }
        fn debit_ledger(n: i64) -> Account { Account { balance_cents: n } }
        fn bad_charge(n: i64) -> Account { Account { balance_cents: n } }
        fn main() { debit_ledger(1); bad_charge(2); }
        "#,
    );
    fixture.install_pack("banking-v0-test", &["debit_ledger"], &["Account"]);

    let summary = fixture.verify();
    let violations = summary.violations();
    assert!(
        violations.iter().any(|f| f.message.contains("Account") && f.function.as_deref() == Some("bad_charge")),
        "expected a protected-struct-construction violation naming `bad_charge`, got: {:?}",
        violations.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_mandatory_primitive_with_no_call_site_anywhere_is_refused() {
    let fixture = Fixture::new(
        r#"
        fn debit_ledger(n: i64) -> i64 { n }
        fn main() {}
        "#,
    );
    fixture.install_pack("banking-v0-test", &["debit_ledger"], &[]);

    let summary = fixture.verify();
    let violations = summary.violations();
    assert!(
        violations.iter().any(|f| f.message.contains("debit_ledger") && f.message.contains("mandatory")),
        "expected a missing-mandatory-call-site violation, got: {:?}",
        violations.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_revoked_pack_no_longer_governs_the_build() {
    let fixture = Fixture::new(
        r#"
        struct Account { balance_cents: i64 }
        fn bad_charge(n: i64) -> Account { Account { balance_cents: n } }
        fn main() { bad_charge(1); }
        "#,
    );
    fixture.install_pack("banking-v0-test", &[], &["Account"]);
    // Revoke it.
    let conn = Connection::open(fixture.0.join(".nir").join("hi.db")).unwrap();
    conn.execute("UPDATE plugins SET revoked_at = '2026-01-01' WHERE id = 'banking-v0-test'", []).unwrap();

    assert!(fixture.verify().violations().is_empty());
}
