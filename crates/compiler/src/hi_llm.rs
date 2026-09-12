//! LLM client and network-touching orchestration for rfcs/0014's
//! prompt -> build -> generate -> publish pipeline. Kept as its own
//! module, apart from `hi_graph.rs` (the pure, offline-testable graph
//! store) and `hi_api.rs` (the transport-neutral route table that wires
//! this and the graph store together): this is the one place under the
//! `hi_*` surface that makes an outbound network call. Its activation
//! contract and self-repair-loop shape are ported, not reinvented, from
//! RFC 0012's now-deleted `hi.rs` console (see `main.rs::cmd_hi`'s own
//! doc comment for why that front end is gone) -- the same environment-
//! variable trio, the same bounded retry-against-a-real-diagnostic
//! discipline, just retargeted at populating/materializing graph nodes
//! instead of a single whole-program request.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::hi_graph::CandidateUnit;

const PROVIDER_KEY_VAR: &str = "NIRDOSHA_LLM_PROVIDER_KEY";
const PROVIDER_MODEL_VAR: &str = "NIRDOSHA_LLM_PROVIDER_MODEL";
const PROVIDER_BASE_VAR: &str = "NIRDOSHA_LLM_PROVIDER_BASE";
const DEFAULT_PROVIDER_BASE: &str = "https://api.openai.com/v1";

const PROVIDER_TIMEOUT_SECS_VAR: &str = "NIRDOSHA_LLM_PROVIDER_TIMEOUT_SECS";
const DEFAULT_PROVIDER_TIMEOUT_SECS: u64 = 300;

/// Real OpenAI is OpenAI-compatible by definition, so this fallback
/// needs no separate wire-format adapter. Checked only if the trio
/// above isn't set at all (not merged with it) -- a half-set trio is a
/// mistake to report precisely, not to silently patch over.
const OPENAI_KEY_VAR: &str = "OPENAI_API_KEY";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o-mini";

/// A resolved, ready-to-use provider configuration. No `#[derive(Debug)]`
/// -- a hand-written impl below redacts `api_key` unconditionally, so
/// `{:?}` on this struct can never become the accidental way a real key
/// leaks into an error message or a log line.
pub struct Activation {
    api_key: String,
    model: String,
    base_url: String,
    timeout_secs: u64,
}

impl Activation {
    fn redacted_key(&self) -> String {
        if self.api_key.len() <= 4 {
            "****".to_string()
        } else {
            format!("****{}", &self.api_key[self.api_key.len() - 4..])
        }
    }
}

impl std::fmt::Debug for Activation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Activation").field("api_key", &self.redacted_key()).field("model", &self.model).field("base_url", &self.base_url).finish()
    }
}

/// `env` is injected rather than calling `std::env::var` directly so
/// this whole activation contract is a pure function, testable without
/// touching real process env vars or a network call.
pub fn resolve_activation(env: &dyn Fn(&str) -> Option<String>) -> Result<Activation, String> {
    let timeout_secs = match env(PROVIDER_TIMEOUT_SECS_VAR) {
        Some(raw) => raw.parse::<u64>().map_err(|_| format!("{PROVIDER_TIMEOUT_SECS_VAR} is set to `{raw}`, which isn't a whole number of seconds"))?,
        None => DEFAULT_PROVIDER_TIMEOUT_SECS,
    };
    let key = env(PROVIDER_KEY_VAR);
    let model = env(PROVIDER_MODEL_VAR);
    match (key, model) {
        (Some(api_key), Some(model)) => Ok(Activation { api_key, model, base_url: env(PROVIDER_BASE_VAR).unwrap_or_else(|| DEFAULT_PROVIDER_BASE.to_string()), timeout_secs }),
        (Some(_), None) => Err(format!("{PROVIDER_KEY_VAR} is set but {PROVIDER_MODEL_VAR} is not -- both are required together")),
        (None, Some(_)) => Err(format!("{PROVIDER_MODEL_VAR} is set but {PROVIDER_KEY_VAR} is not -- both are required together")),
        (None, None) => match env(OPENAI_KEY_VAR) {
            Some(api_key) => Ok(Activation { api_key, model: DEFAULT_OPENAI_MODEL.to_string(), base_url: DEFAULT_PROVIDER_BASE.to_string(), timeout_secs }),
            None => Err(format!(
                "no LLM provider configured -- set either:\n  \
                 {PROVIDER_KEY_VAR} + {PROVIDER_MODEL_VAR} (optionally {PROVIDER_BASE_VAR}, default {DEFAULT_PROVIDER_BASE})\n\
                 or:\n  \
                 {OPENAI_KEY_VAR} (a real OpenAI key)"
            )),
        },
    }
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: String,
}

/// Every call returns `Result` -- a caller must degrade to an error
/// message, never crash the whole window/server process because one
/// request timed out or one response didn't parse.
pub struct LlmClient {
    http: reqwest::blocking::Client,
    activation: Activation,
}

impl LlmClient {
    pub fn new(activation: Activation) -> Self {
        let timeout = Duration::from_secs(activation.timeout_secs);
        LlmClient { http: reqwest::blocking::Client::builder().timeout(timeout).build().expect("building a blocking reqwest client with only a timeout set cannot fail"), activation }
    }

    fn complete(&self, history: &[ChatMessage]) -> Result<String, String> {
        let request = ChatCompletionRequest { model: self.activation.model.clone(), messages: history.iter().map(|m| ChatMessage { role: m.role, content: m.content.clone() }).collect(), temperature: 0.2 };
        let url = format!("{}/chat/completions", self.activation.base_url.trim_end_matches('/'));
        let response = self.http.post(&url).bearer_auth(&self.activation.api_key).json(&request).send().map_err(|e| {
            if e.is_timeout() {
                format!("the model didn't respond within {}s (timed out) -- ambitious requests to a reasoning model can need longer; raise {PROVIDER_TIMEOUT_SECS_VAR}", self.activation.timeout_secs)
            } else {
                format!("request to {url} failed: {e}")
            }
        })?;
        let status = response.status();
        let body = response.text().map_err(|e| format!("reading response body: {e}"))?;
        if !status.is_success() {
            return Err(format!("{url} returned {status}: {body}"));
        }
        let parsed: ChatCompletionResponse = serde_json::from_str(&body).map_err(|e| format!("parsing response JSON: {e} (body: {body})"))?;
        parsed.choices.into_iter().next().map(|c| c.message.content).ok_or_else(|| "response had no choices".to_string())
    }
}

/// The one system prompt every whole-program generation call sends --
/// `agent-skills/nirdosha/paste-anywhere-prompt.md` is already a
/// complete, maintained, self-contained guide for "write valid `.nir`
/// code."
const NIR_SYSTEM_PROMPT: &str = include_str!("../../../agent-skills/nirdosha/paste-anywhere-prompt.md");

const POPULATE_SYSTEM_PROMPT: &str = "You are populating a project knowledge graph from a user's natural-language prompt, for the Nirdosha programming language. \
Read the prompt and propose the set of top-level fn/struct/enum/screen units it implies. \
Reply with ONLY a JSON array (no prose, no markdown fence) of objects shaped exactly like: \
{\"kind\": \"fn\", \"name\": \"transfer_funds\", \"driving_text\": \"one or two sentences describing what this unit must do\", \"depends_on\": [\"other unit names this one calls or references\"]}. \
\"kind\" must be one of fn, struct, enum, screen. Functions and struct/screen fields are snake_case; struct/enum/screen names are PascalCase. \
Propose the smallest set of units that actually covers the request -- do not invent unrelated functionality, and do not include a Nirdosha prelude type (Option, Result, Money, HttpResponse, ...) as a candidate of your own.";

/// One `CodeUnit` candidate the LLM proposed while populating the graph
/// from a prompt (rfcs/0014's "1. Prompt mode") -- plain data, parsed
/// from the model's own JSON response. `hi_api.rs`'s `/api/prompt`
/// handler turns each of these into a real `hi_graph::add_candidate`/
/// `add_relation` call; this function itself never touches the graph.
#[derive(Deserialize)]
pub struct PromptCandidate {
    pub kind: String,
    pub name: String,
    pub driving_text: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

const CANDIDATE_KINDS: &[&str] = &["fn", "struct", "enum", "screen"];

/// Prompt mode's own LLM call (rfcs/0014's "1. Prompt mode"). **v1
/// scope cut, disclosed:** the RFC's own cheap-before-expensive funnel
/// (a deterministic Tier-0 regex/schema extraction pass grounding a
/// Tier-2 LLM pass for the residue, with benchmark-derived rejection
/// thresholds) isn't built -- this is Tier 2 alone, LLM-only, with no
/// ceiling/floor gating. Every candidate this produces still lands
/// `confirmed = 0` (`hi_graph::add_candidate`'s own default), so
/// nothing it proposes reaches Generate mode without a human
/// confirming it first -- the funnel's *purpose* (never trust raw LLM
/// output enough to compile it unreviewed) still holds even though its
/// specific two-tier mechanism doesn't exist yet.
pub fn populate_candidates(client: &LlmClient, prompt: &str) -> Result<Vec<PromptCandidate>, String> {
    let history = [ChatMessage { role: "system", content: POPULATE_SYSTEM_PROMPT.to_string() }, ChatMessage { role: "user", content: prompt.to_string() }];
    let raw = client.complete(&history)?;
    let json = extract_json_array(&raw);
    let candidates: Vec<PromptCandidate> = serde_json::from_str(&json).map_err(|e| format!("the model's response wasn't the expected JSON array of candidates: {e} (raw response: {raw})"))?;
    if candidates.is_empty() {
        return Err("the model proposed no candidates for this prompt".to_string());
    }
    for c in &candidates {
        if !CANDIDATE_KINDS.contains(&c.kind.as_str()) {
            return Err(format!("the model proposed an illegal kind `{}` for `{}` -- expected one of {CANDIDATE_KINDS:?}", c.kind, c.name));
        }
    }
    Ok(candidates)
}

const ANSWER_QUESTION_SYSTEM_PROMPT: &str = "You answer questions about a software project for the person building it. \
You are given a plain-text summary of the project's own components (name: description, one per line) -- use ONLY that summary, never outside knowledge about unrelated software. \
If the summary doesn't actually contain enough information to answer, say so plainly rather than guessing or inventing detail.";

/// The whole-project fallback `hi_api.rs`'s `/api/ask` route reaches
/// for once `hi_graph::ask`'s own local keyword search comes up empty
/// -- "what is this project about" matches no single `CodeUnit`'s name
/// or driving text, because it was never really about any one node.
/// **A deliberate, bounded exception to RFC 0014's "semantic search
/// stays opt-in, not a default" posture (Open Question 8), not a
/// silent violation of it:** that open question is about *embedding-
/// based* similarity search running proactively over every query; this
/// is a plain chat completion, triggered only as a fallback after local
/// search already found nothing, and only when an LLM is already
/// configured -- the same one `:prompt`/`:generate` already send
/// driving text to, so this adds no new category of external exposure,
/// just a second use of the same already-opted-into channel.
pub fn answer_question(client: &LlmClient, question: &str, project_context: &str) -> Result<String, String> {
    let context = if project_context.is_empty() { "(no code units in this project's graph yet)".to_string() } else { format!("Project summary (component: description):\n{project_context}") };
    let history = [ChatMessage { role: "system", content: ANSWER_QUESTION_SYSTEM_PROMPT.to_string() }, ChatMessage { role: "user", content: format!("{context}\nQuestion: {question}") }];
    client.complete(&history)
}

/// The model's response can (and often does) wrap the JSON array in
/// prose or a markdown fence despite being told not to -- find the
/// outermost `[...]` rather than requiring the response to be nothing
/// but JSON.
fn extract_json_array(raw: &str) -> String {
    let trimmed = raw.trim();
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        if end >= start {
            return trimmed[start..=end].to_string();
        }
    }
    trimmed.to_string()
}

/// Models routinely wrap code in a fenced block even when told not to
/// -- stripping it here, once, keeps every caller of `complete()` from
/// having to know about this.
fn extract_nir_source(raw: &str) -> String {
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

/// 4, not 3, since 2026-09-11: parse errors surface one at a time
/// (LL(1), first error only), so a program with N grammar slips needs
/// N repair rounds, and 3 total compile attempts left only 2. The
/// `self_repair_hint` arms below (knowledge injection per failure
/// class) are what actually fixes runs; this is headroom for the
/// serial-slip case that remains when the model's first draft carries
/// several gaps at once.
const MAX_SELF_REPAIR_ATTEMPTS: u32 = 4;

/// The pointed follow-up appended to a failed attempt's generic "fix
/// it" request, one arm per *diagnostic class this loop has actually
/// failed on in the field* -- not per error. A bounded retry loop's
/// real enemy is a diagnostic the model can read but can't act on:
/// each arm below exists because a real `:generate` run burned all
/// `MAX_SELF_REPAIR_ATTEMPTS` tries on exactly that (see each arm's
/// own comment), and each says what to *do*, not just what went wrong.
/// Every line number a diagnostic points at, extracted from the two
/// formats this pipeline actually emits: the parser's
/// `... at {line}:{col}: ...` (also `type error: ...` / `ownership
/// error: ...`, whose `Display` starts with `{line}:{col}:`). Capped
/// by the caller; duplicate lines reported once.
fn diagnostic_line_numbers(diagnostic: &str) -> Vec<usize> {
    let mut found: Vec<usize> = Vec::new();
    for line in diagnostic.lines() {
        let rest = line
            .rsplit_once(" at ")
            .map(|(_, r)| r)
            .or_else(|| line.split_once("type error: ").map(|(_, r)| r))
            .or_else(|| line.split_once("ownership error: ").map(|(_, r)| r));
        let Some(rest) = rest else { continue };
        let Some(digits) = rest.split(':').next() else { continue };
        if let Ok(n) = digits.trim().parse::<usize>() {
            if n >= 1 && !found.contains(&n) {
                found.push(n);
            }
        }
    }
    found
}

/// `{line}:{col}` out of the parser's own `... at {line}:{col}: ...`
/// error format, so parse-stage diagnostics can carry the same
/// structured fields type/ownership-stage errors get from their
/// `Span`s.
fn first_span_in(s: &str) -> Option<(usize, usize)> {
    let rest = s.rsplit_once(" at ").map(|(_, r)| r)?;
    let mut it = rest.split(':');
    let line = it.next()?.trim().parse::<usize>().ok()?;
    let col = it.next()?.trim().parse::<usize>().ok()?;
    Some((line, col))
}

fn machine_error(stage: &str, line: Option<usize>, col: Option<usize>, message: &str) -> String {
    serde_json::json!({ "stage": stage, "line": line, "col": col, "message": message }).to_string()
}

// ===========================================================================
// RFC 0016 Phase 1: the contract coverage gate.
//
// The demonstrated seam (2026-09-11, the run that opened the RFC): decompose
// proposes what should be proven, generate can drop it, publish doesn't
// notice -- a clean-compiling banking program shipped with zero `validate`
// contracts and `nirdosha verify` reported `PROVED 0/0`, which is silence,
// not safety. This gate turns "the units demanded a contract" from prompt
// decoration into a publish-blocking check, exactly the way
// `typecheck_and_build_check` already turns type rules into one.

/// Why a coverage check failed -- the class drives the repair loop's budget
/// discipline (RFC 0016's VIOLATED/ENGINE_LIMIT split):
/// - everything except `EngineLimit` is the model's to fix and consumes the
///   repair budget, each class with its own `self_repair_hint` teaching;
/// - `EngineLimit` is *nobody's* fault, consumes no budget, gets exactly one
///   off-budget simplification attempt, and then escalates to the operator
///   with the proof obligation attached instead of blaming the draft.
#[derive(Debug, Clone, PartialEq)]
pub enum CoverageFailureClass {
    /// A confirmed fn unit demanded a `validate` contract; the draft's fn
    /// carries none (or the fn itself is absent).
    ContractDropped,
    /// A demanded contract exists but the fn genuinely breaks it (or the
    /// contract itself is malformed: unbound identifier, no such fn,
    /// predicate parse error) -- the model's to fix.
    ContractViolated,
    /// A demanded contract's `pre:` is unsatisfiable -- it passes vacuously
    /// (Phase 0's `VacuousPrecondition`, surfaced here per-unit).
    VacuousContract,
    /// The proof engine's deterministic fuel ran out (Phase 0's
    /// `EngineLimit`) -- an engine limit, never a code bug.
    EngineLimit,
    /// A demanded contract uses a shape the Tier-1 walker can't model
    /// (`Unsupported`) -- a demanded contract is not optional, so "can't
    /// decide" fails the gate and the hint teaches the provable subset.
    ContractUnprovable,
}

/// One coverage-gate failure, ready for the repair conversation: the class
/// (for budget discipline) and the full diagnostic (prose + machine-readable
/// errors, the 2026-09-11 format), attributed to the unit that demanded the
/// contract. Phase 2 packs will add pack-ID attribution on the same struct.
#[derive(Debug, Clone)]
pub struct CoverageFailure {
    pub class: CoverageFailureClass,
    pub diagnostic: String,
}

/// Phase 1's demand convention: an attribute line beginning with
/// `validate contract` (optionally with a `:` after) or `contract:` is a
/// **proof demand** -- the attribute prose says what must hold; the gate
/// requires the named unit's fn to carry a `validate` block that Z3
/// actually PROVES. Returns the demand text (everything after the marker,
/// trimmed) for attribution. Deliberately a fixed, documented convention
/// rather than fuzzy matching: Phase 2's pack manifests carry structured
/// demands, and this form is what `:attach` writes when a human states the
/// law by hand (`validate contract balance_nonnegative: ...`).
pub fn demanded_contract(attr_line: &str) -> Option<&str> {
    let t = attr_line.trim();
    let rest = t
        .strip_prefix("validate contract")
        .or_else(|| t.strip_prefix("contract:"))
        .map(|r| r.trim_start_matches([':', ' ']).trim());
    match rest {
        Some(r) if !r.is_empty() => Some(r),
        _ => None,
    }
}

/// The coverage gate itself: parse `source`, then require that (1) every
/// confirmed fn unit whose attributes carry a proof demand actually has a
/// `validate` block on its fn in the draft, and (2) every demanded fn's
/// contract PROVES -- no counterexample, non-vacuous, within the engine's
/// deterministic fuel, inside the provable subset. Parses its own copy of
/// the source because the generate loop only holds a `&str` (the parse is
/// cheap next to the LLM call that produced it); publish's re-check calls
/// [`contract_coverage_check_program`] with its already-loaded `Program`.
/// Programs from graphs where no unit demands anything pass untouched --
/// every existing project is unaffected until someone states a law.
pub fn contract_coverage_check(source: &str, units: &[crate::hi_graph::CandidateUnit]) -> Result<(), CoverageFailure> {
    let toks = crate::token::Lexer::new(source).tokenize();
    let toks = match toks {
        Ok(t) => t,
        Err(e) => {
            return Err(CoverageFailure {
                class: CoverageFailureClass::ContractViolated,
                diagnostic: format!("contract coverage failure: the source no longer lexes, so demanded contracts cannot be checked: {e:?}"),
            })
        }
    };
    let program = match crate::parser::Parser::new(toks).parse_program() {
        Ok(p) => p,
        Err(e) => {
            return Err(CoverageFailure {
                class: CoverageFailureClass::ContractViolated,
                diagnostic: format!("contract coverage failure: the source no longer parses, so demanded contracts cannot be checked: {e:?}"),
            })
        }
    };
    contract_coverage_check_program(&program, units)
}

/// [`contract_coverage_check`] against an already-parsed program -- what
/// `handle_publish`'s re-check calls with its own loaded `Program`, so the
/// gate runs identically at generate time (where the model can repair) and
/// at publish time (where a hand-edited file can't sneak past).
pub fn contract_coverage_check_program(program: &crate::ast::Program, units: &[crate::hi_graph::CandidateUnit]) -> Result<(), CoverageFailure> {
    // (fn_name, demand text) for every confirmed fn unit carrying a proof
    // demand. Only fn units can demand: a `validate` block targets a fn.
    let mut demanded_fns: Vec<(String, String)> = Vec::new();
    for u in units {
        if u.kind != "fn" {
            continue;
        }
        for attr in &u.attributes {
            for line in attr.lines() {
                if let Some(demand) = demanded_contract(line) {
                    demanded_fns.push((u.name.clone(), demand.to_string()));
                }
            }
        }
    }
    if demanded_fns.is_empty() {
        return Ok(());
    }
    let demanded_names: Vec<&str> = demanded_fns.iter().map(|(n, _)| n.as_str()).collect();

    let mut failures: Vec<(CoverageFailureClass, String, Option<(usize, usize)>)> = Vec::new();

    // (1) Presence: each demanded fn must exist AND carry a validate block.
    let mut reported_dropped_fns: Vec<&str> = Vec::new();
    for (fn_name, demand) in &demanded_fns {
        match program.fns.iter().find(|f| &f.name == fn_name) {
            None => {
                if !reported_dropped_fns.contains(&fn_name.as_str()) {
                    reported_dropped_fns.push(fn_name);
                    failures.push((
                        CoverageFailureClass::ContractDropped,
                        format!("the confirmed unit `{fn_name}` (demand: `{demand}`) demands a proving `validate` block, but the draft has no fn `{fn_name}` at all -- the unit itself was dropped"),
                        None,
                    ));
                }
            }
            Some(f) => {
                if !program.validates.iter().any(|v| &v.fn_name == fn_name) && !reported_dropped_fns.contains(&fn_name.as_str()) {
                    reported_dropped_fns.push(fn_name);
                    failures.push((
                        CoverageFailureClass::ContractDropped,
                        format!("the confirmed unit `{fn_name}` (demand: `{demand}`) demands a proving `validate` block, but the draft's fn `{fn_name}` carries none -- write `validate {fn_name} {{ pre: ... post: ... }}`; it must PROVE, not merely parse"),
                        Some((f.span.line, f.span.col)),
                    ));
                }
            }
        }
    }

    // (2) Proof: every demanded fn's validate outcomes must hold. This runs
    // even for fns whose block was just reported missing -- a missing block
    // has no outcomes, so the loop below simply finds nothing for it.
    // NOTE on `Unsupported`: `contract_error_message` deliberately returns
    // `None` for it (check_program_contracts' long-standing "never an error
    // there" policy), but a DEMANDED contract is not optional -- the gate
    // maps Unsupported to `ContractUnprovable` with its own message instead
    // of skipping it, which is why that arm is handled here and not via the
    // shared helper.
    let outcomes = crate::contract_check::run_program_validates(program);
    for outcome in &outcomes {
        if !demanded_names.contains(&outcome.fn_name.as_str()) {
            continue; // a present-but-undemanded contract is verify's report, not the gate's scope
        }
        let (class, message): (CoverageFailureClass, String) = match &outcome.result {
            crate::contract_check::ContractCheckResult::Proved => continue,
            crate::contract_check::ContractCheckResult::Unsupported(msg) => {
                (CoverageFailureClass::ContractUnprovable, format!("the proof engine can't model this contract's shape: {msg}"))
            }
            crate::contract_check::ContractCheckResult::VacuousPrecondition => (CoverageFailureClass::VacuousContract, crate::contract_check::contract_error_message(outcome).expect("VacuousPrecondition always carries a message")),
            crate::contract_check::ContractCheckResult::EngineLimit { .. } => (CoverageFailureClass::EngineLimit, crate::contract_check::contract_error_message(outcome).expect("EngineLimit always carries a message")),
            crate::contract_check::ContractCheckResult::Counterexample { .. }
            | crate::contract_check::ContractCheckResult::UnboundIdentifier { .. }
            | crate::contract_check::ContractCheckResult::NoSuchFunction(_)
            | crate::contract_check::ContractCheckResult::PredicateParseError(_) => (CoverageFailureClass::ContractViolated, crate::contract_check::contract_error_message(outcome).expect("failure classes always carry a message")),
        };
        let span = program
            .validates
            .iter()
            .find(|v| v.fn_name == outcome.fn_name)
            .map(|v| (v.span.line, v.span.col));
        let mut line = format!("the confirmed unit `{}` demands a `validate` contract that proves; its contract failed: {}", outcome.fn_name, message);
        if class == CoverageFailureClass::ContractUnprovable {
            line.push_str(" -- a demanded contract is not optional: rewrite it in the provable subset (integer-only params/result, linear arithmetic, no loops/calls)");
        }
        failures.push((class, line, span));
    }

    if failures.is_empty() {
        return Ok(());
    }
    let machine: Vec<String> = failures
        .iter()
        .map(|(_, message, span)| machine_error("coverage", span.map(|s| s.0), span.map(|s| s.1), message))
        .collect();
    // EngineLimit dominates the run's class (RFC 0016): when the engine
    // couldn't decide *any* demanded contract, the model must not be charged
    // budget for work it cannot influence -- even if other failures are
    // also present, the escalation message carries the full list.
    let class = if failures.iter().any(|(c, _, _)| *c == CoverageFailureClass::EngineLimit) {
        CoverageFailureClass::EngineLimit
    } else {
        failures[0].0.clone()
    };
    Err(CoverageFailure {
        class,
        diagnostic: format!(
            "contract coverage failure: {}\nmachine-readable errors: [{}]",
            failures.iter().map(|(_, m, _)| m.as_str()).collect::<Vec<_>>().join(" "),
            machine.join(", ")
        ),
    })
}

/// How one failed attempt charges the repair budget (RFC 0016 Phase 1's
/// VIOLATED/ENGINE_LIMIT split, extracted pure so the discipline itself is
/// unit-tested without an LLM client):
/// - a VIOLATED-class failure (anything the model can fix, including every
///   compile failure) consumes one of `MAX_SELF_REPAIR_ATTEMPTS`;
/// - an ENGINE_LIMIT failure consumes none, but only ONE off-budget
///   simplification attempt is allowed -- a second engine limit escalates
///   to the operator instead of looping on work no code edit can fix.
#[derive(Debug, PartialEq)]
enum BudgetCharge {
    /// Push a repair turn and continue.
    Continue,
    /// Budget exhausted -- give up with the last diagnostic.
    StopGiveUp,
    /// Second engine limit -- escalate to the operator with the obligation.
    StopEscalate,
}
fn charge_budget(budget: &mut u32, engine_limit_simplifications: &mut u32, class: CoverageFailureClass) -> BudgetCharge {
    if class == CoverageFailureClass::EngineLimit {
        *engine_limit_simplifications += 1;
        if *engine_limit_simplifications > 1 {
            BudgetCharge::StopEscalate
        } else {
            BudgetCharge::Continue // off-budget: the model gets one simplification
        }
    } else {
        if *budget == 0 {
            BudgetCharge::StopGiveUp
        } else {
            *budget -= 1;
            if *budget == 0 { BudgetCharge::StopGiveUp } else { BudgetCharge::Continue }
        }
    }
}

/// Appends the offending source line(s) to a diagnostic so the model
/// sees WHAT it wrote at the position, not just where. A model cannot
/// reliably count lines of its own previous output -- especially after
/// a repair edit shifted everything below the edit -- so a bare
/// `at 154:19` asks it to find the line by arithmetic it does badly;
/// quoting the line removes that whole failure mode. The check owns
/// the exact source text (`typecheck_and_build_check`'s own input),
/// so this costs one vector lookup, not a re-read.
fn attach_source_lines(source: &str, diagnostic: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = diagnostic.to_string();
    for n in diagnostic_line_numbers(diagnostic).into_iter().take(3) {
        if let Some(text) = lines.get(n - 1) {
            out.push_str(&format!("\n  the source line that points at (line {n}) is: `{text}`"));
        }
    }
    out
}

fn self_repair_hint(diagnostic: &str) -> &'static str {
    if diagnostic.contains("contract coverage failure") {
        // RFC 0016 Phase 1: the coverage gate's own classes. Sub-dispatched
        // on one outer marker so no compile diagnostic can misfire these
        // arms, and ordered engine-limit-first because a combined
        // diagnostic mentioning a fuel exhaustion must never be answered
        // with "fix your code" -- that is exactly the budget-split mistake
        // the RFC exists to prevent.
        if diagnostic.contains("engine limit") {
            " This is an engine limit, not a code bug: the demanded contract is too hard for the solver's deterministic fuel. Simplify the arithmetic into provable form -- linearize the fee (bound it with `pre:` instead of a nested min/max), split a tiered rule into one provable branch per tier -- while keeping the contract's MEANING; never loosen it to pass. If it cannot be simplified, the run will escalate to the operator rather than charge you for it."
        } else if diagnostic.contains("vacuously") {
            " The demanded contract's `pre:` can never be true for any input the parameter types admit -- an impossible range is almost always a typo. Fix the precondition to the fn's real domain, or fix the code if the domain was genuinely meant to be that narrow."
        } else if diagnostic.contains("carries none") || diagnostic.contains("no fn") {
            " Write the missing contract as a separate top-level block: `validate <fn> { pre: <param assumptions> post: <result property> }` -- integer params/result only, linear arithmetic (`+`, `-`, comparisons), no loops or calls in the predicate. It must PROVE under Z3, not merely parse."
        } else if diagnostic.contains("provable subset") {
            " Rewrite the demanded contract in the provable subset: integer-only parameters and return, linear arithmetic, no loops, no function calls, no floats -- the strongest true contract that subset can state. A demanded contract is mandatory, so `UNSUPPORTED` from the walker is a rewrite instruction, not a pass."
        } else {
            " The fn genuinely breaks the demanded contract at the counterexample input shown -- fix the code, or fix the contract if it misstates the unit's demand; never loosen a predicate just to make it pass."
        }
    } else if diagnostic.contains("no `fn main()` found") {
        // Defense in depth on top of `units_prompt`'s own fix, not a
        // substitute for it: a model that still drops `fn main()`
        // despite the now-explicit instruction gets one more, pointed
        // reminder rather than a generic "fix it" that repeats
        // whatever ambiguity caused this in the first place.
        " Add a `fn main()` -- it is required in addition to every named component from the original request, not instead of any of them."
    } else if diagnostic.contains("is a builtin name and cannot be used as a function name") {
        // Field-failure from the v4 generate attempt under the banking
        // pack: the model declared its own `fn check_role` and
        // `fn acquire_role`, which collide with runtime builtins.
        " Delete the custom `fn check_role(...)` / `fn acquire(...)` / any function whose name matches a builtin. Identity and role handling are runtime builtins: the only identity type is `VerifiedIdentity`; roles are plain strings like \"finance_director\"; call `check_role(identity, \"role\")` and unwrap its `Result(RoleView, str)`, then pass the `RoleView` to `acquire fn_name(proof)` to get a callable. Never invent a `User` or `UserRole` type."
    } else if diagnostic.contains("expected `VerifiedIdentity`, found `User`") || diagnostic.contains("expected `VerifiedIdentity`, found `UserRole`") {
        " The identity type is `VerifiedIdentity`, not a `User` struct or `UserRole` enum you invent. Roles are plain strings (\"requester\", \"finance_director\", \"admin\"). Pass a `VerifiedIdentity` value (built with `VerifiedIdentity(subject, issuer, audience, iat, exp, roles_csv)`) to every role-gated fn."
    } else if diagnostic.contains("`transact`'s `network` slot must pass the implicit `txn_id`") || diagnostic.contains("unknown variable `txn_id`") {
        " In a `transact` block, `txn_id` is an implicit binding provided by the desugaring. Every function used in `network:`, `verify:`, `commit:`, `compensate:`, or `log:` must accept `txn_id: str` as one of its parameters and actually use it in the call: `network: call_processor(txn_id, amount)`, `commit: commit_payment(txn_id, network)`, etc. The block itself does not declare `txn_id`."
    } else if diagnostic.contains("unknown variable `commit`") && diagnostic.contains("transact") {
        " Inside a `transact` block, `commit` is a step NAME, not a variable you can read directly. If you need the commit result in a later step (like `log:`), the step itself must return the value and the later step must call a function that receives it -- or use `verify` as the boolean guard and keep the committed amount as the step's return value."
    } else if diagnostic.contains("non-scalar element type") && diagnostic.contains("Vector") {
        // Field-failure from the v4 generate attempt: the model used
        // `Vector(PaymentRequest, 1)` as a variable-length list of
        // structs, which codegen cannot represent and used to panic on.
        " `Vector(T, N)` and `Matrix(T, R, C)` only support scalar element types (i64, f64, etc.) in the compiled backend. A variable-length list of structs, enums, or other aggregates must be `json`: build it with `json_set_str`/`json_set_i64` or return it from `db_query`, then walk it with `json_array_len` + `json_array_get`. Never use `Vector(StructName, N)` for a resizable queue."
    } else if diagnostic.contains("expected an expression, found the reserved keyword `return`") {
        // Field-failure 2026-09-11, and the one that forced the
        // holistic re-read: the model wrote `Ok(id) => return false`
        // inside a match arm. Rule 9 already forbade exactly this in
        // the system prompt, the model violated it anyway, and a bare
        // "fix it" retry re-taught nothing -- the proof that prompt-
        // resident rules are not retained under generation pressure
        // and the repair message is the only guaranteed-fresh
        // attention channel. This arm must sit ABOVE the generic
        // reserved-keyword arm, whose "rename that identifier"
        // advice would be nonsense for `return`.
        " `return` is a statement, never an expression: it cannot appear inside a `match` arm (`Ok(x) => return ...`), on the right of `=`, or inside a call's arguments. Restructure: every arm yields a value, bind the whole match (`let ok: bool = match ... { ... }`), then `return ok` (or print it) after the match ends."
    } else if diagnostic.contains("arms must name a variant") || diagnostic.contains("doesn't cover") {
        // Field-failure 2026-09-11 (the v4 attempt-4 give-up): the model
        // matched an enum-typed field with Rust-flavored arms -- string
        // literals (`\"approved\" => ...`) and `_ => 0` -- while rule 8
        // already forbade both. Same class as the `=> return` failure:
        // a rule that lives in the system prompt is not retained under
        // generation pressure; the repair turn is the teaching moment.
        // Both diagnostics are unusually self-describing (the second
        // literally lists the missing variants) -- this arm converts
        // that into the exact rewrite shape.
        " Enum matches: every arm names a bare VARIANT of the enum (`Pending => ...`, `Approved => ...`) -- never a string/number literal, never `_`. The `doesn't cover` errors list the exact variants missing: give each of them its own arm. If you meant to match string values, the SCRUTINEE is the bug, not the arms: the field/variable is enum-typed (`RequestStatus`), so either the arms must become variant names or the field's declared type must change -- you cannot string-match an enum."
    } else if diagnostic.contains("found the reserved type name") {
        // Field-failure 2026-09-11: `return unit`. Notably the EXISTING
        // "reserved keyword" arm never fired -- this message says
        // "reserved TYPE name" -- so even the one-arm-per-class idea
        // had a gap; the class matcher must match the class's actual
        // message text.
        " A type name (`unit`, `json`, `i64`, `db`, ...) can never appear where an expression is expected. `unit` has no literal at all: a `-> unit` function simply ends after its last statement, and an early exit returns a unit-typed CALL, e.g. `return print(\"done\")`. To get a `json`/`db` value, call the builtin that produces one (`json_parse(...)`, `db_connect(...)`), never write the type name."
    } else if diagnostic.contains("expected a type, found") {
        // Field-failure 2026-09-11: `Result((i64, str), E)` -- a tuple
        // nested inside a generic, so the rule-5 ban the model did
        // read didn't register as covering this shape.
        " There are no tuple types, including nested inside `Result`/`Option`: `Result((i64, str), E)` is a parse error exactly like a bare `(i64, str)`. To return more than one value, declare a `struct` to carry the fields, or return `json`. If the diagnostic names a reserved word instead, it was used as a type -- types are only the names in the Types table."
    } else if diagnostic.contains("doesn't support `print` on") {
        // Field-failure 2026-09-11: `print(some_json)`/`print(vector)`
        // -- the model treated print as a debug-print of anything.
        " `print` takes scalars only: `i64`/`f64`/`str`/`bool`/`unit`, any number of them -- never `json`, `Vector`/`Matrix`, `struct`, `enum`, or `Result` (and no stringify builtin exists). To display an aggregate, extract scalars first: `json_get_str(j, \"field\")` / `json_get_i64`, and for arrays `json_array_len(rows)` + `json_array_get(rows, i)` inside a `while` loop -- every json accessor returns a `Result`, always `match` it. A `struct` prints field by field; an `enum` matches to its variants; a `Result` is matched before anything inside it is shown."
    } else if diagnostic.contains("unexpected character `;`") {
        // Field-failure 2026-09-11: found it in the transact recipe of
        // the teaching prompt itself (fixed there too) -- models
        // coming from C-family languages add separators by reflex.
        " Nirdosha has no statement separators at all: delete the `;` and put each statement on its own line."
    } else if diagnostic.contains("codegen doesn't support `workflow") {
        // Field-failure class from the first v4 attempt: a workflow
        // written with a non-empty `data { ... }` block, which parses
        // fine and is rejected only at codegen -- the exact
        // superset-vs-subset trap a grammar can't see.
        " The compiled backend rejects a workflow's `data { ... }` block unless it is EMPTY: write `data {}` and keep anything instances must remember in your own struct keyed by `instance_id`. `on_entry` actions may only use `instance_id` and fn calls."
    } else if diagnostic.contains("found the reserved keyword") {
        // Root-caused from a real give-up: the model named a struct
        // field `state`, the parser answered `expected identifier,
        // found State` -- `Tok`'s derived-Debug variant name, not the
        // source text -- and the model, whose own types were
        // `GameState`-shaped, had no way to tell that "State" meant
        // the lowercase keyword it had written, so every retry
        // "fixed" something else until the loop gave up. The parser
        // now names the exact reserved word itself (`token.rs`'s
        // `Display for Tok`), which makes the diagnostic readable;
        // this arm makes the *action* explicit too: rename it, because
        // no escaping mechanism exists -- a model that doesn't know
        // that might just re-spell or case-shift the same word and
        // hand the next retry the identical failure.
        " Rename that identifier: a reserved keyword can never be a variable, field, parameter, or function name in Nirdosha (there is no quoting/escaping mechanism), so pick a different word -- `state` -> `app_state`, `open` -> `open_order`, and so on."
    } else {
        ""
    }
}

pub fn units_prompt(units: &[CandidateUnit], edges: &[crate::hi_graph::ConfirmedEdge]) -> String {
    // A real generation failure, root-caused rather than guessed at:
    // `populate_candidates`'s own system prompt asks for the
    // *conceptual* components a description implies (for a game:
    // `tick`, `move_piece`, ...) -- `main` is never one of those, so it
    // never lands in the confirmed set this function builds a prompt
    // from. The old wording here ("containing exactly the following
    // components") then read, correctly, as an instruction to leave
    // `main` out entirely -- and because that instruction sits at the
    // *start* of the conversation, every later self-repair retry
    // ("fix it and reply with the corrected source") inherited the same
    // conflict without ever being told it no longer applies, so the
    // model kept regenerating the same main()-less shape across all
    // `MAX_SELF_REPAIR_ATTEMPTS` tries. Naming `fn main()` explicitly
    // here, once, up front, fixes every retry at the source instead of
    // patching each one around a standing contradiction.
    let mut out = String::from(
        "Implement a single Nirdosha (.nir) program. It must contain exactly the following named components, each using its own exact name below for the corresponding fn/struct/enum/screen declaration -- and it must ALSO contain a `fn main()`, even though `main` is not itself one of the named components: Nirdosha requires exactly one entry point to compile and run at all. `fn main()` should be a real body that wires the components below together and exercises the behavior they describe, not an empty stub.\n\n",
    );
    for u in units {
        out.push_str(&format!("### {} {}\n{}\n", u.kind, u.name, u.driving_text));
        for attr in &u.attributes {
            if let Some(demand) = attr.lines().find_map(demanded_contract) {
                // RFC 0016 Phase 1: a proof demand is not decoration. The
                // coverage gate refuses any draft whose fn lacks a proving
                // `validate` block, so say so at the only point the model
                // reads the demand -- first-try compliance beats repair
                // turns every time.
                out.push_str(&format!("- PROOF DEMAND, not optional -- `validate` contract: {demand}: the generated fn `{}` MUST carry a separate top-level `validate {}` block whose `pre:`/`post:` Z3 actually PROVES; Generate refuses the program otherwise\n", u.name, u.name));
            } else {
                out.push_str(&format!("- attribute to attach: {attr}\n"));
            }
        }
        out.push('\n');
    }
    // The translation contract + the design's own relationships. Until
    // 2026-09-11 this prompt carried only the flattened node list: the
    // decompose step had already recorded `depends_on` relationships in
    // the graph, Generate mode just never read them back out, so the
    // code model invented wiring from nothing -- and a component the
    // design never wired became decorative (declared, never referenced:
    // the v2 `PaymentChannel` enum, zero edges, zero call sites). Edges
    // + an explicit what-an-edge-means contract close that gap at the
    // only point where a model reads them.
    if !edges.is_empty() {
        out.push_str("### How the components relate (confirmed design facts, not suggestions)\n");
        out.push_str("Each edge below is a confirmed relationship from the design graph. It translates to .nir concretely: the source component's declaration must genuinely reference the target -- a parameter of its type, a call, or a variant in a match -- and `fn main()` must wire executed calls so the relationship appears in real running code, not in a comment.\n\n");
        for e in edges {
            out.push_str(&format!("- {} {} {}\n", e.src, e.kind.to_lowercase(), e.dst));
        }
        out.push('\n');
    }
    out.push_str("A component that no other component references and that `fn main()` never calls or mentions is a bug in the generated program: it orphans the design. Every declared component must appear in at least one function's signature, or in a call from `fn main()` -- an enum in a parameter/return type counts only if some confirmed fn actually uses that type.\n");
    out.push_str(&crate::hi_plugin::plugin_law_prompt(units));
    out
}

/// Where Generate mode writes the program it materializes from every
/// confirmed candidate -- one stable path, not a PID-scoped temp file,
/// so a later `hi sync`/Publish mode call can find it again.
/// **v1 scope cut, disclosed:** this is one file for the whole
/// project's confirmed candidates, regenerated in full on every
/// Generate pass -- RFC 0014 asks for per-unit files/granularity;
/// composing a correct multi-file program that way is real, unbuilt
/// compiler work this slice doesn't attempt (see `generate_program`'s
/// own doc comment).
pub fn generated_source_path(root: &Path) -> PathBuf {
    crate::hi_graph::hi_dir(root).join("generated").join("hi_build.nir")
}

/// Generate mode's own bounded NL -> `.nir` -> build round trip
/// (rfcs/0014's "Generate mode"), reusing RFC 0012's exact self-repair
/// discipline (`MAX_SELF_REPAIR_ATTEMPTS` retries against the real
/// compiler diagnostic) -- just over a prompt built from every confirmed
/// candidate's driving text/attributes together, not one free-form
/// whole-program request. On success, writes the final source to
/// `generated_source_path(root)` and returns that path; the stable path
/// is only ever overwritten by an attempt that actually typechecks,
/// ownership-checks, and builds -- a failed attempt never clobbers the
/// last good generated source. `on_log` reports intermediate retries
/// the same way RFC 0012's own `LogEvent` did for the now-deleted
/// console.
///
/// **v1 scope cut, disclosed:** RFC 0014 asks for each unit to be
/// generated and typechecked individually against its neighbors'
/// *current* `.nir` (real per-unit composition). Building that would
/// need per-unit codegen the compiler doesn't have; this generates
/// every confirmed unit as one whole program in one shot instead --
/// still a real compile with real self-repair, just file-granularity
/// locking (`hi_graph::lock_units_after_sync`) rather than the RFC's
/// finer per-unit one.
pub fn generate_program(root: &Path, client: &LlmClient, units: &[CandidateUnit], edges: &[crate::hi_graph::ConfirmedEdge], on_log: &mut dyn FnMut(&str)) -> Result<PathBuf, String> {
    if units.is_empty() {
        return Err("nothing confirmed and unlocked to generate -- `:confirm <node>` at least one candidate first".to_string());
    }
    let mut history = vec![ChatMessage { role: "system", content: NIR_SYSTEM_PROMPT.to_string() }, ChatMessage { role: "user", content: units_prompt(units, edges) }];
    let mut last_diagnostic = String::new();
    // RFC 0016 Phase 1's budget discipline: `violation_budget` counts only
    // failures the model can fix (compile errors, dropped/violated/vacuous/
    // unprovable contracts). An ENGINE_LIMIT failure charges nothing and gets
    // exactly one off-budget simplification attempt (charge_budget) -- a
    // second one escalates to the operator with the proof obligation, since
    // no code edit can fix a solver fuel limit and four retries would just
    // produce a correct-but-ungeneratable program with no lever.
    let mut violation_budget = MAX_SELF_REPAIR_ATTEMPTS;
    let mut engine_limit_simplifications = 0u32;
    let mut attempt = 0usize;
    while violation_budget > 0 {
        attempt += 1;
        let raw = client.complete(&history).map_err(|e| format!("couldn't reach the model: {e}"))?;
        let source = extract_nir_source(&raw);
        // RFC 0016 Phase 2: 5a pack injection.  If a demanded fn is
        // present but the model forgot its contract, the pack's sealed
        // template is appended.  A signature mismatch is a coverage
        // failure (the model violated the domain law's exact shape).
        let source = match crate::hi_plugin::inject_pack_validates_into_source(root, &source) {
            Ok(s) => s,
            Err(failure) => {
                let diagnostic = failure.diagnostic;
                last_diagnostic = diagnostic.clone();
                let class = failure.class;
                match charge_budget(
                    &mut violation_budget,
                    &mut engine_limit_simplifications,
                    class.clone(),
                ) {
                    BudgetCharge::Continue => {
                        history.push(ChatMessage { role: "assistant", content: source });
                        history.push(ChatMessage { role: "user", content: format!("That attempt failed with this diagnostic:\n{diagnostic}\nFix it and reply with the corrected, complete `.nir` source only.{}", self_repair_hint(&diagnostic)) });
                    }
                    BudgetCharge::StopGiveUp => break,
                    BudgetCharge::StopEscalate => {
                        return Err(format!("escalated to the operator (RFC 0016): the proof engine's deterministic fuel ran out on a demanded contract -- an engine limit, NOT a code bug. The model's one off-budget simplification attempt did not clear it. Proof obligation, verbatim:\n{last_diagnostic}\nOperator options: state a weaker-but-provable demand on the unit, raise the fuel (`nirdosha::contract_check::set_proof_fuel_rlimit`), or waive the demand (`:waive`) and re-generate."));
                    }
                }
                continue;
            }
        };
        // Every draft is persisted BEFORE the check runs (2026-09-11,
        // born from the user's "take out the first generated code"
        // instruction): until now a failed attempt existed only in
        // conversation memory -- the check's scratch file is deleted
        // per call -- so "what did attempt 1 actually look like" was
        // unanswerable after the fact. Drafts land under
        // .nir/generated/attempts/, one file per attempt, overwritten
        // by the next generate run; the give-up error names the
        // directory. Persisting first, checking second also means a
        // crash mid-check still leaves the draft for inspection.
        let drafts_dir = generated_source_path(root)
            .parent()
            .expect("generated_source_path always has a parent")
            .join("attempts");
        if let Err(e) = std::fs::create_dir_all(&drafts_dir).and_then(|()| std::fs::write(drafts_dir.join(format!("attempt_{attempt}.nir")), &source)) {
            on_log(&format!("warning: could not persist attempt {attempt}'s draft: {e}"));
        }
        // The gate order is load-bearing: build checks first (a draft that
        // doesn't compile has no meaningful contracts to check), then the
        // coverage gate on the compiled draft (RFC 0016 Phase 1) -- a
        // program that typechecks but proves nothing about its money math
        // no longer passes, which is the demonstrated seam this closes.
        let outcome: Result<(), (String, Option<CoverageFailureClass>)> = match typecheck_and_build_check(&source) {
            Err(diagnostic) => Err((diagnostic, None)),
            Ok(()) => match contract_coverage_check(&source, units) {
                Err(failure) => Err((failure.diagnostic, Some(failure.class))),
                Ok(()) => Ok(()),
            },
        };
        match outcome {
            Ok(()) => {
                let out_path = generated_source_path(root);
                std::fs::create_dir_all(out_path.parent().expect("generated_source_path always has a parent")).map_err(|e| format!("creating {}: {e}", out_path.display()))?;
                std::fs::write(&out_path, &source).map_err(|e| format!("writing {}: {e}", out_path.display()))?;
                on_log(&format!("wrote {} (attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS})", out_path.display()));
                return Ok(out_path);
            }
            Err((diagnostic, class)) => {
                last_diagnostic = diagnostic.clone();
                let class = class.unwrap_or(CoverageFailureClass::ContractViolated);
                match charge_budget(&mut violation_budget, &mut engine_limit_simplifications, class.clone()) {
                    BudgetCharge::Continue => {
                        let engine_limit_turn = class == CoverageFailureClass::EngineLimit;
                        if engine_limit_turn {
                            on_log(&format!("attempt {attempt}: the proof engine hit its deterministic fuel limit on a demanded contract -- an engine limit, not a code bug; asking the model for one off-budget simplification..."));
                        } else {
                            on_log(&format!("attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS} failed the checks, asking the model to fix it..."));
                        }
                        history.push(ChatMessage { role: "assistant", content: source });
                        // One pointed follow-up per diagnostic class this loop
                        // has actually failed on in the field
                        // (`self_repair_hint`'s own doc comment) -- never a
                        // generic "fix it" that just re-sends whatever
                        // ambiguity caused the failure in the first place.
                        history.push(ChatMessage { role: "user", content: format!("That attempt failed with this diagnostic:\n{diagnostic}\nFix it and reply with the corrected, complete `.nir` source only.{}", self_repair_hint(&diagnostic)) });
                    }
                    BudgetCharge::StopGiveUp => {
                        break;
                    }
                    BudgetCharge::StopEscalate => {
                        // RFC 0016: escalate to the operator, never blame the
                        // model for a solver limit. The obligation rides
                        // along verbatim, the draft is already on disk, and
                        // the operator's levers are named.
                        return Err(format!("escalated to the operator (RFC 0016): the proof engine's deterministic fuel ran out on a demanded contract -- an engine limit, NOT a code bug. The model's one off-budget simplification attempt (draft kept at .nir/generated/attempts/attempt_{attempt}.nir) did not clear it. Proof obligation, verbatim:\n{last_diagnostic}\nOperator options: state a weaker-but-provable demand on the unit, raise the fuel (`nirdosha::contract_check::set_proof_fuel_rlimit`), or waive the demand (`:waive`) and re-generate."));
                    }
                }
            }
        }
    }
    Err(format!("gave up after {MAX_SELF_REPAIR_ATTEMPTS} attempts -- every attempt's full draft is kept under .nir/generated/attempts/ for inspection. Last diagnostic:\n{last_diagnostic}"))
}

/// A bounded generate/self-repair round trip over a *plain* natural-
/// language task prompt, rather than `generate_program`'s own
/// `CandidateUnit`-list prompt -- the primitive `crates/bench`'s
/// pass@1/self-repair-rate harness needs
/// (`nirdosha-master-plan.md` Part 3 Sprint 2's "Benchmark harness
/// v1"). Reuses every piece of `generate_program`'s already-tested
/// retry discipline (`extract_nir_source`, `self_repair_hint`,
/// `typecheck_and_build_check`, `MAX_SELF_REPAIR_ATTEMPTS`, the same
/// `NIR_SYSTEM_PROMPT` a real generation call sends) rather than a
/// second copy of it -- this and `generate_program` differ only in
/// what the first user message is and what happens after a success
/// (this one has no project directory to write into; the caller
/// decides what to do with the returned source).
///
/// Returns `Ok((source, attempt))` on success -- `attempt` is 1-based,
/// so `1` means it compiled on the first try (a harness's pass@1
/// signal) and anything higher means the self-repair loop rescued it.
/// `Err` carries the last diagnostic once every attempt is exhausted.
pub fn generate_from_task_prompt(client: &LlmClient, task_prompt: &str, on_log: &mut dyn FnMut(&str)) -> Result<(String, u32), String> {
    let mut history = vec![ChatMessage { role: "system", content: NIR_SYSTEM_PROMPT.to_string() }, ChatMessage { role: "user", content: task_prompt.to_string() }];
    let mut last_diagnostic = String::new();
    for attempt in 1..=MAX_SELF_REPAIR_ATTEMPTS {
        let raw = client.complete(&history).map_err(|e| format!("couldn't reach the model: {e}"))?;
        let source = extract_nir_source(&raw);
        match typecheck_and_build_check(&source) {
            Ok(()) => {
                on_log(&format!("compiled on attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS}"));
                return Ok((source, attempt));
            }
            Err(diagnostic) => {
                last_diagnostic = diagnostic.clone();
                if attempt == MAX_SELF_REPAIR_ATTEMPTS {
                    break;
                }
                on_log(&format!("attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS} failed to compile, asking the model to fix it..."));
                history.push(ChatMessage { role: "assistant", content: source });
                history.push(ChatMessage {
                    role: "user",
                    content: format!(
                        "That failed to compile with this diagnostic:\n{diagnostic}\nFix it and reply with the corrected, complete `.nir` source only.{}",
                        self_repair_hint(&diagnostic)
                    ),
                });
            }
        }
    }
    Err(format!("gave up after {MAX_SELF_REPAIR_ATTEMPTS} attempts -- last diagnostic:\n{last_diagnostic}"))
}

/// One-shot, no self-repair, no Nirdosha system prompt -- the plain
/// "ask a model for code" call `crates/bench`'s cross-language
/// baseline needs to ask the *same* model for a TypeScript/Rust
/// solution to a task, for comparison against `generate_from_task_prompt`'s
/// Nirdosha-specific self-repair loop. Deliberately not routed through
/// `generate_from_task_prompt` -- that function's whole point is
/// looping against `.nir`-specific compile diagnostics, which makes no
/// sense for a language this compiler doesn't parse.
pub fn generate_plain(client: &LlmClient, system_prompt: &str, user_prompt: &str) -> Result<String, String> {
    let history = [ChatMessage { role: "system", content: system_prompt.to_string() }, ChatMessage { role: "user", content: user_prompt.to_string() }];
    client.complete(&history)
}

/// `nirdosha-master-plan.md` Part 3 Q1 2027's "`nirdosha suggest-
/// contracts` v1 -- LLM-assisted contract inference" (parity target:
/// Kōdo's own `kodoc annotate --ai`, Certora AutoProver). One-shot,
/// deliberately: unlike `generate_from_task_prompt`'s bounded self-
/// repair loop (which knows "did it compile" as its own stopping
/// criterion), a *suggested contract* has no such self-checkable
/// signal here -- whether it's actually true needs a real
/// `nirdosha verify`/`certify` run against the spliced-in result,
/// which is `cmd_suggest_contracts`'s job, not this function's; a
/// contract can compile as valid `.nir` syntax and still be a false
/// statement about the function (which is exactly what Certora's own
/// AutoProver found on Aave v4: "independently derived one invariant,
/// missed a human-written one" -- coverage is not the same as
/// correctness, and this suggestion is explicitly not presented as
/// either).
pub fn suggest_contract(client: &LlmClient, file_source: &str, fn_name: &str) -> Result<String, String> {
    let prompt = format!(
        "Here is a Nirdosha (.nir) program:\n\n{file_source}\n\nSuggest a `validate {fn_name} {{ ... }}` block stating the strongest true Hoare pre/post contract you can infer for `{fn_name}` from its body, parameter names, and return type. Reply with ONLY the validate block source (starting with `validate {fn_name} {{` and ending with the matching `}}`), no prose, no markdown fence, no other declarations."
    );
    let history = vec![ChatMessage { role: "system", content: NIR_SYSTEM_PROMPT.to_string() }, ChatMessage { role: "user", content: prompt }];
    let raw = client.complete(&history).map_err(|e| format!("couldn't reach the model: {e}"))?;
    Ok(extract_nir_source(&raw))
}

/// Verifies a candidate source string actually typechecks, ownership-
/// checks, *and* builds -- Generate mode's own lock definition
/// (rfcs/0014: "a CodeUnit locks when its generated `.nir` both exists
/// and builds successfully") means a typecheck pass alone isn't enough
/// evidence. Runs against a throwaway, per-call-unique temp file/binary
/// (`loader::load_program` reads from a path, not a string) that's
/// removed either way -- this never touches the stable generated-source
/// path itself; only `generate_program`'s own caller, once this
/// returns `Ok`, does that.
///
/// `SCRATCH_COUNTER`, not PID alone: PID is constant across every
/// thread in this process, so two *concurrent* calls (two parallel
/// `#[test]`s, or two real concurrent `hi_server.rs`/window requests --
/// `hi_window.rs`'s own custom-protocol handler now spawns a thread per
/// request specifically so slow calls like this one don't block the
/// window) used to race on the exact same path, each one liable to
/// overwrite the other's scratch file mid-write. A real, confirmed bug
/// (not a defensive guess): two `#[test]`s sharing this function
/// started flaking with exactly this symptom -- a parse error on
/// content that was neither test's own source -- the moment a third,
/// unrelated test made their scheduling interleave differently.
fn typecheck_and_build_check(source: &str) -> Result<(), String> {
    static SCRATCH_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = SCRATCH_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!("nirdosha_hi_generate_check_{}_{unique}.nir", std::process::id()));
    std::fs::write(&path, source).map_err(|e| format!("writing a scratch file to typecheck: {e}"))?;
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_hi_generate_check_{}_{unique}", std::process::id()));

    let result = (|| -> Result<(), String> {
        let path_str = path.to_str().ok_or_else(|| format!("temp path {} is not valid UTF-8", path.display()))?;
        let (program, _src): (crate::ast::Program, String) = crate::loader::load_program(path_str).map_err(|e| {
            let mut machine = vec![machine_error("parse", None, None, &e)];
            if let Some((line, col)) = first_span_in(&e) {
                machine = vec![machine_error("parse", Some(line), Some(col), &e)];
            }
            attach_source_lines(source, &format!("{e}\nmachine-readable errors: [{}]", machine.join(", ")))
        })?;
        if let Err(errors) = crate::typeck::typecheck(&program) {
            let machine: Vec<String> = errors.iter().map(|e| machine_error("typecheck", Some(e.span.line as usize), Some(e.span.col as usize), &format!("{e}"))).collect();
            return Err(attach_source_lines(source, &format!("{}\nmachine-readable errors: [{}]", errors.iter().map(|e| format!("type error: {e}")).collect::<Vec<_>>().join("\n"), machine.join(", "))));
        }
        if let Err(errors) = crate::ownership::check_ownership(&program) {
            let machine: Vec<String> = errors.iter().map(|e| machine_error("ownership", Some(e.span.line as usize), Some(e.span.col as usize), &format!("{e}"))).collect();
            return Err(attach_source_lines(source, &format!("{}\nmachine-readable errors: [{}]", errors.iter().map(|e| format!("ownership error: {e}")).collect::<Vec<_>>().join("\n"), machine.join(", "))));
        }
        let smt_report = crate::smt::analyze(&program);
        crate::codegen::build(&program, &smt_report, &out_path, crate::codegen::OptLevel::O2)
    })();

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&out_path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_map(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn explicit_trio_wins_even_when_openai_key_is_also_set() {
        let env = env_map(&[(PROVIDER_KEY_VAR, "sk-explicit"), (PROVIDER_MODEL_VAR, "custom-model"), (OPENAI_KEY_VAR, "sk-openai")]);
        let activation = resolve_activation(&env).expect("should activate");
        assert_eq!(activation.model, "custom-model");
        assert_eq!(activation.base_url, DEFAULT_PROVIDER_BASE);
    }

    #[test]
    fn openai_key_fallback_activates_with_defaults() {
        let env = env_map(&[(OPENAI_KEY_VAR, "sk-openai")]);
        let activation = resolve_activation(&env).expect("should activate");
        assert_eq!(activation.model, DEFAULT_OPENAI_MODEL);
        assert_eq!(activation.timeout_secs, DEFAULT_PROVIDER_TIMEOUT_SECS);
    }

    #[test]
    fn nothing_configured_names_both_activation_paths() {
        let env = env_map(&[]);
        let err = resolve_activation(&env).unwrap_err();
        assert!(err.contains(PROVIDER_KEY_VAR) && err.contains(OPENAI_KEY_VAR), "error should name both activation paths, got: {err}");
    }

    #[test]
    fn redacted_key_never_exposes_the_real_value() {
        let activation = Activation { api_key: "sk-abcdefghijklmnop".to_string(), model: "m".to_string(), base_url: "b".to_string(), timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS };
        let redacted = activation.redacted_key();
        assert!(!redacted.contains("abcdefghijkl"));
        assert!(redacted.ends_with("mnop"));
    }

    #[test]
    fn units_prompt_explicitly_requires_fn_main_even_when_no_unit_is_named_main() {
        // Regression: a candidate set that never includes a `main`
        // component (the ordinary case -- `populate_candidates`'s own
        // system prompt asks for conceptual components, and `main`
        // isn't one) used to produce a prompt read, correctly, as "do
        // not add anything beyond these names" -- the model complied,
        // Nirdosha requires exactly one `fn main()` to typecheck at
        // all, and every self-repair retry inherited the same
        // contradiction instead of ever being told it no longer
        // applies. See units_prompt's own doc comment for the full RCA.
        let units = vec![CandidateUnit { id: "code:fn:tick".to_string(), kind: "fn".to_string(), name: "tick".to_string(), driving_text: "advances the game clock".to_string(), attributes: vec![] }];
        let prompt = units_prompt(&units, &[]);
        assert!(prompt.contains("fn main()"), "the generate-mode prompt must explicitly require fn main(), got: {prompt}");
        assert!(prompt.contains("not itself one of the named components") || prompt.contains("even though"), "the prompt should make clear main() is required in *addition* to the named components, not instead of asking for it plainly");
    }

    #[test]
    fn a_missing_main_diagnostic_gets_the_add_main_hint() {
        assert!(self_repair_hint("type error: no `fn main()` found").contains("fn main()"), "the hint must say what to do, not just repeat the problem");
    }

    #[test]
    fn a_reserved_keyword_diagnostic_gets_the_rename_hint() {
        // Regression: the exact give-up shape -- the parser used to
        // report `found State` (the Tok variant's Debug name), the
        // model had no way to map that to the `state` it wrote, and
        // all three retries failed identically. With the parser now
        // naming the word itself, the hint's remaining job is to make
        // the *action* explicit: rename it, since no escaping exists.
        let diagnostic = "parse error in /tmp/nirdosha_hi_generate_check_595385_2.nir at 224:15: expected identifier, found the reserved keyword `state`";
        let hint = self_repair_hint(diagnostic);
        assert!(hint.contains("Rename"), "the hint must say to rename, got: {hint}");
        assert!(hint.contains("state"), "the hint should speak in the same terms as the diagnostic: {hint}");
    }

    #[test]
    fn an_ordinary_diagnostic_gets_no_hint() {
        assert_eq!(self_repair_hint("type error: expected `i64`, found `f64`"), "", "unrecognized diagnostics stay a plain fix-it request");
    }

    #[test]
    fn extract_nir_source_strips_a_fenced_code_block() {
        assert_eq!(extract_nir_source("```nir\nfn main() { }\n```"), "fn main() { }");
    }

    #[test]
    fn extract_nir_source_passes_through_plain_text() {
        assert_eq!(extract_nir_source("fn main() { }"), "fn main() { }");
    }

    #[test]
    fn extract_json_array_finds_the_array_despite_surrounding_prose() {
        let raw = "Sure, here you go:\n```json\n[{\"kind\":\"fn\",\"name\":\"add\",\"driving_text\":\"adds\",\"depends_on\":[]}]\n```\nLet me know if you need more.";
        let json = extract_json_array(raw);
        let parsed: Vec<PromptCandidate> = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "add");
    }

    #[test]
    fn extract_json_array_passes_through_plain_json() {
        let raw = "[{\"kind\":\"struct\",\"name\":\"Point\",\"driving_text\":\"a 2D point\",\"depends_on\":[]}]";
        assert_eq!(extract_json_array(raw), raw);
    }

    #[test]
    fn typecheck_and_build_check_accepts_valid_source() {
        typecheck_and_build_check("fn add(a: i64, b: i64) -> i64 { return a + b }\nfn main() { }\n").expect("should build");
    }

    #[test]
    fn typecheck_and_build_check_reports_a_real_type_error() {
        let err = typecheck_and_build_check("fn add(a: i64, b: i64) -> i64 { return \"nope\" }\n").unwrap_err();
        assert!(err.contains("type error"), "expected a type error, got: {err}");
    }
}

    #[test]
    fn self_repair_hints_cover_every_field_failure_class() {
        // Each entry is a diagnostic string a real `:generate` run
        // burned all its attempts on (2026-09-11, the v4 fintech runs)
        // plus the needle its actionable hint must contain. A bare
        // "fix it" for any of these re-sent the same ignorance the
        // failure came from -- the whole point of `self_repair_hint`.
        let cases = [
            (
                "parse error in /tmp/x.nir at 154:19: expected an expression, found the reserved keyword `return`",
                "never an expression",
            ),
            (
                "parse error in /tmp/x.nir at 151:95: expected a type, found `(`",
                "no tuple types",
            ),
            (
                "parse error in /tmp/x.nir at 12:9: expected an expression, found the reserved type name `unit`",
                "unit` has no literal",
            ),
            (
                "codegen doesn't support `print` on a Vector/Matrix argument yet — only integer/f64/str/bool/unit-typed arguments are supported so far",
                "scalars only",
            ),
            (
                "lex error in /tmp/x.nir at 20:63: unexpected character `;`",
                "no statement separators",
            ),
            (
                "codegen doesn't support `workflow Approval` yet — its `data { ... }` block is non-empty",
                "EMPTY",
            ),
        ];
        for (diag, needle) in cases {
            let hint = self_repair_hint(diag);
            assert!(!hint.is_empty(), "diagnostic class must have a hint, got none for: {diag}");
            assert!(hint.contains(needle), "hint for\n  {diag}\nshould mention `{needle}`, got:\n  {hint}");
        }
    }

    #[test]
    fn return_hint_wins_over_the_generic_keyword_rename_arm() {
        // The `=> return ...` message literally contains "found the
        // reserved keyword", so the generic rename-identifier arm
        // would fire first -- and tell the model to rename `return`.
        // Arm order is load-bearing; this pins it.
        let hint = self_repair_hint("parse error in /tmp/x.nir at 3:18: expected an expression, found the reserved keyword `return`");
        assert!(hint.contains("never an expression"), "the `return`-as-expression arm must win, got: {hint}");
        let rename = self_repair_hint("parse error in /tmp/x.nir at 5:14: expected identifier, found the reserved keyword `state`");
        assert!(rename.contains("Rename"), "identifier-shaped violations still get the rename hint, got: {rename}");
    }

    #[test]
    fn offending_source_line_is_attached_to_parse_diagnostics() {
        // The model gets told WHAT it wrote at the line, not just a
        // line number it cannot reliably count to in its own output
        // -- especially after a repair edit shifted the lines below.
        let src = "fn main() {\n    let ok: bool = match json_parse(\"{}\") {\n        Ok(d) => return false,\n    }\n}\n";
        let diag = "parse error in /tmp/x.nir at 3:18: expected an expression, found the reserved keyword `return`";
        let with = attach_source_lines(src, diag);
        assert!(with.contains("Ok(d) => return false"), "the offending line itself must be quoted, got:\n{with}");
        // Type errors use a different prefix but must attach too.
        let tdiag = "type error: 3:18: `x` is not defined";
        let twith = attach_source_lines(src, tdiag);
        assert!(twith.contains("Ok(d) => return false"), "type-error diagnostics attach the line too, got:\n{twith}");
    }

    #[test]
    fn units_prompt_renders_confirmed_edges_and_the_wiring_contract() {
        // (a) of the 2026-09-11 RCA: the decompose step records
        // `depends_on` edges in the graph, and Generate mode used to
        // drop them -- the model free-ranged the wiring and components
        // went decorative. The prompt must now carry both the edges
        // AND the translation contract (what an edge means in .nir).
        let units = vec![
            CandidateUnit { id: "code:fn:authorize_payment_cents".to_string(), kind: "fn".to_string(), name: "authorize_payment_cents".to_string(), driving_text: "checks balance and limits".to_string(), attributes: vec![] },
            CandidateUnit { id: "code:fn:channel_daily_limit_cents".to_string(), kind: "fn".to_string(), name: "channel_daily_limit_cents".to_string(), driving_text: "the per-channel cap".to_string(), attributes: vec![] },
        ];
        let edges = vec![crate::hi_graph::ConfirmedEdge { src: "authorize_payment_cents".to_string(), dst: "channel_daily_limit_cents".to_string(), kind: "RELATES_TO".to_string() }];
        let prompt = units_prompt(&units, &edges);
        assert!(prompt.contains("authorize_payment_cents relates_to channel_daily_limit_cents"), "the edge must be rendered in the wiring section, got: {prompt}");
        assert!(prompt.contains("confirmed design facts"), "the wiring section must mark these as design facts, got: {prompt}");
        assert!(prompt.contains("orphans the design"), "the orphan rule must be stated, got: {prompt}");
        // With no edges, the wiring section is omitted but the orphan rule still applies.
        let bare = units_prompt(&units, &[]);
        assert!(!bare.contains("How the components relate"));
        assert!(bare.contains("orphans the design"));
    }

    #[test]
    fn diagnostics_carry_a_machine_readable_block() {
        // (b) of the 2026-09-11 RCA: the repair conversation should
        // carry the compiler's own structured error data, not just
        // prose a model has to re-parse -- plus the offending source
        // line, since it cannot count to line 154 of its own output.
        let src = "fn main() {\n    let ok: bool = match json_parse(\"{}\") {\n        Ok(d) => return false,\n    }\n}\n";
        let err = typecheck_and_build_check(src).expect_err("the match-arm return must fail the check");
        assert!(err.contains("machine-readable errors: ["), "structured block expected, got:\n{err}");
        assert!(err.contains("\"stage\":\"parse\""), "parse-stage machine errors expected, got:\n{err}");
        assert!(err.contains("\"line\":3"), "the structured block must carry the real line number, got:\n{err}");
        assert!(err.contains("\"col\":18"), "the structured block must carry the real column, got:\n{err}");
        assert!(err.contains("Ok(d) => return false"), "the offending source line must still be attached, got:\n{err}");
    }

    // =========================================================================
    // RFC 0016 Phase 1: the contract coverage gate.
    //
    // One lock serializes every test that runs a proof (the fuel override
    // is process-global, and tests run in parallel threads within this
    // binary -- without the lock, the engine-limit test's starved fuel
    // could flip a concurrently-running prove test to EngineLimit).
    static COVERAGE_TESTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn demand_unit(name: &str, demand: &str) -> crate::hi_graph::CandidateUnit {
        crate::hi_graph::CandidateUnit {
            id: format!("code:fn:{name}"),
            kind: "fn".to_string(),
            name: name.to_string(),
            driving_text: format!("{name} does money math"),
            attributes: vec![format!("validate contract {demand}")],
        }
    }

    const CHARGED_WITH_CONTRACT: &str = r#"
fn charge_cents(amount_cents: i64, balance_cents: i64) -> i64 {
    return balance_cents - amount_cents
}

validate charge_cents {
    pre: amount_cents >= 0 && amount_cents <= balance_cents
    post: result >= 0
}
"#;

    #[test]
    fn demanded_contract_recognizes_the_canonical_forms() {
        assert_eq!(demanded_contract("validate contract balance_nonnegative: result >= 0").unwrap(), "balance_nonnegative: result >= 0");
        assert_eq!(demanded_contract("contract: no overspend".trim()).unwrap(), "no overspend");
        assert_eq!(demanded_contract("validate contract:"), None, "an empty demand is not a demand");
        assert_eq!(demanded_contract("attribute to attach: fast"), None, "ordinary attributes are not demands");
        assert_eq!(demanded_contract("  validate contract spaced: yes").unwrap(), "spaced: yes");
    }

    #[test]
    fn coverage_check_passes_when_nothing_is_demanded() {
        let _g = COVERAGE_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // The compat property: every existing graph (no demand
        // attributes anywhere) passes the gate untouched -- the gate
        // is inert until someone states a law.
        let units = vec![crate::hi_graph::CandidateUnit {
            id: "code:fn:double".to_string(),
            kind: "fn".to_string(),
            name: "double".to_string(),
            driving_text: "doubles".to_string(),
            attributes: vec!["fast".to_string()],
        }];
        contract_coverage_check(CHARGED_WITH_CONTRACT, &units).expect("no demands means no gate");
    }

    #[test]
    fn coverage_check_passes_when_the_demanded_contract_proves() {
        let _g = COVERAGE_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let units = vec![demand_unit("charge_cents", "balance_nonnegative: result >= 0")];
        contract_coverage_check(CHARGED_WITH_CONTRACT, &units).expect("a demanded, proving contract must pass the gate");
    }

    #[test]
    fn coverage_check_flags_a_dropped_contract() {
        let _g = COVERAGE_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let no_contract = CHARGED_WITH_CONTRACT
            .split("validate charge_cents")
            .next()
            .unwrap()
            .to_string();
        let units = vec![demand_unit("charge_cents", "balance_nonnegative: result >= 0")];
        let failure = contract_coverage_check(&no_contract, &units).expect_err("a dropped demanded contract must fail");
        assert_eq!(failure.class, CoverageFailureClass::ContractDropped);
        assert!(failure.diagnostic.contains("carries none"), "the hint-arm marker must appear, got:\n{}", failure.diagnostic);
        assert!(failure.diagnostic.contains("balance_nonnegative"), "the demand text must be attributed, got:\n{}", failure.diagnostic);
        assert!(failure.diagnostic.contains("machine-readable errors: ["), "the structured block must ride along, got:\n{}", failure.diagnostic);
        assert!(failure.diagnostic.contains("\"stage\":\"coverage\""), "coverage-stage entries expected, got:\n{}", failure.diagnostic);
    }

    #[test]
    fn coverage_check_flags_a_dropped_fn() {
        let _g = COVERAGE_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // The unit itself was dropped from the draft: a stronger
        // dropped case than a present fn with no contract.
        let units = vec![demand_unit("authorize_payment_cents", "no overspend")];
        let failure = contract_coverage_check(CHARGED_WITH_CONTRACT, &units).expect_err("a demanded unit absent from the draft must fail");
        assert_eq!(failure.class, CoverageFailureClass::ContractDropped);
        assert!(failure.diagnostic.contains("no fn `authorize_payment_cents`"), "got:\n{}", failure.diagnostic);
    }

    #[test]
    fn coverage_check_flags_a_violated_contract() {
        let _g = COVERAGE_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let violated = r#"
fn double(x: i32) -> i32 {
    return x * 2
}

validate double {
    post: result > x
}
"#;
        let units = vec![demand_unit("double", "always increases")];
        let failure = contract_coverage_check(violated, &units).expect_err("a demanded contract the fn breaks must fail");
        assert_eq!(failure.class, CoverageFailureClass::ContractViolated);
        assert!(failure.diagnostic.contains("violated when"), "the real counterexample wording must ride along, got:\n{}", failure.diagnostic);
    }

    #[test]
    fn coverage_check_flags_a_vacuous_contract() {
        let _g = COVERAGE_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let vacuous = r#"
fn double(x: i32) -> i32 {
    return x * 2
}

validate double {
    pre: x > 10 && x < 5
    post: result > 0
}
"#;
        let units = vec![demand_unit("double", "positive inputs stay positive")];
        let failure = contract_coverage_check(vacuous, &units).expect_err("a vacuous demanded contract must fail");
        assert_eq!(failure.class, CoverageFailureClass::VacuousContract);
        assert!(failure.diagnostic.contains("vacuously"), "the vacuity wording must ride along, got:\n{}", failure.diagnostic);
    }

    #[test]
    fn coverage_check_classifies_engine_limit_and_it_dominates_the_class() {
        let _g = COVERAGE_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let hard = r#"
fn double(x: i32) -> i32 {
    return x * 2
}

validate double {
    post: result > x
}
"#;
        let units = vec![demand_unit("double", "always increases")];
        crate::contract_check::set_proof_fuel_rlimit(1);
        let failure = contract_coverage_check(hard, &units).expect_err("a fuel-starved demanded contract must fail closed");
        crate::contract_check::set_proof_fuel_rlimit(0); // restore BEFORE any assertion can bail
        assert_eq!(failure.class, CoverageFailureClass::EngineLimit, "engine limit, never a silent Proved, never a violation");
        assert!(failure.diagnostic.contains("engine limit"), "the hint-arm marker must appear, got:\n{}", failure.diagnostic);
        assert!(failure.diagnostic.contains("rlimit=1"), "the fuel actually in force must be reported, got:\n{}", failure.diagnostic);
    }

    #[test]
    fn coverage_check_flags_an_unprovable_demanded_contract() {
        let _g = COVERAGE_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A demanded contract is not optional, so `Unsupported` from the
        // walker is a rewrite instruction -- unlike verify's long-standing
        // policy of reporting unsupported contracts without failing.
        let unprovable = r#"
fn rate(base: f64) -> i64 {
    return 0
}

validate rate {
    pre: base > 1.0
    post: result >= 0
}
"#;
        let units = vec![demand_unit("rate", "rate never negative")];
        let failure = contract_coverage_check(unprovable, &units).expect_err("a demanded contract outside the provable subset must fail");
        assert_eq!(failure.class, CoverageFailureClass::ContractUnprovable);
        assert!(failure.diagnostic.contains("provable subset"), "the hint-arm marker must appear, got:\n{}", failure.diagnostic);
    }

    #[test]
    fn coverage_hint_arms_fire_per_class_and_outrank_the_generic_arms() {
        // The coverage dispatch is FIRST in the chain and gated on one
        // outer marker, so a diagnostic that happens to also contain a
        // compile-class marker must still get its coverage hint.
        let dropped = "contract coverage failure: the confirmed unit `x` demands a proving `validate` block, but the draft's fn `x` carries none. no `fn main()` found";
        let hint = self_repair_hint(dropped);
        assert!(hint.contains("separate top-level block: `validate <fn>"), "the dropped arm must teach the exact shape, got:\n{hint}");

        let engine_limit = "contract coverage failure: the confirmed unit `x` demands a `validate` contract that proves; its contract failed: `validate x`: couldn't decide -- post_logic exhausted the solver fuel (rlimit=1; an engine limit, not a violation)";
        let hint = self_repair_hint(engine_limit);
        assert!(hint.contains("Simplify the arithmetic into provable form"), "the engine-limit arm must teach simplification, not a code fix, got:\n{hint}");

        let vacuous = "contract coverage failure: the confirmed unit `x` demands a `validate` contract that proves; its contract failed: pre_logic can never be true -- every post_logic would pass vacuously";
        let hint = self_repair_hint(vacuous);
        assert!(hint.contains("impossible range"), "the vacuous arm must point at the precondition, got:\n{hint}");

        let unprovable = "contract coverage failure: ... rewrite it in the provable subset (integer-only params/result, linear arithmetic, no loops/calls)";
        let hint = self_repair_hint(unprovable);
        assert!(hint.contains("provable subset"), "got:\n{hint}");

        let violated = "contract coverage failure: the confirmed unit `x` demands a `validate` contract that proves; its contract failed: `result > x` is violated when x = -1";
        let hint = self_repair_hint(violated);
        assert!(hint.contains("never loosen a predicate"), "the violated arm must forbid predicate-gaming, got:\n{hint}");
    }

    #[test]
    fn charge_budget_splits_violated_from_engine_limit() {
        // RFC 0016 Phase 1's budget discipline, pure: violated-class
        // failures consume MAX_SELF_REPAIR_ATTEMPTS; engine-limit
        // consumes none but allows exactly ONE simplification, then
        // escalates -- a correct-but-hard program never burns the
        // model's budget on work no code edit can fix.
        let mut budget = 4u32;
        let mut simplifications = 0u32;
        assert_eq!(charge_budget(&mut budget, &mut simplifications, CoverageFailureClass::ContractViolated), BudgetCharge::Continue);
        assert_eq!(budget, 3, "a violation consumes budget");
        assert_eq!(charge_budget(&mut budget, &mut simplifications, CoverageFailureClass::EngineLimit), BudgetCharge::Continue);
        assert_eq!(budget, 3, "an engine limit consumes NO budget");
        assert_eq!(simplifications, 1);
        assert_eq!(charge_budget(&mut budget, &mut simplifications, CoverageFailureClass::EngineLimit), BudgetCharge::StopEscalate, "a second engine limit escalates to the operator");
        assert_eq!(budget, 3, "escalation still never charged the model");
        let mut last = 1u32;
        assert_eq!(charge_budget(&mut last, &mut simplifications, CoverageFailureClass::ContractDropped), BudgetCharge::StopGiveUp);
        assert_eq!(last, 0);
    }

    #[test]
    fn units_prompt_renders_proof_demands_as_mandatory() {
        let units = vec![
            demand_unit("charge_cents", "balance_nonnegative: result >= 0"),
            crate::hi_graph::CandidateUnit {
                id: "code:fn:double".to_string(),
                kind: "fn".to_string(),
                name: "double".to_string(),
                driving_text: "doubles".to_string(),
                attributes: vec!["fast".to_string()],
            },
        ];
        let prompt = units_prompt(&units, &[]);
        assert!(prompt.contains("PROOF DEMAND, not optional"), "the demand must be unmissable, got:\n{prompt}");
        assert!(prompt.contains("MUST carry a separate top-level `validate charge_cents` block"), "the exact validate target must be named, got:\n{prompt}");
        assert!(prompt.contains("- attribute to attach: fast"), "ordinary attributes render unchanged, got:\n{prompt}");
    }
