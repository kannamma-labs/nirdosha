//! Issue #74: `--deep` (Stage 2, MIR effect lattice) must be the
//! *default* certifying gate; Stage 1's body-local source scan is
//! demoted to an opt-in `--fast`/`--shallow` escape hatch.
//!
//! `examples/rt-payroll-pure-chain` exists exactly to demonstrate the
//! gap: every `effects(pure)`-claiming function is locally clean, but
//! `net_total -> ledger_overrides -> read_ledger` reaches file I/O only
//! through the call graph. Stage 1 cannot see through the call — only
//! Stage 2 resolves the transitive closure and refuses.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn example_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join(name)
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-nirdosha"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("cargo-nirdosha must run")
}

fn run(args: &[&str]) -> Output {
    run_in(&example_dir("rt-payroll-pure-chain"), args)
}

#[test]
fn default_gate_catches_the_indirect_purity_lie() {
    // No `--deep`, no `--fast` — plain `cargo nirdosha check`. Before
    // issue #74's fix this passed (Stage 1 was the default and its
    // body-local scan cannot see the lie); after the fix, Stage 2 rides
    // along unconditionally and refuses it.
    let out = run(&["check"]);
    assert!(
        !out.status.success(),
        "default `cargo nirdosha check` must refuse an indirect purity lie \
         (the whole point of making Stage 2 the certifying default) — \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("net_total") && stderr.contains("effect lattice"),
        "refusal must name the lying chain, not fail for an unrelated reason: {stderr}"
    );
}

#[test]
fn compliant_flagship_example_still_passes_the_new_default() {
    // rt-payroll is the dialect's own "this is what compliant code looks
    // like" demo. Flipping the default gate to Stage 2 must not turn a
    // false positive into the compliant example's own regression: its
    // `#[contract(effects(pure), ...)]` expansion calls `nirdosha_rt::
    // enter` (the NFR guard) and owns the `Guard` it returns until scope
    // exit, both of which need curated effect-summary coverage (the
    // call itself, and its destructor) to be recognized as pure.
    let out = run_in(&example_dir("rt-payroll"), &["check"]);
    assert!(
        out.status.success(),
        "the compliant flagship example must still pass the default gate — stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn shallow_escape_hatch_opts_back_into_stage_1_only() {
    // `--fast`/`--shallow` explicitly asks for the weaker, documented
    // over-approximation — it must NOT see the indirect lie (that's the
    // known, honest limit Stage 1 is being demoted to admit).
    for flag in ["--fast", "--shallow"] {
        let out = run(&["check", flag]);
        assert!(
            out.status.success(),
            "`cargo nirdosha check {flag}` must still pass (Stage-1-only, \
             body-local scan) — stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
