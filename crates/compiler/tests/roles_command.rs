//! `nirdosha roles` (`ROADMAP.md` A6, "Roles -> functions/fields
//! report") -- pure static analysis grouping every role/claim gate in a
//! program by the role/claim itself: which fns it gates
//! (`requires(role/claim: ...)`) and which `screen` fields it gates
//! (`view`/`edit`). Same `std::process::Command`-against-the-real-binary
//! pattern `fix_command.rs`/`verify_verdict.rs` use.

fn run_roles(src: &str) -> (serde_json::Value, i32) {
    let mut path = std::env::temp_dir();
    path.push(format!("nirdosha_roles_command_test_{}_{}.nir", std::process::id(), unique_suffix()));
    std::fs::write(&path, src).expect("scratch .nir file should write");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("roles")
        .arg(&path)
        .output()
        .expect("nirdosha roles should run");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let code = output.status.code().expect("process should exit with a status code, not be signal-killed");
    let _ = std::fs::remove_file(&path);
    if code != 0 {
        panic!("nirdosha roles failed (exit {code}): {}", String::from_utf8_lossy(&output.stderr));
    }
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("roles should print valid JSON");
    (value, code)
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[test]
fn fn_level_role_and_claim_gates_are_grouped_by_the_role_claim() {
    let src = r#"
        fn admin_only() -> i64 requires(role: "admin") { return 1 }
        fn dept_only() -> i64 requires(claim: "department", "eng") { return 1 }
        fn main() {}
    "#;
    let (report, _code) = run_roles(src);

    assert_eq!(report["roles"]["admin"]["functions"], serde_json::json!(["admin_only"]), "report: {report}");
    assert_eq!(report["claims"][0]["key"], "department", "report: {report}");
    assert_eq!(report["claims"][0]["value"], "eng", "report: {report}");
    assert_eq!(report["claims"][0]["functions"], serde_json::json!(["dept_only"]), "report: {report}");
}

#[test]
fn screen_field_view_edit_role_gates_are_attributed_to_their_struct() {
    let src = r#"
        struct Widget { id: i64, name: str, price: i64 }
        fn list_widget() -> i64 { return 0 }
        screen Widget {
            field name {
                view: role("admin", "analyst")
            }
            field price {
                edit: role("admin")
            }
        }
        fn main() {}
    "#;
    let (report, _code) = run_roles(src);

    let admin_view = &report["roles"]["admin"]["view_fields"];
    assert_eq!(admin_view, &serde_json::json!([{"struct": "Widget", "field": "name"}]), "report: {report}");
    let admin_edit = &report["roles"]["admin"]["edit_fields"];
    assert_eq!(admin_edit, &serde_json::json!([{"struct": "Widget", "field": "price"}]), "report: {report}");
    let analyst_view = &report["roles"]["analyst"]["view_fields"];
    assert_eq!(analyst_view, &serde_json::json!([{"struct": "Widget", "field": "name"}]), "report: {report}");
    assert_eq!(report["roles"]["analyst"]["functions"], serde_json::json!([]), "a role that only gates a field, never a fn, still has an entry with an empty functions list: {report}");
}

#[test]
fn a_program_with_no_role_or_claim_gates_yields_empty_roles_and_claims() {
    let src = "fn f() -> i64 requires(public) { return 1 }\nfn main() {}\n";
    let (report, _code) = run_roles(src);
    assert_eq!(report["roles"], serde_json::json!({}), "report: {report}");
    assert_eq!(report["claims"], serde_json::json!([]), "report: {report}");
}
