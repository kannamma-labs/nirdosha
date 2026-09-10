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

fn units_prompt(units: &[CandidateUnit]) -> String {
    let mut out = String::from(
        "Implement a single Nirdosha (.nir) program containing exactly the following components. Use each component's own exact name for its corresponding fn/struct/enum/screen declaration.\n\n",
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
                history.push(ChatMessage { role: "user", content: format!("That failed to compile with this diagnostic:\n{diagnostic}\nFix it and reply with the corrected, complete `.nir` source only.") });
            }
        }
    }
    Err(format!("gave up after {MAX_SELF_REPAIR_ATTEMPTS} attempts -- last diagnostic:\n{last_diagnostic}"))
}

/// Verifies a candidate source string actually typechecks, ownership-
/// checks, *and* builds -- Generate mode's own lock definition
/// (rfcs/0014: "a CodeUnit locks when its generated `.nir` both exists
/// and builds successfully") means a typecheck pass alone isn't enough
/// evidence. Runs against a throwaway PID-scoped temp file/binary
/// (`loader::load_program` reads from a path, not a string) that's
/// removed either way -- this never touches the stable generated-source
/// path itself; only `generate_program`'s own caller, once this
/// returns `Ok`, does that.
fn typecheck_and_build_check(source: &str) -> Result<(), String> {
    let mut path = std::env::temp_dir();
    path.push(format!("nirdosha_hi_generate_check_{}.nir", std::process::id()));
    std::fs::write(&path, source).map_err(|e| format!("writing a scratch file to typecheck: {e}"))?;
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_hi_generate_check_{}", std::process::id()));

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
