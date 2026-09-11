//! `nirdosha-master-plan.md` Part 3 Oct 10-30's comparison matrix:
//! "Nirdosha vs TypeScript vs Rust vs plain-LLM ...". `main.rs`'s own
//! doc comment named this a real, un-built gap in the Sprint 2 pass --
//! this module closes the TypeScript/Rust half of it, honestly scoped:
//! the **same** model (via the same `LlmClient`/Ollama endpoint
//! `main.rs` already uses), asked **once, no self-repair loop** (the
//! "plain-LLM" column specifically, not Nirdosha's own self-repair
//! column), for the *same two failure classes* `main.rs::TASKS`
//! already covers by real execution (`overflow`, `type_confusion`);
//! `injection` is included too but checked by a static heuristic, not
//! execution -- see `run_injection_task`'s own doc comment for why,
//! disclosed rather than silently downgraded to look equivalent to the
//! other two.
//!
//! **What this module deliberately does not attempt**: LLM+XGrammar
//! (needs raw logit access for grammar-constrained decoding -- not
//! available through Ollama's OpenAI-compatible chat-completions
//! endpoint, which only returns finished text) and LLM+Imandra (a
//! commercial tool, no license available in this environment) stay
//! real, disclosed gaps, not faked. AlgoVeri/Vericoding comparison
//! (`docs/PUBLIC_ROADMAP.md`'s own entry has the finding) turned out to
//! need a fundamentally different verification tier -- quantified
//! postconditions over arbitrary-length sequences and loop invariants
//! -- that this compiler's Tier-1 Z3 encoding has no support for at
//! all (confirmed by grep: zero `forall`/quantifier handling anywhere
//! in `contract_check.rs`), not a narrower version of the same thing.

use nirdosha::hi_llm::{self, LlmClient};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

const CODEGEN_SYSTEM_PROMPT: &str = "You are a careful software engineer. Reply with ONLY the requested source code -- no prose, no markdown code fences, no explanation.";

/// Same fence-stripping shape as `hi_llm`'s private `extract_nir_source`
/// (language-agnostic in practice despite that name) -- not reused
/// directly since it's private to that module and this module's need
/// is generic enough not to warrant widening `hi_llm`'s public surface
/// for it.
fn strip_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(fence_start) = trimmed.find("```") {
        let rest = &trimmed[fence_start + 3..];
        let after_info_string = match rest.find('\n') {
            Some(newline) => &rest[newline + 1..],
            None => rest,
        };
        if let Some(end) = after_info_string.find("```") {
            return after_info_string[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn generate(client: &LlmClient, prompt: &str) -> Result<String, String> {
    let raw = hi_llm::generate_plain(client, CODEGEN_SYSTEM_PROMPT, prompt)?;
    Ok(strip_fence(&raw))
}

#[derive(Serialize, Clone)]
pub struct CrossLangResult {
    pub task_id: String,
    pub language: String,
    /// "safe" (correct result, or a loud failure that a real Nirdosha
    /// `certify PROVED`/`DISPROVED` counterpart would also flag) |
    /// "silently_wrong" (wrong result, no error raised -- the failure
    /// mode Nirdosha's Z3 proof or by-construction guarantee rules out
    /// and a plain LLM in this language did not) | "generation_failed" |
    /// "compile_failed" | "heuristic_pass" / "heuristic_fail" (the
    /// injection task only, see its own doc comment).
    pub outcome: String,
    pub detail: String,
}

fn write_result(out_dir: &Path, task_id: &str, language: &str, filename: &str, source: &str) {
    let dir = out_dir.join("cross_lang");
    std::fs::create_dir_all(&dir).expect("creating cross_lang artifact dir should not fail");
    std::fs::write(dir.join(format!("{task_id}_{language}_{filename}")), source).expect("writing a cross-lang artifact should not fail");
}

// ---------------------------------------------------------------------
// Task 1: overflow -- order_total_cents(unit_price_cents, quantity)
// ---------------------------------------------------------------------
//
// `main.rs::TASKS`'s Nirdosha version is proved by Z3 never to
// overflow within the bounds its own `pre:` states. This asks the same
// question with no such bound and no formal backend: unit_price_cents
// x quantity chosen so the exact product (121932631124827861592745)
// both overflows i64 (max ~9.22e18) *and* exceeds what an IEEE-754
// double can represent exactly (>2^53) -- so neither language's
// default numeric type can silently get this right by luck.
const OVERFLOW_TS_PROMPT: &str = "Write a JavaScript function `orderTotalCents(unitPriceCents, quantity)` that returns the total price in cents for an order (unit price times quantity). Immediately after the function, add exactly one line: `console.log(orderTotalCents(123456789012345, 987654321));`";
const OVERFLOW_RUST_PROMPT: &str = "Write a Rust function `fn order_total_cents(unit_price_cents: i64, quantity: i64) -> i64` that returns the total price in cents for an order (unit price times quantity). Include a `fn main()` whose only statement is `println!(\"{}\", order_total_cents(123456789012345i64, 987654321i64));`";
const OVERFLOW_EXACT: &str = "121932631124827861592745";

fn run_overflow_ts(client: &LlmClient, out_dir: &Path, work_dir: &Path) -> CrossLangResult {
    let source = match generate(client, OVERFLOW_TS_PROMPT) {
        Ok(s) => s,
        Err(e) => return CrossLangResult { task_id: "overflow".into(), language: "typescript".into(), outcome: "generation_failed".into(), detail: e },
    };
    write_result(out_dir, "overflow", "typescript", "source.js", &source);
    let path = work_dir.join("overflow_ts.js");
    std::fs::write(&path, &source).expect("writing scratch JS file should not fail");
    let output = match Command::new("node").arg(&path).output() {
        Ok(o) => o,
        Err(e) => return CrossLangResult { task_id: "overflow".into(), language: "typescript".into(), outcome: "compile_failed".into(), detail: format!("couldn't run node: {e}") },
    };
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        // A thrown exception (loud failure) is the "safe" outcome here --
        // the language stopped rather than returning a wrong number.
        return CrossLangResult { task_id: "overflow".into(), language: "typescript".into(), outcome: "safe".into(), detail: format!("threw rather than returning a silently wrong value: {}", String::from_utf8_lossy(&output.stderr).trim()) };
    }
    if stdout == OVERFLOW_EXACT {
        CrossLangResult { task_id: "overflow".into(), language: "typescript".into(), outcome: "safe".into(), detail: format!("returned the exact correct value {stdout} (e.g. via BigInt)") }
    } else {
        CrossLangResult { task_id: "overflow".into(), language: "typescript".into(), outcome: "silently_wrong".into(), detail: format!("expected exact value {OVERFLOW_EXACT}, got {stdout} -- wrong, and no error was raised") }
    }
}

fn run_overflow_rust(client: &LlmClient, out_dir: &Path, work_dir: &Path) -> CrossLangResult {
    let source = match generate(client, OVERFLOW_RUST_PROMPT) {
        Ok(s) => s,
        Err(e) => return CrossLangResult { task_id: "overflow".into(), language: "rust".into(), outcome: "generation_failed".into(), detail: e },
    };
    write_result(out_dir, "overflow", "rust", "source.rs", &source);
    let src_path = work_dir.join("overflow_rust.rs");
    let bin_path = work_dir.join("overflow_rust_bin");
    std::fs::write(&src_path, &source).expect("writing scratch Rust file should not fail");
    let compile = Command::new("rustc").arg("-o").arg(&bin_path).arg(&src_path).output();
    let compile = match compile {
        Ok(c) => c,
        Err(e) => return CrossLangResult { task_id: "overflow".into(), language: "rust".into(), outcome: "compile_failed".into(), detail: format!("couldn't run rustc: {e}") },
    };
    if !compile.status.success() {
        return CrossLangResult { task_id: "overflow".into(), language: "rust".into(), outcome: "compile_failed".into(), detail: String::from_utf8_lossy(&compile.stderr).trim().to_string() };
    }
    let output = Command::new(&bin_path).output().expect("running the compiled overflow binary should not fail to spawn");
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        return CrossLangResult { task_id: "overflow".into(), language: "rust".into(), outcome: "safe".into(), detail: format!("panicked rather than returning a silently wrong value: {}", String::from_utf8_lossy(&output.stderr).trim()) };
    }
    if stdout == OVERFLOW_EXACT {
        CrossLangResult { task_id: "overflow".into(), language: "rust".into(), outcome: "safe".into(), detail: format!("returned the exact correct value {stdout}") }
    } else {
        CrossLangResult { task_id: "overflow".into(), language: "rust".into(), outcome: "silently_wrong".into(), detail: format!("expected exact value {OVERFLOW_EXACT}, got {stdout} -- wrong, and no error was raised") }
    }
}

// ---------------------------------------------------------------------
// Task 2: type_confusion -- average_score(total_points, num_students),
// floored integer average. Nirdosha's version is reframed (see
// `main.rs::TASKS`'s own doc comment) as "does the model reach for f64
// where integer arithmetic suffices" -- this asks the identical
// question in each language. 275 / 4 = 68.75; the correct floored
// integer answer is "68".
// ---------------------------------------------------------------------
const AVERAGE_TS_PROMPT: &str = "Write a JavaScript function `averageScore(totalPoints, numStudents)` that returns the average score, rounded down (floored) to a whole number, given the total points scored across numStudents students. Immediately after the function, add exactly one line: `console.log(averageScore(275, 4));`";
const AVERAGE_RUST_PROMPT: &str = "Write a Rust function `fn average_score(total_points: i64, num_students: i64) -> i64` that returns the average score, rounded down (floored), given the total points scored across num_students students. Include a `fn main()` whose only statement is `println!(\"{}\", average_score(275, 4));`";
const AVERAGE_EXACT: &str = "68";

fn run_average_ts(client: &LlmClient, out_dir: &Path, work_dir: &Path) -> CrossLangResult {
    let source = match generate(client, AVERAGE_TS_PROMPT) {
        Ok(s) => s,
        Err(e) => return CrossLangResult { task_id: "type_confusion".into(), language: "typescript".into(), outcome: "generation_failed".into(), detail: e },
    };
    write_result(out_dir, "type_confusion", "typescript", "source.js", &source);
    let path = work_dir.join("average_ts.js");
    std::fs::write(&path, &source).expect("writing scratch JS file should not fail");
    let output = match Command::new("node").arg(&path).output() {
        Ok(o) => o,
        Err(e) => return CrossLangResult { task_id: "type_confusion".into(), language: "typescript".into(), outcome: "compile_failed".into(), detail: format!("couldn't run node: {e}") },
    };
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        return CrossLangResult { task_id: "type_confusion".into(), language: "typescript".into(), outcome: "compile_failed".into(), detail: String::from_utf8_lossy(&output.stderr).trim().to_string() };
    }
    if stdout == AVERAGE_EXACT {
        CrossLangResult { task_id: "type_confusion".into(), language: "typescript".into(), outcome: "safe".into(), detail: "returned the correctly-floored integer".into() }
    } else {
        CrossLangResult { task_id: "type_confusion".into(), language: "typescript".into(), outcome: "silently_wrong".into(), detail: format!("expected {AVERAGE_EXACT}, got {stdout} -- JS has no int/float distinction to catch this, unlike Nirdosha's static typing or Rust's i64 return type") }
    }
}

fn run_average_rust(client: &LlmClient, out_dir: &Path, work_dir: &Path) -> CrossLangResult {
    let source = match generate(client, AVERAGE_RUST_PROMPT) {
        Ok(s) => s,
        Err(e) => return CrossLangResult { task_id: "type_confusion".into(), language: "rust".into(), outcome: "generation_failed".into(), detail: e },
    };
    write_result(out_dir, "type_confusion", "rust", "source.rs", &source);
    let src_path = work_dir.join("average_rust.rs");
    let bin_path = work_dir.join("average_rust_bin");
    std::fs::write(&src_path, &source).expect("writing scratch Rust file should not fail");
    let compile = Command::new("rustc").arg("-o").arg(&bin_path).arg(&src_path).output();
    let compile = match compile {
        Ok(c) => c,
        Err(e) => return CrossLangResult { task_id: "type_confusion".into(), language: "rust".into(), outcome: "compile_failed".into(), detail: format!("couldn't run rustc: {e}") },
    };
    if !compile.status.success() {
        // A model that drifted to f64 internally but declared an i64
        // return type is a real, informative *compile* failure here --
        // Rust's static typing catching the exact class of mistake
        // Nirdosha's Tier-0 typecheck also catches, worth keeping
        // distinct from a "wrong at runtime" result, not just lumped
        // into a generic failure bucket.
        return CrossLangResult { task_id: "type_confusion".into(), language: "rust".into(), outcome: "safe".into(), detail: format!("rejected at compile time (Rust's static typing caught a type mismatch before it could run): {}", String::from_utf8_lossy(&compile.stderr).trim()) };
    }
    let output = Command::new(&bin_path).output().expect("running the compiled average binary should not fail to spawn");
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout == AVERAGE_EXACT {
        CrossLangResult { task_id: "type_confusion".into(), language: "rust".into(), outcome: "safe".into(), detail: "returned the correctly-floored integer".into() }
    } else {
        CrossLangResult { task_id: "type_confusion".into(), language: "rust".into(), outcome: "silently_wrong".into(), detail: format!("expected {AVERAGE_EXACT}, got {stdout}") }
    }
}

// ---------------------------------------------------------------------
// Task 3: injection -- find_user_email(conn, user_id). **Heuristic,
// not executed**, disclosed plainly: Nirdosha's own version is scored
// by real execution against `nirdosha certify` because injection is
// *inexpressible* in Nirdosha (`str` has no concatenation operator at
// all -- a language-level guarantee, checked by the typechecker, not a
// runtime test). TypeScript/Rust have no equivalent by-construction
// guarantee to execute against; standing up a real SQL engine plus a
// real injection payload for two languages is a second harness this
// pass doesn't build. What's checked instead: does the generated
// source use a parameterized-query placeholder (`?`/`$1`) for
// `user_id`, or does it interpolate `user_id` directly into a SQL
// string (the textbook injectable pattern) -- a real, automatable
// signal, but a coding-discipline heuristic on generated *text*, not a
// proof of anything at runtime. Explicitly weaker evidence than the
// other two tasks, and reported as such rather than presented at equal
// weight.
// ---------------------------------------------------------------------
const INJECTION_TS_PROMPT: &str = "Write a JavaScript function `findUserEmail(db, userId)` that looks up and returns the email address for the given userId from a SQL table named `account` (columns `id`, `email`), using `db.query(sql, params)` where `db` is some SQL client object. Reply with ONLY the function.";
const INJECTION_RUST_PROMPT: &str = "Write a Rust function `fn find_user_email(conn: &Connection, user_id: i64) -> Option<String>` that looks up and returns the email address for the given user_id from a SQL table named `account` (columns `id`, `email`), assuming `conn` is a database connection with a `.query(sql, params)`-style method. Reply with ONLY the function.";

fn looks_parameterized(source: &str) -> bool {
    let has_placeholder = source.contains('?') || source.contains("$1");
    let has_raw_interpolation = source.contains("${") || source.contains("format!(") || (source.contains('+') && source.to_lowercase().contains("select"));
    has_placeholder && !has_raw_interpolation
}

fn run_injection_task(client: &LlmClient, language: &str, prompt: &str, out_dir: &Path) -> CrossLangResult {
    let source = match generate(client, prompt) {
        Ok(s) => s,
        Err(e) => return CrossLangResult { task_id: "injection".into(), language: language.into(), outcome: "generation_failed".into(), detail: e },
    };
    let ext = if language == "typescript" { "js" } else { "rs" };
    write_result(out_dir, "injection", language, &format!("source.{ext}"), &source);
    if looks_parameterized(&source) {
        CrossLangResult { task_id: "injection".into(), language: language.into(), outcome: "heuristic_pass".into(), detail: "uses a parameterized-query placeholder, no direct string interpolation of user_id into SQL detected".into() }
    } else {
        CrossLangResult { task_id: "injection".into(), language: language.into(), outcome: "heuristic_fail".into(), detail: "no parameterized-query placeholder detected (or raw interpolation into SQL text found) -- a real SQL injection pattern, if this is representative of the model's real output".into() }
    }
}

pub fn run(client: &LlmClient, out_dir: &Path) -> Vec<CrossLangResult> {
    let work_dir = std::env::temp_dir().join(format!("nirdosha_bench_cross_lang_{}", std::process::id()));
    std::fs::create_dir_all(&work_dir).expect("creating a scratch work dir should not fail");
    let results = vec![
        run_overflow_ts(client, out_dir, &work_dir),
        run_overflow_rust(client, out_dir, &work_dir),
        run_average_ts(client, out_dir, &work_dir),
        run_average_rust(client, out_dir, &work_dir),
        run_injection_task(client, "typescript", INJECTION_TS_PROMPT, out_dir),
        run_injection_task(client, "rust", INJECTION_RUST_PROMPT, out_dir),
    ];
    let _ = std::fs::remove_dir_all(&work_dir);
    let summary_path = out_dir.join("cross_lang_summary.json");
    std::fs::write(&summary_path, serde_json::to_string_pretty(&results).expect("serializing cross-lang results should not fail")).expect("writing cross_lang_summary.json should not fail");
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_exact_constant_is_the_real_product_not_a_typo() {
        // u128 has ample headroom for this (result is ~1.2e23, u128
        // maxes out around 3.4e38) -- this is the independent
        // ground-truth check the whole overflow task's scoring leans
        // on, so it can't itself be computed with i64/f64 (the exact
        // types under test) without begging the question.
        let unit_price_cents: u128 = 123456789012345;
        let quantity: u128 = 987654321;
        assert_eq!((unit_price_cents * quantity).to_string(), OVERFLOW_EXACT);
    }

    #[test]
    fn overflow_exact_constant_genuinely_exceeds_i64_max_and_f64_precision() {
        let exact: u128 = OVERFLOW_EXACT.parse().unwrap();
        assert!(exact > i64::MAX as u128, "must overflow i64 for the Rust leg of this task to mean anything");
        assert!(exact > (1u128 << 53), "must exceed 2^53 for the TS leg (IEEE-754 double exact-integer range) to mean anything");
    }

    #[test]
    fn average_exact_constant_is_the_real_floored_average() {
        assert_eq!((275_i64 / 4_i64).to_string(), AVERAGE_EXACT);
        // Confirms this genuinely tests flooring, not an average that
        // happens to divide evenly (which would test nothing about
        // float-confusion since floor(x) == x for integers already).
        assert_ne!(275 % 4, 0);
    }

    #[test]
    fn strip_fence_removes_a_fenced_code_block() {
        let raw = "```javascript\nconsole.log(1);\n```";
        assert_eq!(strip_fence(raw), "console.log(1);");
    }

    #[test]
    fn strip_fence_passes_through_plain_text() {
        assert_eq!(strip_fence("console.log(1);"), "console.log(1);");
    }

    #[test]
    fn looks_parameterized_accepts_a_real_placeholder_query() {
        assert!(looks_parameterized("db.query('SELECT email FROM account WHERE id = ?', [userId])"));
        assert!(looks_parameterized("conn.query(\"SELECT email FROM account WHERE id = $1\", &[&user_id])"));
    }

    #[test]
    fn looks_parameterized_rejects_string_interpolation_into_sql() {
        assert!(!looks_parameterized("db.query(`SELECT email FROM account WHERE id = ${userId}`)"));
        assert!(!looks_parameterized(&format!("conn.query(&format!(\"SELECT email FROM account WHERE id = {{}}\", user_id))")));
    }

    #[test]
    fn looks_parameterized_rejects_plain_concatenation_into_sql() {
        assert!(!looks_parameterized("db.query(\"SELECT email FROM account WHERE id = \" + userId)"));
    }
}
