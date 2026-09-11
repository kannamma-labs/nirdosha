//! End-to-end tests for `nirdosha explain` (`cmd_explain`/`explain.rs`,
//! `nirdosha-master-plan.md` Part 3 Sprint 1: "machine-learnable error
//! index", parity target: Kōdo, Midspiral). Same
//! `std::process::Command`-against-the-real-binary pattern
//! `verify_verdict.rs`/`fix_command.rs` use.

fn run_explain(code: Option<&str>) -> (Vec<u8>, i32) {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"));
    cmd.arg("explain");
    if let Some(c) = code {
        cmd.arg(c);
    }
    let output = cmd.output().expect("nirdosha explain should run");
    let code = output.status.code().expect("process should exit with a status code, not be signal-killed");
    (output.stdout, code)
}

#[test]
fn no_argument_lists_every_code_with_a_title_only() {
    let (stdout, code) = run_explain(None);
    let value: serde_json::Value = serde_json::from_slice(&stdout).expect("should print valid JSON");
    let entries = value.as_array().expect("index should be a JSON array");
    assert!(entries.len() >= 13, "should list at least the 12 AGENTS.md rules plus NIR0013: {value}");
    for entry in entries {
        assert!(entry["code"].as_str().expect("code should be a string").starts_with("NIR"), "entry: {entry}");
        assert!(!entry["title"].as_str().expect("title should be present").is_empty(), "entry: {entry}");
        assert!(entry.get("explanation").is_none(), "the no-argument index should be titles only, not full entries: {entry}");
    }
    assert_eq!(code, 0);
}

#[test]
fn a_known_code_prints_the_full_entry() {
    let (stdout, code) = run_explain(Some("NIR0009"));
    let value: serde_json::Value = serde_json::from_slice(&stdout).expect("should print valid JSON");
    assert_eq!(value["code"], "NIR0009", "entry: {value}");
    assert!(value["title"].as_str().expect("title should be present").to_lowercase().contains("match"), "entry: {value}");
    let explanation = value["explanation"].as_str().expect("explanation should be present");
    assert!(explanation.contains("helper function"), "entry: {value}");
    assert!(!value["wrong"].as_str().expect("wrong example should be present").is_empty(), "entry: {value}");
    assert!(!value["right"].as_str().expect("right example should be present").is_empty(), "entry: {value}");
    assert_eq!(code, 0);
}

#[test]
fn lookup_is_case_insensitive() {
    let (stdout, code) = run_explain(Some("nir0009"));
    let value: serde_json::Value = serde_json::from_slice(&stdout).expect("should print valid JSON");
    assert_eq!(value["code"], "NIR0009", "entry: {value}");
    assert_eq!(code, 0);
}

#[test]
fn an_unknown_code_fails_with_a_helpful_message_not_a_crash() {
    let (stdout, code) = run_explain(Some("NIR9999"));
    assert!(stdout.is_empty(), "an unknown code should print nothing on stdout, not a null/empty JSON value");
    assert_eq!(code, 1);
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_file(name: &str, src: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_explain_command_test_{}_{}_{name}.nir", std::process::id(), unique_suffix()));
    std::fs::write(&p, src).expect("scratch .nir file should write");
    p
}

#[test]
fn a_reserved_word_used_as_a_field_name_is_auto_tagged_nir0012_in_verify() {
    let path = scratch_file("nir0012", "struct Player {\n    state: str\n}\n\nfn main() {\n}\n");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("verify")
        .arg(&path)
        .output()
        .expect("nirdosha verify should run");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("verify should print valid JSON");
    assert_eq!(value["load"]["errors"][0]["code"], "NIR0012", "verdict: {value}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn str_in_a_fn_signature_is_auto_tagged_nir0002_in_verify() {
    let path = scratch_file("nir0002", "fn greet(name: str) -> str {\n    return name\n}\n\nfn main() {\n}\n");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("verify")
        .arg(&path)
        .output()
        .expect("nirdosha verify should run");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("verify should print valid JSON");
    assert_eq!(value["typecheck"]["errors"][0]["code"], "NIR0002", "verdict: {value}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unbound_identifier_typo_is_auto_tagged_nir0013_in_verify() {
    let path = scratch_file("nir0013", "fn main() {\n    let amount: i64 = 5\n    print(ammount)\n}\n");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("verify")
        .arg(&path)
        .output()
        .expect("nirdosha verify should run");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("verify should print valid JSON");
    assert_eq!(value["typecheck"]["errors"][0]["code"], "NIR0013", "verdict: {value}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_bare_variant_name_used_without_call_syntax_is_auto_tagged_nir0001_in_verify() {
    // NIR0001's own "wrong" example: a bare variant name (`Circle`, no
    // `(...)`) parses as a plain identifier, so it's an `UnknownVar` --
    // indistinguishable from a real typo (NIR0013) unless the unresolved
    // name is checked against the program's own declared variant names.
    let path = scratch_file(
        "nir0001",
        "enum Shape {\n    Circle(i64),\n    Square(i64),\n}\n\nfn main() {\n    let s: Shape = Circle\n}\n",
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("verify")
        .arg(&path)
        .output()
        .expect("nirdosha verify should run");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("verify should print valid JSON");
    assert_eq!(value["typecheck"]["errors"][0]["code"], "NIR0001", "verdict: {value}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn mixing_i32_and_i64_without_a_conversion_is_auto_tagged_nir0006_in_verify() {
    let path = scratch_file(
        "nir0006",
        "fn main() {\n    let a: i32 = 1\n    let b: i64 = 2\n    let c: i64 = a + b\n}\n",
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("verify")
        .arg(&path)
        .output()
        .expect("nirdosha verify should run");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("verify should print valid JSON");
    assert_eq!(value["typecheck"]["errors"][0]["code"], "NIR0006", "verdict: {value}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_wildcard_arm_on_an_enum_match_is_auto_tagged_nir0008_in_verify() {
    let path = scratch_file(
        "nir0008_wildcard",
        "enum Shape {\n    Circle(i64),\n    Square(i64),\n}\n\nfn area(shape: Shape) -> i64 {\n    return match shape {\n        Circle(r) => 1,\n        _ => 0,\n    }\n}\n\nfn main() {\n}\n",
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("verify")
        .arg(&path)
        .output()
        .expect("nirdosha verify should run");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("verify should print valid JSON");
    assert_eq!(value["typecheck"]["errors"][0]["code"], "NIR0008", "verdict: {value}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_missing_variant_arm_is_auto_tagged_nir0008_in_verify() {
    let path = scratch_file(
        "nir0008_missing_arm",
        "enum Shape {\n    Circle(i64),\n    Square(i64),\n}\n\nfn area(shape: Shape) -> i64 {\n    return match shape {\n        Circle(r) => 1,\n    }\n}\n\nfn main() {\n}\n",
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .arg("verify")
        .arg(&path)
        .output()
        .expect("nirdosha verify should run");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("verify should print valid JSON");
    assert_eq!(value["typecheck"]["errors"][0]["code"], "NIR0008", "verdict: {value}");
    let _ = std::fs::remove_file(&path);
}
