//! A live, compiler-verified inventory of what Nirdosha the current
//! compiler build can actually compile (and, for the ones with a
//! `main`, run) -- not prose. `hi_llm.rs`'s `NIR_SYSTEM_PROMPT` (the
//! paste-anywhere prompt baked into `hi generate`'s system message) is
//! a static, hand-maintained file: `tests/paste_prompt_recipes.rs`
//! catches a *broken* recipe in it, but nothing catches a *stale claim*
//! about what the compiler supports beyond that suite's two hard-coded
//! substring checks (`prompt_has_no_stale_unsupported_claims`).
//!
//! This module runs one self-contained program per major language
//! construct through the real `nirdosha build` pipeline (lex -> parse
//! -> typecheck -> ownership -> `smt::analyze` -> `codegen::build`) and
//! reports pass/fail per construct, straight from the compiler itself.
//! Some snippets are hand-written (the constructs the prompt only ever
//! shows inline: bare `fn`, `struct`, `enum`/`match`, a `validate`
//! contract); the rest are the prompt's own taught recipes -- workflow,
//! transact, transact w/ `txn_id`, screen+serve, the json display
//! loops, the identity/`acquire` pattern -- read live from the same
//! embedded doc and assembled exactly the way `tests/
//! paste_prompt_recipes.rs` assembles them for its own compile tests,
//! so a recipe edit there is picked up here automatically.
//!
//! **What this is not (yet)**: a generator that rewrites
//! `paste-anywhere-prompt.md`'s prose. Turning a pass/fail bit into
//! the English paragraph teaching that construct would need a template
//! per construct, not just a boolean -- real, unbuilt work. What this
//! gives today is the trustworthy half of that pipeline: one call that
//! tells you, from the compiler itself, which constructs are real
//! right now. `tests/capabilities.rs`'s `compiler_capability_report`
//! test is the "what can this compiler pass" check -- run it with
//! `--nocapture` for a live markdown table, or let CI fail it the
//! moment a construct this module claims works stops compiling.

use crate::codegen::{self, OptLevel};
use crate::ownership::check_ownership;
use crate::parser::Parser;
use crate::smt::analyze;
use crate::token::Lexer;
use crate::typeck::typecheck;

/// The same doc `hi_llm::NIR_SYSTEM_PROMPT` embeds -- read a second,
/// independent time here rather than importing that constant, so a
/// recipe's pass/fail in this report is never accidentally coupled to
/// `hi_llm`'s own module-private wiring.
const PROMPT_MD: &str = include_str!("../../../agent-skills/nirdosha/paste-anywhere-prompt.md");

/// One named, self-contained Nirdosha program this module claims
/// compiles against the current build.
pub struct Capability {
    pub name: &'static str,
    pub source: String,
}

/// The outcome of actually trying a `Capability`'s source against the
/// real pipeline.
pub struct CapabilityResult {
    pub name: &'static str,
    pub passed: bool,
    /// The failing stage's diagnostic, verbatim -- `None` when `passed`.
    pub diagnostic: Option<String>,
}

/// Every ```nirdosha fenced block in `PROMPT_MD`, in source order.
/// Deliberately a separate copy of `tests/paste_prompt_recipes.rs`'s
/// own extractor rather than a shared helper: a test-only utility
/// moving or changing shape should never silently take this module's
/// capability list down with it.
fn prompt_blocks() -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in PROMPT_MD.lines() {
        let trimmed = line.trim_start();
        if trimmed == "```nirdosha" && current.is_none() {
            current = Some(Vec::new());
        } else if trimmed == "```" && current.is_some() {
            let raw = current.take().unwrap();
            blocks.push(dedent(&raw).join("\n"));
        } else if let Some(lines) = current.as_mut() {
            lines.push(line.to_string());
        }
    }
    blocks
}

/// Remove the largest common leading whitespace prefix shared by all
/// non-empty lines.
fn dedent(lines: &[String]) -> Vec<String> {
    let non_empty: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.is_empty() {
        return lines.to_vec();
    }
    let min_indent = non_empty.iter().map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    lines.iter().map(|l| if l.trim().is_empty() { String::new() } else { l.chars().skip(min_indent).collect() }).collect()
}

/// The first taught block containing `anchor` -- panics loudly (with
/// enough context to fix it) if the prompt ever moves or drops the
/// recipe this capability depends on, the same failure mode `tests/
/// paste_prompt_recipes.rs::block_containing` guards against.
fn block_containing(anchor: &str) -> String {
    prompt_blocks().into_iter().find(|b| b.contains(anchor)).unwrap_or_else(|| panic!("no ```nirdosha block containing {anchor:?} found in the paste-anywhere prompt -- recipe was moved or removed; update capabilities.rs's anchor"))
}

/// The capability list this module currently checks. Extend this
/// whenever a new construct earns a place in the prompt -- an entry
/// here is the only thing standing between "the prompt claims this
/// compiles" and "the compiler was actually asked to prove it."
pub fn compiler_capabilities() -> Vec<Capability> {
    vec![
        Capability {
            name: "fn + arithmetic",
            source: "fn add(a: i64, b: i64) -> i64 requires(public) {\n    return a + b\n}\n\nfn main() {\n    print(\"sum\", add(2, 3))\n}".to_string(),
        },
        Capability {
            name: "struct + field access",
            source: "struct Point {\n    x: i64,\n    y: i64,\n}\n\nfn main() {\n    let p: Point = Point(1, 2)\n    print(\"x\", p.x)\n}".to_string(),
        },
        Capability {
            name: "enum + match",
            source: "enum Status {\n    Pending,\n    Approved,\n    Rejected(str),\n}\n\nfn describe(s: Status) -> Status {\n    return match s {\n        Pending => Approved(),\n        Approved => Approved(),\n        Rejected(reason) => Rejected(reason),\n    }\n}\n\nfn main() {\n    let s: Status = describe(Pending())\n    print(\"described\")\n}".to_string(),
        },
        Capability {
            name: "validate (Hoare) contract",
            source: "fn charge_cents(amount_cents: i64, balance_cents: i64) -> i64 requires(public) {\n    return balance_cents - amount_cents\n}\n\nvalidate charge_cents {\n    pre: amount_cents >= 0 && amount_cents <= balance_cents\n    post: result >= 0\n}\n\nfn main() {\n    print(\"charged\", charge_cents(10, 100))\n}".to_string(),
        },
        Capability {
            name: "workflow (state machine)",
            source: {
                let block = block_containing("workflow Approval {");
                format!(
                    "{block}\n\nfn empty_doc() -> json {{\n    return match json_parse(\"[]\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n}}\n\nfn main() {{\n    let identity: VerifiedIdentity = VerifiedIdentity(\"ravi\", \"https://idp.example.com\", \"corp-pay\", 0, 0, \"requester\")\n    let started: Result(i64, WorkflowActionError) = start_approval(None(), ApprovalData())\n    let ok: bool = match started {{\n        Ok(id) => match advance_approval(identity, id, Approve(), empty_doc()) {{\n            Ok(_) => true,\n            Err(e) => false,\n        }},\n        Err(e) => false,\n    }}\n    print(\"advanced\", ok)\n}}"
                )
            },
        },
        Capability {
            name: "transact (two-sided settlement)",
            source: {
                let block = block_containing("fn db_up()");
                format!("{block}\n\nfn main() {{\n    let ok: bool = settle(10)\n    print(\"settled\", ok)\n}}")
            },
        },
        Capability {
            name: "transact w/ txn_id",
            source: {
                let block = block_containing("fn call_processor(txn_id: str");
                format!("{block}\n\nfn main() {{\n    let ok: bool = settle(10)\n    print(\"settled\", ok)\n}}")
            },
        },
        Capability {
            name: "screen + serve",
            source: {
                let screen = block_containing("screen PaymentRequest {");
                let serve = block_containing("serve {");
                format!(
                    "struct PaymentRequest {{\n    request_id: i64,\n    account_id: i64,\n    amount_cents: i64,\n    status: str,\n}}\n\nfn approve_payment(instance_id: i64) -> bool {{ return true }}\n\nfn empty_doc() -> json {{\n    return match json_parse(\"[]\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n}}\n\nfn list_payment_request() -> json {{ return empty_doc() }}\n\nfn stat_open_approval_count() -> i64 {{ return 0 }}\n\n{screen}\n\n{serve}\n\nfn main() {{\n    print(\"ready\")\n}}"
                )
            },
        },
        Capability {
            name: "json display loops",
            source: {
                let block = block_containing("landing for");
                format!(
                    "fn empty_doc() -> json {{\n    return match json_parse(\"[]\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n}}\n\nfn main() {{\n    let doc: json = match json_parse(\"{{\\\"user\\\":\\\"ravi\\\"}}\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n    let queue_json: json = match json_parse(\"[{{\\\"status\\\":\\\"pending\\\"}},{{\\\"status\\\":\\\"paid\\\"}}]\") {{\n        Ok(d) => d,\n        Err(e) => empty_doc(),\n    }}\n{block}\n}}"
                )
            },
        },
        Capability {
            name: "identity + acquire",
            source: {
                let block = block_containing("fn try_approve");
                format!(
                    "{block}\n\nfn main() {{\n    let identity: VerifiedIdentity = VerifiedIdentity(\"meera\", \"https://idp.example.com\", \"corp-pay\", 0, 0, \"finance_director\")\n    let ok: bool = try_approve(identity, 1)\n    print(\"approved\", ok)\n}}"
                )
            },
        },
    ]
}

/// Runs every stage `nirdosha build` runs -- lex, parse, typecheck,
/// ownership, `smt::analyze`, `codegen::build` -- and reports the first
/// stage that fails, if any. Never runs the compiled binary: a
/// capability is "does this compile", not "does main print the right
/// thing" -- `tests/paste_prompt_recipes.rs` already owns the latter,
/// stricter check for the recipes it compiles and runs.
fn check_one(source: &str) -> Result<(), String> {
    let toks = Lexer::new(source).tokenize().map_err(|e| format!("lex error: {e:?}"))?;
    let program = Parser::new(toks).parse_program().map_err(|e| format!("parse error: {e:?}"))?;
    if let Err(errors) = typecheck(&program) {
        return Err(format!("type error: {}", errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")));
    }
    if let Err(errors) = check_ownership(&program) {
        return Err(format!("ownership error: {}", errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")));
    }
    let report = analyze(&program);
    static SCRATCH_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = SCRATCH_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_capability_check_{}_{unique}", std::process::id()));
    let result = codegen::build(&program, &report, &out_path, OptLevel::O2).map_err(|e| format!("codegen error: {e}"));
    let _ = std::fs::remove_file(&out_path);
    result
}

/// Runs every `compiler_capabilities()` entry against the real
/// pipeline and reports pass/fail per construct.
pub fn run_capability_checks() -> Vec<CapabilityResult> {
    compiler_capabilities()
        .into_iter()
        .map(|cap| match check_one(&cap.source) {
            Ok(()) => CapabilityResult { name: cap.name, passed: true, diagnostic: None },
            Err(diagnostic) => CapabilityResult { name: cap.name, passed: false, diagnostic: Some(diagnostic) },
        })
        .collect()
}

/// Renders a `run_capability_checks()` result as a markdown table --
/// suitable for pasting into an issue, printing with `--nocapture`, or
/// (future work, not done by this module) folding into a generated
/// prompt appendix.
pub fn format_report(results: &[CapabilityResult]) -> String {
    let mut out = String::from("| capability | status |\n|---|---|\n");
    for r in results {
        let status = if r.passed { "✅ compiles" } else { "❌ FAILS" };
        out.push_str(&format!("| {} | {} |\n", r.name, status));
    }
    let failures: Vec<&CapabilityResult> = results.iter().filter(|r| !r.passed).collect();
    if !failures.is_empty() {
        out.push_str("\n### Failures\n\n");
        for r in failures {
            out.push_str(&format!("- **{}**: {}\n", r.name, r.diagnostic.as_deref().unwrap_or("(no diagnostic)")));
        }
    }
    out
}
