//! End-to-end tests for `nirdosha suggest-contracts`
//! (`cmd_suggest_contracts`/`hi_llm::suggest_contract` in `main.rs`/
//! `hi_llm.rs`, `nirdosha-master-plan.md` Part 3 Q1 2027's "LLM-
//! assisted contract inference"). The parts that don't need a live LLM
//! provider (usage errors, missing function, an already-`validate`d
//! function refused, no credentials configured) are tested here for
//! real, the same "spawn the real binary" pattern every other
//! integration test in this crate uses.
//!
//! **The actual LLM-calling path was verified manually against real
//! providers, not simulated here** -- CI/regression runs can't depend
//! on a live API key/quota/local Ollama daemon, the same
//! `crates/bench/RESULTS.md`-disclosed constraint for the same reason.
//! Three real, observed runs, worth recording as the evidence this
//! feature's core claim ("never trust the suggestion blindly, always
//! really check it") actually holds:
//! 1. Against Gemini 2.5 Flash, before `agent-skills/nirdosha/
//!    paste-anywhere-prompt.md` had any `validate`/`pre`/`post` syntax
//!    documentation at all (a real gap this feature's own manual
//!    testing discovered and fixed), the model produced
//!    `requires(...)`/`ensures(...)` -- plausible in other languages, a
//!    parse error in Nirdosha. Correctly reported as `DISPROVED` with
//!    the real diagnostic, not silently accepted.
//! 2. Still against Gemini, after the prompt fix: syntactically correct
//!    `pre:`/`post:` Nirdosha for an `abs_value` function, including a
//!    precondition explicitly excluding `i64::MIN` (mathematically the
//!    right call -- `abs(i64::MIN)` overflows `i64`) -- but spelled the
//!    exclusion as `x > -9223372036854775808`, and Nirdosha's lexer
//!    rejects that specific literal as out of range for a *positive*
//!    `i64` token before the unary minus ever applies. A real, honest
//!    `DISPROVED` (a hard load-stage failure, same bucket a parse error
//!    lands in) surfaced a real language quirk, not a logic mistake.
//!    Gemini's free-tier daily quota ran out on the very next call,
//!    genuinely blocking further manual testing with it that day.
//! 3. Against a local Ollama daemon proxying a cloud-hosted code model
//!    (`kimi-k2.7-code:cloud`, `http://localhost:11434/v1` -- no API
//!    key, no quota, `NIRDOSHA_LLM_PROVIDER_BASE` pointed at it): a
//!    genuine success, `validate clamp_to_zero { pre: true post:
//!    result >= 0 && (result == x || x < 0 && result == 0) }`, real Z3
//!    proof (`contracts_proved: 1`, `proven_in_range: 1`). Ollama's
//!    OpenAI-compatible endpoint needs no code change at all here --
//!    `hi_llm::resolve_activation`'s existing `NIRDOSHA_LLM_PROVIDER_*`
//!    trio already covers any OpenAI-compatible base URL, Ollama
//!    included; it is a genuinely good default for local testing of
//!    this feature and `crates/bench` alike, since it needs no
//!    external account or billing.

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn scratch_file(name: &str, src: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nirdosha_suggest_contracts_test_{}_{}_{name}.nir", std::process::id(), unique_suffix()));
    std::fs::write(&p, src).expect("scratch .nir file should write");
    p
}

fn run(args: &[&str]) -> (Vec<u8>, Vec<u8>, i32) {
    // Clear any LLM credentials this environment happens to have, so
    // every test below reaches the same "no provider configured"
    // outcome deterministically, regardless of what's set in whatever
    // shell runs the suite.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nirdosha"))
        .args(args)
        .env_remove("NIRDOSHA_LLM_PROVIDER_KEY")
        .env_remove("NIRDOSHA_LLM_PROVIDER_MODEL")
        .env_remove("NIRDOSHA_LLM_PROVIDER_BASE")
        .env_remove("OPENAI_API_KEY")
        .output()
        .expect("nirdosha should run");
    (output.stdout, output.stderr, output.status.code().expect("process should exit with a status code, not be signal-killed"))
}

#[test]
fn missing_arguments_is_a_usage_error() {
    let (_stdout, stderr, code) = run(&["suggest-contracts"]);
    assert_eq!(code, 1);
    assert!(String::from_utf8_lossy(&stderr).contains("usage:"), "stderr: {}", String::from_utf8_lossy(&stderr));
}

#[test]
fn an_unknown_function_name_fails_before_ever_touching_an_llm_provider() {
    let path = scratch_file("no_such_fn", "fn f(a: i64) -> i64 {\n    return a\n}\n");
    let (_stdout, stderr, code) = run(&["suggest-contracts", path.to_str().unwrap(), "does_not_exist"]);
    assert_eq!(code, 1);
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("no such function"), "stderr: {stderr}");
    assert!(!stderr.contains("provider"), "an unknown function should fail before activation is even attempted: {stderr}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_function_that_already_has_a_validate_block_is_refused() {
    let path = scratch_file(
        "already_validated",
        "fn f(a: i64) -> i64 {\n    return a\n}\n\nvalidate f {\n    post: result == a\n}\n",
    );
    let (_stdout, stderr, code) = run(&["suggest-contracts", path.to_str().unwrap(), "f"]);
    assert_eq!(code, 1);
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("already has a `validate` block"), "stderr: {stderr}");
    assert!(!stderr.contains("provider"), "an already-validated function should be refused before activation is even attempted: {stderr}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn no_llm_provider_configured_is_a_clear_error_not_a_panic() {
    let path = scratch_file("no_provider", "fn f(a: i64) -> i64 {\n    return a\n}\n");
    let (_stdout, stderr, code) = run(&["suggest-contracts", path.to_str().unwrap(), "f"]);
    assert_eq!(code, 1);
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stderr.contains("no LLM provider configured"), "stderr: {stderr}");
    let _ = std::fs::remove_file(&path);
}
