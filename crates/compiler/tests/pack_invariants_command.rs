//! Issue #76: "a signed pack that claims 'certified'/'trusted' but is
//! never checked by the compiler is a soundness bug." Before this fix,
//! `nirdosha build` never consulted an installed, active domain pack's
//! `mandatory_fns`/`protected_structs` invariants at all —
//! `contract_check::check_mandatory_primitive_call_sites`/
//! `check_primitive_exclusivity` (both real, already unit-tested) had
//! zero callers anywhere in this binary; only `nirdosha-hi`'s own
//! LLM-generation guidance loop ever consulted a pack's rules. This
//! test fails against that prior behavior (a real, active pack's
//! `protected_structs` rule is silently ignored) and passes once
//! `cmd_build` calls those checkers for real, refusing the build.
use std::sync::atomic::{AtomicU64, Ordering};

fn unique_suffix() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn project_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nirdosha_pack_invariants_command_{}_{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&dir).expect("scratch project dir should create");
    dir
}

/// Installs an active pack exactly the way `nirdosha-hi::hi_plugin`
/// does: a manifest under `<root>/.nir/plugins/<id>/pack.json`, plus a
/// non-revoked row in `<root>/.nir/hi.db`'s `plugins` table.
fn install_pack(root: &std::path::Path, id: &str, mandatory_fns: &[&str], protected_structs: &[&str]) {
    let plugins_dir = root.join(".nir").join("plugins").join(id);
    std::fs::create_dir_all(&plugins_dir).unwrap();
    std::fs::write(
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
    let conn = rusqlite::Connection::open(root.join(".nir").join("hi.db")).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS plugins (id TEXT PRIMARY KEY, name TEXT, sha256 TEXT, manifest_path TEXT, installed_at TEXT, revoked_at TEXT)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO plugins (id, name, sha256) VALUES (?1, ?1, 'x')", [id]).unwrap();
}

fn build(dir: &std::path::Path, src_name: &str) -> std::process::Output {
    let src = dir.join(src_name);
    let out = dir.join("out_bin");
    std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .current_dir(dir)
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("nirdosha build should run")
}

const MAIN_NIR: &str = "fn main() requires(public) {\n}\n";

#[test]
fn no_active_pack_is_a_silent_no_op() {
    let dir = project_dir();
    std::fs::write(dir.join("app.nir"), MAIN_NIR).unwrap();
    let output = build(&dir, "app.nir");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn constructing_a_protected_struct_outside_the_certified_primitive_is_refused() {
    let dir = project_dir();
    std::fs::write(
        dir.join("app.nir"),
        r#"
        struct Account {
            balance_cents: i64,
        }

        fn debit_ledger(n: i64) -> Account {
            return Account(n)
        }

        fn bad_charge(n: i64) -> Account {
            return Account(n)
        }

        fn main() requires(public) {
        }
        "#,
    )
    .unwrap();
    install_pack(&dir, "banking-v0-test", &[], &["Account"]);

    let output = build(&dir, "app.nir");
    assert!(!output.status.success(), "expected the build to refuse, stdout: {}", String::from_utf8_lossy(&output.stdout));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bad_charge") && stderr.contains("Account"),
        "expected a violation naming `bad_charge`/`Account`, got: {stderr}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_revoked_pack_no_longer_governs_the_build() {
    let dir = project_dir();
    std::fs::write(
        dir.join("app.nir"),
        r#"
        struct Account {
            balance_cents: i64,
        }

        fn bad_charge(n: i64) -> Account {
            return Account(n)
        }

        fn main() requires(public) {
        }
        "#,
    )
    .unwrap();
    install_pack(&dir, "banking-v0-test", &[], &["Account"]);
    let conn = rusqlite::Connection::open(dir.join(".nir").join("hi.db")).unwrap();
    conn.execute("UPDATE plugins SET revoked_at = '2026-01-01' WHERE id = 'banking-v0-test'", []).unwrap();

    let output = build(&dir, "app.nir");
    assert!(output.status.success(), "a revoked pack must not govern the build: {}", String::from_utf8_lossy(&output.stderr));
    std::fs::remove_dir_all(&dir).ok();
}
