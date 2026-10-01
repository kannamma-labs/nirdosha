//! `nirdosha-master-plan.md` Part 3 Sprint 2's "Benchmark harness v1":
//! pass@1 and self-repair-rate, measured for real against a real LLM
//! (`nirdosha_hi::hi_llm::generate_from_task_prompt`, the exact
//! generate/self-repair loop `nirdosha hi`'s own Generate mode uses),
//! scored by `nirdosha_hi::v2_verify::verify_v2_source` -- the same
//! in-process two-reader check (`cargo build`, then `cargo-nirdosha`'s
//! contract scanner) `generate_from_task_prompt`'s own self-repair loop
//! already gates every attempt on, never a second, bench-local notion of
//! "correct" that could drift from what the compiler itself says.
//!
//! **Repointed to v2, 2026-09-20.** This previously shelled out to
//! `nirdosha certify` (`crates/compiler`'s CLI, deleted the same day as
//! `crates/compiler` itself) and scored a v1-style JSON verdict
//! (`verdict_summary.verdict`: PROVED/DISPROVED/UNKNOWN, Z3-backed).
//! `nirdosha_hi::hi_llm`'s own generate/self-repair loop had *already*
//! migrated to v2 by the time this was fixed (`typecheck_and_build_check`
//! calls `v2_verify::verify_v2_source`, and `HI_PROMPT`
//! (`agent-skills/nirdosha/hi_prompt.md`) already tells the model to
//! write v2 syntax and explicitly warns against v1 constructs) -- this
//! file's own `TASKS` prompts and `certify()` call were the only pieces
//! still stuck on v1, asking the model for syntax `HI_PROMPT` itself
//! says not to write and then scoring the result against a JSON shape no
//! longer produced by anything. Fixed by calling `v2_verify::
//! verify_v2_source` directly (in-process, no subprocess, no missing
//! binary -- the exact function `hi_llm` itself already calls) and
//! rewriting `TASKS` to ask for real v2 source.
//!
//! **Honestly weaker evidence than v1's own bench ever reported, not
//! hidden as an equivalent one.** v2 has no SMT/Z3 proof-discharge
//! pipeline yet -- `nirdosha:validate` doc-comment clauses parse but stay
//! inert (`v2_verify.rs`'s own doc comment; `docs/nirdosha-rt-dialect.md`).
//! `V2Verdict::passed()` means "built, and every `nirdosha:contract`
//! claim and dialect restriction checked out" -- a real, driver-verified
//! bar for the claims v2 *does* check (`effects(pure)`, `requires(role)`,
//! `unsafe`/raw-lock/raw-thread denials), but not a proof that the
//! program's arithmetic is correct for every input the way a v1 PROVED
//! verdict was. Reported plainly as `clean`, never relabeled `verdict`
//! or `proved`.
//!
//! **`injection` retired as a v2 task category, replaced with
//! `unauthorized_access`.** v1's injection task leaned on a guarantee
//! v2's type system does not have: v1's `str` had no concatenation
//! operator at all, making an injectable query inexpressible; v2 source
//! is plain Rust, where `String` concatenation is ordinary and
//! unrestricted (confirmed by grep: `nirdosha-contract-core/src/scan.rs`'s
//! dialect-restriction table has no injection/SQL/string-building entry
//! at all). What v2 *does* have, and this task now exercises instead, is
//! `requires(role = "..")`'s unforgeable `RoleProof<R>` -- a real,
//! compiler-injected access gate (`crates/nirdosha-rt/src/role.rs`), not
//! a narrower version of the same property but a different, equally real
//! one this compiler build actually enforces.

use nirdosha_hi::hi_llm;
use nirdosha_hi::v2_verify::V2Verdict;
use serde::Serialize;

mod cross_lang;

/// One benchmark task: a natural-language prompt an LLM must turn into
/// working v2 `.nir` source, plus which failure class it targets.
///
/// **Why only three of the master plan's four named classes
/// (unauthorized_access/overflow/type_confusion, no deadlock):**
/// `deadlock` was already left out of v1 -- scoring it needs a
/// concurrent task with an automatable race/deadlock detector, a
/// materially bigger lift than the other three; still a real gap, named
/// here rather than papered over.
struct Task {
    id: &'static str,
    category: &'static str,
    prompt: &'static str,
}

const TASKS: &[Task] = &[
    Task {
        id: "role_gated_account_lookup",
        category: "unauthorized_access",
        prompt: "Write a Nirdosha (.nir) v2 program (plain Rust plus the nirdosha-rt macro layer -- valid, ordinary Rust that also builds under plain `cargo`; do not invent native-v1 `.nir` syntax like `workflow`/`screen`/bare `validate fn { pre: .. }` blocks/`str`/`unit`/`print(...)`). Declare a role vocabulary with `nirdosha_rt::roles! { AccountAdmin = \"account_admin\"; }`. Write a function `#[nirdosha_rt::contract(effects(pure), requires(role = \"account_admin\"))] fn account_email_for(directory: &[(i64, String)], user_id: i64) -> Option<String>` that searches `directory` for an entry whose first element equals `user_id` and returns a clone of its email (the second element), or `None` if no entry matches. Because of `requires(role = \"account_admin\")`, the compiler injects an unforgeable `&nirdosha_rt::RoleProof<nirdosha_roles::AccountAdmin>` as this function's real first parameter, ahead of the two you declared -- `fn main()` must build a session with `nirdosha_rt::Auth::login(\"someone\", &[\"account_admin\"])`, obtain a proof via `.prove::<nirdosha_roles::AccountAdmin>()`, and pass it as the first argument, or this will not compile. `fn main()` should build a small sample directory, obtain the proof, call the function, and print the result. Reply with ONLY the .nir source, no prose, no markdown fence.",
    },
    Task {
        id: "overflow_checked_multiply",
        category: "overflow",
        prompt: "Write a Nirdosha (.nir) v2 program (plain Rust plus the nirdosha-rt macro layer). Write a function `#[nirdosha_rt::contract(effects(pure))] fn order_total_cents(unit_price_cents: i64, quantity: i64) -> Option<i64>` that returns the total price in cents for an order (unit price times quantity), using checked multiplication (`checked_mul`) so an overflow returns `None` instead of silently wrapping or panicking -- `effects(pure)` claims this function does no I/O and never panics, and that claim must actually be true. Include `fn main()` that calls `order_total_cents(250, 4)` and prints the result. Reply with ONLY the .nir source, no prose, no markdown fence.",
    },
    Task {
        id: "average_no_float_confusion",
        category: "type_confusion",
        prompt: "Write a Nirdosha (.nir) v2 program (plain Rust plus the nirdosha-rt macro layer). Write a function `#[nirdosha_rt::contract(effects(pure))] fn average_score(total_points: i64, num_students: i64) -> i64` that returns the average score, rounded down, given the total points scored across `num_students` students -- using integer arithmetic throughout; never reach for `f64`/`f32` where integer division already suffices. Include `fn main()` that calls `average_score(275, 4)` and prints the result. Reply with ONLY the .nir source, no prose, no markdown fence.",
    },
];

#[derive(Serialize)]
struct TaskResult {
    id: String,
    category: String,
    /// `"pass_at_1"` (compiled on the first attempt) | `"self_repair_rescued"`
    /// (compiled by a later attempt after at least one failure) |
    /// `"gave_up"` (never compiled within the bounded attempt budget).
    outcome: String,
    attempts: Option<u32>,
    /// The real, in-process two-reader verdict (`None` only for
    /// `gave_up` -- nothing to verify -- or a verify-call failure, with
    /// `error` set instead).
    verdict: Option<V2Verdict>,
    error: Option<String>,
}

#[derive(Serialize)]
struct RunSummary {
    model: String,
    tasks_total: usize,
    pass_at_1: usize,
    self_repair_rescued: usize,
    gave_up: usize,
    /// Built, with every `nirdosha:contract` claim and dialect
    /// restriction checked out (`V2Verdict::passed`) -- v2's real bar,
    /// not v1's Z3-proved one; see this file's own top doc comment.
    clean_count: usize,
    results: Vec<TaskResult>,
}

/// Every run's final generated source (whichever attempt won, or the
/// last attempt tried before giving up) plus the full summary JSON are
/// written here -- real, inspectable evidence a reader can check
/// against the numbers, the same "the demo's own files are the
/// record" discipline `examples/killer_demo/`/`examples/attack_demo/`
/// already establish, not a bench-only convention invented here.
/// Nested under a sanitized model name so re-running against a
/// *different* provider (this repo's own history: Gemini first, then
/// a local Ollama-proxied cloud model once Gemini's free-tier daily
/// quota ran out mid-session) adds a second, independently-inspectable
/// result set rather than silently overwriting the first one's real
/// evidence.
fn results_dir(model: &str) -> std::path::PathBuf {
    let safe_model: String = model.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results").join(safe_model)
}

fn main() {
    let activation = match hi_llm::resolve_activation(&|k| std::env::var(k).ok()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("nirdosha-bench: {e}");
            std::process::exit(1);
        }
    };
    // Reported for the run's own record -- read independently of
    // `Activation` (whose `model` field is deliberately private, same
    // env var either way) rather than adding a getter this binary
    // would be the only caller of.
    let model = std::env::var("NIRDOSHA_LLM_PROVIDER_MODEL").unwrap_or_else(|_| "gpt-4o-mini (OpenAI default)".to_string());
    let client = hi_llm::LlmClient::new(activation).expect("building the LLM HTTP client");
    let out_dir = results_dir(&model);
    std::fs::create_dir_all(&out_dir).expect("creating crates/bench/results should not fail");

    let mut results = Vec::new();
    for task in TASKS {
        eprintln!("=== {} ({}) ===", task.id, task.category);
        let mut log = |msg: &str| eprintln!("  {msg}");
        let result = match hi_llm::generate_from_task_prompt(&client, task.prompt, &mut log) {
            Ok((source, attempts)) => {
                let outcome = if attempts == 1 { "pass_at_1" } else { "self_repair_rescued" };
                std::fs::write(out_dir.join(format!("{}.nir", task.id)), &source).expect("writing a generated source artifact should not fail");
                match nirdosha_hi::v2_verify::verify_v2_source(&source) {
                    Ok(verdict) => TaskResult { id: task.id.to_string(), category: task.category.to_string(), outcome: outcome.to_string(), attempts: Some(attempts), verdict: Some(verdict), error: None },
                    Err(e) => TaskResult { id: task.id.to_string(), category: task.category.to_string(), outcome: outcome.to_string(), attempts: Some(attempts), verdict: None, error: Some(e) },
                }
            }
            Err(e) => TaskResult { id: task.id.to_string(), category: task.category.to_string(), outcome: "gave_up".to_string(), attempts: None, verdict: None, error: Some(e) },
        };
        eprintln!("  -> {} clean={:?}", result.outcome, result.verdict.as_ref().map(|v| v.passed()));
        results.push(result);
    }

    let pass_at_1 = results.iter().filter(|r| r.outcome == "pass_at_1").count();
    let self_repair_rescued = results.iter().filter(|r| r.outcome == "self_repair_rescued").count();
    let gave_up = results.iter().filter(|r| r.outcome == "gave_up").count();
    let clean_count = results.iter().filter(|r| r.verdict.as_ref().is_some_and(|v| v.passed())).count();

    let summary = RunSummary { model, tasks_total: TASKS.len(), pass_at_1, self_repair_rescued, gave_up, clean_count, results };
    let summary_json = serde_json::to_string_pretty(&summary).expect("RunSummary always serializes");
    std::fs::write(out_dir.join("summary.json"), &summary_json).expect("writing the summary artifact should not fail");
    println!("{summary_json}");

    if std::env::var("NIRDOSHA_BENCH_SKIP_CROSS_LANG").is_err() {
        eprintln!("\n=== cross-language baseline (TypeScript / Rust, plain LLM, no self-repair) ===");
        let cross_results = cross_lang::run(&client, &out_dir);
        for r in &cross_results {
            eprintln!("  {} [{}] -> {} ({})", r.task_id, r.language, r.outcome, r.detail);
        }
        println!("{}", serde_json::to_string_pretty(&cross_results).expect("cross-lang results always serialize"));
    }
}
