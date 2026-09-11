//! Regression coverage for `examples/fintech-canon/` (`nirdosha-
//! master-plan.md` Part 3 Dec 2026's "Fintech canon v1"): every
//! `validate`-based template must stay genuinely `PROVED` with a real
//! Z3 proof (`contracts_proved: 1`), not just compile -- a future
//! compiler change that silently regresses Tier-1's modeling of one of
//! these patterns should fail a test here, not go unnoticed until
//! someone reads `examples/fintech-canon/RESULTS.md` by hand. Real
//! `nirdosha certify`/`build` runs against the actual checked-in
//! files, the same "spawn the real binary" discipline every other
//! integration test in this crate already uses.

use std::path::PathBuf;

fn canon_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/fintech-canon")
}

fn certify(file_name: &str) -> serde_json::Value {
    let path = canon_dir().join(file_name);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("certify").arg(&path).output().expect("nirdosha certify should run");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{file_name}: certify did not print valid JSON ({e}): stderr={}", String::from_utf8_lossy(&output.stderr)))
}

fn assert_proved_with_one_real_obligation(file_name: &str) {
    let cert = certify(file_name);
    assert_eq!(cert["verdict_summary"]["verdict"], "PROVED", "{file_name} should be PROVED: {cert}");
    assert_eq!(cert["verdict_summary"]["contracts_proved"], 1, "{file_name} should have exactly one real proved obligation: {cert}");
    assert_eq!(cert["verdict_summary"]["contracts_failed"], 0, "{file_name}: {cert}");
    assert_eq!(cert["verdict_summary"]["contracts_unsupported"], 0, "{file_name}: {cert}");
    assert_eq!(cert["evidence_tier"], "proved", "{file_name}: {cert}");
}

#[test]
fn canon_01_nonnegative_balance_after_debit() {
    assert_proved_with_one_real_obligation("01_nonnegative_balance_after_debit.nir");
}

#[test]
fn canon_02_overdraft_within_limit() {
    assert_proved_with_one_real_obligation("02_overdraft_within_limit.nir");
}

#[test]
fn canon_03_fee_never_reduces_total() {
    assert_proved_with_one_real_obligation("03_fee_never_reduces_total.nir");
}

#[test]
fn canon_04_refund_never_exceeds_original() {
    assert_proved_with_one_real_obligation("04_refund_never_exceeds_original.nir");
}

#[test]
fn canon_05_late_fee_capped() {
    assert_proved_with_one_real_obligation("05_late_fee_capped.nir");
}

#[test]
fn canon_06_minimum_payment_enforced() {
    assert_proved_with_one_real_obligation("06_minimum_payment_enforced.nir");
}

#[test]
fn canon_07_discount_bounded_price() {
    assert_proved_with_one_real_obligation("07_discount_bounded_price.nir");
}

#[test]
fn canon_08_transaction_within_daily_limit() {
    assert_proved_with_one_real_obligation("08_transaction_within_daily_limit.nir");
}

#[test]
fn canon_09_interest_accrual_nonnegative() {
    assert_proved_with_one_real_obligation("09_interest_accrual_nonnegative.nir");
}

#[test]
fn canon_10_ledger_debit_credit_conserved() {
    assert_proved_with_one_real_obligation("10_ledger_debit_credit_conserved.nir");
}

#[test]
fn canon_11_withdrawal_within_balance() {
    assert_proved_with_one_real_obligation("11_withdrawal_within_balance.nir");
}

#[test]
fn canon_12_credit_limit_not_exceeded() {
    assert_proved_with_one_real_obligation("12_credit_limit_not_exceeded.nir");
}

#[test]
fn canon_13_masked_account_pii_compiles_and_masks_for_real() {
    let src = canon_dir().join("13_masked_account_pii.nir");
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_fintech_canon_13_{}", std::process::id()));

    let build = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha")).arg("build").arg(&src).arg("-o").arg(&out_path).output().expect("nirdosha build should run");
    assert!(build.status.success(), "build should succeed: stderr={}", String::from_utf8_lossy(&build.stderr));

    let run = std::process::Command::new(&out_path).output().expect("the compiled binary should run");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("ACC-000123456"), "the compliance-cleared caller should see the real account number: {stdout}");
    assert!(stdout.contains("Dana Support"), "the unmasked display_name field should still pass through: {stdout}");
    // The masked field prints as an empty string -- a blank line
    // between two other lines of real output, not "ACC-000123456"
    // repeated a second time for the non-compliance caller.
    assert_eq!(stdout.matches("ACC-000123456").count(), 1, "the account number must appear only once (masked for the second caller): {stdout}");

    let _ = std::fs::remove_file(&out_path);
}
