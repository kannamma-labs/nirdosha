//! End-to-end tests for `nirdosha fix` (`cmd_fix`/`fix_unbound_identifier`
//! in `main.rs`, `nirdosha-master-plan.md` Part 3 Sprint 1: "byte-offset
//! `FixPatch`, fixability classes (auto / assisted / manual)"). Same
//! `std::process::Command`-against-the-real-binary pattern
//! `verify_verdict.rs` uses, extended to also exercise `--apply`
//! actually rewriting the file on disk.
//!
//! `auto_typo_patch_is_applied_to_disk` is a regression test for a real
//! bug caught by hand-running this command end to end: `--apply` only
//! ever collected `Auto` patches from `contracts.obligations`, silently
//! dropping every `Auto` fix attached to a `typecheck`-stage
//! `VerifyDiagnostic` (`fix_unbound_identifier` firing out of
//! `typeck.rs`'s general `UnknownVar` case, not just `contract_check.rs`'s
//! `validate`-predicate case) -- the JSON proposed the correct patch,
//! `--apply` wrote zero of them.

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_file(name: &str, src: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_fix_command_test_{}_{}_{name}.nir", std::process::id(), unique_suffix()));
    std::fs::write(&p, src).expect("scratch .nir file should write");
    p
}

fn run_fix(path: &std::path::Path, apply: bool) -> (serde_json::Value, i32) {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"));
    cmd.arg("fix").arg(path);
    if apply {
        cmd.arg("--apply");
    }
    let output = cmd.output().expect("nirdosha fix should run");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("fix should print valid JSON");
    let code = output.status.code().expect("process should exit with a status code, not be signal-killed");
    (value, code)
}

#[test]
fn auto_typo_report_without_apply_proposes_a_patch_but_writes_nothing() {
    let src = "fn main() {\n    let amount: i64 = 5\n    print(ammount)\n}\n";
    let path = scratch_file("auto_no_apply", src);
    let (report, code) = run_fix(&path, false);
    assert_eq!(code, 1, "a real typecheck failure is DISPROVED, exit 1: {report}");

    let fix = &report["before"]["typecheck"]["errors"][0]["fix"];
    assert_eq!(fix["applicability"], "auto", "report: {report}");
    assert_eq!(fix["patch"]["replacement"], "amount", "report: {report}");

    assert_eq!(report["applied"].as_array().expect("applied should be an array"), &Vec::<serde_json::Value>::new(), "no --apply means nothing written: {report}");
    assert!(report["after"].is_null(), "no --apply means no re-verify pass: {report}");
    assert_eq!(std::fs::read_to_string(&path).expect("scratch file should still exist"), src, "file must be untouched without --apply: {report}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn auto_typo_patch_is_applied_to_disk() {
    // Regression test: this is the exact shape of fix that a prior
    // revision's `--apply` silently dropped (see module doc comment) --
    // an `UnknownVar` typo caught by `typeck.rs`'s general check, not
    // `contract_check.rs`'s `validate`-predicate-only path.
    let src = "fn main() {\n    let amount: i64 = 5\n    print(ammount)\n}\n";
    let path = scratch_file("auto_apply", src);
    let (report, code) = run_fix(&path, true);

    let applied = report["applied"].as_array().expect("applied should be an array");
    assert_eq!(applied.len(), 1, "exactly one Auto patch should have been written: {report}");
    assert_eq!(applied[0]["replacement"], "amount", "report: {report}");

    let patched = std::fs::read_to_string(&path).expect("scratch file should still exist");
    assert_eq!(patched, "fn main() {\n    let amount: i64 = 5\n    print(amount)\n}\n", "the typo must be corrected on disk, byte-for-byte, nothing else touched");

    assert!(!report["after"].is_null(), "--apply must re-verify after patching: {report}");
    assert_eq!(report["after"]["typecheck"]["status"], "passed", "the corrected file must typecheck now: {report}");
    assert_eq!(code, 0, "a fully corrected file is PROVED, exit 0: {report}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn two_equally_close_candidates_is_assisted_not_a_guess() {
    let src = "fn main() {\n    let foo1: i64 = 1\n    let foo2: i64 = 2\n    print(foo3)\n}\n";
    let path = scratch_file("assisted", src);
    let (report, code) = run_fix(&path, false);
    assert_eq!(code, 1, "report: {report}");

    let fix = &report["before"]["typecheck"]["errors"][0]["fix"];
    assert_eq!(fix["applicability"], "assisted", "`foo3` is equidistant from `foo1` and `foo2`, must not silently pick one: {report}");
    assert!(fix["patch"].is_null(), "an assisted fix with more than one tied candidate offers no single patch: {report}");
    let rationale = fix["rationale"].as_str().expect("rationale should be a string");
    assert!(rationale.contains("foo1") && rationale.contains("foo2"), "rationale should name both tied candidates: {rationale}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn unrelated_name_is_manual_not_a_forced_guess() {
    let src = "fn main() {\n    let x: i64 = 1\n    print(zzzzzzz)\n}\n";
    let path = scratch_file("manual", src);
    let (report, code) = run_fix(&path, false);
    assert_eq!(code, 1, "report: {report}");

    let fix = &report["before"]["typecheck"]["errors"][0]["fix"];
    assert_eq!(fix["applicability"], "manual", "`zzzzzzz` isn't a plausible typo of anything in scope: {report}");
    assert!(fix["patch"].is_null(), "a manual fix has no patch by definition: {report}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_target_typo_proposes_an_auto_patch_against_real_fn_names() {
    // `validate <fn_name> { ... }` where `<fn_name>` doesn't resolve is
    // caught by `typeck.rs`'s `ValidateFnNotFound` -- same typo shape as
    // `UnknownVar`, `fix_unbound_identifier` reused directly against
    // `program.fns`' own names. Regression-relevant: the patch must land
    // on `addonex`'s own byte range (`ValidateDecl::fn_name_span`), not the
    // `validate` keyword's.
    let src = "fn addone(x: i64) -> i64 {\n    return x + 1\n}\n\nvalidate addonex {\n    post: result == x + 1\n}\n";
    let path = scratch_file("validate_typo_no_apply", src);
    let (report, code) = run_fix(&path, false);
    assert_eq!(code, 1, "report: {report}");

    let fix = &report["before"]["typecheck"]["errors"][0]["fix"];
    assert_eq!(fix["applicability"], "auto", "report: {report}");
    assert_eq!(fix["patch"]["replacement"], "addone", "report: {report}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn validate_target_typo_patch_is_applied_to_disk_at_the_right_offset() {
    let src = "fn addone(x: i64) -> i64 {\n    return x + 1\n}\n\nvalidate addonex {\n    post: result == x + 1\n}\n";
    let path = scratch_file("validate_typo_apply", src);
    let (report, code) = run_fix(&path, true);

    let applied = report["applied"].as_array().expect("applied should be an array");
    assert_eq!(applied.len(), 1, "exactly one Auto patch should have been written: {report}");
    assert_eq!(applied[0]["replacement"], "addone", "report: {report}");

    let patched = std::fs::read_to_string(&path).expect("scratch file should still exist");
    assert_eq!(
        patched,
        "fn addone(x: i64) -> i64 {\n    return x + 1\n}\n\nvalidate addone {\n    post: result == x + 1\n}\n",
        "only `addonex` -> `addone` must change, at its own byte offset (not the `validate` keyword's)"
    );
    assert!(!report["after"].is_null(), "--apply must re-verify after patching: {report}");
    assert_eq!(report["after"]["typecheck"]["status"], "passed", "report: {report}");
    assert_eq!(code, 0, "the corrected, Tier-1-provable contract is PROVED, exit 0: {report}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn apply_with_nothing_auto_to_fix_still_reruns_and_reports_empty() {
    // `contracts.obligations`'s own `unsupported`/`counterexample` kinds
    // carry no `Fix` at all -- `--apply` on a file whose only problem is
    // one of those must not error, just report zero applied patches and
    // still run the `after` pass (matching `apply`'s documented always-
    // re-verify behavior).
    let src = "fn flip(x: bool) -> bool {\n    return x\n}\n\nvalidate flip {\n    post: true\n}\n";
    let path = scratch_file("nothing_to_apply", src);
    let (report, code) = run_fix(&path, true);
    assert_eq!(report["applied"].as_array().expect("applied should be an array").len(), 0, "report: {report}");
    assert!(!report["after"].is_null(), "report: {report}");
    assert_eq!(code, 2, "UNKNOWN (Z3 can't model a bool predicate) stays UNKNOWN, exit 2: {report}");

    let _ = std::fs::remove_file(&path);
}
