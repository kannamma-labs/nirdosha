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

use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::ast::Program;
use crate::codegen::OptLevel;

/// `tui` is a *child* module of `hi` (declared here via `#[path]` rather
/// than living under a `hi/` directory, since `hi.rs` predates it and a
/// directory move would be a pointless diff) -- that ancestry, not any
/// `pub` on the items themselves, is what lets it reach `Activation`'s
/// fields, `LlmClient`, `generate_and_build`, etc. directly: Rust's
/// privacy rule is "visible to the defining module and its descendants",
/// so nothing in this file needs to widen its visibility just to gain a
/// second (TUI) front end alongside the plain line-based one below.
#[path = "hi_tui.rs"]
mod tui;

/// The three outcomes `generate_and_build`'s self-repair loop reports as
/// it runs, factored out so both front ends (the plain line-based loop
/// and `tui`) render the *same* sequence of events through their own
/// styling instead of the loop hard-coding `println!`/`eprintln!` --
/// which would otherwise corrupt `tui`'s raw-mode alternate screen.
pub(crate) enum LogEvent {
    Info(String),
    Error(String),
    Success(String),
}

/// What one line of console input means, shared by both front ends so
/// `:quit`/`:explain`/unknown-`:`-command/plain-request parsing can't
/// drift between them.
pub(crate) enum Command<'a> {
    Quit,
    Explain,
    /// `:ask <question>` -- FTS-backed Q&A over `.nir/realm.db`'s
    /// ingested requirements/decisions/code (rfcs/0013, closes RFC
    /// 0012 capability 5). Purely local -- no model call.
    Ask(&'a str),
    /// `:impact <target>` -- the same bounded bidirectional report
    /// `nirdosha realm impact` prints, inline (rfcs/0013). Purely
    /// local -- no model call.
    Impact(&'a str),
    Unknown(&'a str),
    /// `serve` is only ever `true` via the explicit `:build --serve
    /// <description>` spelling -- a plain-text request (no `:build` at
    /// all) always builds an ordinary, non-serving binary, matching
    /// this console's own "compile it and make it runnable *if asked
    /// to*" scope: nothing here ever decides on its own that a request
    /// "looks like" a web app and should be served without being told.
    Request { description: &'a str, serve: bool },
}

pub(crate) fn parse_line(line: &str) -> Command<'_> {
    match line {
        ":quit" | ":exit" => Command::Quit,
        ":explain" => Command::Explain,
        other if other == ":ask" || other.starts_with(":ask ") => match other[":ask".len()..].trim() {
            "" => Command::Unknown(":ask (usage: `:ask <question>`)"),
            question => Command::Ask(question),
        },
        other if other == ":impact" || other.starts_with(":impact ") => match other[":impact".len()..].trim() {
            "" => Command::Unknown(":impact (usage: `:impact <target>` -- a requirement/decision id, or a code-unit name/kind:name)"),
            target => Command::Impact(target),
        },
        // `:build <description>` is just an explicit spelling of the
        // plain-text request below -- both end up as the exact same
        // `Command::Request` -- for anyone who'd rather type a `:`
        // command than rely on "no prefix at all" meaning "build this."
        // `:build --serve <description>` additionally asks for a real
        // compiled `--serve` binary (`codegen::build_serve`) instead of
        // a plain one -- reviving compiled `nirdosha serve`
        // (`rfcs/0010-landing-and-serve-exposure.md`).
        other if other == ":build" || other.starts_with(":build ") => {
            let rest = other[":build".len()..].trim();
            let (serve, description) = match rest.strip_prefix("--serve") {
                Some(after) => (true, after.trim()),
                None => (false, rest),
            };
            match description {
                "" => Command::Unknown(
                    ":build (usage: `:build <description>`, `:build --serve <description>` -- or just type the description with no `:build` at all)",
                ),
                description => Command::Request { description, serve },
            }
        }
        other if other.starts_with(':') => Command::Unknown(other),
        request => Command::Request { description: request, serve: false },
    }
}

/// A plain-text, append-as-you-go trace of one `nirdosha hi` session --
/// activation (model/base URL, never the real key), every command/
/// request typed, and every `LogEvent` the self-repair loop reports --
/// written to the same "disclosed, not hidden" temp-file convention
/// `typecheck_and_build_to_temp_file` below already uses, so a
/// confusing run (a build that silently didn't do what was asked, an
/// `:explain` that hit an unreachable model) leaves something to go
/// back and read instead of vanishing the moment `tui`'s alternate
/// screen exits. This is *not* the real `kernel::recorder` flight
/// recorder this module's own doc comment defers to future cross-
/// workspace work -- just a plain file. `Clone`+`Arc`-backed because
/// `tui`'s background worker threads need to append to the exact same
/// trace the main thread does.
#[derive(Clone)]
pub(crate) struct SessionLog {
    sink: Arc<Mutex<Box<dyn Write + Send>>>,
    started: Instant,
    path: Option<std::path::PathBuf>,
}

impl SessionLog {
    /// Never fails outwardly: a session log is a debugging aid, not a
    /// feature the console depends on, so a permissions error or a full
    /// disk degrades to a silently-discarded log (reported once, here)
    /// rather than refusing to start the console at all.
    fn open() -> SessionLog {
        let mut path = std::env::temp_dir();
        path.push(format!("nirdosha_hi_{}.log", std::process::id()));
        match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => SessionLog { sink: Arc::new(Mutex::new(Box::new(file))), started: Instant::now(), path: Some(path) },
            Err(e) => {
                eprintln!("nirdosha hi: couldn't open a session log at {} ({e}) -- continuing without one", path.display());
                SessionLog { sink: Arc::new(Mutex::new(Box::new(io::sink()))), started: Instant::now(), path: None }
            }
        }
    }

    pub(crate) fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    /// Best-effort: a write failure here (disk full mid-session, say)
    /// is silently dropped rather than surfaced, for the same reason
    /// `open` degrades instead of erroring -- logging must never be
    /// the thing that crashes an otherwise-working console session.
    pub(crate) fn log(&self, msg: impl std::fmt::Display) {
        let elapsed = self.started.elapsed().as_secs_f64();
        if let Ok(mut sink) = self.sink.lock() {
            let _ = writeln!(sink, "[+{elapsed:8.3}s] {msg}");
            let _ = sink.flush();
        }
    }
}

/// Durable, cross-process counters for the two distinct ways a
/// `nirdosha hi` build can fail. Persisted to disk (not just kept in
/// this process's memory) because the more dangerous of the two -- a
/// compiler panic -- kills the process before it could ever report its
/// own count: without a file surviving the crash there would be no way
/// to answer "how many times has this actually happened" for anything
/// but the self-repair kind, which is exactly the gap that made "how
/// many compile errors have we hit" impossible to answer honestly
/// before this existed.
const FAILURE_COUNTS_FILE: &str = "nirdosha_hi_failure_counts.json";

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub(crate) struct FailureCounters {
    /// Every time `generate_and_build`'s self-repair loop had to ask the
    /// model to fix a diagnostic and try again -- the LLM producing code
    /// that doesn't typecheck/ownership-check yet. Bounded and
    /// retryable by construction; a rising count here is a
    /// `paste-anywhere-prompt.md` coverage gap (the guide didn't say
    /// enough), not a compiler bug.
    pub(crate) self_repair_retries: u64,
    /// Every time the compiler itself panicked (an `unreachable!()`, an
    /// index-out-of-bounds, ...) on code `typeck`/`ownership` had
    /// *already proved valid*. The self-repair loop never even gets to
    /// see these -- the process dies before it can log a diagnostic and
    /// retry. A single one of these is a compiler bug, full stop; no
    /// prompt wording could have prevented it, since the input was
    /// legal.
    pub(crate) compiler_panics: u64,
}

/// One file, machine-wide (not per-PID like `SessionLog`) -- the point
/// is a count that outlives any one process, including the one that's
/// about to panic.
fn failure_counts_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(FAILURE_COUNTS_FILE);
    path
}

/// Missing file, corrupt JSON, anything -- degrades to a zeroed count
/// rather than erroring, same "a diagnostic aid must never be the thing
/// that breaks the console" contract `SessionLog` already follows.
fn load_failure_counters_from(path: &std::path::Path) -> FailureCounters {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_failure_counters_to(path: &std::path::Path, counters: &FailureCounters) {
    if let Ok(json) = serde_json::to_string_pretty(counters) {
        let _ = std::fs::write(path, json);
    }
}

/// Read by both front ends to show a live, durable "how many times has
/// this actually happened" -- reflecting every `nirdosha hi` process
/// that has ever run on this machine, not just the current one.
pub(crate) fn failure_counters() -> FailureCounters {
    load_failure_counters_from(&failure_counts_path())
}

/// Read-modify-write on a shared file races across concurrent
/// `nirdosha hi` processes (two incrementing at the same instant could
/// lose one count) -- acceptable for a diagnostic counter that's never
/// used for anything load-bearing, not worth a real file lock for.
fn record_self_repair_retry() -> FailureCounters {
    let path = failure_counts_path();
    let mut counters = load_failure_counters_from(&path);
    counters.self_repair_retries += 1;
    save_failure_counters_to(&path, &counters);
    counters
}

fn record_compiler_panic() -> FailureCounters {
    let path = failure_counts_path();
    let mut counters = load_failure_counters_from(&path);
    counters.compiler_panics += 1;
    save_failure_counters_to(&path, &counters);
    counters
}

/// Wraps (never replaces) the default panic hook so a compiler panic's
/// normal message + backtrace-hint still prints exactly as before --
/// this only adds the two things that would otherwise be lost the
/// instant the process dies: a session-log entry with a real timestamp,
/// and a durable increment to the on-disk count the TUI header and the
/// plain console both go on reporting long after this process is gone.
/// Also drops the terminal out of raw/alternate-screen mode
/// best-effort, in case the panic happened while `tui` had it in that
/// state -- otherwise this very message would be swallowed by a screen
/// buffer nothing would ever restore, and the shell would be left
/// unusable after the crash on top of losing the diagnostic.
fn install_panic_hook(log: SessionLog) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let counters = record_compiler_panic();
        log.log(format!("COMPILER PANIC #{} (all-time): {info}", counters.compiler_panics));
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        default_hook(info);
    }));
}

/// Nirdosha's own explicit, project-owned activation path always wins
/// over guessing at a well-known vendor's env var — unambiguous, and
/// works with any OpenAI-compatible endpoint (a local proxy, a
/// self-hosted gateway, a non-OpenAI vendor), not just OpenAI itself.
const PROVIDER_KEY_VAR: &str = "NIRDOSHA_LLM_PROVIDER_KEY";
const PROVIDER_MODEL_VAR: &str = "NIRDOSHA_LLM_PROVIDER_MODEL";
const PROVIDER_BASE_VAR: &str = "NIRDOSHA_LLM_PROVIDER_BASE";
const DEFAULT_PROVIDER_BASE: &str = "https://api.openai.com/v1";

/// The old hardcoded 120s was tuned for a plain chat completion, not
/// for asking a reasoning model to generate a whole "production
/// quality" app in one shot: a real captured session hit exactly
/// 120.001s against a local Ollama-hosted reasoning model on a
/// moderately ambitious request, and reported it as "couldn't reach
/// the model" -- indistinguishable, to whoever read that message, from
/// an actual connectivity failure. 300s is enough headroom for that
/// case without making a *genuinely* unreachable endpoint hang the
/// console for five minutes before saying so; overridable per-provider
/// since "enough headroom" depends entirely on the model and request.
const PROVIDER_TIMEOUT_SECS_VAR: &str = "NIRDOSHA_LLM_PROVIDER_TIMEOUT_SECS";
const DEFAULT_PROVIDER_TIMEOUT_SECS: u64 = 300;

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
    timeout_secs: u64,
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
    let timeout_secs = match env(PROVIDER_TIMEOUT_SECS_VAR) {
        Some(raw) => raw.parse::<u64>().map_err(|_| format!("{PROVIDER_TIMEOUT_SECS_VAR} is set to `{raw}`, which isn't a whole number of seconds"))?,
        None => DEFAULT_PROVIDER_TIMEOUT_SECS,
    };
    let key = env(PROVIDER_KEY_VAR);
    let model = env(PROVIDER_MODEL_VAR);
    match (key, model) {
        (Some(api_key), Some(model)) => Ok(Activation {
            api_key,
            model,
            base_url: env(PROVIDER_BASE_VAR).unwrap_or_else(|| DEFAULT_PROVIDER_BASE.to_string()),
            timeout_secs,
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
    // Missing entirely (some non-conforming OpenAI-compatible backends
    // omit it) shouldn't fail parsing the rest of an otherwise-good
    // response -- degrades to a zeroed `TokenUsage` instead.
    #[serde(default)]
    usage: TokenUsage,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: String,
}

/// One response's token accounting, straight from the OpenAI-
/// compatible `usage` object -- and, via `AddAssign`, also
/// `LlmClient`'s own running session total (`usage_totals`). Every
/// call this console makes (generation, each self-repair retry,
/// `:explain`) folds in here, so the count shown is "tokens spent this
/// session," not just the last request's.
#[derive(Deserialize, Clone, Copy, Default)]
pub(crate) struct TokenUsage {
    #[serde(default)]
    pub(crate) prompt_tokens: u64,
    #[serde(default)]
    pub(crate) completion_tokens: u64,
}

impl std::ops::AddAssign for TokenUsage {
    fn add_assign(&mut self, other: Self) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
    }
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
pub(crate) struct LlmClient {
    http: reqwest::blocking::Client,
    activation: Activation,
    usage: Mutex<TokenUsage>,
}

impl LlmClient {
    fn new(activation: Activation) -> Self {
        let timeout = Duration::from_secs(activation.timeout_secs);
        LlmClient {
            http: reqwest::blocking::Client::builder().timeout(timeout).build().expect("building a blocking reqwest client with only a timeout set cannot fail"),
            activation,
            usage: Mutex::new(TokenUsage::default()),
        }
    }

    /// This session's running total across every completion this
    /// client has made -- read by both front ends to show a live
    /// input/output token count.
    pub(crate) fn usage_totals(&self) -> TokenUsage {
        self.usage.lock().map(|guard| *guard).unwrap_or_default()
    }

    /// One non-streaming chat completion (RFC 0010's own recommended v1
    /// scope-limiter -- see this module's own doc comment). `history` is
    /// the full turn sequence so far, including the leading system
    /// prompt -- the caller owns conversation state, this just sends it.
    fn complete(&self, history: &[ChatMessage]) -> Result<String, String> {
        let request = ChatCompletionRequest { model: self.activation.model.clone(), messages: history.iter().map(|m| ChatMessage { role: m.role, content: m.content.clone() }).collect(), temperature: 0.2 };
        let url = format!("{}/chat/completions", self.activation.base_url.trim_end_matches('/'));
        let response = self.http.post(&url).bearer_auth(&self.activation.api_key).json(&request).send().map_err(|e| {
            // A client-side request timeout and an actually-unreachable
            // endpoint look identical unless called out separately --
            // a real captured session hit exactly this client's
            // timeout on an ambitious request and reported it as
            // "couldn't reach the model," indistinguishable from a
            // real connectivity failure to whoever read it.
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
        if let Ok(mut usage) = self.usage.lock() {
            *usage += parsed.usage;
        }
        parsed.choices.into_iter().next().map(|c| c.message.content).ok_or_else(|| "response had no choices".to_string())
    }
}

/// Bounded, not unlimited -- an author should see the last real
/// compiler diagnostic and decide for themselves once the model can't
/// fix its own output after a few tries, rather than the console
/// looping silently forever against the same stuck failure.
const MAX_SELF_REPAIR_ATTEMPTS: u32 = 3;

/// rfcs/0013-nirdosha-realm.md's auto-scaffold: creates/opens
/// `.nir/realm.db` and re-runs the code-hash sync, only ever called
/// from `run_console` (i.e. only after RFC 0012's activation contract
/// already resolved credentials -- a failed activation should never
/// leave a stray `.nir/` behind for someone who was only checking
/// whether `hi` was configured). Never fails the console over this:
/// same "a diagnostic aid must never be the thing that breaks the
/// console" posture `SessionLog::open`/`FailureCounters` already
/// follow -- a Realm problem degrades to a logged warning and `:ask`/
/// `:impact` reporting "nothing ingested yet" rather than refusing to
/// start.
fn open_realm_or_warn(log: &SessionLog) -> Option<rusqlite::Connection> {
    if crate::realm::is_disabled(&|k| std::env::var(k).ok()) {
        return None;
    }
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            log.log(format!("realm: couldn't resolve the current directory, continuing without it: {e}"));
            return None;
        }
    };
    match crate::realm::open(&cwd) {
        Ok(conn) => {
            match crate::realm::sync(&conn, &cwd, &[]) {
                Ok(r) => log.log(format!(
                    "realm sync: {} file(s), {} unit(s) seen, {} added, {} changed, {} edge(s) flagged possibly_stale",
                    r.files_scanned, r.units_seen, r.units_added, r.units_changed, r.edges_flagged
                )),
                Err(e) => log.log(format!("realm sync failed, continuing with a possibly-stale graph: {e}")),
            }
            Some(conn)
        }
        Err(e) => {
            log.log(format!("realm: couldn't open .nir/realm.db, continuing without it ({}=1 to silence this): {e}", crate::realm::REALM_DISABLE_VAR));
            None
        }
    }
}

/// Entry point `main.rs::cmd_hi` calls -- picks the front end at
/// runtime rather than at compile time so the same binary stays
/// scriptable (piped stdin, or output redirected to a file/CI log) even
/// though interactive use now gets the full `tui` front end: a raw-mode
/// alternate-screen app assumes a real terminal on both ends, and
/// silently garbles a pipe (or hangs reading a TTY that isn't there).
pub fn run_console(activation: Activation) {
    let log = SessionLog::open();
    install_panic_hook(log.clone());
    log.log(format!("session start -- model={} base_url={} key={}", activation.model, activation.base_url, activation.redacted_key()));
    let realm = open_realm_or_warn(&log);
    if io::stdout().is_terminal() && io::stdin().is_terminal() {
        if let Err(e) = tui::run(activation, log.clone(), realm) {
            log.log(format!("terminal UI error: {e}"));
            eprintln!("nirdosha hi: terminal UI error: {e}");
        }
    } else {
        run_console_plain(activation, log, realm);
    }
}

/// The original v1 front end, kept verbatim in behavior for the
/// non-interactive case above -- same prompts, same output, just
/// routed through `parse_line`/`LogEvent` now instead of matching on
/// raw strings and printing inline, so this and `tui` can't drift.
fn run_console_plain(activation: Activation, log: SessionLog, realm: Option<rusqlite::Connection>) {
    println!("Nirdosha Agentic Console -- model: {}, key: {}", activation.model, activation.redacted_key());
    if let Some(path) = log.path() {
        println!("session log: {}", path.display());
    }
    println!(
        "Type a description of the program you want (or `:build <description>`), `:explain` to explain the last build error, `:ask <question>`/`:impact <target>` to query the project's realm graph, or `:quit` to exit."
    );
    print_failure_counters();
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
        log.log(format!("> {line}"));
        match parse_line(line) {
            Command::Quit => break,
            Command::Explain => {
                match &last_diagnostic {
                    Some(diagnostic) => match explain_diagnostic(&client, diagnostic) {
                        Ok(explanation) => {
                            log.log(format!("explanation: {explanation}"));
                            println!("{explanation}");
                        }
                        Err(e) => {
                            log.log(format!("explain error: {e}"));
                            eprintln!("couldn't reach the model to explain that: {e}");
                        }
                    },
                    None => println!("no build has failed yet in this session -- nothing to explain."),
                }
                print_token_usage(&client);
            }
            Command::Ask(question) => match &realm {
                Some(conn) => match crate::realm::ask(conn, question) {
                    Ok(hits) => print!("{}", format_ask_hits(question, &hits)),
                    Err(e) => eprintln!("{e}"),
                },
                None => println!("the realm graph isn't available this session (see the session log for why) -- try `nirdosha realm ingest`/`sync` from a shell instead."),
            },
            Command::Impact(target) => match &realm {
                Some(conn) => match crate::realm::impact(conn, target) {
                    Ok(report) => print!("{}", format_impact_report(target, &report)),
                    Err(e) => eprintln!("{e}"),
                },
                None => println!("the realm graph isn't available this session (see the session log for why) -- try `nirdosha realm impact` from a shell instead."),
            },
            Command::Unknown(other) => println!("unrecognized command `{other}` -- try `:explain`, `:ask`, `:impact`, or `:quit`."),
            Command::Request { description, serve } => {
                last_diagnostic = generate_and_build(&client, description, serve, &|event| {
                    log.log(match &event {
                        LogEvent::Info(s) => format!("info: {s}"),
                        LogEvent::Error(s) => format!("error: {s}"),
                        LogEvent::Success(s) => format!("success: {s}"),
                    });
                    match event {
                        LogEvent::Info(s) | LogEvent::Error(s) => eprintln!("{s}"),
                        LogEvent::Success(s) => println!("{s}"),
                    }
                });
                print_token_usage(&client);
            }
        }
    }
}

/// Shared by the two model-calling `Command` arms above: the session's
/// running input/output token total after whatever call just
/// happened. Plain-text equivalent of `tui`'s live header line --
/// printed once per command rather than live, since this front end has
/// no persistent screen to update in place.
fn print_token_usage(client: &LlmClient) {
    let usage = client.usage_totals();
    println!("tokens (session total): in {}  out {}", usage.prompt_tokens, usage.completion_tokens);
    print_failure_counters();
}

/// The durable, cross-process count from `failure_counters()` -- "how
/// many times has this actually happened," covering every `nirdosha hi`
/// process that ever ran on this machine, not just this one.
fn print_failure_counters() {
    let counters = failure_counters();
    println!("compile failures (all-time): self-repair retries {}  compiler panics {}", counters.self_repair_retries, counters.compiler_panics);
}

/// Shared by both `hi` front ends and `main.rs::cmd_realm`'s standalone
/// `realm impact` -- one rendering of a bounded impact walk
/// (rfcs/0013), flagged nodes listed first (`realm::impact` already
/// sorts them that way).
pub fn format_impact_report(target: &str, report: &crate::realm::ImpactReport) -> String {
    if report.hits.is_empty() {
        return format!("no reachable nodes from `{target}` -- try `nirdosha realm link` or `nirdosha realm sync` first.\n");
    }
    let mut out = format!("impact of `{target}` ({} node(s){}):\n", report.hits.len(), if report.partial { ", partial -- bound reached" } else { "" });
    for h in &report.hits {
        let flag = h.flag.as_deref().map(|f| format!("  [{f}]")).unwrap_or_default();
        // `source_ref`/`line`/`col` are only ever set on a `CodeUnit`
        // node (`Requirement`/`Document`/`Chunk` have nothing to point
        // at) -- printed only when present, same "NULL means nothing to
        // show" convention the rest of this report already follows.
        let location = match (&h.source_ref, h.line, h.col) {
            (Some(path), Some(line), Some(col)) => format!("  ({path}:{line}:{col})"),
            _ => String::new(),
        };
        out.push_str(&format!("  depth {} {} {} `{}`{location}{flag}\n", h.depth, h.kind, h.edge_kind, h.title.as_deref().unwrap_or(&h.node_id)));
    }
    out
}

/// Shared by both `hi` front ends: one rendering of an FTS `:ask`/
/// `realm ask` result set.
pub fn format_ask_hits(query: &str, hits: &[crate::realm::AskHit]) -> String {
    if hits.is_empty() {
        return format!("no ingested chunks match `{query}` -- try `nirdosha realm ingest <doc.md>` first.\n");
    }
    let mut out = format!("{} match(es) for `{query}`:\n", hits.len());
    for h in hits {
        out.push_str(&format!("  [{}] {}\n", h.doc_id, h.content));
    }
    out
}

/// Shared by both front ends: turn a failed build's diagnostic into a
/// plain-language explanation via one more model call.
pub(crate) fn explain_diagnostic(client: &LlmClient, diagnostic: &str) -> Result<String, String> {
    client.complete(&[
        ChatMessage { role: "system", content: "You are explaining a Nirdosha compiler diagnostic to the person who asked for the program that produced it. Be plain and concrete; do not repeat the raw diagnostic text back verbatim.".to_string() },
        ChatMessage { role: "user", content: diagnostic.to_string() },
    ])
}

/// One full NL -> `.nir` -> `build` round trip, including the bounded
/// self-repair loop. Returns the last compiler diagnostic on failure
/// (so `:explain` has something to work with), `None` on success.
/// `on_log` replaces the direct `println!`/`eprintln!` calls this used
/// to make, so `tui` can route the exact same sequence of events into
/// its own styled transcript instead of them landing on stdout/stderr
/// underneath its raw-mode alternate screen.
pub(crate) fn generate_and_build(client: &LlmClient, request: &str, serve: bool, on_log: &dyn Fn(LogEvent)) -> Option<String> {
    let mut history = vec![ChatMessage { role: "system", content: NIR_SYSTEM_PROMPT.to_string() }, ChatMessage { role: "user", content: request.to_string() }];
    for attempt in 1..=MAX_SELF_REPAIR_ATTEMPTS {
        let source = match client.complete(&history) {
            Ok(s) => extract_nir_source(&s),
            Err(e) => {
                on_log(LogEvent::Error(format!("couldn't reach the model: {e}")));
                return None;
            }
        };
        match typecheck_and_build_to_temp_file(&source, serve) {
            Ok(path) => {
                on_log(LogEvent::Success(format!("built {path} (attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS})")));
                return None;
            }
            Err(diagnostic) => {
                if attempt == MAX_SELF_REPAIR_ATTEMPTS {
                    on_log(LogEvent::Error(format!("gave up after {MAX_SELF_REPAIR_ATTEMPTS} attempts -- last diagnostic:\n{diagnostic}")));
                    return Some(diagnostic);
                }
                record_self_repair_retry();
                on_log(LogEvent::Info(format!("attempt {attempt}/{MAX_SELF_REPAIR_ATTEMPTS} failed to compile, asking the model to fix it...")));
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
    // The opening fence can be *anywhere* in the response, not just at
    // its very start: models routinely lead with an "**Approach:**
    // ..." explanation before the fenced code (and often follow it
    // with a rules self-check afterward), despite being told to reply
    // with source only. Searching for "```" instead of requiring it as
    // a prefix finds that fence wherever it actually sits, and lets
    // whatever comes before it -- prose this function has no business
    // trying to compile -- be discarded along with everything after.
    if let Some(fence_start) = trimmed.find("```") {
        let rest = &trimmed[fence_start + 3..];
        // Whatever's left on the opening fence's own line is a
        // language tag ("nir", "nirdosha", or nothing) -- discarded
        // wholesale rather than prefix-matched against specific
        // spellings, which broke the moment a model chose a spelling
        // ("nirdosha") that has a *shorter* recognized tag ("nir") as
        // a literal prefix: `"nirdosha".strip_prefix("nir")` succeeds
        // (`Some("dosha")`), so the `or_else` alternative never even
        // ran, and "dosha" leaked into the extracted source as if it
        // were code.
        let after_info_string = match rest.find('\n') {
            Some(newline) => &rest[newline + 1..],
            None => rest,
        };
        // The *first* closing fence, not the last: text some models
        // add after the code block (an explanation, a reminder) can
        // itself contain a stray "```", which `rfind` would mistake
        // for where the source actually ends, pulling that trailing
        // prose into the "source" along with it.
        if let Some(end) = after_info_string.find("```") {
            return after_info_string[..end].trim().to_string();
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
/// `serve`'s own default port — a fixed local constant, not shared
/// with `main.rs::DEFAULT_SERVE_PORT`: that one lives in the `nirdosha`
/// *binary* crate, this module in the library half (same "two separate
/// compilation units, nothing to `use` across that boundary" reason
/// this function's own doc comment already gives) — kept at the same
/// value (`examples/features/51_compiled_serve.nir`'s own demo port)
/// purely for a consistent "if you don't say otherwise" default across
/// both entry points, not because anything enforces they match.
const HI_DEFAULT_SERVE_PORT: u16 = 8080;

fn typecheck_and_build_to_temp_file(source: &str, serve: bool) -> Result<String, String> {
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
    if serve {
        // The UI is generated *now*, at compile time, and baked into
        // the binary (`codegen::build_serve`'s own doc comment has the
        // full "a compiled process has no AST left at runtime" reason)
        // — same recipe `main.rs::cmd_build`'s own `--serve` branch
        // uses, demo mode only, deliberately (real production identity
        // flags for `--serve` are real, separate follow-up work, not
        // something `nirdosha hi` decides on its own either).
        let registry = crate::ast::TypeRegistry::build(&program);
        let effects = crate::effects::infer_effects(&program, &registry);
        let ui_html = crate::ui_gen::generate(&program, &effects, None, false, true, false, None).into_bytes();
        let opts = crate::codegen::ServeCodegenOptions { port: HI_DEFAULT_SERVE_PORT, ui_html };
        crate::codegen::build_serve(&program, &smt_report, &out_path, OptLevel::O2, &opts)?;
    } else {
        crate::codegen::build(&program, &smt_report, &out_path, OptLevel::O2)?;
    }
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
        assert_eq!(activation.timeout_secs, DEFAULT_PROVIDER_TIMEOUT_SECS);
    }

    #[test]
    fn timeout_override_applies_to_both_activation_paths() {
        let explicit = env_map(&[(PROVIDER_KEY_VAR, "sk-explicit"), (PROVIDER_MODEL_VAR, "custom-model"), (PROVIDER_TIMEOUT_SECS_VAR, "600")]);
        assert_eq!(resolve_activation(&explicit).expect("should activate").timeout_secs, 600);

        let openai = env_map(&[(OPENAI_KEY_VAR, "sk-openai"), (PROVIDER_TIMEOUT_SECS_VAR, "45")]);
        assert_eq!(resolve_activation(&openai).expect("should activate").timeout_secs, 45);
    }

    #[test]
    fn non_numeric_timeout_override_is_reported_precisely() {
        let env = env_map(&[(OPENAI_KEY_VAR, "sk-openai"), (PROVIDER_TIMEOUT_SECS_VAR, "soon")]);
        let err = resolve_activation(&env).unwrap_err();
        assert!(err.contains(PROVIDER_TIMEOUT_SECS_VAR) && err.contains("soon"), "error should name the var and the bad value, got: {err}");
    }

    #[test]
    fn token_usage_accumulates_across_calls() {
        let mut total = TokenUsage::default();
        total += TokenUsage { prompt_tokens: 120, completion_tokens: 340 };
        total += TokenUsage { prompt_tokens: 80, completion_tokens: 200 };
        assert_eq!(total.prompt_tokens, 200);
        assert_eq!(total.completion_tokens, 540);
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
        let raw = "```nir\nfn main() { }\n```";
        assert_eq!(extract_nir_source(raw), "fn main() { }");
    }

    #[test]
    fn extract_nir_source_passes_through_plain_text() {
        let raw = "fn main() { }";
        assert_eq!(extract_nir_source(raw), "fn main() { }");
    }

    #[test]
    fn extract_nir_source_strips_a_nirdosha_tagged_fence() {
        // Regression: "nirdosha" has "nir" as a literal prefix, which
        // used to make the old prefix-matching logic strip only "nir"
        // and leave "dosha" as stray text on the first line of the
        // "extracted" source.
        let raw = "```nirdosha\nfn main() { }\n```";
        assert_eq!(extract_nir_source(raw), "fn main() { }");
    }

    #[test]
    fn extract_nir_source_stops_at_the_first_closing_fence_not_the_last() {
        // Regression: trailing prose after the code block containing
        // its own "```" (a real response shape captured from a live
        // session's `.log`) used to make `rfind` pull that prose --
        // and the real closing fence before it -- into the "source".
        let raw = "```nirdosha\nfn main() { }\n```\n\nRemember to strip the ```nirdosha fence.";
        assert_eq!(extract_nir_source(raw), "fn main() { }");
    }

    #[test]
    fn extract_nir_source_finds_a_fence_that_is_not_at_the_very_start() {
        // Regression: a real captured session response (see a live
        // `nirdosha_hi_*.log`/`.nir` pair) led with a prose
        // "**Approach:** ..." paragraph before the fenced code, and
        // followed it with a "**Self-check against the rules:**"
        // section. The old code only ever looked for a fence via
        // `trimmed.strip_prefix("```")` -- since the response didn't
        // *start* with "```", that check never ran at all, and the
        // entire blob (both prose sections plus the fence markers
        // themselves) was written to the `.nir` file as if it were
        // all source, guaranteed to fail to lex.
        let raw = "**Approach:** I'll use a match expression.\n\n```nirdosha\nfn main() { }\n```\n\n**Self-check:** looks right, no `::` anywhere. ✓";
        assert_eq!(extract_nir_source(raw), "fn main() { }");
    }

    #[test]
    fn build_command_is_equivalent_to_a_plain_request() {
        assert!(matches!(parse_line("build me a calculator app"), Command::Request { description: "build me a calculator app", serve: false }));
        assert!(matches!(parse_line(":build build me a calculator app"), Command::Request { description: "build me a calculator app", serve: false }));
    }

    #[test]
    fn build_command_without_a_description_is_reported_not_silently_dropped() {
        assert!(matches!(parse_line(":build"), Command::Unknown(_)));
        assert!(matches!(parse_line(":build   "), Command::Unknown(_)));
    }

    #[test]
    fn build_prefixed_word_that_is_not_the_build_command_stays_unknown() {
        // `:buildfoo` must not be parsed as `:build` with description "foo".
        assert!(matches!(parse_line(":buildfoo"), Command::Unknown(":buildfoo")));
    }

    /// Reviving compiled `nirdosha serve` -- `serve` is `true` only via
    /// this exact explicit spelling, never inferred from the
    /// description's own wording.
    #[test]
    fn build_serve_command_requests_a_servable_build() {
        assert!(matches!(parse_line(":build --serve a task management app"), Command::Request { description: "a task management app", serve: true }));
    }

    #[test]
    fn build_serve_command_without_a_description_is_reported_not_silently_dropped() {
        assert!(matches!(parse_line(":build --serve"), Command::Unknown(_)));
        assert!(matches!(parse_line(":build --serve   "), Command::Unknown(_)));
    }

    #[test]
    fn plain_request_never_infers_serve_from_its_own_wording() {
        assert!(matches!(parse_line("build me a servable web app"), Command::Request { serve: false, .. }));
    }

    #[test]
    fn ask_command_carries_its_question() {
        assert!(matches!(parse_line(":ask what does R17 say?"), Command::Ask("what does R17 say?")));
    }

    #[test]
    fn ask_command_without_a_question_is_reported_not_silently_dropped() {
        assert!(matches!(parse_line(":ask"), Command::Unknown(_)));
        assert!(matches!(parse_line(":ask   "), Command::Unknown(_)));
    }

    #[test]
    fn ask_prefixed_word_that_is_not_the_ask_command_stays_unknown() {
        assert!(matches!(parse_line(":askew"), Command::Unknown(":askew")));
    }

    #[test]
    fn impact_command_carries_its_target() {
        assert!(matches!(parse_line(":impact R17"), Command::Impact("R17")));
        assert!(matches!(parse_line(":impact fn:transfer_funds"), Command::Impact("fn:transfer_funds")));
    }

    #[test]
    fn impact_command_without_a_target_is_reported_not_silently_dropped() {
        assert!(matches!(parse_line(":impact"), Command::Unknown(_)));
        assert!(matches!(parse_line(":impact   "), Command::Unknown(_)));
    }

    /// A private per-test path, not `failure_counts_path()`'s shared
    /// machine-wide file -- parallel test threads writing the real one
    /// at once would flake on each other's counts.
    fn scratch_counts_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("nirdosha_hi_test_failure_counts_{name}_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn missing_failure_counts_file_loads_as_zeroed() {
        let path = scratch_counts_path("missing");
        let counters = load_failure_counters_from(&path);
        assert_eq!(counters.self_repair_retries, 0);
        assert_eq!(counters.compiler_panics, 0);
    }

    #[test]
    fn failure_counts_round_trip_through_disk() {
        let path = scratch_counts_path("roundtrip");
        save_failure_counters_to(&path, &FailureCounters { self_repair_retries: 3, compiler_panics: 1 });
        let loaded = load_failure_counters_from(&path);
        assert_eq!(loaded.self_repair_retries, 3);
        assert_eq!(loaded.compiler_panics, 1);
    }

    #[test]
    fn corrupt_failure_counts_file_degrades_to_zeroed_rather_than_erroring() {
        let path = scratch_counts_path("corrupt");
        std::fs::write(&path, "not json").unwrap();
        let counters = load_failure_counters_from(&path);
        assert_eq!(counters.self_repair_retries, 0);
        assert_eq!(counters.compiler_panics, 0);
    }

    #[test]
    fn recording_increments_persist_and_accumulate() {
        let path = scratch_counts_path("increment");
        let mut counters = load_failure_counters_from(&path);
        counters.self_repair_retries += 1;
        save_failure_counters_to(&path, &counters);
        counters = load_failure_counters_from(&path);
        counters.self_repair_retries += 1;
        counters.compiler_panics += 1;
        save_failure_counters_to(&path, &counters);
        let loaded = load_failure_counters_from(&path);
        assert_eq!(loaded.self_repair_retries, 2);
        assert_eq!(loaded.compiler_panics, 1);
    }
}
