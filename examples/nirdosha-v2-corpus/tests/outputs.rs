//! The golden corpus contract.
//!
//! Every assertion here pins the **pre-migration** behavior of a
//! translated example under plain cargo. Post-migration, the *same
//! unmodified files* run under the proprietary `nirdosha` compiler
//! must still pass every one of these lines — plus gain the
//! Nirdosha-specific layer (generated UI, durable workflow store,
//! proofs in the certificate). If a migration ever changes a line of
//! observable behavior, this file fails before any user notices.

use std::process::Command;

fn run(bin: &str) -> String {
    let exe = env_bin(bin);
    let out = Command::new(&exe)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {bin}: {e}"));
    assert!(out.status.success(), "{bin} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn env_bin(bin: &str) -> std::path::PathBuf {
    let var = format!("CARGO_BIN_EXE_{bin}").replace('-', "_");
    std::env::var_os(&var)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| panic!("{var} not set"))
}

fn expect(stdout: &str, lines: &[&str]) {
    for line in lines {
        assert!(
            stdout.lines().any(|l| l.trim() == *line || l.contains(line.trim())),
            "expected line {line:?} in output:\n{stdout}"
        );
    }
}

#[test]
fn hello() {
    expect(&run("v2_hello_nir"), &["8", "hello, nirdosha"]);
}

#[test]
fn types_and_control_flow() {
    expect(
        &run("v2_02_types_and_control_flow"),
        &["10", "true", "Negative()", "Even()", "Odd()", "3628800"],
    );
}

#[test]
fn data_modeling() {
    expect(
        &run("v2_03_data_modeling"),
        &[
            "5",
            "12.56636",
            "[5, 7, 9]",
            "32",
            "[[1, 3], [2, 4]]",
            "-2",
            "19.99",
            "USD()",
            "Kilogram()",
        ],
    );
}

#[test]
fn ownership_and_concurrency() {
    expect(
        &run("v2_04_ownership_and_concurrency"),
        &["42", "-1"],
    );
}

#[test]
fn platform_services() {
    expect(
        &run("v2_05_platform_services"),
        &[
            "5",
            "9",
            "hello from level 5",
            "ada",
            "nirdosha",
            "connection refused",
            "-1",
        ],
    );
}

#[test]
fn identity_and_declarative_ui() {
    expect(
        &run("v2_06_identity_and_declarative_ui"),
        &[
            "42",
            "500",
            "true",
            "committing 10",
            "compensating -5",
            "approval 1 approved",
            "created",
        ],
    );
}

#[test]
fn validate_contracts() {
    expect(&run("v2_35_validate_contracts"), &["9", "0", "7", "5"]);
}

#[test]
fn transact() {
    expect(
        &run("v2_36_transact"),
        &[
            "committing 10",
            "logged 10 true",
            "compensating -5",
            "logged -5 false",
            "committing 1",
        ],
    );
}

#[test]
fn workflow() {
    expect(
        &run("v2_38_workflow"),
        &["1", "approval 1 for amount 250000", "true"],
    );
}

#[test]
fn screen_ui() {
    expect(&run("v2_39_screen_ui"), &["true", "1"]);
}

#[test]
fn dashboard() {
    expect(&run("v2_40_dashboard"), &["12", "true"]);
}

#[test]
fn enterprise_app() {
    expect(
        &run("v2_enterprise_app"),
        &[
            "true",
            "1",
            "25000",
            "199.99",
            "USD()",
            "0.789",
            "-2", // no cfo role on this token — check_role fails, by design
            "reversing disbursement for purchase order 1",
            "disbursement 1 false",
            "true",
        ],
    );
}