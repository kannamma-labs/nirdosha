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
//! That sharing used to stop at `main.rs`'s own commands, though --
//! `hi_llm`'s Generate-mode self-repair loop (`typecheck_and_build_check`)
//! re-derived its own load/typecheck/ownership checks straight against
//! `typeck`/`ownership` instead of coming through here, so it could
//! (and did, silently) drift from what `verify_code` actually checks.
//! [`typecheck_and_check_ownership`] closes that: it's the one place
//! `require_main = true` (a whole generated program) or `false` (a
//! served/emit-ui program) typechecks and ownership-checks, and both
//! `run_verify_pipeline` and `hi_llm` call it instead of the raw
//! `typeck`/`ownership` functions directly.
//!
//! Everything a caller needs is `pub`; the helpers only the pipeline
//! itself uses (`levenshtein`, `classify_load_diagnostic`,
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
/// Load-stage (`LoadStage::Lex`/`Parse`) diagnostics all still arrive as
/// prose (`ParseError`/`LexError` don't carry a structured kind the way
/// `TypeErrorKind`/`OwnershipErrorKind` do), so this is necessarily
/// string-matching against the exact messages `parser.rs`/`token.rs`
/// construct -- an honest, bounded heuristic, same caveat as before
/// this was split out of `run_verify_pipeline`. Each arm here matches
/// one of the field-failure-shaped messages `parser.rs` now raises
/// on purpose (the `let`-without-a-type/`mut`/field-assignment/
/// foreign-top-level-keyword traps), plus the pre-existing reserved-
/// word case. `diag.message` is the bare `ParseError`/`LexError`
/// message (no `"parse error in {path} at {line}:{col}:"` prefix), so
/// these substrings never have to account for that wrapper.
fn classify_load_diagnostic(diag: &crate::loader::LoadDiagnostic) -> (Option<Fix>, Option<&'static str>) {
    let msg = diag.message.as_str();
    const IDENT_PREFIX: &str = "expected identifier, found ";
    if let Some(after) = msg.find(IDENT_PREFIX).map(|i| i + IDENT_PREFIX.len()) {
        let found = &msg[after..];
        if found.starts_with("the reserved keyword ") || found.starts_with("the reserved type name ") {
            return (None, Some("NIR0012"));
        }
    }
    if msg.contains("there is no type inference and no `let _ = expr` discard form") {
        return (
            Some(Fix {
                applicability: Applicability::Manual,
                patch: None,
                rationale: "Bind the result to a real, typed name: `let <name>: <Type> = <expr>` -- there is no type inference and no `let _ = expr` discard form.".to_string(),
            }),
            None,
        );
    }
    if msg.contains("Nirdosha has no `mut` qualifier") {
        return (
            Some(Fix {
                applicability: Applicability::Manual,
                patch: None,
                rationale: "Drop `mut` -- every `let` binding is already reassignable by name.".to_string(),
            }),
            None,
        );
    }
    if msg.contains("structs and array/vector elements can't be assigned into") {
        return (
            Some(Fix {
                applicability: Applicability::Manual,
                patch: None,
                rationale: "Rebuild the whole value instead of assigning into a field/index: `s = StructName(new_field, s.other_field, ...)`, or reassign the whole `let`-bound variable.".to_string(),
            }),
            None,
        );
    }
    if msg.contains("there are no `for` loops, `def`/`class` declarations, or `try`/`catch`") {
        // NIR0005 ("No `for` loops, no closures/lambdas, no tuples") is
        // a real, direct match only for the `for` spelling specifically
        // -- `def`/`class`/`try`/`catch` are the same class of foreign-
        // keyword mistake but aren't what that rule documents, so they
        // stay uncoded rather than force-fit.
        let code = if msg.starts_with("Nirdosha has no `for`") { Some("NIR0005") } else { None };
        return (
            Some(Fix {
                applicability: Applicability::Manual,
                patch: None,
                rationale: "Loop with `while` -- there are no `for` loops, `def`/`class` declarations, or `try`/`catch` in Nirdosha.".to_string(),
            }),
            code,
        );
    }
    (None, None)
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

/// The two checks after a `.nir` file loads that both `run_verify_pipeline`
/// and `hi_llm`'s self-repair loop need identically -- `typecheck`/
/// `typecheck_optional_main` (whichever `require_main` selects) plus
/// `ownership::check_ownership`, including the per-`TypeErrorKind`
/// `NIR0xxx` code classification and `Fix` suggestions
/// (`fix_unbound_identifier` et al.) either consumer gets for free.
///
/// Factored out so `hi_llm::typecheck_and_build_check` (Generate mode's
/// own self-repair loop) calls this instead of re-deriving "typechecks
/// and passes ownership" a second time against `typeck`/`ownership`
/// directly -- the two used to be separate call sites that could
/// silently drift on exactly what that means (see this module's own
/// doc comment: the console and the wire are supposed to run the same
/// code, and until now this particular pair of checks was the one
/// place that promise didn't hold). `require_main` matches whichever of
/// `typecheck`/`typecheck_optional_main` the caller actually needs --
/// `run_verify_pipeline` passes `false` (a served/emit-ui program has
/// no `main`); `hi_llm` passes `true` (a whole generated program
/// always does, and Generate mode's own build-check relies on that).
pub struct TypecheckOwnershipOutcome {
    pub typecheck: StageResult,
    pub ownership: StageResult,
    /// `Some` only when both stages passed.
    pub program: Option<crate::ast::Program>,
}

pub fn typecheck_and_check_ownership(program: crate::ast::Program, require_main: bool) -> TypecheckOwnershipOutcome {
    let mut typecheck = StageResult { status: StageStatus::Passed, errors: vec![] };
    // `Skipped`, not `Passed`: ownership only actually runs below when
    // typecheck produced a `program` to check -- a `None` program (the
    // `.and_then` below short-circuits without touching `ownership` at
    // all) must leave this stage reporting the truth, "never ran,"
    // rather than defaulting to a claim ownership checking never made.
    let mut ownership = StageResult { status: StageStatus::Skipped, errors: vec![] };

    let typecheck_result =
        if require_main { crate::typeck::typecheck(&program) } else { crate::typeck::typecheck_optional_main(&program) };

    let program = match typecheck_result {
        Ok(()) => Some(program),
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
                        // `number`/`string`/`boolean`/`int`/`float` --
                        // parse fine as an unknown named type, and only
                        // fail here; `foreign_type_name_suggestion` is
                        // the same lookup the `Display` message itself
                        // now consults, so the JSON `fix.rationale`
                        // agrees with the prose exactly.
                        crate::typeck::TypeErrorKind::UnknownType(name) => {
                            let fix = crate::typeck::foreign_type_name_suggestion(name).map(|suggestion| Fix {
                                applicability: Applicability::Manual,
                                patch: None,
                                rationale: format!("`{name}` isn't a Nirdosha type -- use `{suggestion}` instead."),
                            });
                            (fix, None)
                        }
                        _ => (None, None),
                    };
                    VerifyDiagnostic { line: e.span.line, col: e.span.col, message: e.to_string(), fix, code }
                })
                .collect();
            None
        }
    };

    let program = program.and_then(|program| match crate::ownership::check_ownership(&program) {
        Ok(()) => {
            ownership.status = StageStatus::Passed;
            Some(program)
        }
        Err(errs) => {
            ownership.status = StageStatus::Failed;
            ownership.errors = errs
                .iter()
                .map(|e| {
                    // Matched explicitly per variant, not a bare
                    // `Some`/wildcard, so a future new variant fails to
                    // compile here until it gets its own real rationale
                    // instead of silently inheriting someone else's.
                    let fix = match &e.kind {
                        crate::ownership::OwnershipErrorKind::UseAfterMove { name } => Some(Fix {
                            applicability: Applicability::Manual,
                            patch: None,
                            rationale: format!(
                                "`box` values are affine -- borrow with `&{name}` to reuse it without moving, or restructure so it's consumed once."
                            ),
                        }),
                        crate::ownership::OwnershipErrorKind::MoveWhileBorrowed { name, borrower } => Some(Fix {
                            applicability: Applicability::Manual,
                            patch: None,
                            rationale: format!(
                                "moving `{name}` while `{borrower}` still borrows it would leave `{borrower}` pointing at freed memory -- use `{name}` through `{borrower}` instead, or move `{name}` only after `{borrower}`'s own scope ends."
                            ),
                        }),
                    };
                    VerifyDiagnostic { line: e.span.line, col: e.span.col, message: e.to_string(), fix, code: None }
                })
                .collect();
            None
        }
    });

    TypecheckOwnershipOutcome { typecheck, ownership, program }
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
    let mut contracts = ContractsResult {
        status: StageStatus::Skipped,
        verdict: ProofVerdict::Proved,
        proved: 0,
        unsupported: 0,
        failed: 0,
        obligations: vec![],
    };
    let mut proof_obligations = ProofObligations { proven_in_range: 0, proven_nonzero_divisor: 0, proven_index_bounds: 0 };

    let program = match crate::loader::load_program_diag(path) {
        Ok((program, _src)) => Some(program),
        Err(diag) => {
            load.status = StageStatus::Failed;
            let (fix, code) = classify_load_diagnostic(&diag);
            // `diag.span` is the real lex/parse position now (or the
            // `0,0` placeholder for an `Io`/`Import`-stage failure,
            // which never had one) -- `message` stays the same fully-
            // worded text `load_program`'s old `String` error always
            // carried (`LoadDiagnostic`'s own `Display`), so this is
            // strictly more information, not a behavior change for any
            // existing consumer of the prose.
            load.errors.push(VerifyDiagnostic { line: diag.span.line, col: diag.span.col, message: diag.to_string(), fix, code });
            None
        }
    };

    let (typecheck, ownership, program) = match program {
        Some(program) => {
            let outcome = typecheck_and_check_ownership(program, false);
            (outcome.typecheck, outcome.ownership, outcome.program)
        }
        None => (StageResult::skipped(), StageResult::skipped(), None),
    };

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
                ContractCheckResult::VacuousPrecondition => {
                    // RFC 0016 fail-closed semantics: a vacuous proof is
                    // reported as such and counts as a failure for gating
                    // purposes -- PROVED with zero reachable states is the
                    // vacuous sibling of PROVED 0/0, not a pass.
                    contracts.failed += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "vacuous_precondition",
                        detail: Some("pre_logic can never be true for any input the parameter types admit -- every post_logic passes vacuously; check for a typo'd precondition".to_string()),
                        fix: None,
                    });
                }
                ContractCheckResult::EngineLimit { obligation, fuel } => {
                    // RFC 0016 fail-closed semantics: "we couldn't check it"
                    // must never publish. EngineLimit counts as a failed
                    // proof for the verdict (Disproved, not Unknown) but is
                    // NOT a code violation -- its status is its own, so the
                    // Phase 1 coverage gate can tell VIOLATED from
                    // ENGINE_LIMIT and stop the model being blamed for a
                    // solver limit.
                    contracts.failed += 1;
                    contracts.obligations.push(ContractObligation {
                        fn_name: outcome.fn_name,
                        status: "engine_limit",
                        detail: Some(format!("{obligation} (rlimit={fuel}; fail-closed -- blocks publishing, but an engine limit, not a code bug)")),
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

/// `crypto_backend::sha256` (2026-09) -- `fips`-feature-aware, same
/// swap point `hi_graph::sha256_hex`/`audit_chain.rs` use, not a third
/// independent direct `sha2` call left un-swapped.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let hex: String = crate::crypto_backend::sha256(bytes).iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
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
    /// RFC 0016 Phase 4 (`rfcs/0016-implementation-plan.md`): the
    /// installed domain packs (`hi_plugin::installed_pack_ids`) that
    /// governed this artifact's confirmed graph, if any. Empty for
    /// every certificate `cmd_certify`/`certify_code` issue today --
    /// neither has a project graph to attribute against, only a bare
    /// source file -- populated only by `hi_api::handle_publish`, the
    /// one call site that has an open project connection. `#[serde(
    /// default)]` so a pre-Phase-4 certificate JSON (no such field at
    /// all) still deserializes through `verify-certificate`'s round
    /// trip instead of failing on an unknown-but-required field.
    #[serde(default)]
    pub governing_packs: Vec<String>,
    /// Real per-invariant attribution: which *specific* active pack
    /// demanded which *specific* proved contract, not just "these packs
    /// are active" (`governing_packs` above). Named by an existing test
    /// comment (`hi_api.rs::fintech_app_under_the_banking_pack_
    /// publishes_...`) as separate "generation audit and governing-set
    /// snapshot" future work; built for real 2026-09-15
    /// (`hi_plugin::governing_invariants`). Same "only `handle_publish`
    /// has the extra context" shape `governing_packs`/`nfr_commitments`
    /// already use -- empty for a bare `certify`/`verify`.
    #[serde(default)]
    pub governing_invariants: Vec<GoverningInvariant>,
    /// This artifact's declared `nfr(...)` commitments (`ast::NfrSpec`
    /// per fn), attached at publish time. Deliberately **not** folded
    /// into `verdict_summary`/`proof_obligations` above: those are Z3-
    /// *proved* facts about this exact artifact; an NFR threshold is
    /// *monitored* at runtime by the APM kernel (`rfcs/0007-apm-
    /// runtime-kernel.md`) and was never any solver's business to
    /// begin with -- keeping it a separate field with its own
    /// evidence_tier (`NfrCommitment::EVIDENCE_TIER`, always
    /// `"monitored"`) is what stops a runtime-tracked claim from ever
    /// reading as a compile-time proof inside the same certificate,
    /// the same discipline "Compliance profiles"' `external_conformance`
    /// requirement kind already holds itself to. Empty for the same
    /// reason `governing_packs` is: only `handle_publish` has a parsed
    /// `Program` to walk for `nfr` attributes.
    #[serde(default)]
    pub nfr_commitments: Vec<NfrCommitment>,
    /// A real, observed transaction-isolation anomaly (`crates/
    /// isolation-core`'s Direct Serialization Graph detector, shared
    /// verbatim with `runtime-kernels::kernel::isolation_check`'s live
    /// FFI path), attached after the fact via `nirdosha certify
    /// --isolation-log <ops.json>`. Named as a real possibility in
    /// `isolation_check.rs`'s own module doc (RFC 0016 Phase 4, "surface
    /// it in the certificate over time"), built for real 2026-09-14.
    /// Deliberately **not** folded into `verdict_summary`/
    /// `proof_obligations`: those are Z3-*proved* facts about this
    /// artifact's source; a detected isolation anomaly is an
    /// a-posteriori *observation* about one run's actual operation
    /// history, the same "runtime-tracked, not solver-proved" split
    /// `nfr_commitments` above already holds itself to (own
    /// `evidence_tier`, `"monitored"`, per entry). Empty at plain
    /// `certify`/`verify` time -- a bare source file has no operation
    /// history to check; a caller wanting to attest an observed
    /// anomaly re-issues the certificate with `--isolation-log`. This
    /// is genuinely a *point-in-time snapshot of what one log
    /// contained*, not a live, continuously-updating claim -- a
    /// certificate is still a static, re-issued artifact each time,
    /// not a subscription; "over time" means "re-attestable as new
    /// evidence arrives," not "auto-updating."
    #[serde(default)]
    pub isolation_violations: Vec<IsolationViolation>,
    /// RFC 0016's "Compliance profiles" section ("What certification
    /// emits") -- the piece that was designed and built (`hi_plugin.rs`'s
    /// `ComplianceProfile`/`check_static_rules`/`SUPPORTED_WIRING_
    /// REQUIREMENT_KINDS`, a real FAPI 2.0 pack fixture, all real and
    /// tested) but never actually surfaced into a `Certificate` --
    /// `check_static_rules` had no caller outside its own unit tests
    /// before this field existed. Built for real 2026-09-15 (`hi_plugin::
    /// compliance_profile_reports`, the one place with both the parsed
    /// `Program` and the active-pack manifests, same shape as
    /// `governing_invariants`). Empty for the same reason `governing_
    /// packs` is: only `handle_publish` has a parsed `Program` to check
    /// profiles against.
    #[serde(default)]
    pub compliance_profiles: Vec<ComplianceProfileReport>,
}

/// One active pack's compliance profile, as attested against *this*
/// published artifact -- RFC 0016's four requirement kinds, each with
/// its own honest `evidence_tier` rather than one blanket "compliant"
/// bit. `validate_contract` isn't a separate field here: it's exactly
/// `Certificate::verdict_summary`/`governing_invariants` already are,
/// for a profile that layers no additional requirement on top of an
/// already-proved `validate` block.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ComplianceProfileReport {
    pub pack_id: String,
    pub profile_name: String,
    pub profile_version: String,
    /// A real AST-level check the toolchain ran against this exact
    /// `Program` (`hi_plugin::check_static_rules`) -- present here only
    /// if it passed; `hi_plugin::compliance_profile_reports`'own doc
    /// comment on why a failure refuses the whole publish rather than
    /// silently recording an unsatisfied claim (RFC 0016's own "not a
    /// checkbox in a wiki" framing).
    pub static_rules: Vec<ComplianceRequirementReport>,
    /// Declared, not verified here -- RFC 0016's own scoping: nirdosha
    /// never runs an authorization server, so most of these (`par`,
    /// `pkce_s256`, `iss_check`, `par_request_uri_lifetime`) have no
    /// resource-server enforcement point this toolchain could check
    /// against. `sender_constrained_tokens` is the one exception with
    /// real runtime enforcement (`compiled_serve`'s DPoP check, wired
    /// separately via `wiring_requires_sender_constrained_tokens`) --
    /// still reported as `"declared"` here, not `"checked"`, because
    /// this report is a static fact about the pack's own declaration,
    /// not a live attestation that a particular running deployment
    /// actually enforced it.
    pub wiring_requirements: Vec<ComplianceRequirementReport>,
    /// RFC 0016's own "NOT provable by us" category verbatim: AS-runtime
    /// behavior and OIDF certification are external/legal acts. Carried
    /// here as the pack's own declared requirement text; whether a real
    /// conformance report has been attached and verified against its
    /// issuer is real, separate follow-up (RFC 0016's own note: "the
    /// attachment must be verifiable against the issuer... not merely
    /// content-hashed").
    pub external_conformance: Vec<ExternalConformanceReport>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ComplianceRequirementReport {
    pub kind: String,
    pub evidence_tier: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ExternalConformanceReport {
    pub kind: String,
    pub description: String,
    pub evidence_tier: String,
}

impl ComplianceRequirementReport {
    pub const STATIC_RULE_EVIDENCE_TIER: &'static str = "checked";
    pub const WIRING_REQUIREMENT_EVIDENCE_TIER: &'static str = "declared";
}

impl ExternalConformanceReport {
    pub const EVIDENCE_TIER: &'static str = "external";
}

/// One detected isolation anomaly, carried into the certificate
/// verbatim -- see `Certificate::isolation_violations`'s own doc
/// comment for why this is a distinct field with its own tier rather
/// than folded into the Z3-proved `verdict_summary`.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct IsolationViolation {
    /// The `txn_id`s involved, in encounter order (the cycle's first
    /// txn repeats as the last element) -- `nirdosha_isolation_core::
    /// Anomaly::cycle`, carried through unchanged.
    pub cycle: Vec<String>,
    /// Always `"monitored"` -- see `NfrCommitment::evidence_tier`'s own
    /// doc comment for why this is a real per-value field, not just
    /// inferred from the struct's presence.
    pub evidence_tier: String,
}

impl IsolationViolation {
    pub const EVIDENCE_TIER: &'static str = "monitored";
}

/// Maps every anomaly a real `Checker::check()` run found to the
/// certificate-carried shape -- the one place `evidence_tier` gets
/// stamped onto an observed isolation anomaly, so every call site
/// (`cmd_certify`'s `--isolation-log`, and any future one) gets it
/// right the same way `nfr_commitments_from_program` is the one place
/// that happens for NFR commitments.
pub fn isolation_violations_from_anomalies(anomalies: &[nirdosha_isolation_core::Anomaly]) -> Vec<IsolationViolation> {
    anomalies.iter().map(|a| IsolationViolation { cycle: a.cycle.clone(), evidence_tier: IsolationViolation::EVIDENCE_TIER.to_string() }).collect()
}

/// One `validate`-covered fn, attributed to the specific active pack
/// that demanded it -- see `Certificate::governing_invariants`'s own
/// doc comment for why this is a distinct field from the flat
/// `governing_packs` list. Built by `hi_plugin::governing_invariants`
/// (the one place with both the parsed `Program` and the active-pack
/// manifests to compute this against), not here -- this struct is just
/// the certificate-carried shape, matching `NfrCommitment`/
/// `IsolationViolation`'s own pattern.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GoverningInvariant {
    pub fn_name: String,
    pub pack_id: String,
}

/// One function's declared non-functional requirements, carried into
/// the certificate verbatim -- see `Certificate::nfr_commitments`'s own
/// doc comment for why this is a distinct field with its own tier
/// rather than folded into the Z3-proved `verdict_summary`.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct NfrCommitment {
    pub fn_name: String,
    pub nfr: crate::ast::NfrSpec,
    /// Always `"monitored"` -- a `const fn`-shaped constant would be
    /// tidier, but this field must actually serialize per-value (not
    /// be inferred from the struct's mere presence), so a real string
    /// field keeps the JSON self-describing for a reader who only has
    /// the certificate, not this struct's source.
    pub evidence_tier: String,
}

impl NfrCommitment {
    pub const EVIDENCE_TIER: &'static str = "monitored";
}

/// Walks every `fn` in `program` carrying a non-empty `nfr(...)` and
/// returns one `NfrCommitment` per match, sorted by `fn_name` --
/// deterministic order, matching `Certificate`'s own "no timestamp, no
/// random nonce, always the same bytes for the same input" discipline.
/// `NfrSpec::is_empty` filters out a `None`/all-`None` spec (the
/// grammar can't actually produce a bare `nfr()` with nothing inside,
/// but this stays defensive rather than assuming that of every future
/// caller).
pub fn nfr_commitments_from_program(program: &crate::ast::Program) -> Vec<NfrCommitment> {
    let mut commitments: Vec<NfrCommitment> = program
        .fns
        .iter()
        .filter_map(|f| f.nfr.filter(|spec| !spec.is_empty()).map(|nfr| NfrCommitment { fn_name: f.name.clone(), nfr, evidence_tier: NfrCommitment::EVIDENCE_TIER.to_string() }))
        .collect();
    commitments.sort_by(|a, b| a.fn_name.cmp(&b.fn_name));
    commitments
}

/// One real, observed NFR-threshold crossing -- the *exact* wire shape
/// `runtime-kernels::kernel::nfr`'s own `send_escalation` already POSTs
/// to `NIRDOSHA_OBSERVABILITY_URL` (`{"function":...,"nfr":...,
/// "threshold":...,"actual":...,"timestamp_ms":...}`), reused verbatim
/// as `nirdosha check-drift`'s own input format rather than inventing a
/// second one. Practical consequence: an operator whose observability
/// endpoint already logs every incoming escalation POST body to a file
/// (`examples/isolation_demo/listener.py`'s own pattern, generalized)
/// has a valid `check-drift` input with zero new capture code --
/// unlike `check-isolation`'s own ops-log, which nothing produces yet,
/// this one's producer already exists and already ships.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NfrEscalation {
    pub function: String,
    /// `"latency_ms"` | `"error_rate_max"` | `"throughput_min_per_sec"`
    /// | `"concurrency_max"` -- `ast::NfrSpec`'s own four field names,
    /// exactly as `nfr.rs`'s `check_and_escalate` names them.
    pub nfr: String,
    pub threshold: f64,
    pub actual: f64,
    #[serde(default)]
    pub timestamp_ms: u128,
}

/// One compile-time `nfr(...)` assumption a real, observed escalation
/// log shows was actually crossed in practice -- Phase 4 item 1 of
/// `docs/research/2026-09-pending-verification-differentiation-
/// work.md`: "detect when a compile-time nfr(...) assumption diverges
/// from what the APM kernel actually observes in production."
#[derive(Debug, serde::Serialize)]
pub struct DriftFinding {
    pub fn_name: String,
    pub nfr_kind: String,
    pub declared_threshold: f64,
    /// The single worst observed value among every matching escalation
    /// -- highest `actual` for a must-stay-under kind (`latency_ms`/
    /// `error_rate_max`/`concurrency_max`), lowest for the one
    /// must-stay-over kind (`throughput_min_per_sec`).
    pub worst_observed: f64,
    pub violation_count: usize,
}

/// Compares every declared `nfr(...)` field (a fn can declare more than
/// one -- `latency_ms` *and* `concurrency_max` together, say) against a
/// real observed escalation log, one `DriftFinding` per `(fn_name,
/// nfr_kind)` pair with at least one matching escalation.
///
/// **No extra "is this actually past threshold" logic here, on
/// purpose**: `nfr.rs`'s own `escalate()` only ever fires once a real
/// per-call check already confirmed a crossing (`check_and_escalate`),
/// so any escalation naming a given `(function, nfr)` pair is already
/// conclusive evidence that commitment drifted in practice -- this
/// function's whole job is matching declared assumptions against
/// confirmed observations, not re-deriving the threshold logic `nfr.rs`
/// already owns.
pub fn nfr_drift(commitments: &[NfrCommitment], escalations: &[NfrEscalation]) -> Vec<DriftFinding> {
    let mut findings = Vec::new();
    for c in commitments {
        let declared: [(&str, Option<f64>); 4] = [
            ("latency_ms", c.nfr.latency_ms.map(|v| v as f64)),
            ("error_rate_max", c.nfr.error_rate_max),
            ("throughput_min_per_sec", c.nfr.throughput_min_per_sec.map(|v| v as f64)),
            ("concurrency_max", c.nfr.concurrency_max.map(|v| v as f64)),
        ];
        for (kind, threshold) in declared {
            let Some(declared_threshold) = threshold else { continue };
            let matching: Vec<&NfrEscalation> = escalations.iter().filter(|e| e.function == c.fn_name && e.nfr == kind).collect();
            if matching.is_empty() {
                continue;
            }
            let worst_observed = if kind == "throughput_min_per_sec" {
                matching.iter().map(|e| e.actual).fold(f64::INFINITY, f64::min)
            } else {
                matching.iter().map(|e| e.actual).fold(f64::NEG_INFINITY, f64::max)
            };
            findings.push(DriftFinding { fn_name: c.fn_name.clone(), nfr_kind: kind.to_string(), declared_threshold, worst_observed, violation_count: matching.len() });
        }
    }
    findings.sort_by(|a, b| (a.fn_name.as_str(), a.nfr_kind.as_str()).cmp(&(b.fn_name.as_str(), b.nfr_kind.as_str())));
    findings
}

/// Certificate v1 (`nirdosha-master-plan.md` Part 3 Nov 2026, "Signed
/// certificates (v1) -- key-pinned verdicts", parity target: Velvet)
/// -- Certificate v0 plus a real Ed25519 signature (`ring`, already a
/// dependency; no hand-rolled crypto) over v0's own canonical bytes.
/// `#[serde(flatten)]` puts every v0 field back at the top level
/// (additive over v0, per `docs/STABILITY_AND_RELEASES.md`'s own rule
/// for this schema -- a v0-only consumer reading a v1 certificate
/// still finds every field it expects, plus three it can ignore).
/// "Key-pinned": the public key travels with the certificate so a
/// verifier never needs external key discovery to check the
/// signature -- trust is established by the *verifier* pinning which
/// public keys it accepts in advance (an operational policy, not
/// something this format enforces), the same model TLS certificate
/// pinning uses for the same reason.
///
/// Lives here, not in `main.rs`, for the same reason every other
/// verify/fix/certify pipeline function does (this module's own doc
/// comment) -- `hi_api::handle_publish`'s optional publish-time
/// signing (RFC 0016 Phase 4) needs it from the library side, not just
/// `main.rs::cmd_certify`'s CLI `--sign` flag.
#[derive(serde::Serialize)]
pub struct SignedCertificate {
    #[serde(flatten)]
    pub certificate: Certificate,
    pub signature_algorithm: &'static str,
    pub public_key: String,
    pub signature: String,
}

/// Signs `certificate`'s own canonical byte serialization
/// (`serde_json::to_vec` on the plain `Certificate` struct --
/// `Certificate` derives `Serialize` with no `#[serde(rename_all)]`
/// alphabetizing pass, so this is always the same bytes for the same
/// values, independent of what order any particular JSON *source*
/// text happened to list fields in) with the Ed25519 private key at
/// `key_path` (raw PKCS#8, as `nirdosha keygen` writes). Verification
/// (`cmd_verify_certificate`) does the mirror operation: parse the
/// signed JSON back into a plain `Certificate` (ignoring the three
/// signature-related fields, which `Certificate` doesn't declare),
/// re-serialize *that*, and check the signature against those exact
/// bytes -- so the two sides never need to agree on a JSON
/// canonicalization scheme beyond "both go through the same Rust
/// struct's own `Serialize` impl."
pub fn sign_certificate(certificate: &Certificate, key_path: &str) -> Result<SignedCertificate, String> {
    let canonical = serde_json::to_vec(certificate).expect("Certificate always serializes");
    let (public_key, signature) = sign_bytes(&canonical, key_path)?;
    Ok(SignedCertificate {
        certificate: serde_json::from_slice(&canonical).expect("re-parsing what was just serialized cannot fail"),
        signature_algorithm: "ed25519",
        public_key,
        signature,
    })
}

/// Ed25519-signs arbitrary `bytes` with the PKCS#8 private key at
/// `key_path` (raw DER, as `nirdosha keygen` writes) -- the primitive
/// behind both `sign_certificate` above and `hi_plugin`'s pack-signing
/// layer (RFC 0016 Phase 4, Sigstore-*pattern* trust for packs): one
/// Ed25519 signing implementation in this crate, not a hand-rolled copy
/// per signer. Returns `(public_key_base64, signature_base64)`.
pub fn sign_bytes(bytes: &[u8], key_path: &str) -> Result<(String, String), String> {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;
    use crate::crypto_backend::signature::KeyPair;

    let pkcs8 = std::fs::read(key_path).map_err(|e| format!("reading private key {key_path}: {e}"))?;
    let keypair = crate::crypto_backend::signature::Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|e| format!("{key_path} is not a valid Ed25519 PKCS#8 private key: {e}"))?;
    let signature = keypair.sign(bytes);
    Ok((BASE64_STANDARD.encode(keypair.public_key().as_ref()), BASE64_STANDARD.encode(signature.as_ref())))
}

/// The verify-side mirror of [`sign_bytes`]: checks `signature_b64`
/// against `bytes` under `public_key_b64`, both base64 exactly as
/// `sign_bytes`/`nirdosha keygen` produce them. Returns `Ok(false)`,
/// not `Err`, for a well-formed but non-matching signature -- "checked,
/// and it didn't match" is a real, distinct outcome from "couldn't even
/// attempt the check" (malformed base64/key bytes), which stays `Err`.
/// Says nothing about whether `public_key_b64` is a key the *caller*
/// should trust -- pinning acceptable keys is the caller's own
/// operational policy (`SignedCertificate`'s own doc comment on this
/// same point; `hi_plugin`'s trust-anchor list is that policy for
/// packs).
pub fn verify_bytes(bytes: &[u8], public_key_b64: &str, signature_b64: &str) -> Result<bool, String> {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;

    let signature_bytes = BASE64_STANDARD.decode(signature_b64).map_err(|e| format!("signature is not valid base64: {e}"))?;
    let public_key_bytes = BASE64_STANDARD.decode(public_key_b64).map_err(|e| format!("public_key is not valid base64: {e}"))?;
    let public_key = crate::crypto_backend::signature::UnparsedPublicKey::new(&crate::crypto_backend::signature::ED25519, &public_key_bytes);
    Ok(public_key.verify(bytes, &signature_bytes).is_ok())
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
        // Empty here -- neither `cmd_certify` nor `certify_code` has a
        // project graph or a parsed `Program` to attribute against, only
        // a bare source file. `hi_api::handle_publish` fills both in
        // after calling this, the one call site with the extra context
        // (`Certificate::governing_packs`/`nfr_commitments`'s own doc
        // comments).
        governing_packs: Vec::new(),
        governing_invariants: Vec::new(),
        nfr_commitments: Vec::new(),
        // Empty here too -- see `Certificate::isolation_violations`'s
        // own doc comment: a bare source file has no operation history
        // to check against. `cmd_certify`'s `--isolation-log` flag
        // fills this in after calling `build_certificate`, the same
        // "populated by the one call site with the extra context"
        // shape `governing_packs`/`nfr_commitments` already use.
        isolation_violations: Vec::new(),
        // Empty here too, same reason as `governing_invariants`: only
        // `hi_api::handle_publish` has both a parsed `Program` and the
        // active-pack manifests to check compliance profiles against.
        compliance_profiles: Vec::new(),
    }
}

/// Baked into the binary at compile time (`include_str!`), the same
/// "ships inside the binary" posture `STD_CATALOG_JSON` documents for
/// `emit-catalog` -- `nirdosha.gbnf` is checked into the repo at the
/// crate root and re-generated by `crates/grammar_export`'s own tests
/// against `llama-cpp-gbnf`, never hand-edited independently of
/// the grammar it's exported from.
const NIRDOSHA_GBNF: &str = include_str!("../nirdosha.gbnf");

/// The v2 declarative layer's real grammar: `docs/
/// nirdosha-v2-comment-layer.md` §4's spec, plus every comment kind
/// actually shipped in `examples/nirdosha-v2-corpus` today (not a
/// forward-looking wishlist -- `nirdosha:validate`/`workflow`/
/// `screen`/etc. are inert under every checker in this codebase right
/// now; this teaches the *encoding*, not a claim that they're
/// cross-referenced). Hand-maintained the same way `get_ui_conventions`
/// already is (this module's own doc comment on that function explains
/// why: static content, not `include_str!`, so it needs a matching edit
/// if the registry moves) -- update this alongside a new comment kind
/// landing in the corpus.
const NIRDOSHA_V2_COMMENT_GRAMMAR: &str = r#"Every declarative construct rides one or more `/// nirdosha:<kind> {json}` doc-comment lines directly above the Rust item they describe (a struct, a fn). Consecutive `///` lines are joined before parsing, so a multi-line JSON payload is fine. The payload is always exactly one JSON object; unknown keys are a hard error (deny_unknown_fields), the same rule the shipped `nirdosha:contract` encoding already enforces. A single item may carry more than one kind (e.g. `nirdosha:contract` + `nirdosha:transact` on the same fn).

Known kinds, and the shape their JSON object takes (every example below is real, taken from a file in examples/nirdosha-v2-corpus/src that plain cargo builds and runs today):

- nirdosha:contract {"effects":["pure"|"io"|...], "requires":{"role":"..."}, "nfr":{"latency_ms":N,"concurrency_max":N}, "crud":{...}} -- the shipped form; also expressible as the `#[nirdosha_rt::contract(effects(pure), requires(role = "..."), nfr(latency_ms = 50, concurrency_max = 1000))]` attribute macro, which additionally injects the unforgeable RoleProof parameter (see get_grammar's `role_gating` field). The ONLY kind any checker in this codebase (`cargo-nirdosha`) actually validates today.

- nirdosha:validate {"fn":"name","pre":["expr", ...],"post":["expr", ...]} -- Hoare pre/post clauses over the decorated fn's own parameter names and `result`. One JSON array entry per clause; every `pre` is a conjunctive hypothesis, every `post` is checked independently.

- nirdosha:workflow {"name":"...", "data":{"struct":"StructName","fields":{...}}, "states":{"StateName":{"transitions":{"EventName":"NextState"}, "terminal":true, "on_entry":"fn_name"}, ...}} -- decorates the state-carrying struct; the state-transition functions themselves (e.g. `start_x`/`advance_x`) are ordinary Rust fns nearby, referenced by name, not enumerated in the JSON.

- nirdosha:screen {"for":"StructName", "title":"...", "fields":{"field_name":{"label":"..."}}} -- decorates the struct a generated UI screen is about. Composes with the plain naming-convention inference (`list_/create_/update_/delete_<struct>` fns) -- this comment is additive, never a replacement for it.

- nirdosha:dashboard {"tiles":[...], "charts":[...]} -- each tile/chart names an existing `stat_*`/`chart_*` fn.

- nirdosha:transact {"verify":"fn_name", "compensate":"fn_name"} (+ optional "retry"/"timeout") -- decorates a fn whose real Rust body performs the saga (calls its own verify/commit/compensate fns visibly); the comment is a claim about what that body does, checked declared-vs-actual, never a replacement for the body.

- nirdosha:serve {"routes":[...]} -- serve-surface metadata for a fn exposing an HTTP route.

- nirdosha:field {"struct":"StructName","field":"field_name","requires":{"role":"..."}} -- field-level masking: the named field must be independently gated in the fn body via a hand-written `Option<&RoleProof<R>>` parameter checked by a `mask_unless`-shaped helper (per-field gating is not a macro feature the way fn-level `requires` is).

- nirdosha:schema / nirdosha:role_mapping -- db-table/column and role-to-table-permission convention declarations (46_db_schema_and_role_mapping_conventions.nir).

- nirdosha:visual / nirdosha:workspace / nirdosha:layout / nirdosha:nav -- additional declarative-UI kinds (graph/heatmap/timeline visuals, workspace panels, page layout, module nav grouping) with the same "one JSON object, deny_unknown_fields" shape as the rest.

- nirdosha:audited -- the one `//`-form (not `///`) kind: doc comments cannot attach to a statement inside a fn body, so an audited block is marked `// nirdosha:audited` immediately above it with an `'audited: <label>` loop/block label, not a doc comment on an item.

The registry is extensible: a signed plugin may declare `nirdosha:plugin:<name>:...` kinds later (not shipped yet)."#;

fn require_str_arg<'a>(arguments: &'a serde_json::Value, name: &str) -> Result<&'a str, String> {
    arguments.get(name).and_then(|v| v.as_str()).ok_or_else(|| format!("Missing required parameter '{name}'"))
}

/// `verify_code` -- v2 (`docs/nirdosha-v2-comment-layer.md`): runs
/// `v2_verify::verify_v2_source` against inline source (real Rust,
/// checked by the two readers a v2 file is actually held to: `cargo
/// build`, then `cargo-nirdosha`'s in-process contract scanner). There
/// is no Z3/SMT proof pipeline for v2 yet (Phase 4 of the migration
/// doc is still open), so unlike the retired native `VerifyVerdict`
/// this never reports a `Proved`/`Disproved` verdict -- `passed` is
/// exactly "it builds and its `nirdosha:contract` claims are well-
/// formed," nothing stronger claimed.
pub fn verify_code(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let verdict = crate::v2_verify::verify_v2_source(source)?;
    let mut value = serde_json::to_value(&verdict).expect("V2Verdict always serializes");
    value["verdict"] = json!(verdict.verdict());
    Ok(value)
}

/// `get_grammar` -- v2 has no GBNF: a v2 `.nir` file is *valid Rust*,
/// a grammar every model already knows deeply, so there is nothing to
/// constrain-decode against beyond what any Rust-capable model already
/// has. What v2 actually adds is the `nirdosha:*` doc-comment
/// declarative layer (`docs/nirdosha-v2-comment-layer.md` §4) -- the
/// one part of v2 syntax a model has never seen, and the one this tool
/// returns.
pub fn get_grammar(_arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(json!({
        "format": "v2-comment-layer",
        "base_syntax": "A v2 .nir file is valid, ordinary Rust -- compiled unmodified by plain `cargo`, extension intact. Write normal Rust: `fn`/`struct`/`enum`/`match`, `String`/`&str`, `println!`, semicolons, no bare top-level `return` needed on a trailing expression. Every declarative construct native `.nir` used a keyword for (`workflow`, `screen`, `dashboard`, `transact`, `validate`) is NOT special syntax in v2 -- it rides a `/// nirdosha:<kind> {json}` doc comment on the Rust item it describes instead.",
        "comment_layer_grammar": NIRDOSHA_V2_COMMENT_GRAMMAR,
        "role_gating": "requires(role: \"...\") becomes #[nirdosha_rt::contract(requires(role = \"...\"))] on the fn, after `nirdosha_rt::roles! { Name = \"...\"; }` declares the role once at the crate root. The macro injects an unforgeable `&nirdosha_rt::RoleProof<Name>` as the function's actual (hidden) leading parameter -- callers obtain one via `nirdosha_rt::Auth::login(subject, &roles).prove::<nirdosha_roles::Name>()` and pass it as the first argument.",
    }))
}

/// `fix` -- v2 has no auto-patcher: `write_auto_patches` rewrites
/// diagnostics against this crate's own native `ast::Program`, which a
/// v2 candidate (real Rust, `syn`-parsed) never produces. Rather than
/// fabricate one, `fix` runs the identical `verify_code` check and
/// always reports `applied: []` -- an honest "here is what's wrong,
/// nothing was auto-patched" until a real v2 fixer exists. `apply` is
/// still accepted (and echoed) so an existing caller's request shape
/// doesn't break; it has no effect yet.
pub fn fix(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let apply = arguments.get("apply").and_then(|v| v.as_bool()).unwrap_or(false);
    let verdict = crate::v2_verify::verify_v2_source(source)?;
    let mut before = serde_json::to_value(&verdict).expect("V2Verdict always serializes");
    before["verdict"] = json!(verdict.verdict());
    Ok(json!({
        "before": before,
        "applied": Vec::<String>::new(),
        "apply_requested": apply,
        "note": "no v2 auto-fixer exists yet; nothing is applied. Re-read the diagnostics in `before` and edit the source yourself, then call verify_code again.",
    }))
}

/// `describe` -- v2: parses inline source as plain Rust (`syn`, does
/// not require it to build, the same "still legitimate to inspect"
/// posture the native `describe` took) and returns a curated
/// structural summary via `v2_verify::describe_v2_source`: every
/// fn/struct/enum, plus every `nirdosha:*` doc-comment declaration
/// found and which item it decorates.
pub fn describe(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    crate::v2_verify::describe_v2_source(source)
}

/// `certify_code` -- v2: verifies (`verify_code`'s exact pipeline),
/// then wraps the result in a minimal, honestly-scoped certificate.
/// Deliberately **not** the native `Certificate` shape
/// (`build_certificate`): that struct's `proof_obligations`/
/// `verdict_summary` encode Z3/SMT-proved facts about a native
/// `ast::Program`, and no such proof pipeline exists for v2 source
/// (Phase 4 of `docs/nirdosha-v2-comment-layer.md` is still open) --
/// claiming that shape for v2 would assert a stronger guarantee than
/// was actually checked. `evidence_tier` mirrors `cargo-nirdosha`'s own
/// Stage-1 posture (`nirdosha_contract_core::evidence::Coverage::
/// source_scan`): "source scanned + built", not "proved".
///
/// Deliberately **unsigned**, same reasoning as the native tool: key
/// custody is a human decision that must not cross an agent-callable
/// boundary.
pub fn certify_code(arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let source = require_str_arg(arguments, "source")?;
    let verdict = crate::v2_verify::verify_v2_source(source)?;
    Ok(json!({
        "certificate_version": "nirdosha.certificate/v2-source-scan",
        "source_hash": sha256_hex(source.as_bytes()),
        "toolchain_version": env!("CARGO_PKG_VERSION"),
        "evidence_tier": "source_scan",
        "verdict": verdict.verdict(),
        "builds": verdict.builds,
        "build_diagnostic": verdict.build_diagnostic,
        "contracts_found": verdict.contracts_found,
        "violations": verdict.violations,
    }))
}

/// `get_nirdosha_constructs` -- v2 analogue of the native tool: the
/// live, two-reader-verified inventory of v2's major constructs
/// (`crate::v2_capabilities`): `fn`, `struct`, `enum`/`match`,
/// `nirdosha:validate`, `nirdosha:workflow`, `nirdosha:transact`,
/// `nirdosha:screen`+`serve`, `serde_json`, identity+`RoleProof`. Each
/// entry's `source` is a real program just run through `cargo build` +
/// `cargo-nirdosha`'s scanner against *this* build -- not a hand-typed
/// claim that can drift stale, same discipline the native
/// `capabilities.rs` documents for why that drift is a real,
/// previously-observed failure mode.
pub fn get_nirdosha_constructs(_arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    let constructs: Vec<serde_json::Value> = crate::v2_capabilities::run_v2_capability_checks()
        .into_iter()
        .map(|r| json!({ "name": r.name, "supported": r.passed, "example": r.source, "diagnostic": r.diagnostic }))
        .collect();
    Ok(json!({ "constructs": constructs }))
}

/// `get_ui_conventions` -- v2: there is no `ui_gen.rs`-style naming-
/// convention inference over v2/Rust source anywhere in this codebase
/// (that machinery only ever walked the native `ast::Program`) -- so
/// unlike the retired native tool, this isn't teaching an inference
/// rule, it's pointing at the REAL, working v2 UI mechanism: a family
/// of declarative `nirdosha_rt::*!` proc-macros (`crates/
/// nirdosha-macros/src/{crud_screens,dashboard,kanban_board,wizard,
/// settings_screen,communication_feed,categorical}.rs`, RFC 0009 Track
/// C) that expand to ordinary, `rustc`-checked Rust -- a typo'd field
/// or a wrong access level is a real compile error, not a silent
/// no-screen-generated gap the way a naming-convention miss used to be
/// in the native tier. Every invocation shape below is quoted verbatim
/// from that macro's own module doc comment, not paraphrased. The
/// `nirdosha:screen`/`dashboard`/`workspace`/`layout`/`nav`/`visual`
/// comment-layer kinds (`get_grammar`'s `comment_layer_grammar`) are a
/// SEPARATE, currently-inert declarative layer (no checker cross-
/// references them yet anywhere in this codebase) -- prefer the
/// macros below for anything that needs to actually render today.
pub fn get_ui_conventions(_arguments: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(json!({
        "archetype_macros": [
            {
                "macro": "nirdosha_rt::crud_screens!",
                "archetype": "List/Detail/Create-Form/Edit-Form/Delete-confirmation over one entity",
                "invocation": "nirdosha_rt::crud_screens! {\n    mount: mount_product_screens,\n    entity: Product,\n    store: product_store,\n    path: \"/products\",\n    fields: [ name: String, price_cents: i64, cost_cents: i64 ],\n    create: requires role \"admin\",\n    read: public,\n    update: requires role \"admin\",\n    delete: requires role \"admin\",\n}",
                "requirements": "entity must derive Clone, Default, and serde::Serialize; every field type must implement ParseField (a missing impl is an ordinary rustc trait-bound error).",
                "checked_by": "rustc (compile-time); a typo'd field name is a real \"no such field\" error"
            },
            {
                "macro": "nirdosha_rt::dashboard!",
                "archetype": "Dashboard -- stat tiles + bar charts",
                "invocation": "nirdosha_rt::dashboard! {\n    mount: mount_sales_dashboard,\n    path: \"/dashboard\",\n    title: \"Sales Overview\",\n    refresh_seconds: 300,\n    widgets {\n        Metric { label: \"Revenue MTD\", fn: stat_revenue_mtd, target: 1000000, alert_below: true },\n        Chart  { label: \"By Region\", fn: chart_revenue_by_region, mark: bar },\n    }\n}",
                "requirements": "each widget's fn is called directly in generated code -- a wrong name or signature is an ordinary rustc error, not a runtime surprise.",
                "checked_by": "rustc (compile-time)"
            },
            {
                "macro": "nirdosha_rt::kanban_board!",
                "archetype": "Board/Canvas -- drag-and-drop, grouped by one categorical field",
                "invocation": "nirdosha_rt::kanban_board! {\n    mount: mount_status_board,\n    entity: Product,\n    store: product_store,\n    path: \"/board\",\n    access: public,\n    title_field: name,\n    column_field: status,\n    columns: [ \"backlog\", \"in_progress\", \"done\" ],\n    move_path: \"/products/{id}/move/{to}\",\n}",
                "requirements": "presentational only -- a card move POSTs to move_path, typically a route the app wires by hand to a categorical_actions!-generated fn. The board itself carries no authorization logic; the security boundary is that move endpoint.",
                "checked_by": "rustc (compile-time) for the board itself; the move endpoint's own #[contract(requires(role=..))] for authorization"
            },
            {
                "macro": "nirdosha_rt::wizard!",
                "archetype": "Workflow/Wizard -- multi-step form, server-side progress",
                "invocation": "nirdosha_rt::wizard! {\n    mount: mount_onboarding_wizard,\n    entity: Employee,\n    store: employee_store,\n    path: \"/onboarding\",\n    access: public,\n    steps: [\n        { name: \"Basics\", fields: [ name: String, department: String ] },\n        { name: \"Compensation\", fields: [ salary: f64 ] },\n    ],\n}",
                "requirements": "each step's fields are fixed at compile time (one literal route per step); the final step assembles the full entity and inserts it via the same datasource crud_screens! uses.",
                "checked_by": "rustc (compile-time)"
            },
            {
                "macro": "nirdosha_rt::settings_screen!",
                "archetype": "Settings/Configuration -- one always-present record, no list, no id, no delete",
                "invocation": "nirdosha_rt::settings_screen! {\n    mount: mount_app_settings,\n    entity: AppSettings,\n    store: app_settings_store,\n    path: \"/settings\",\n    fields: [ site_name: String, maintenance_mode: bool ],\n    access: requires role \"admin\",\n}",
                "requirements": "the datasource is a bare `fn() -> &'static Mutex<Entity>` (exactly one row), not the HashMap<i64, Entity> crud_screens!/wizard! use.",
                "checked_by": "rustc (compile-time)"
            },
            {
                "macro": "nirdosha_rt::communication_feed!",
                "archetype": "Communication -- append-only, newest-first message feed",
                "invocation": "nirdosha_rt::communication_feed! {\n    mount: mount_team_feed,\n    entity: Message,\n    store: message_store,\n    path: \"/feed\",\n    fields: [ author: String, body: String ],\n    post_access: requires role \"member\",\n    read_access: public,\n    refresh_seconds: 5,\n}",
                "requirements": "honest client-side polling (a <meta http-equiv=\"refresh\"> tag, dashboard!'s exact mechanism) -- not real server push; the generated routes never pretend otherwise.",
                "checked_by": "rustc (compile-time)"
            },
            {
                "macro": "nirdosha_rt::categorical_actions!",
                "archetype": "One role-gated transition function per value of a categorical field",
                "invocation": "nirdosha_rt::categorical_actions! {\n    entity: Product,\n    store: product_store,\n    field: is_approved: bool,\n    actions {\n        true  => approve_product    requires role \"approver\",\n        false => disapprove_product requires role \"compliance_officer\",\n    }\n}",
                "requirements": "generates real #[contract(requires(role = ..))]-gated functions (the only thing that decides access) plus a derived, NON-authoritative role_for_<field> projection for audit logs/UI hints -- never itself consulted for authorization. Leaving a field value uncovered is rustc's own E0004 non-exhaustive-patterns error, not a scanner check.",
                "checked_by": "rustc (compile-time) for both the actions and their exhaustiveness"
            }
        ],
        "role_gating": "Every archetype macro's `access`/`create`/`read`/`update`/`delete`/`post_access`/`read_access` field takes `public` or `requires role \"<name>\"`, expanding to the same #[nirdosha_rt::contract(requires(role = \"...\"))] mechanism get_grammar's `role_gating` field describes -- one unforgeable RoleProof-gated fn per protected action, never a runtime string comparison.",
        "declarative_comment_layer": "A separate, currently-inert layer: nirdosha:screen/dashboard/workspace/layout/nav/visual doc-comment kinds (see get_grammar's comment_layer_grammar) describe the SAME archetypes in JSON form, but nothing in this codebase cross-references them against real code yet (docs/nirdosha-v2-comment-layer.md's own Phase 2, 'emit-ui reads the comment layer,' is still open). Use them to document intent machine-readably; don't rely on them to make anything render -- use the archetype macros above for that.",
        "sources": [
            "crates/nirdosha-macros/src/{crud_screens,dashboard,kanban_board,wizard,settings_screen,communication_feed,categorical}.rs (every invocation above is quoted verbatim from that file's own module doc comment)",
            "rfcs/0009-ui-catalog-extensibility.md (Track C archetypes)",
            "docs/nirdosha-v2-comment-layer.md §3/§4/§6 (the comment-layer kinds' design, and why they're additive/inert today)"
        ]
    }))
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
/// (parity target: Acutis, Imandra, Kōdo), plus `get_nirdosha_constructs`
/// and `get_ui_conventions` (this project's own additions: a live
/// capability inventory and a curated UI/naming/annotation reference,
/// neither a parity-target tool). Every `inputSchema` is plain JSON
/// Schema, per spec.
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
            {
                "name": "get_nirdosha_constructs",
                "title": "Get Nirdosha's supported language constructs",
                "description": "Returns a live, compiler-verified inventory of Nirdosha's major language constructs (fn, struct, enum/match, validate contracts, workflow, transact, transact w/ txn_id, screen+serve, json display loops, identity+acquire) -- each one just run through the real nirdosha build pipeline against this compiler build. Every entry carries a worked source example and whether it currently compiles; a failing one also carries the real compiler diagnostic. Call this to discover what Nirdosha actually supports right now, instead of relying on documentation that can drift stale.",
                "inputSchema": { "type": "object", "properties": {} },
            },
            {
                "name": "get_ui_conventions",
                "title": "Get Nirdosha's UI naming conventions, fn annotations, and UI grammar",
                "description": "Returns a structured reference for generating a program that actually gets a web UI out of nirdosha emit-ui/serve: (1) the list_/get_/create_/update_/delete_<struct_snake_case> and stat_/chart_<name> function-naming conventions that determine whether a struct gets a screen at all, plus the silent failure modes when a name doesn't match; (2) every annotation a fn or struct field can carry (requires(public), requires(role:...), requires(claim:...,...), nfr(...), effect(...), audited \"...\" { ... }) -- what each does and whether it's typeck-only or compiled/enforced; (3) the screen/dashboard/landing/serve/workspace/layout UI DSL's real EBNF grammar and what each key changes in the generated UI. Call this before writing a program that's meant to render a UI, or when a struct isn't showing up in the generated nav.",
                "inputSchema": { "type": "object", "properties": {} },
            },
        ],
    })
}

/// `tools/call` -- dispatches `params.name` to one of the seven MCP
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
        "get_nirdosha_constructs" => get_nirdosha_constructs(arguments),
        "get_ui_conventions" => get_ui_conventions(arguments),
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
    fn tools_list_advertises_all_seven_tools() {
        let list = tools_list();
        let names: Vec<&str> = list["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["verify_code", "get_grammar", "fix", "describe", "certify_code", "get_nirdosha_constructs", "get_ui_conventions"]);
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

    fn commitment(fn_name: &str, nfr: crate::ast::NfrSpec) -> NfrCommitment {
        NfrCommitment { fn_name: fn_name.to_string(), nfr, evidence_tier: NfrCommitment::EVIDENCE_TIER.to_string() }
    }

    fn escalation(function: &str, nfr: &str, threshold: f64, actual: f64) -> NfrEscalation {
        NfrEscalation { function: function.to_string(), nfr: nfr.to_string(), threshold, actual, timestamp_ms: 0 }
    }

    #[test]
    fn nfr_drift_finds_a_real_crossing_for_a_declared_field() {
        let commitments = vec![commitment("slow_lookup", crate::ast::NfrSpec { latency_ms: Some(50), error_rate_max: None, throughput_min_per_sec: None, concurrency_max: None })];
        let escalations = vec![escalation("slow_lookup", "latency_ms", 50.0, 87.0)];
        let findings = nfr_drift(&commitments, &escalations);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].fn_name, "slow_lookup");
        assert_eq!(findings[0].nfr_kind, "latency_ms");
        assert_eq!(findings[0].declared_threshold, 50.0);
        assert_eq!(findings[0].worst_observed, 87.0);
        assert_eq!(findings[0].violation_count, 1);
    }

    #[test]
    fn nfr_drift_reports_the_worst_of_several_escalations() {
        let commitments = vec![commitment("hot_path", crate::ast::NfrSpec { latency_ms: Some(50), error_rate_max: None, throughput_min_per_sec: None, concurrency_max: None })];
        let escalations = vec![escalation("hot_path", "latency_ms", 50.0, 60.0), escalation("hot_path", "latency_ms", 50.0, 200.0), escalation("hot_path", "latency_ms", 50.0, 75.0)];
        let findings = nfr_drift(&commitments, &escalations);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].worst_observed, 200.0, "the highest actual latency, not the first or last escalation");
        assert_eq!(findings[0].violation_count, 3);
    }

    #[test]
    fn nfr_drift_uses_the_lowest_observed_for_a_throughput_minimum() {
        // throughput_min_per_sec is a "must stay over" bound -- the
        // *worst* real observation is the lowest one, the opposite of
        // every other kind's "must stay under."
        let commitments = vec![commitment("ingest", crate::ast::NfrSpec { latency_ms: None, error_rate_max: None, throughput_min_per_sec: Some(100), concurrency_max: None })];
        let escalations = vec![escalation("ingest", "throughput_min_per_sec", 100.0, 80.0), escalation("ingest", "throughput_min_per_sec", 100.0, 40.0)];
        let findings = nfr_drift(&commitments, &escalations);
        assert_eq!(findings[0].worst_observed, 40.0, "the lowest observed rate is the worst violation of a minimum");
    }

    #[test]
    fn nfr_drift_ignores_an_escalation_for_an_undeclared_nfr_kind_on_the_same_fn() {
        // `hot_path` only declares `latency_ms` -- an escalation naming
        // a *different* kind for the same function (nothing declared
        // it) must not be reported as drift against a commitment that
        // was never made.
        let commitments = vec![commitment("hot_path", crate::ast::NfrSpec { latency_ms: Some(50), error_rate_max: None, throughput_min_per_sec: None, concurrency_max: None })];
        let escalations = vec![escalation("hot_path", "concurrency_max", 10.0, 15.0)];
        assert!(nfr_drift(&commitments, &escalations).is_empty());
    }

    #[test]
    fn nfr_drift_ignores_an_escalation_for_a_different_function() {
        let commitments = vec![commitment("fn_a", crate::ast::NfrSpec { latency_ms: Some(50), error_rate_max: None, throughput_min_per_sec: None, concurrency_max: None })];
        let escalations = vec![escalation("fn_b", "latency_ms", 50.0, 200.0)];
        assert!(nfr_drift(&commitments, &escalations).is_empty());
    }

    #[test]
    fn nfr_drift_finds_every_declared_field_independently_on_a_multi_nfr_fn() {
        let commitments = vec![commitment(
            "busy_endpoint",
            crate::ast::NfrSpec { latency_ms: Some(50), error_rate_max: None, throughput_min_per_sec: None, concurrency_max: Some(10) },
        )];
        let escalations = vec![escalation("busy_endpoint", "latency_ms", 50.0, 90.0), escalation("busy_endpoint", "concurrency_max", 10.0, 14.0)];
        let findings = nfr_drift(&commitments, &escalations);
        assert_eq!(findings.len(), 2, "{findings:?}");
        let kinds: Vec<&str> = findings.iter().map(|f| f.nfr_kind.as_str()).collect();
        assert!(kinds.contains(&"latency_ms") && kinds.contains(&"concurrency_max"), "{kinds:?}");
    }

    #[test]
    fn nfr_drift_with_no_escalations_at_all_is_empty() {
        let commitments = vec![commitment("quiet_fn", crate::ast::NfrSpec { latency_ms: Some(50), error_rate_max: None, throughput_min_per_sec: None, concurrency_max: None })];
        assert!(nfr_drift(&commitments, &[]).is_empty());
    }
}