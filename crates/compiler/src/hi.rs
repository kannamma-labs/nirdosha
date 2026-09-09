//! `nirdosha hi` — an interactive, LLM-backed console for writing
//! Nirdosha by natural language instead of hand-written `.nir` —
//! rfcs/0012-nirdosha-hi-agentic-console.md's v1 slice: activation
//! contract, NL-to-`.nir` generation with a bounded compiler-feedback
//! self-repair loop, and an on-request diagnostic explainer. Everything
//! else that RFC's own design section describes (model-driven dispatch
//! to other subcommands, read-only Q&A over project files, read-only
//! ingestion of other languages, streaming responses, native Anthropic
//! wire format, reusing RFC 0011's pooled/admission-controlled
//! `call`-shape provider) is an explicit, named non-goal here — see the
//! RFC's own "Explicit non-goals" section for why each one is deferred
//! rather than silently dropped.
//!
//! **Why a bespoke `reqwest` client, not RFC 0011's `call`-shape
//! provider.** That machinery (`kernel::http`/`plugin_provider`/
//! `PoolRegistry`) is real, reusable Rust — but `crates/runtime-kernels`
//! is its own separate Cargo workspace (deadlock-avoidance reason, see
//! its own `Cargo.toml`), while `crates/compiler` (this crate, where the
//! `nirdosha` binary lives) is a member of the *root* workspace.
//! `compiled-serve` only got to reuse that machinery by *joining*
//! `runtime-kernels`'s workspace (`docs/adr/0010-runtime-kernels-rlib-
//! for-compiled-serve.md`); the same "multiple workspace roots" error
//! blocks a naive path-dependency from here. Building that cross-
//! workspace plumbing is real, separate follow-up work, not a v1
//! blocker — `reqwest` is already a resolved dependency elsewhere in
//! this same workspace (`crates/presence-gateway/Cargo.toml`), and the
//! deleted `crates/bench/src/real_model.rs` (recovered via `git show`
//! on the commit that deleted it) is exactly the right prior art to
//! mirror: blocking client, OpenAI-compatible `/chat/completions`,
//! env-var-only configuration, no `.env` file support (same choice RFC
//! 0011's `env()` builtin already made, for the same reason).

use std::io::{self, BufRead, Write};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ast::Program;
use crate::codegen::OptLevel;

/// Nirdosha's own explicit, project-owned activation path always wins
/// over guessing at a well-known vendor's env var — unambiguous, and
/// works with any OpenAI-compatible endpoint (a local proxy, a
/// self-hosted gateway, a non-OpenAI vendor), not just OpenAI itself.
const PROVIDER_KEY_VAR: &str = "NIRDOSHA_LLM_PROVIDER_KEY";
const PROVIDER_MODEL_VAR: &str = "NIRDOSHA_LLM_PROVIDER_MODEL";
const PROVIDER_BASE_VAR: &str = "NIRDOSHA_LLM_PROVIDER_BASE";
const DEFAULT_PROVIDER_BASE: &str = "https://api.openai.com/v1";

/// Real OpenAI is OpenAI-compatible by definition, so this fallback
/// needs no separate wire-format adapter — same request/response shapes
/// as the explicit-trio path above, just a different way to find the
/// key. Checked only if the trio above isn't set at all (not merged
/// with it) — a half-set trio is a mistake to report precisely, not to
/// silently patch over with this fallback (see `resolve_activation`).
const OPENAI_KEY_VAR: &str = "OPENAI_API_KEY";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o-mini";

/// A resolved, ready-to-use provider configuration. Deliberately has no
/// No `#[derive(Debug)]` — a hand-written impl below that redacts
/// `api_key` unconditionally, so `{:?}` on this struct (directly, or
/// nested inside some future wrapping struct that derives `Debug`) can
/// never become the accidental way this leaks into an error message, a
/// log line, or (once `kernel::recorder` integration exists, RFC 0010's
/// own open question, not built here) a flight-recorder entry.
pub struct Activation {
    api_key: String,
    model: String,
    base_url: String,
}

impl Activation {
    /// Shared by the hand-written `Debug` impl below and anything that
    /// wants to show the user *which* provider got resolved without
    /// risking the real key.
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
/// touching real process env vars or a network call — the RFC's own
/// "testing without live keys" open question, resolved for this piece
/// by construction rather than by a mock server.
pub fn resolve_activation(env: &dyn Fn(&str) -> Option<String>) -> Result<Activation, String> {
    let key = env(PROVIDER_KEY_VAR);
    let model = env(PROVIDER_MODEL_VAR);
    match (key, model) {
        (Some(api_key), Some(model)) => Ok(Activation {
            api_key,
            model,
            base_url: env(PROVIDER_BASE_VAR).unwrap_or_else(|| DEFAULT_PROVIDER_BASE.to_string()),
        }),
        // A partial trio is almost always a typo, not a deliberate
        // choice -- named precisely (which one, not a generic "not
        // configured") rather than silently falling through to the
        // OPENAI_API_KEY path below, which could paper over a real
        // misconfiguration with an unrelated key the user forgot was
        // still set in their shell.
        (Some(_), None) => Err(format!("{PROVIDER_KEY_VAR} is set but {PROVIDER_MODEL_VAR} is not -- both are required together")),
        (None, Some(_)) => Err(format!("{PROVIDER_MODEL_VAR} is set but {PROVIDER_KEY_VAR} is not -- both are required together")),
        (None, None) => match env(OPENAI_KEY_VAR) {
            Some(api_key) => Ok(Activation { api_key, model: DEFAULT_OPENAI_MODEL.to_string(), base_url: DEFAULT_PROVIDER_BASE.to_string() }),
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

/// The one system prompt every `.nir`-generation call sends —
/// `agent-skills/nirdosha/paste-anywhere-prompt.md` is already a
/// complete, maintained, self-contained guide for "write valid `.nir`
/// code" (the README points humans at pasting this into any chat LLM
/// directly); embedding it here means this console and the copy-paste
/// path can never silently drift apart.
const NIR_SYSTEM_PROMPT: &str = include_str!("../../../agent-skills/nirdosha/paste-anywhere-prompt.md");

/// Unlike the deleted `real_model.rs` (a benchmark harness, `panic!`ed
/// on any failure), every call here returns `Result` -- a console must
/// degrade to an error message and keep looping, never crash the whole
/// process because one request timed out or one response didn't parse.
struct LlmClient {
    http: reqwest::blocking::Client,
    activation: Activation,
}

impl LlmClient {
    fn new(activation: Activation) -> Self {
        LlmClient {
            http: reqwest::blocking::Client::builder().timeout(Duration::from_secs(120)).build().expect("building a blocking reqwest client with only a timeout set cannot fail"),
            activation,
        }
    }

    /// One non-streaming chat completion (RFC 0010's own recommended v1
    /// scope-limiter -- see this module's own doc comment). `history` is
    /// the full turn sequence so far, including the leading system
    /// prompt -- the caller owns conversation state, this just sends it.
    fn complete(&self, history: &[ChatMessage]) -> Result<String, String> {
        let request = ChatCompletionRequest { model: self.activation.model.clone(), messages: history.iter().map(|m| ChatMessage { role: m.role, content: m.content.clone() }).collect(), temperature: 0.2 };
        let url = format!("{}/chat/completions", self.activation.base_url.trim_end_matches('/'));
        let response = self.http.post(&url).bearer_auth(&self.activation.api_key).json(&request).send().map_err(|e| format!("request to {url} failed: {e}"))?;
        let status = response.status();
        let body = response.text().map_err(|e| format!("reading response body: {e}"))?;
        if !status.is_success() {
            return Err(format!("{url} returned {status}: {body}"));
        }
        let parsed: ChatCompletionResponse = serde_json::from_str(&body).map_err(|e| format!("parsing response JSON: {e} (body: {body})"))?;
        parsed.choices.into_iter().next().map(|c| c.message.content).ok_or_else(|| "response had no choices".to_string())
    }
}

/// Bounded, not unlimited -- an author should see the last real
/// compiler diagnostic and decide for themselves once the model can't
/// fix its own output after a few tries, rather than the console
/// looping silently forever against the same stuck failure.
const MAX_SELF_REPAIR_ATTEMPTS: u32 = 3;

pub fn run_console(activation: Activation) {
    println!("Nirdosha Agentic Console -- model: {}, key: {}", activation.model, activation.redacted_key());
    println!("Type a description of the program you want, `:explain` to explain the last build error, or `:quit` to exit.");
    let client = LlmClient::new(activation);
    let stdin = io::stdin();
    let mut last_diagnostic: Option<String> = None;
    loop {
        print!("nirdosha hi> ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break; // EOF (e.g. piped input, or Ctrl-D) -- exit cleanly, not an error.
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line {
            ":quit" | ":exit" => break,
            ":explain" => match &last_diagnostic {
                Some(diagnostic) => match client.complete(&[
                    ChatMessage { role: "system", content: "You are explaining a Nirdosha compiler diagnostic to the person who asked for the program that produced it. Be plain and concrete; do not repeat the raw diagnostic text back verbatim.".to_string() },
                    ChatMessage { role: "user", content: diagnostic.clone() },
                ]) {
                    Ok(explanation) => println!("{explanation}"),
                    Err(e) => eprintln!("couldn't reach the model to explain that: {e}"),
                },
                None => println!("no build has failed yet in this session -- nothing to explain."),
            },
            other if other.starts_with(':') => println!("unrecognized command `{other}` -- try `:explain` or `:quit`."),
            request => {
                last_diagnostic = generate_and_build(&client, request);
            }
        }
    }
}

/// One full NL -> `.nir` -> `build` round trip, including the bounded
/// self-repair loop. Returns the last compiler diagnostic on failure
/// (so `:explain` has something to work with), `None` on success.
fn generate_and_build(client: &LlmClient, request: &str) -> Option<String> {
    let mut history = vec![ChatMessage { role: "system", content: NIR_SYSTEM_PROMPT.to_string() }, ChatMessage { role: "user", content: request.to_string() }];
    for attempt in 1..=MAX_SELF_REPAIR_ATTEMPTS {
        let source = match client.complete(&history) {
            Ok(s) => extract_nir_source(&s),
            Err(e) => {
                eprintln!("couldn't reach the model: {e}");
                return None;
            }
        };
        match typecheck_and_build_to_temp_file(&source) {
            Ok(path) => {
                println!("built {path} (attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS})");
                return None;
            }
            Err(diagnostic) => {
                if attempt == MAX_SELF_REPAIR_ATTEMPTS {
                    eprintln!("gave up after {MAX_SELF_REPAIR_ATTEMPTS} attempts -- last diagnostic:\n{diagnostic}");
                    return Some(diagnostic);
                }
                eprintln!("attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS} failed to compile, asking the model to fix it...");
                history.push(ChatMessage { role: "assistant", content: source });
                history.push(ChatMessage { role: "user", content: format!("That failed to compile with this diagnostic:\n{diagnostic}\nFix it and reply with the corrected, complete `.nir` source only.") });
            }
        }
    }
    None
}

/// Models routinely wrap code in a fenced block even when told not to
/// (the same real behavior the deleted `real_model.rs` stripped for its
/// own `completion_text()`) -- stripping it here, once, keeps every
/// caller of `complete()` from having to know about this.
fn extract_nir_source(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.strip_prefix("nir").or_else(|| rest.strip_prefix("nirdosha")).unwrap_or(rest);
        let rest = rest.trim_start_matches('\n');
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}

/// The same load -> typecheck -> ownership-check -> SMT -> codegen
/// sequence `main.rs::typecheck_and_own`/`cmd_build` already run,
/// mirrored here rather than shared: that helper is private to the
/// `nirdosha` *binary* crate, while this module lives in the library
/// half (so it can be unit-tested and so `main.rs::cmd_hi` can stay a
/// thin wrapper) -- two separate compilation units, nothing to `use`
/// across that boundary. Writes to a real temp file (kept, not
/// cleaned up -- same "disclosed, not hidden" convention
/// `nir_transact_decode_args`'s own leaked output already uses) so a
/// successful build leaves the author something to actually run.
fn typecheck_and_build_to_temp_file(source: &str) -> Result<String, String> {
    let mut path = std::env::temp_dir();
    path.push(format!("nirdosha_hi_{}.nir", std::process::id()));
    std::fs::write(&path, source).map_err(|e| format!("writing generated source to {}: {e}", path.display()))?;

    let path_str = path.to_str().ok_or_else(|| format!("temp path {} is not valid UTF-8", path.display()))?;
    let (program, _src): (Program, String) = crate::loader::load_program(path_str)?;
    if let Err(errors) = crate::typeck::typecheck(&program) {
        return Err(errors.iter().map(|e| format!("type error: {e}")).collect::<Vec<_>>().join("\n"));
    }
    if let Err(errors) = crate::ownership::check_ownership(&program) {
        return Err(errors.iter().map(|e| format!("ownership error: {e}")).collect::<Vec<_>>().join("\n"));
    }
    let smt_report = crate::smt::analyze(&program);
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("nirdosha_hi_{}", std::process::id()));
    crate::codegen::build(&program, &smt_report, &out_path, OptLevel::O2)?;
    Ok(out_path.display().to_string())
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
        assert_eq!(activation.api_key, "sk-explicit");
        assert_eq!(activation.model, "custom-model");
        assert_eq!(activation.base_url, DEFAULT_PROVIDER_BASE);
    }

    #[test]
    fn explicit_trio_honors_a_custom_base_url() {
        let env = env_map(&[(PROVIDER_KEY_VAR, "sk-explicit"), (PROVIDER_MODEL_VAR, "custom-model"), (PROVIDER_BASE_VAR, "https://my-proxy.example/v1")]);
        let activation = resolve_activation(&env).expect("should activate");
        assert_eq!(activation.base_url, "https://my-proxy.example/v1");
    }

    #[test]
    fn partial_trio_names_the_missing_var_precisely() {
        let env = env_map(&[(PROVIDER_KEY_VAR, "sk-explicit")]);
        let err = resolve_activation(&env).unwrap_err();
        assert!(err.contains(PROVIDER_MODEL_VAR), "error should name the missing var, got: {err}");
    }

    #[test]
    fn openai_key_fallback_activates_with_defaults() {
        let env = env_map(&[(OPENAI_KEY_VAR, "sk-openai")]);
        let activation = resolve_activation(&env).expect("should activate");
        assert_eq!(activation.api_key, "sk-openai");
        assert_eq!(activation.model, DEFAULT_OPENAI_MODEL);
        assert_eq!(activation.base_url, DEFAULT_PROVIDER_BASE);
    }

    #[test]
    fn nothing_configured_names_both_activation_paths() {
        let env = env_map(&[]);
        let err = resolve_activation(&env).unwrap_err();
        assert!(err.contains(PROVIDER_KEY_VAR) && err.contains(OPENAI_KEY_VAR), "error should name both activation paths, got: {err}");
    }

    #[test]
    fn redacted_key_never_exposes_the_real_value() {
        let activation = Activation { api_key: "sk-abcdefghijklmnop".to_string(), model: "m".to_string(), base_url: "b".to_string() };
        let redacted = activation.redacted_key();
        assert!(!redacted.contains("abcdefghijkl"));
        assert!(redacted.ends_with("mnop"));
    }

    #[test]
    fn extract_nir_source_strips_a_fenced_code_block() {
        let raw = "```nir\nfn main() { }\n```";
        assert_eq!(extract_nir_source(raw), "fn main() { }");
    }

    #[test]
    fn extract_nir_source_passes_through_plain_text() {
        let raw = "fn main() { }";
        assert_eq!(extract_nir_source(raw), "fn main() { }");
    }
}
