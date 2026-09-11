//! `nirdosha-master-plan.md` Part 3 Sprint 2's "Benchmark harness v1":
//! pass@1 and self-repair-rate, measured for real against a real LLM
//! (`nirdosha::hi_llm::generate_from_task_prompt`, the exact
//! generate/self-repair loop `nirdosha hi`'s own Generate mode uses),
//! scored by `nirdosha certify`'s own JSON verdict -- never a second,
//! bench-local notion of "correct" that could drift from what the
//! compiler itself says.
//!
//! **Honest v1 scope, per the master plan's own comparison matrix**
//! (`Benchmark harness v1 (public, reproducible): Nirdosha vs
//! TypeScript vs Rust vs plain-LLM vs LLM+XGrammar vs LLM+Imandra ...
//! AlgoVeri/Vericoding tasks -> direct published comparison vs Kōdo's
//! 20/20 claim`): this harness measures **Nirdosha only** --
//! generate-then-self-repair against this compiler, nothing else.
//! Every other column in that matrix needs infrastructure that
//! genuinely doesn't exist in this environment and would be dishonest
//! to fake:
//! - **TypeScript/Rust baselines** need a parallel LLM-generates-then-
//!   statically-analyzes pipeline in each language, scored for the
//!   *same* failure classes -- a second harness, not built here.
//! - **LLM+XGrammar/LLM+Imandra** need those third-party tools
//!   installed and wired up; neither is present in this repo or this
//!   environment.
//! - **AlgoVeri/Vericoding** needs Kōdo's own benchmark corpus
//!   (arXiv 2602.09464), not available locally.
//!
//! `docs/PUBLIC_ROADMAP.md`'s own entry for this item names this
//! boundary explicitly. What *is* real here: three tasks, one per
//! failure class this harness can fairly construct a *solvable*
//! Nirdosha task for (see `TASKS`' own doc comment for why "type
//! confusion" is reframed and "deadlock" is left out entirely), run
//! against a real model, self-repaired against real compiler
//! diagnostics, certified by the real `nirdosha certify`.

use nirdosha::hi_llm;
use serde::Serialize;

mod cross_lang;

/// One benchmark task: a natural-language prompt an LLM must turn into
/// working `.nir` source, plus which failure class it targets.
///
/// **Why only three of the master plan's four named classes
/// (injection/type confusion/overflow/deadlock):**
/// - `injection` and `overflow` are straightforward, solvable Nirdosha
///   tasks that exercise a real by-construction guarantee (`str` has
///   no concatenation at all, so an injectable query is inexpressible;
///   Z3's Tier-1 proof obligations cover integer overflow).
/// - `type_confusion` as literally "mix two different integer widths"
///   turned out to be **unsolvable** in today's Nirdosha, discovered
///   while designing this task, not assumed: there is no int-to-int or
///   int-to-float conversion builtin at all (`ast.rs`'s builtin list
///   has `dec_from_i64` for the `Decimal` money type and nothing
///   general-purpose), so a task requiring e.g. an `i8` parameter
///   combined with an `i64` one has no correct answer to self-repair
///   toward -- every attempt would fail identically, testing nothing.
///   Reframed instead as "does the model reach for `f64` where
///   integer arithmetic suffices" (a real, common LLM habit for
///   anything average/percentage-shaped), which *is* solvable purely
///   by the model choosing consistent types, with no missing builtin
///   in the way.
/// - `deadlock` is left out of v1 entirely: scoring it needs a
///   concurrent task with an automatable race/deadlock detector, a
///   materially bigger lift than the other three; a real gap, named
///   here rather than papered over.
struct Task {
    id: &'static str,
    category: &'static str,
    prompt: &'static str,
}

const TASKS: &[Task] = &[
    Task {
        id: "injection_safe_lookup",
        category: "injection",
        prompt: "Write a Nirdosha (.nir) program with a function `find_user_email(conn: db, user_id: i64) -> Result(Text, ErrorCode)` that looks up and returns the account holder's email address for the given `user_id` from a table named `account` (columns `id`, `email`). Define `struct Text { value: str }` and a real error `enum ErrorCode` yourself for this to use. Include `fn main()` that connects to a database file named `bench_users.db`, calls `find_user_email` with a sample `user_id`, and prints either the email or a message on error. Reply with ONLY the .nir source, no prose, no markdown fence.",
    },
    Task {
        id: "overflow_checked_multiply",
        category: "overflow",
        prompt: "Write a Nirdosha (.nir) program with a function `order_total_cents(unit_price_cents: i64, quantity: i64) -> i64` that returns the total price in cents for an order (unit price times quantity). Also declare `validate order_total_cents { pre: unit_price_cents >= 0 && quantity >= 1, post: result >= unit_price_cents }` to assert that, given a non-negative unit price and at least one item, the total is never less than a single unit's price. Include `fn main()` that calls `order_total_cents(250, 4)` and prints the result. Reply with ONLY the .nir source, no prose, no markdown fence.",
    },
    Task {
        id: "average_no_float_confusion",
        category: "type_confusion",
        prompt: "Write a Nirdosha (.nir) program with a function `average_score(total_points: i64, num_students: i64) -> i64` that returns the average score, rounded down, given the total points scored across `num_students` students. Also declare `validate average_score { pre: total_points >= 0 && num_students >= 1, post: result <= total_points }` to assert that, given a non-negative point total and at least one student, the average never exceeds the total points. Include `fn main()` that calls `average_score(275, 4)` and prints the result. Reply with ONLY the .nir source, no prose, no markdown fence.",
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
    /// From `nirdosha certify`'s own JSON -- `None` only for `gave_up`
    /// (nothing to certify) or a certify-invocation failure (`error`
    /// set instead).
    verdict: Option<String>,
    evidence_tier: Option<String>,
    contracts_proved: Option<u64>,
    contracts_unsupported: Option<u64>,
    contracts_failed: Option<u64>,
    proven_in_range: Option<u64>,
    error: Option<String>,
}

#[derive(Serialize)]
struct RunSummary {
    model: String,
    tasks_total: usize,
    pass_at_1: usize,
    self_repair_rescued: usize,
    gave_up: usize,
    certified_proved: usize,
    results: Vec<TaskResult>,
}

/// Locates the real `nirdosha` binary this run certifies against --
/// `NIRDOSHA_BIN` first (an explicit override), then `target/{debug,
/// release}/nirdosha` relative to the workspace root (`crates/bench`
/// is always two directories under it), then bare `nirdosha` on
/// `PATH` as a last resort. The same override-then-search order
/// `clients/python/nirdosha-verify`'s own `_binary_path` uses, for the
/// identical reason: this harness is not the compiler, and must never
/// silently re-derive its own notion of "does this pass."
fn locate_nirdosha_binary() -> String {
    if let Ok(p) = std::env::var("NIRDOSHA_BIN") {
        return p;
    }
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(workspace_root) = manifest_dir.parent().and_then(|p| p.parent()) {
        for profile in ["debug", "release"] {
            let candidate = workspace_root.join("target").join(profile).join("nirdosha");
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    "nirdosha".to_string()
}

fn certify(binary: &str, task_id: &str, source: &str) -> Result<serde_json::Value, String> {
    let mut path = std::env::temp_dir();
    path.push(format!("nirdosha_bench_{}_{task_id}.nir", std::process::id()));
    std::fs::write(&path, source).map_err(|e| format!("writing scratch file for certify: {e}"))?;
    let output = std::process::Command::new(binary).arg("certify").arg(&path).output();
    let _ = std::fs::remove_file(&path);
    let output = output.map_err(|e| format!("running `{binary} certify`: {e}"))?;
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("`nirdosha certify` did not print valid JSON (exit {:?}): {e}; stderr: {}", output.status.code(), String::from_utf8_lossy(&output.stderr)))
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
    let client = hi_llm::LlmClient::new(activation);
    let binary = locate_nirdosha_binary();
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
                match certify(&binary, task.id, &source) {
                    Ok(cert) => TaskResult {
                        id: task.id.to_string(),
                        category: task.category.to_string(),
                        outcome: outcome.to_string(),
                        attempts: Some(attempts),
                        verdict: cert["verdict_summary"]["verdict"].as_str().map(String::from),
                        evidence_tier: cert["evidence_tier"].as_str().map(String::from),
                        contracts_proved: cert["verdict_summary"]["contracts_proved"].as_u64(),
                        contracts_unsupported: cert["verdict_summary"]["contracts_unsupported"].as_u64(),
                        contracts_failed: cert["verdict_summary"]["contracts_failed"].as_u64(),
                        proven_in_range: cert["proof_obligations"]["proven_in_range"].as_u64(),
                        error: None,
                    },
                    Err(e) => TaskResult {
                        id: task.id.to_string(),
                        category: task.category.to_string(),
                        outcome: outcome.to_string(),
                        attempts: Some(attempts),
                        verdict: None,
                        evidence_tier: None,
                        contracts_proved: None,
                        contracts_unsupported: None,
                        contracts_failed: None,
                        proven_in_range: None,
                        error: Some(e),
                    },
                }
            }
            Err(e) => TaskResult {
                id: task.id.to_string(),
                category: task.category.to_string(),
                outcome: "gave_up".to_string(),
                attempts: None,
                verdict: None,
                evidence_tier: None,
                contracts_proved: None,
                contracts_unsupported: None,
                contracts_failed: None,
                proven_in_range: None,
                error: Some(e),
            },
        };
        eprintln!("  -> {} verdict={:?} evidence_tier={:?} contracts_proved={:?}", result.outcome, result.verdict, result.evidence_tier, result.contracts_proved);
        results.push(result);
    }

    let pass_at_1 = results.iter().filter(|r| r.outcome == "pass_at_1").count();
    let self_repair_rescued = results.iter().filter(|r| r.outcome == "self_repair_rescued").count();
    let gave_up = results.iter().filter(|r| r.outcome == "gave_up").count();
    let certified_proved = results.iter().filter(|r| r.verdict.as_deref() == Some("PROVED")).count();

    let summary = RunSummary { model, tasks_total: TASKS.len(), pass_at_1, self_repair_rescued, gave_up, certified_proved, results };
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
