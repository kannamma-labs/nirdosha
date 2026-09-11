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

const MAX_SELF_REPAIR_ATTEMPTS: u32 = 3;

/// The pointed follow-up appended to a failed attempt's generic "fix
/// it" request, one arm per *diagnostic class this loop has actually
/// failed on in the field* -- not per error. A bounded retry loop's
/// real enemy is a diagnostic the model can read but can't act on:
/// each arm below exists because a real `:generate` run burned all
/// `MAX_SELF_REPAIR_ATTEMPTS` tries on exactly that (see each arm's
/// own comment), and each says what to *do*, not just what went wrong.
fn self_repair_hint(diagnostic: &str) -> &'static str {
    if diagnostic.contains("no `fn main()` found") {
        // Defense in depth on top of `units_prompt`'s own fix, not a
        // substitute for it: a model that still drops `fn main()`
        // despite the now-explicit instruction gets one more, pointed
        // reminder rather than a generic "fix it" that repeats
        // whatever ambiguity caused this in the first place.
        " Add a `fn main()` -- it is required in addition to every named component from the original request, not instead of any of them."
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

fn units_prompt(units: &[CandidateUnit]) -> String {
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
            out.push_str(&format!("- attribute to attach: {attr}\n"));
        }
        out.push('\n');
    }
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
pub fn generate_program(root: &Path, client: &LlmClient, units: &[CandidateUnit], on_log: &mut dyn FnMut(&str)) -> Result<PathBuf, String> {
    if units.is_empty() {
        return Err("nothing confirmed and unlocked to generate -- `:confirm <node>` at least one candidate first".to_string());
    }
    let mut history = vec![ChatMessage { role: "system", content: NIR_SYSTEM_PROMPT.to_string() }, ChatMessage { role: "user", content: units_prompt(units) }];
    let mut last_diagnostic = String::new();
    for attempt in 1..=MAX_SELF_REPAIR_ATTEMPTS {
        let raw = client.complete(&history).map_err(|e| format!("couldn't reach the model: {e}"))?;
        let source = extract_nir_source(&raw);
        match typecheck_and_build_check(&source) {
            Ok(()) => {
                let out_path = generated_source_path(root);
                std::fs::create_dir_all(out_path.parent().expect("generated_source_path always has a parent")).map_err(|e| format!("creating {}: {e}", out_path.display()))?;
                std::fs::write(&out_path, &source).map_err(|e| format!("writing {}: {e}", out_path.display()))?;
                on_log(&format!("wrote {} (attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS})", out_path.display()));
                return Ok(out_path);
            }
            Err(diagnostic) => {
                last_diagnostic = diagnostic.clone();
                if attempt == MAX_SELF_REPAIR_ATTEMPTS {
                    break;
                }
                on_log(&format!("attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS} failed to compile, asking the model to fix it..."));
                history.push(ChatMessage { role: "assistant", content: source });
                // One pointed follow-up per diagnostic class this loop
                // has actually failed on in the field
                // (`self_repair_hint`'s own doc comment) -- never a
                // generic "fix it" that just re-sends whatever
                // ambiguity caused the failure in the first place.
                history.push(ChatMessage { role: "user", content: format!("That failed to compile with this diagnostic:\n{diagnostic}\nFix it and reply with the corrected, complete `.nir` source only.{}", self_repair_hint(&diagnostic)) });
            }
        }
    }
    Err(format!("gave up after {MAX_SELF_REPAIR_ATTEMPTS} attempts -- last diagnostic:\n{last_diagnostic}"))
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
        let (program, _src): (crate::ast::Program, String) = crate::loader::load_program(path_str)?;
        if let Err(errors) = crate::typeck::typecheck(&program) {
            return Err(errors.iter().map(|e| format!("type error: {e}")).collect::<Vec<_>>().join("\n"));
        }
        if let Err(errors) = crate::ownership::check_ownership(&program) {
            return Err(errors.iter().map(|e| format!("ownership error: {e}")).collect::<Vec<_>>().join("\n"));
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
        let prompt = units_prompt(&units);
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
