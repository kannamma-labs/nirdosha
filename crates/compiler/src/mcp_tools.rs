//! The one MCP tool layer, shared by both of its consumers: `nirdosha
//! mcp` (stdio JSON-RPC, for external agents) and `nirdosha hi` (the
//! embedded console, in-process -- no subprocess, no wire). This module
//! exists so there is exactly one implementation of the capability
//! surface and one call log, not one per transport: the stdio server
//! and the console both end up here, through
//! [`tools_call`], and every call from either surface is
//! recorded by [`McpCallLog`] -- what tool, with what input, what
//! outcome, from which surface, when, and how long it took.
//!
//! Historically these handlers, the pipelines they call
//! ([`run_verify_pipeline`]/[`write_auto_patches`]/
//! [`build_certificate`]), and every type in between lived in the
//! *binary* crate (`main.rs`), which forced `hi`'s library-side
//! self-repair loop to *mirror* the pipeline instead of sharing it
//! ("two separate compilation units, nothing to `use` across that
//! boundary"). Moving the cluster here removes that boundary as a
//! reason for duplication: the console calls the same functions the
//! wire serves, and `main.rs`'s `cmd_verify`/`cmd_fix`/`cmd_certify`/
//! `cmd_mcp` stay thin CLI wrappers over the same library code.
//!
//! Everything a caller needs is `pub`; the helpers only the pipeline
//! itself uses (`levenshtein`, `classify_load_error_code`,
//! `fix_unbound_identifier`, `require_str_arg`, `describe_program`,
//! `tool_ok`) stay private to this module.

use serde_json::json;
use std::io::Write as _;

#[derive(serde::Serialize)]
pub struct VerifyDiagnostic {
    pub line: usize,
    pub col: usize,
    pub message: String,
    /// Same meaning as `ContractObligation::fix` -- `None` unless this
    /// specific diagnostic kind has real fix analysis attached (v1:
    /// only `typeck::TypeErrorKind::UnknownVar`, an edit-distance typo
    /// correction against the names actually in scope -- see
    /// `fix_unbound_identifier`).
    pub fix: Option<Fix>,
    /// `nirdosha explain <code>`'s index key (`explain::REGISTRY`) --
    /// `None` unless this specific diagnostic's site can identify one
    /// of the twelve `AGENTS.md`-numbered rules (or `NIR0013`,
    /// unbound-identifier) with certainty; see `explain.rs`'s own doc
    /// comment for exactly which sites do today and why the rest
    /// honestly don't yet.
    pub code: Option<&'static str>,
}

#[derive(serde::Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Passed,
    Failed,
    Skipped,
}

/// The top-level verdict `nirdosha verify` reports, and the one
/// `ContractsResult` reports for its own stage -- three-valued on
/// purpose, never collapsed to pass/fail. `StageStatus` above answers
/// "did this stage run and complete" (a pipeline-mechanics question,
/// genuinely binary: typecheck either finds no error or it does); this
/// answers "what do we actually know about the code's correctness" (an
/// epistemic question, and not binary at all): `Unsupported` -- Z3
/// couldn't model a predicate, not "it found no problem" -- used to be
/// folded into an overall `Passed` verdict, which reported confidence
/// this pipeline never earned. A caller (CI, an agent's own repair
/// loop) needs a real, distinguishable third answer for "we don't know"
/// so it doesn't treat unmodeled code as proved safe. Exit codes follow
/// the same three-way split (0/1/2, `cmd_verify`), not just this
/// JSON field, for the same reason: a caller that only inspects `$?`
/// must be able to see the difference too.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProofVerdict {
    Proved,
    Disproved,
    Unknown,
}

#[derive(serde::Serialize)]
pub struct StageResult {
    pub status: StageStatus,
    pub errors: Vec<VerifyDiagnostic>,
}

impl StageResult {
    pub fn skipped() -> Self {
        StageResult { status: StageStatus::Skipped, errors: vec![] }
    }
}

/// `nirdosha fix`'s three fixability classes (`nirdosha-master-plan.md`
/// Part 3, Sprint 1: "fixability classes (auto / assisted / manual)",
/// parity target: Kōdo). Modeled directly on `rustc`'s own
/// `Applicability` enum (`MachineApplicable`/`MaybeIncorrect`/
/// `HasPlaceholders`/`Unspecified`, the real precedent `rustfix`/
/// `cargo fix` already ship against) rather than invented from scratch:
/// `Auto` == `MachineApplicable` (safe to apply without review -- the
/// only class `nirdosha fix --apply` ever writes to disk on its own);
/// `Assisted` == `MaybeIncorrect`/`HasPlaceholders` collapsed into one
/// (a real fix exists, or a shape of one does, but it needs a judgment
/// call `nirdosha fix` can't make safely by itself); `Manual` ==
/// `Unspecified` (no mechanical fix known for this diagnostic kind at
/// all, today).
#[derive(serde::Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Applicability {
    Auto,
    Assisted,
    Manual,
}

/// A byte-offset text replacement -- `[start_byte, end_byte)` in the
/// original source file, replaced with `replacement`. Byte offsets, not
/// line/column: a patch applier needs to slice and splice the exact
/// original bytes, and `token::Span::byte`'s own doc comment names the
/// same reason `rustc`'s `Span`/`BytePos` is byte-addressed rather than
/// line/column-addressed. Only ever present when `Fix::applicability`
/// is `Auto` or `Assisted` -- a `Manual` fix has no patch to offer by
/// definition, so its `Fix::patch` is always `None`, not a patch that
/// happens to be a no-op.
#[derive(serde::Serialize)]
pub struct FixPatch {
    pub start_byte: usize,
    pub end_byte: usize,
    pub replacement: String,
}

#[derive(serde::Serialize)]
pub struct Fix {
    pub applicability: Applicability,
    /// Present for `Auto` (always) and `Assisted` (when a concrete
    /// replacement text exists, even if it needs review before
    /// trusting it); absent for `Manual`, and absent for an `Assisted`
    /// fix that only has *guidance* to offer, not literal replacement
    /// text (e.g. "wrap this in a `struct Text` or a real `enum`,
    /// your call" -- two shapes, no single patch is honest to propose).
    pub patch: Option<FixPatch>,
    /// Human-readable explanation of what this fix does and why this
    /// applicability class, not a fix format's own vocabulary --
    /// printed as-is by any caller that doesn't want to interpret
    /// `applicability` itself.
    pub rationale: String,
}

#[derive(serde::Serialize)]
pub struct ContractObligation {
    pub fn_name: String,
    pub status: &'static str,
    pub detail: Option<String>,
    /// `None` means no automated-fix analysis produced anything for
    /// this obligation's kind yet -- not the same claim as `Manual`
    /// (which is a considered "no mechanical fix exists"). `nirdosha
    /// fix` v1 only analyzes `unbound_identifier` obligations (a real,
    /// bounded, edit-distance typo correction against the function's
    /// own parameter names); every other obligation kind is `None`
    /// here, honestly, rather than a blanket `Manual` that implies more
    /// analysis happened than actually did.
    pub fix: Option<Fix>,
}

#[derive(serde::Serialize)]
pub struct ContractsResult {
    /// Whether this stage ran at all -- `Skipped` only if an earlier
    /// stage (load/typecheck/ownership) already failed, `Passed`
    /// otherwise, even when `verdict` below is `Disproved` or
    /// `Unknown`: this stage genuinely *ran*, it's `verdict` that says
    /// what it found, not whether it executed. Never set to `Failed` by
    /// this stage -- an earlier revision conflated "ran and found a
    /// counterexample" with `StageStatus::Failed`, which left no room
    /// for `Unsupported` to mean anything but a silent pass; `verdict`
    /// is the fix, this field's meaning is now consistent with `load`/
    /// `typecheck`/`ownership`'s.
    pub status: StageStatus,
    pub verdict: ProofVerdict,
    pub proved: usize,
    pub unsupported: usize,
    pub failed: usize,
    pub obligations: Vec<ContractObligation>,
}

/// `smt::analyze`'s per-span proof counts (Tier 1: overflow/division/
/// array-bounds obligations `codegen.rs` elides a runtime trap for once
/// proven) -- informational, not a pass/fail gate on its own. A span
/// missing from these counts isn't a defect: it's still enforced, just
/// at runtime instead (the same "proved vs. still-safely-checked"
/// distinction `contract_check.rs`'s `Unsupported` already draws for
/// `validate` blocks).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProofObligations {
    pub proven_in_range: usize,
    pub proven_nonzero_divisor: usize,
    pub proven_index_bounds: usize,
}

#[derive(serde::Serialize)]
pub struct VerifyVerdict {
    pub source: String,
    /// Three-valued, replacing what used to be a `status: StageStatus`
    /// field here (`Passed`/`Failed` only) -- called out explicitly
    /// per `docs/STABILITY_AND_RELEASES.md`'s rule for this exact JSON
    /// schema, not a silent rename: the old field could not represent
    /// "Z3 couldn't decide," so a file with an unmodeled `validate`
    /// predicate and nothing else wrong reported the same `Passed` a
    /// file with a fully proved contract did. `verdict` is `Unknown`
    /// in that case instead, never folded into `Proved`.
    pub verdict: ProofVerdict,
    pub load: StageResult,
    pub typecheck: StageResult,
    pub ownership: StageResult,
    pub contracts: ContractsResult,
    pub proof_obligations: ProofObligations,
}

/// Levenshtein edit distance -- standard textbook dynamic-programming
/// form (a single rolling `prev_row`, `O(len(a) * len(b))` time,
/// `O(min(len(a), len(b)))` space via the shorter string as columns).
/// Used only for `fix_unbound_identifier`'s typo suggestions, over
/// short identifier strings (a handful of characters, never a whole
/// file), so the naive DP form is the right amount of engineering --
/// no need for the banded/early-exit variants a spell-checker over a
/// large dictionary would want.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev_row: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut cur_row = vec![i + 1];
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            let insert = cur_row[j] + 1;
            let delete = prev_row[j + 1] + 1;
            let substitute = prev_row[j] + cost;
            cur_row.push(insert.min(delete).min(substitute));
        }
        prev_row = cur_row;
    }
    prev_row[b.len()]
}

/// Recovers `NIR0012` (reserved word used where an identifier was
/// required) from `loader::load_program`'s already-formatted `String`
/// error, the only signal available at this call site today --
/// `ParseError` doesn't carry a structured code field, and threading
/// one through would mean widening `loader::load_program`'s
/// `Result<_, String>` across every one of its own callers
/// (`build`/`emit-llvm`/`emit-ast`/`gen-crud`/...), a much larger
/// change than one diagnostic's classification justifies today. Safe
/// *because* the substring it looks for is fully deterministic: only
/// `parser.rs`'s `expect_ident` ever produces the literal prefix
/// `"expected identifier, found "`, and only `Display for Tok`
/// (`token.rs`, round-trip tested) ever renders a reserved word as
/// `"the reserved keyword `...`"`/`"the reserved type name `...`"` --
/// so this can't false-positive on an unrelated "expected identifier"
/// site or an unrelated reserved-word mention. A heuristic, not a
/// guess: narrowed by construction to the one message shape that means
/// this rule, nothing looser.
fn classify_load_error_code(msg: &str) -> Option<&'static str> {
    const PREFIX: &str = "expected identifier, found ";
    let after = msg.find(PREFIX)? + PREFIX.len();
    let found = &msg[after..];
    if found.starts_with("the reserved keyword ") || found.starts_with("the reserved type name ") {
        Some("NIR0012")
    } else {
        None
    }
}

/// `nirdosha fix`'s one real, tested fixability analysis for v1
/// (`nirdosha-master-plan.md` Part 3 Sprint 1): a `validate` predicate
/// referencing a name that's neither `result` nor one of the function's
/// own parameters, most often a plain typo (`ammount` for `amount`).
/// Modeled on `rustc`'s own identifier-typo suggestions
/// (`find_best_match_for_name`), which use the same edit-distance
/// technique and the same "only suggest when unambiguous" discipline --
/// a real precedent for exactly this shape of fix, not a heuristic
/// invented here from nothing.
///
/// - Exactly one candidate strictly closer than every other, and within
///   a relative threshold -- `Auto`: the typo is unambiguous, the patch
///   is the obviously-intended one, safe to apply without a human in
///   the loop (matches `rustc`'s own `MachineApplicable` bar for typo
///   fixes).
/// - More than one candidate tied at the closest distance -- `Assisted`:
///   a real fix is one of these names, but which one is a judgment call
///   this analysis can't make safely; no single patch, the rationale
///   lists every tied candidate.
/// - Nothing within the threshold -- `Manual`: this isn't a typo of any
///   parameter name close enough to guess at; the rationale lists what
///   *is* available so a human or an agent's repair loop has the real
///   options in front of it, not just "no".
fn fix_unbound_identifier(name: &str, span: crate::token::Span, candidates: &[String]) -> Fix {
    // A relative threshold, not a flat constant -- `1/3` of the name's
    // own length (floor, minimum 1) roughly matches how many characters
    // a plausible single typo (one substitution/transposition/drop)
    // changes in a short identifier, without also matching two
    // genuinely different short names to each other (e.g. `a` and `b`
    // are distance 1 but not a typo of each other -- a flat threshold
    // of 1 would still suggest one for the other; `a`'s own length-based
    // threshold is 1 too, so this doesn't fully solve that single-char
    // case, but it's the same tradeoff `rustc`'s own suggestion
    // threshold makes, not an oversight unique to this implementation).
    let threshold = (name.chars().count() / 3).max(1);

    let mut distances: Vec<(usize, &String)> =
        candidates.iter().map(|c| (levenshtein(name, c), c)).filter(|(d, _)| *d <= threshold).collect();
    distances.sort_by_key(|(d, name)| (*d, name.to_string()));

    let start_byte = span.byte;
    let end_byte = span.byte + name.len();

    match distances.as_slice() {
        [] => Fix {
            applicability: Applicability::Manual,
            patch: None,
            rationale: format!(
                "`{name}` isn't within edit distance {threshold} of any available name ({}) -- not a plausible typo of one of them, needs a real decision about what this predicate should reference",
                candidates.join(", ")
            ),
        },
        [(only_dist, only_name)] => Fix {
            applicability: Applicability::Auto,
            patch: Some(FixPatch { start_byte, end_byte, replacement: (*only_name).clone() }),
            rationale: format!("`{name}` is edit distance {only_dist} from `{only_name}`, the only candidate this close -- almost certainly a typo"),
        },
        [(best_dist, _), (tied_dist, _), ..] if best_dist == tied_dist => {
            let tied: Vec<&str> = distances.iter().filter(|(d, _)| d == best_dist).map(|(_, n)| n.as_str()).collect();
            Fix {
                applicability: Applicability::Assisted,
                patch: None,
                rationale: format!(
                    "`{name}` is equally close (edit distance {best_dist}) to more than one candidate ({}) -- one of these is almost certainly intended, but which one needs a human or an agent's own judgment, not a guess",
                    tied.join(", ")
                ),
            }
        }
        [(best_dist, best_name), ..] => Fix {
            applicability: Applicability::Auto,
            patch: Some(FixPatch { start_byte, end_byte, replacement: (*best_name).clone() }),
            rationale: format!("`{name}` is edit distance {best_dist} from `{best_name}`, strictly closer than every other candidate -- almost certainly a typo"),
        },
    }
}

/// The shared gate pipeline behind both `nirdosha verify` and `nirdosha
/// fix` -- load, typecheck, ownership, `validate` contract-check (with
/// `nirdosha fix`'s per-obligation `Fix` analysis always attached, per
/// `ContractObligation::fix`'s own doc comment: computing it is cheap
/// and it's additive to the JSON schema, so `verify` callers get it
/// too, not just `fix` ones), plus `smt::analyze`'s Tier-1 counts.
/// Extracted out of `cmd_verify` so `cmd_fix` runs the exact same
/// checks instead of a second, maintained-separately copy that could
/// drift from what `verify` actually checks.
pub fn run_verify_pipeline(path: &str) -> VerifyVerdict {
    let mut load = StageResult { status: StageStatus::Passed, errors: vec![] };
    let mut typecheck = StageResult::skipped();
    let mut ownership = StageResult::skipped();
    let mut contracts = ContractsResult {
        status: StageStatus::Skipped,
        verdict: ProofVerdict::Proved,
        proved: 0,
        unsupported: 0,
        failed: 0,
        obligations: vec![],
    };
    let mut proof_obligations = ProofObligations { proven_in_range: 0, proven_nonzero_divisor: 0, proven_index_bounds: 0 };

    let program = match crate::loader::load_program(path) {
        Ok((program, _src)) => Some(program),
        Err(msg) => {
            load.status = StageStatus::Failed;
            let code = classify_load_error_code(&msg);
            load.errors.push(VerifyDiagnostic { line: 0, col: 0, message: msg, fix: None, code });
            None
        }
    };

    let program = program.and_then(|program| match crate::typeck::typecheck_optional_main(&program) {
        Ok(()) => {
            typecheck.status = StageStatus::Passed;
            Some(program)
        }
        Err(errs) => {
            typecheck.status = StageStatus::Failed;
            typecheck.errors = errs
                .iter()
                .map(|e| {
                    let (fix, code) = match &e.kind {
                        // A bare variant name used without call syntax (`let s:
                        // Shape = Circle`, NIR0001's own "wrong" example) parses
                        // as a plain identifier -- `UnknownVar`, same as any
                        // other unresolved name -- so it's indistinguishable
                        // from a real typo (`NIR0013`) without this extra
                        // check: is `name` one of `program`'s own declared
                        // enum variant names? If so, tag `NIR0001` instead --
                        // `fix_unbound_identifier`'s own candidates are local
                        // scope names, not variant names, so it naturally finds
                        // nothing close and reports `Manual`, which is honest
                        // here (the real fix is adding `(...)`, not a rename).
                        crate::typeck::TypeErrorKind::UnknownVar { name, candidates } => {
                            let is_variant_name = program.enums.iter().any(|en| en.variants.iter().any(|v| &v.name == name));
                            let code = if is_variant_name { "NIR0001" } else { "NIR0013" };
                            (Some(fix_unbound_identifier(name, e.span, candidates)), Some(code))
                        }
                        crate::typeck::TypeErrorKind::StrInFnSignature { .. } => (None, Some("NIR0002")),
                        // Two typed values combined/assigned without a
                        // conversion -- NIR0006's own "wrong" example
                        // (`i32` + `i64`). Narrowed to the numeric-vs-numeric
                        // case specifically: `TypeMismatch` is also the
                        // catch-all for every other kind of type error, most
                        // of which aren't "forgot to convert."
                        crate::typeck::TypeErrorKind::TypeMismatch { expected, found } if expected.is_numeric() && found.is_numeric() => {
                            (None, Some("NIR0006"))
                        }
                        // `_` used as a variant-match arm, or a variant left
                        // uncovered -- both are the same "match is exhaustive,
                        // no wildcard for variants" rule NIR0008 documents,
                        // just the two ways to violate it (the wrong wildcard
                        // pattern, vs. simply missing an arm).
                        crate::typeck::TypeErrorKind::MatchArmMustBeVariant { .. }
                        | crate::typeck::TypeErrorKind::NonExhaustiveMatch { .. } => (None, Some("NIR0008")),
                        // `validate <fn_name> { ... }` where `<fn_name>` doesn't
                        // resolve — same typo shape `UnknownVar` already covers,
                        // `fix_unbound_identifier` reused directly against
                        // `program.fns`' own names as candidates (`e.span` is now
                        // `ValidateDecl::fn_name_span`, the identifier's own real
                        // location, not the `validate` keyword's).
                        crate::typeck::TypeErrorKind::ValidateFnNotFound(name) => {
                            let candidates: Vec<String> = program.fns.iter().map(|f| f.name.clone()).collect();
                            (Some(fix_unbound_identifier(name, e.span, &candidates)), None)
                        }
                        _ => (None, None),
                    };
                    VerifyDiagnostic { line: e.span.line, col: e.span.col, message: e.to_string(), fix, code }
                })
                .collect();
            None
        }
    });

    let program = program.and_then(|program| match crate::ownership::check_ownership(&program) {
        Ok(()) => {
            ownership.status = StageStatus::Passed;
            Some(program)
        }
        Err(errs) => {
            ownership.status = StageStatus::Failed;
            ownership.errors = errs
                .iter()
                .map(|e| VerifyDiagnostic { line: e.span.line, col: e.span.col, message: e.to_string(), fix: None, code: None })
                .collect();
            None
        }
    });

    if let Some(program) = &program {
        contracts.status = StageStatus::Passed;
        for outcome in crate::contract_check::run_program_validates(program) {
            use crate::contract_check::ContractCheckResult;
            match outcome.result {
                ContractCheckResult::Proved => {
                    contracts.proved += 1;
                    contracts.obligations.push(ContractObligation { fn_name: outcome.fn_name, status: "proved", detail: None, fix: None });
                }
                ContractCheckResult::Unsupported(msg) => {
                    contracts.unsupported += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "unsupported",
                        detail: Some(msg),
                        fix: None,
                    });
                }
                ContractCheckResult::Counterexample { violated_predicate, bindings, result } => {
                    contracts.failed += 1;
                    let bindings_str = bindings.iter().map(|(n, v)| format!("{n} = {v}")).collect::<Vec<_>>().join(", ");
                    let detail = format!(
                        "`{violated_predicate}` is violated when {bindings_str} (fn returns {})",
                        result.map(|r| r.to_string()).unwrap_or_else(|| "<uncomputed>".to_string())
                    );
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "counterexample",
                        detail: Some(detail),
                        fix: None,
                    });
                }
                ContractCheckResult::UnboundIdentifier { name, span, candidates } => {
                    contracts.failed += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "unbound_identifier",
                        detail: Some(name.clone()),
                        fix: Some(fix_unbound_identifier(&name, span, &candidates)),
                    });
                }
                ContractCheckResult::NoSuchFunction(name) => {
                    contracts.failed += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "no_such_function",
                        detail: Some(name),
                        fix: None,
                    });
                }
                ContractCheckResult::PredicateParseError(msg) => {
                    contracts.failed += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "predicate_parse_error",
                        detail: Some(msg),
                        fix: None,
                    });
                }
            }
        }

        // `failed` beats `unsupported`, which beats an empty verdict of
        // `Proved` -- a real counterexample is more informative than "Z3
        // couldn't decide," so a file that's both definitely wrong
        // somewhere and unmodeled somewhere else is reported as
        // `Disproved`, not `Unknown`: an agent's repair loop has a
        // concrete counterexample to act on either way, and hiding it
        // behind "unknown" would be a strictly worse answer.
        contracts.verdict = if contracts.failed > 0 {
            ProofVerdict::Disproved
        } else if contracts.unsupported > 0 {
            ProofVerdict::Unknown
        } else {
            ProofVerdict::Proved
        };

        let smt_report = crate::smt::analyze(program);
        proof_obligations.proven_in_range = smt_report.proven_in_range.len();
        proof_obligations.proven_nonzero_divisor = smt_report.proven_nonzero_divisor.len();
        proof_obligations.proven_index_bounds = smt_report.proven_index_bounds.len();
    }

    // A hard pipeline failure (load/typecheck/ownership) is always
    // `Disproved`, not `Unknown` -- there's no uncertainty in "this
    // doesn't typecheck." Only `contracts.verdict` can introduce
    // `Unknown`, and only when nothing else already disproved the file.
    let pipeline_failed =
        [load.status, typecheck.status, ownership.status].iter().any(|s| *s == StageStatus::Failed);
    let verdict_value = if pipeline_failed || contracts.verdict == ProofVerdict::Disproved {
        ProofVerdict::Disproved
    } else if contracts.verdict == ProofVerdict::Unknown {
        ProofVerdict::Unknown
    } else {
        ProofVerdict::Proved
    };

    VerifyVerdict {
        source: path.to_string(),
        verdict: verdict_value,
        load,
        typecheck,
        ownership,
        contracts,
        proof_obligations,
    }
}

/// One `Auto`-class patch `nirdosha fix --apply` actually wrote to
/// disk -- distinct from `Fix`/`FixPatch` above (which describe a
/// *proposed* patch, applied or not): this is the record of one that
/// really was, so `FixReport.applied` is a true audit trail, not a
/// re-derivation of `before`'s obligations filtered by applicability.
///
/// `site`/`detail` are deliberately generic strings, not `fn_name`/
/// `&'static str` (an earlier revision had both, then only ever
/// collected `contracts.obligations`' `Auto` patches -- silently
/// dropping every `Auto` fix attached to a `load`/`typecheck`/
/// `ownership` `VerifyDiagnostic` instead, e.g. `fix_unbound_identifier`
/// firing from `typeck.rs`, because `--apply`'s collector never looked
/// there. Caught by hand-running `nirdosha fix --apply` end to end on a
/// real typo: the JSON proposed the correct patch, `--apply` wrote
/// zero of them). `site` is the function name for a `ContractObligation`
/// or `"<line>:<col>"` for a `VerifyDiagnostic`; `detail` is the
/// obligation's status tag or the diagnostic's own message -- both
/// human-readable provenance, not data the applier itself branches on.
#[derive(serde::Serialize)]
pub struct AppliedPatch {
    pub site: String,
    pub detail: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub replacement: String,
}

#[derive(serde::Serialize)]
pub struct FixReport {
    /// The verdict before any patch was applied -- identical shape to
    /// `nirdosha verify`'s own output, `fix` fields included, whether
    /// or not `--apply` was given.
    pub before: VerifyVerdict,
    /// Every `Auto`-class patch actually written to disk. Always empty
    /// without `--apply` -- `nirdosha fix` on its own only *reports*
    /// patches, matching `rustc`'s own separation between emitting
    /// suggestions and a separate tool (`rustfix`/`cargo fix`)
    /// applying them; `--apply` is that second step folded into the
    /// same command instead of a second binary, not a different
    /// analysis.
    pub applied: Vec<AppliedPatch>,
    /// Re-verification after applying `applied`'s patches -- `None`
    /// unless `--apply` was given. This is the honest check that the
    /// patches this command just wrote actually improved the verdict,
    /// not an assumption that generating a patch means it worked.
    pub after: Option<VerifyVerdict>,
}

/// Collects every `Auto`-class patch across all four `before` stages
/// (`load`/`typecheck`/`ownership`'s `VerifyDiagnostic`s, plus
/// `contracts.obligations`) and writes them to `path`, highest byte
/// offset first -- applying low-to-high would shift every later
/// patch's own byte offsets out from under it the moment an earlier
/// one changed the file's length; high-to-low never does, since
/// nothing after the current patch's end has been touched yet by the
/// time it's applied. Extracted out of `cmd_fix` so the MCP `fix` tool
/// shares this exact algorithm instead of a second,
/// maintained-separately copy -- the real bug `AppliedPatch`'s own doc
/// comment describes (an earlier revision only ever scanned
/// `contracts.obligations`, silently dropping every `Auto` patch
/// attached to a stage diagnostic instead) was exactly this kind of
/// drift, and it only had one call site to go wrong in at the time.
pub fn write_auto_patches(path: &str, before: &VerifyVerdict) -> Result<Vec<AppliedPatch>, String> {
    let mut patches: Vec<(String, String, &FixPatch)> = Vec::new();
    for diag in before.load.errors.iter().chain(before.typecheck.errors.iter()).chain(before.ownership.errors.iter()) {
        if let Some(Fix { applicability: Applicability::Auto, patch: Some(p), .. }) = &diag.fix {
            patches.push((format!("{}:{}", diag.line, diag.col), diag.message.clone(), p));
        }
    }
    for ob in &before.contracts.obligations {
        if let Some(Fix { applicability: Applicability::Auto, patch: Some(p), .. }) = &ob.fix {
            patches.push((ob.fn_name.clone(), ob.status.to_string(), p));
        }
    }
    patches.sort_by(|a, b| b.2.start_byte.cmp(&a.2.start_byte));

    let mut applied = Vec::new();
    if patches.is_empty() {
        return Ok(applied);
    }

    let mut src = std::fs::read_to_string(path).map_err(|e| format!("error reading {path} to apply patches: {e}"))?;
    for (site, detail, patch) in &patches {
        if patch.start_byte > src.len() || patch.end_byte > src.len() || patch.start_byte > patch.end_byte {
            // A patch computed against a stale byte range (shouldn't
            // happen -- `before` was just read from this same file --
            // but a corrupt/concurrently-modified file is a real
            // possibility this must not silently misapply against) is
            // skipped, not forced.
            continue;
        }
        src.replace_range(patch.start_byte..patch.end_byte, &patch.replacement);
        applied.push(AppliedPatch {
            site: site.clone(),
            detail: detail.clone(),
            start_byte: patch.start_byte,
            end_byte: patch.end_byte,
            replacement: patch.replacement.clone(),
        });
    }
    std::fs::write(path, &src).map_err(|e| format!("error writing patched file: {e}"))?;
    Ok(applied)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

/// Certificate v0 (`nirdosha-master-plan.md` Part 3 Sprint 1, parity
/// target: Velvet, Kōdo) -- a deterministic, hash-pinned JSON
/// attestation, reproducible by any third party holding the same
/// source file and the same compiler version: no timestamp, no
/// absolute path, no random nonce anywhere in this struct, only
/// content hashes and counts. `#[derive(Serialize)]` on a plain struct
/// (not a `serde_json::Map`) is itself part of that determinism --
/// serde always serializes struct fields in declaration order, never
/// resorted, so the same `Certificate` value always produces the same
/// JSON bytes.
///
/// `evidence_tier` is the field `docs/PUBLIC_ROADMAP.md`'s master-plan
/// notes say to ship now "so the format never breaks": all four tiers
/// (`proved`/`checked`/`sampled`/`unknown`) are valid values in this
/// schema from v0 onward, even though this compiler can only ever
/// produce `proved` or `unknown` today -- `checked` (sandbox-validated
/// behavioral evidence) and `sampled` (property-based/fuzz evidence)
/// both require infrastructure that doesn't exist yet (the MicroVM
/// sandbox tier, post-seed; see the master plan's "language-agnostic
/// ladder"). A certificate a future `nirdosha check` emits can set
/// either of those without this struct's shape ever changing.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Certificate {
    pub certificate_version: String,
    pub source_hash: String,
    pub grammar_hash: String,
    pub toolchain_version: String,
    /// `"proved"` | `"checked"` | `"sampled"` | `"unknown"` -- see this
    /// struct's own doc comment for why the type is a plain `String`
    /// (an open, forward-declared vocabulary) rather than a 2-variant
    /// enum that would have to grow a breaking variant later. `String`,
    /// not `&'static str`: `Certificate` also derives `Deserialize`
    /// (`cmd_verify_certificate`'s round trip), and a borrowed
    /// `&'static str` field makes a derived `Deserialize` impl
    /// unsatisfiable for any real deserializer (it would need to
    /// borrow from the input buffer for the `'static` lifetime, which
    /// no buffer actually has) -- a real compile error caught here,
    /// not a style preference.
    pub evidence_tier: String,
    pub verdict_summary: VerdictSummary,
    pub proof_obligations: ProofObligations,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct VerdictSummary {
    pub verdict: ProofVerdict,
    pub contracts_proved: usize,
    pub contracts_unsupported: usize,
    pub contracts_failed: usize,
}

/// Builds a `Certificate` from a source file's raw bytes and the
/// pipeline result already run against it -- takes `VerifyVerdict` by
/// value and destructures it (`..` drops `load`/`typecheck`/
/// `ownership`/`source`) rather than borrowing, since nothing after
/// this call needs the full verdict and a certificate is deliberately
/// a compact summary of it, not a re-export of every diagnostic
/// `verify`'s own JSON already carries.
pub fn build_certificate(source_bytes: &[u8], pipeline: VerifyVerdict) -> Certificate {
    let VerifyVerdict { verdict, contracts, proof_obligations, .. } = pipeline;
    // `Skipped` means contract-check never ran at all (a load/typecheck/
    // ownership failure stopped the pipeline first) -- no Z3-backed
    // evidence was ever produced, so `unknown` is the honest tier, not
    // `proved`/`checked` implying analysis that didn't happen. Once
    // contract-check does run, both `Proved` and `Disproved` are a
    // *conclusive* Z3 answer -- `evidence_tier` describes the kind of
    // evidence (formal, either way), `verdict_summary.verdict` separately
    // describes the outcome (pass or fail). Only `Unknown` (Z3 couldn't
    // model at least one obligation) leaves `evidence_tier` at
    // `"unknown"` too: an inconclusive stage produced no full proof.
    let evidence_tier = if contracts.status == StageStatus::Skipped {
        "unknown"
    } else {
        match verdict {
            ProofVerdict::Proved | ProofVerdict::Disproved => "proved",
            ProofVerdict::Unknown => "unknown",
        }
    };
    Certificate {
        certificate_version: "0".to_string(),
        source_hash: sha256_hex(source_bytes),
        grammar_hash: sha256_hex(NIRDOSHA_GBNF.as_bytes()),
        toolchain_version: env!("CARGO_PKG_VERSION").to_string(),
        evidence_tier: evidence_tier.to_string(),
        verdict_summary: VerdictSummary {
            verdict,
            contracts_proved: contracts.proved,
            contracts_unsupported: contracts.unsupported,
            contracts_failed: contracts.failed,
        },
        proof_obligations,
    }
}

/// Baked into the binary at compile time (`include_str!`), the same
/// "ships inside the binary" posture `STD_CATALOG_JSON` documents for
/// `emit-catalog` -- `nirdosha.gbnf` is checked into the repo at the
/// crate root and re-generated by `crates/grammar_export`'s own tests
/// against `llama-cpp-gbnf`, never hand-edited independently of
/// the grammar it's exported from.
const NIRDOSHA_GBNF: &str = include_str!("../nirdosha.gbnf");

/// A `.nir` source file that exists only for the duration of one MCP
/// tool call -- `run_verify_pipeline`/`write_auto_patches`/
/// `loader::load_program` all take a filesystem path (import
/// resolution, `write_auto_patches`'s own read-modify-write, and
/// `db_connect`-relative paths in the source itself all need a real
/// file on disk), but an MCP tool call only ever carries source text
/// inline (`tools_list`'s own schemas -- every tool takes `source`,
/// never `path`, the same choice Kōdo's MCP tools make). Rather than
/// fork the pipeline into a path-based and a source-based variant,
/// every MCP handler below materializes `source` to one of these and
/// reuses the exact same, already-tested pipeline `verify`/`fix` run
/// against a real file. `Drop` removes it unconditionally, not a
/// manual `remove_file` at the end of each handler, so a long-running
/// `nirdosha mcp` process serving many calls never accumulates temp
/// files -- including on an early `?`-return from a handler.
pub(crate) struct TempNirFile(std::path::PathBuf);

impl TempNirFile {
    fn write(source: &str) -> Result<Self, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("nirdosha_mcp_{}_{n}.nir", std::process::id()));
        std::fs::write(&p, source).map_err(|e| format!("failed to stage source for verification: {e}"))?;
        Ok(Self(p))
    }

    fn path_str(&self) -> &str {
        self.0.to_str().expect("temp_dir()-rooted path is always valid UTF-8 on every platform this ships for")
    }
}

impl Drop for TempNirFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn require_str_arg<'a>(arguments: &'a serde_json::Value, name: &str) -> Result<&'a str, String> {
    arguments.get(name).and_then(|v| v.as_str()).ok_or_else(|| format!("Missing required parameter '{name}'"))
}

/// `verify_code` -- runs `run_verify_pipeline` (the exact pipeline
/// `nirdosha verify`/`nirdosha fix` both run) against inline source and
/// returns the identical `VerifyVerdict` JSON shape, `source` replaced
/// with `"<inline>"` since the real value (a temp path) is an
/// implementation detail no caller should key off of.
pub fn verify_code(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let temp = TempNirFile::write(source)?;
    let verdict = run_verify_pipeline(temp.path_str());
    let mut value = serde_json::to_value(&verdict).expect("VerifyVerdict always serializes");
    value["source"] = json!("<inline>");
    Ok(value)
}

/// `get_grammar` -- returns the full LL(1) Nirdosha grammar in GBNF
/// form (constrained-decoding target for llama.cpp/vLLM-style grammar-
/// constrained generation). No arguments; the grammar is one fixed
/// artifact per compiler version, not parameterized per call.
pub fn get_grammar(_arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(json!({ "format": "gbnf", "grammar": NIRDOSHA_GBNF }))
}

/// `fix` -- same pipeline as `verify_code`, plus `write_auto_patches`
/// (the exact algorithm `nirdosha fix --apply` uses, shared not
/// duplicated) when `apply: true`. There's no file for the CLI's own
/// `--apply` to write back to and hand the caller a path for, so this
/// returns the patched source text directly in `patched_source`
/// instead -- the MCP-native equivalent of `--apply` actually rewriting
/// the file on disk.
pub fn fix(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let apply = arguments.get("apply").and_then(|v| v.as_bool()).unwrap_or(false);
    let temp = TempNirFile::write(source)?;

    let before = run_verify_pipeline(temp.path_str());
    let applied = if apply { write_auto_patches(temp.path_str(), &before)? } else { Vec::new() };

    let mut before_value = serde_json::to_value(&before).expect("VerifyVerdict always serializes");
    before_value["source"] = json!("<inline>");
    let mut result = json!({ "before": before_value, "applied": applied });

    if apply {
        let patched = std::fs::read_to_string(temp.path_str()).map_err(|e| format!("failed to read patched source back: {e}"))?;
        let after = run_verify_pipeline(temp.path_str());
        let mut after_value = serde_json::to_value(&after).expect("VerifyVerdict always serializes");
        after_value["source"] = json!("<inline>");
        result["after"] = after_value;
        result["patched_source"] = json!(patched);
    }
    Ok(result)
}

/// `describe` -- parses (does not require it to typecheck, the same
/// "AST of a program that doesn't yet typecheck is still legitimate to
/// inspect" contract `cmd_emit_ast` already documents) inline source
/// and returns a curated structural summary via `describe_program`.
pub fn describe(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let temp = TempNirFile::write(source)?;
    let (program, _src) = crate::loader::load_program(temp.path_str())?;
    Ok(describe_program(&program))
}

/// `certify_code` -- the same pipeline `cmd_certify` runs, over inline
/// source: verify, then wrap in a `Certificate` (`build_certificate`,
/// shared not duplicated). Issues a certificate for *every* verdict,
/// `DISPROVED` included -- an honest "this code is proven wrong, here
/// is the conclusive evidence" is a real, useful attestation, the
/// same reason `cmd_certify` issues one regardless of verdict. This
/// is the Certify half of the Constrain -> Verify -> Repair ->
/// Certify loop, so an agent driving Nirdosha entirely over this tool
/// surface can produce the auditor-facing artifact, not just verdicts.
///
/// Deliberately **unsigned**: `sign_certificate` (Ed25519, `ring`) stays
/// CLI-only behind `nirdosha certify --sign`, because key custody is a
/// human decision that must not cross an agent-callable boundary -- an
/// agent can mint evidence, only a key holder can endorse it. A
/// verifier consumes the signature via `nirdosha verify-certificate`,
/// never this tool.
pub fn certify_code(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let temp = TempNirFile::write(source)?;
    let pipeline = run_verify_pipeline(temp.path_str());
    let certificate = build_certificate(source.as_bytes(), pipeline);
    Ok(serde_json::to_value(&certificate).expect("Certificate always serializes"))
}

fn describe_program(program: &crate::ast::Program) -> serde_json::Value {
    let functions: Vec<serde_json::Value> = program
        .fns
        .iter()
        .map(|f| {
            json!({
                "name": f.name,
                "params": f.params.iter().map(|p| json!({ "name": p.name, "type": p.ty })).collect::<Vec<_>>(),
                "return_type": f.ret,
                "effects": f.declared_effects,
                "requires": f.requires,
                "nfr": f.nfr,
                "exported": f.exported,
            })
        })
        .collect();
    let structs: Vec<serde_json::Value> = program
        .structs
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "fields": s.fields.iter().map(|f| json!({ "name": f.name, "type": f.ty })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let enums: Vec<serde_json::Value> = program
        .enums
        .iter()
        .map(|e| {
            json!({
                "name": e.name,
                "variants": e.variants.iter().map(|v| json!({ "name": v.name, "payload": v.payload })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let validates: Vec<serde_json::Value> = program
        .validates
        .iter()
        .map(|v| {
            json!({
                "fn_name": v.fn_name,
                "entries": v.entries.iter().map(|(key, expr)| json!({ "key": key, "expr": expr })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "functions": functions, "structs": structs, "enums": enums, "validates": validates })
}

/// Wraps one tool handler's structured output in the MCP `tools/call`
/// result shape -- `content[0].text` is the same JSON serialized as
/// text (the spec's own documented backward-compatibility rule for
/// `structuredContent`: "a tool that returns structured content SHOULD
/// also return the serialized JSON in a TextContent block"), and
/// `isError` stays `false` here unconditionally: a `DISPROVED`/
/// `UNKNOWN` verdict, or an `Assisted`/`Manual` (unfixable) diagnostic,
/// is a normal, successful, informative answer to the question asked
/// -- not a tool failure. `isError: true` is reserved for
/// `tools_call` never reaching a handler at all (a missing
/// argument or unknown tool name is a *protocol* error instead, per
/// the spec's own two-tier error model -- see `tools_call`).
fn tool_ok(structured: serde_json::Value) -> serde_json::Value {
    let text = serde_json::to_string(&structured).unwrap_or_else(|_| "{}".to_string());
    json!({
        "content": [ { "type": "text", "text": text } ],
        "structuredContent": structured,
        "isError": false,
    })
}

/// `tools/list` -- one entry per tool `tools_call` dispatches to:
/// `verify_code`, `get_grammar`, `fix`, `describe`, `certify_code`
/// (parity target: Acutis, Imandra, Kōdo). Every `inputSchema` is
/// plain JSON Schema, per spec.
pub fn tools_list() -> serde_json::Value {
    json!({
        "tools": [
            {
                "name": "verify_code",
                "title": "Verify Nirdosha source",
                "description": "Run nirdosha verify's full gate pipeline (load, typecheck, ownership, validate contracts with Z3 proof obligations) against inline .nir source. Returns the same three-valued PROVED/DISPROVED/UNKNOWN verdict JSON `nirdosha verify` prints on stdout.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "source": { "type": "string", "description": "Nirdosha (.nir) source code to verify" } },
                    "required": ["source"],
                },
            },
            {
                "name": "get_grammar",
                "title": "Get the Nirdosha GBNF grammar",
                "description": "Returns the full LL(1) Nirdosha grammar in GBNF form, for constrained decoding (llama.cpp/vLLM-style grammar-constrained generation) -- the exact grammar nirdosha.gbnf ships inside the compiler binary.",
                "inputSchema": { "type": "object", "properties": {} },
            },
            {
                "name": "fix",
                "title": "Propose or apply automated fixes",
                "description": "Runs the same pipeline as verify_code, plus a byte-offset FixPatch for any diagnostic that has one, classified auto/assisted/manual. With apply: true, writes every auto-class patch into the source and returns the patched text alongside a re-verified verdict.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "source": { "type": "string", "description": "Nirdosha (.nir) source code to check and, optionally, patch" },
                        "apply": { "type": "boolean", "description": "If true, apply every auto-class patch and return patched_source. Defaults to false (report only, nothing patched)." },
                    },
                    "required": ["source"],
                },
            },
            {
                "name": "describe",
                "title": "Describe a Nirdosha source file's structure",
                "description": "Parses the given source (does not require it to typecheck) and returns a curated structural summary: every fn's name/params/return type/declared effects/requires/nfr, every struct's fields, every enum's variants, and every top-level validate block's pre/post contracts.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "source": { "type": "string", "description": "Nirdosha (.nir) source code to describe" } },
                    "required": ["source"],
                },
            },
            {
                "name": "certify_code",
                "title": "Issue a verification certificate for Nirdosha source",
                "description": "Runs the same pipeline as verify_code and wraps the result in a deterministic, hash-pinned Certificate v0 (the identical JSON `nirdosha certify <file.nir>` prints). Issues a certificate for every verdict, DISPROVED included. Unsigned by design: signing (`nirdosha certify --sign`) is a key-holder's deliberate act, never an agent-callable one.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "source": { "type": "string", "description": "Nirdosha (.nir) source code to certify" } },
                    "required": ["source"],
                },
            },
        ],
    })
}

/// `tools/call` -- dispatches `params.name` to one of the five MCP
/// handlers above with `params.arguments`, logging every call (from
/// either surface) through `log`. A missing `name`, an unknown tool
/// name, or a missing required argument are all *protocol* errors
/// (JSON-RPC `-32602 Invalid params`, matching the phrasing Kōdo's own
/// `missing_param_error` uses) per the spec's own distinction between
/// protocol errors ("Unknown tools", "Invalid arguments") and
/// tool-execution errors (`isError: true` in a successful result) --
/// see `tool_ok`'s doc comment for why every path that reaches a
/// handler at all comes back `isError: false`.
pub fn tools_call(params: &serde_json::Value, log: &mut McpCallLog) -> Result<serde_json::Value, (i64, String)> {
    let call_id = log.next_call_id();
    let started = std::time::Instant::now();
    // A missing `name` keeps the exact protocol error the wire path
    // has always answered with (`tests/mcp_server.rs` asserts it) --
    // logged first so even a malformed call leaves a record.
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        let message = "Missing required parameter 'name'".to_string();
        log.record_call(call_id, "", params, &Err(message.clone()), started.elapsed().as_millis());
        return Err((-32602, message));
    };
    let empty = json!({});
    let arguments = params.get("arguments").unwrap_or(&empty);

    let outcome = match name {
        "verify_code" => verify_code(arguments),
        "get_grammar" => get_grammar(arguments),
        "fix" => fix(arguments),
        "describe" => describe(arguments),
        "certify_code" => certify_code(arguments),
        other => Err(format!("Unknown tool: {other}")),
    };
    let latency_ms = started.elapsed().as_millis();
    log.record_call(call_id, name, arguments, &outcome, latency_ms);

    outcome
        .map(tool_ok)
        .map_err(|message| (-32602, message))
}

/// The one call log both MCP surfaces share -- newline-delimited JSON
/// appended to a disclosed file, one record per `tools_call` regardless
/// of which surface (`nirdosha mcp`'s stdio wire or `nirdosha hi`'s
/// embedded in-process calls) the call came in on. "Disclosed, not
/// hidden": the same convention hi's session log and
/// `typecheck_and_build_to_temp_file` already follow -- the path is
/// printed by the caller at startup, never only discoverable by
/// knowing where to look. What was called, with what input (hashed +
/// measured, never the full source bloating the log), what came back,
/// from which surface, when, and how long it took: enough to
/// reconstruct any session after the fact without having been watching
/// it live.
pub struct McpCallLog {
    surface: &'static str,
    session: String,
    next_id: u64,
    file: Option<std::fs::File>,
    path: std::path::PathBuf,
}

impl McpCallLog {
    /// Opens (creating if absent) the log file for this process and
    /// surface. Logging is best-effort by design: if the file can't be
    /// opened, the tool layer still works and every call still returns
    /// -- a broken log must never break verification -- only the
    /// record of what happened is lost.
    pub fn new(surface: &'static str) -> Self {
        let path = std::env::temp_dir().join(format!("nirdosha_mcp_{surface}_{}.ndjson", std::process::id()));
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        Self { surface, session: format!("{surface}-{}", std::process::id()), next_id: 0, file, path }
    }

    /// The disclosed path the caller should print at startup.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn next_call_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// One session-start record, so the log names its own surface and
    /// protocol context before any call has happened -- `cmd_mcp` and
    /// `nirdosha hi` each write one, making the "where" of every later
    /// record self-evident even in a merged/rotated file.
    pub fn log_session_start(&mut self, details: serde_json::Value) {
        self.append(json!({
            "ts": rfc3339_now(),
            "session": self.session,
            "surface": self.surface,
            "event": "session_start",
            "details": details,
        }));
    }

    fn record_call(
        &mut self,
        call_id: u64,
        tool: &str,
        arguments: &serde_json::Value,
        outcome: &Result<serde_json::Value, String>,
        latency_ms: u128,
    ) {
        // Input metadata: identify the source without dumping it into
        // the log -- byte length + SHA-256 are enough to correlate the
        // record with the caller's own copy of what it sent (and with
        // a certificate's `source_hash`, below).
        let source_meta = arguments.get("source").and_then(|v| v.as_str()).map(|s| {
            json!({ "bytes": s.len(), "sha256": sha256_hex(s.as_bytes()) })
        });
        let apply = arguments.get("apply").and_then(|v| v.as_bool());

        let mut record = json!({
            "ts": rfc3339_now(),
            "session": self.session,
            "surface": self.surface,
            "call_id": call_id,
            "tool": tool,
            "latency_ms": latency_ms,
        });
        if let Some(source_meta) = source_meta {
            record["source"] = source_meta;
        }
        if let Some(apply) = apply {
            record["apply"] = json!(apply);
        }
        match outcome {
            Ok(value) => {
                record["outcome"] = json!("ok");
                // Outcome summary, generic across tools: the verdict a
                // verify/fix/certify carries (nested for fix's before/
                // after, wrapped for certify's verdict_summary), the
                // patch count a fix reports, and the hash a certificate
                // pins -- present when the tool produced one, absent
                // otherwise (get_grammar/describe).
                for (key, pointer) in [
                    ("verdict", "/verdict"),
                    ("verdict", "/before/verdict"),
                    ("verdict", "/after/verdict"),
                    ("verdict", "/verdict_summary/verdict"),
                ] {
                    if let Some(v) = value.pointer(pointer) {
                        record[key] = v.clone();
                        break;
                    }
                }
                if let Some(applied) = value.get("applied").and_then(|v| v.as_array()) {
                    record["patches_applied"] = json!(applied.len());
                }
                if let Some(hash) = value.get("source_hash").and_then(|v| v.as_str()) {
                    record["cert_source_hash"] = json!(hash);
                }
            }
            Err(message) => {
                record["outcome"] = json!("error");
                record["error"] = json!(message);
            }
        }
        self.append(record);
    }

    fn append(&mut self, entry: serde_json::Value) {
        let Some(file) = &mut self.file else { return };
        if let Ok(line) = serde_json::to_string(&entry) {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

/// RFC 3339 UTC timestamp with millisecond precision, hand-rolled
/// against the Unix epoch (days-from-civil, the standard Howard
/// Hinnant form) -- no time/chrono dependency, consistent with this
/// workspace's "no dependency this repo doesn't already need
/// elsewhere" posture. Correct for every date >= 1970-01-01, which is
/// every date a `SystemTime::now()` can ever produce.
fn rfc3339_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        sod / 3600,
        (sod / 60) % 60,
        sod % 60
    )
}

/// Days since 1970-01-01 -> (year, month, day) in the proleptic
/// Gregorian calendar. Howard Hinnant's `civil_from_days`, the
/// standard closed form (see chrono's own `internal.rs` -- same
/// algorithm) with the eras math done in signed arithmetic so every
/// input >= 0 needs no special-casing.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_advertises_all_five_tools() {
        let list = tools_list();
        let names: Vec<&str> = list["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["verify_code", "get_grammar", "fix", "describe", "certify_code"]);
    }

    #[test]
    fn tools_call_unknown_tool_is_a_protocol_error_and_is_logged() {
        let mut log = McpCallLog::new("unit-test-a");
        log.log_session_start(json!({ "purpose": "unit test" }));
        let err = tools_call(&json!({ "name": "no_such_tool", "arguments": {} }), &mut log).unwrap_err();
        assert_eq!(err.0, -32602);
        assert!(err.1.contains("Unknown tool: no_such_tool"));

        let contents = std::fs::read_to_string(log.path()).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "session_start + one call record");
        let start: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(start["event"], "session_start");
        let record: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(record["surface"], "unit-test-a");
        assert_eq!(record["tool"], "no_such_tool");
        assert_eq!(record["call_id"], 1);
        assert_eq!(record["outcome"], "error");
        assert!(record["error"].as_str().unwrap().contains("Unknown tool: no_such_tool"));
        assert!(record["ts"].as_str().unwrap().ends_with('Z'));
        let _ = std::fs::remove_file(log.path());
    }

    #[test]
    fn missing_required_argument_is_a_protocol_error_and_is_logged() {
        let mut log = McpCallLog::new("unit-test-b");
        let err = tools_call(&json!({ "name": "verify_code", "arguments": {} }), &mut log).unwrap_err();
        assert_eq!(err.0, -32602);
        assert!(err.1.contains("Missing required parameter 'source'"));

        let contents = std::fs::read_to_string(log.path()).unwrap();
        let record: serde_json::Value = serde_json::from_str(contents.lines().last().unwrap()).unwrap();
        assert_eq!(record["tool"], "verify_code");
        assert_eq!(record["outcome"], "error");
        let _ = std::fs::remove_file(log.path());
    }

    #[test]
    fn missing_name_keeps_its_dedicated_protocol_error() {
        // The wire path has always answered a nameless tools/call with
        // this exact message (`tests/mcp_server.rs` asserts it) -- the
        // relocation must not change observable protocol behavior.
        let mut log = McpCallLog::new("unit-test-b2");
        let err = tools_call(&json!({ "arguments": {} }), &mut log).unwrap_err();
        assert_eq!(err.0, -32602);
        assert_eq!(err.1, "Missing required parameter 'name'");
        let _ = std::fs::remove_file(log.path());
    }

    #[test]
    fn call_ids_are_monotonic_per_session() {
        let mut log = McpCallLog::new("unit-test-c");
        for tool in ["verify_code", "describe"] {
            let _ = tools_call(&json!({ "name": tool, "arguments": {} }), &mut log);
        }
        let contents = std::fs::read_to_string(log.path()).unwrap();
        let ids: Vec<u64> = contents.lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["call_id"].as_u64().unwrap()).collect();
        assert_eq!(ids, [1, 2]);
        let _ = std::fs::remove_file(log.path());
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        // Values cross-checked with `date -u -d @$((days * 86400))`, not
        // hand-computed -- the first revision of this test was.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_645), (2026, 7, 11));
        assert_eq!(civil_from_days(20_702), (2026, 9, 6));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1)); // leap-year boundary (2000 IS a leap year)
        assert_eq!(civil_from_days(11_059), (2000, 4, 12)); // post-Feb-29 in a leap year
    }

    #[test]
    fn rfc3339_now_shape() {
        let ts = rfc3339_now();
        // Not asserting the exact instant, just the shape the log's
        // consumers (and any grepping human) rely on.
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[19..20], ".");
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 24);
    }
}