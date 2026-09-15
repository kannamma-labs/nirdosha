//! A small persistent, *empirically-gated* cache of corrective self-
//! repair hints, for diagnostic patterns `hi_llm.rs`'s hand-authored
//! `self_repair_hint` table doesn't (yet) cover.
//!
//! Context (2026-09-14 field failure): a fintech `:generate` run burned
//! all `MAX_SELF_REPAIR_ATTEMPTS` on `acquire fn("Role")`/`EnumName.
//! Variant` misuse -- two diagnostic shapes `self_repair_hint` had no
//! arm for, so every retry got back the bare compiler diagnostic with
//! an empty hint appended and repeated the identical mistake. Adding a
//! hand-authored arm per field failure (the existing discipline --
//! every arm's `// Field failure <date>` comment) is real but reactive:
//! it only grows after a human notices a give-up, reads the log, and
//! ships a patched arm. This module closes that gap *live*, inside the
//! self-repair loop itself, without giving up the safety a hand-
//! authored, test-pinned arm has:
//!
//! - `self_repair_hint`'s table is still consulted FIRST for every
//!   diagnostic pattern (`hi_llm.rs::corrective_hint_for`) -- free,
//!   deterministic, code-reviewed, pinned by
//!   `self_repair_hints_cover_every_field_failure_class`. This cache is
//!   only ever a fallback for what that table misses.
//! - A miss triggers ONE synthesis call to the model itself
//!   (`hi_llm.rs::synthesize_hints`), grounded in the real language
//!   docs -- but that synthesized hint is *provisional*, never written
//!   here directly. It rides along into the very next self-repair
//!   attempt un-cached.
//! - Only after that next attempt's diagnostics no longer contain the
//!   pattern the hint was for (`hi_llm.rs::promote_validated_hints`) is
//!   the hint considered PROVEN and written here. A hint that doesn't
//!   clear the pattern -- wrong, misleading, or just unhelpful -- is
//!   silently dropped and re-synthesized fresh next time, never
//!   persisted.
//!
//! So every entry that ever lands in this file carries real evidence it
//! worked at least once, which is a different (weaker, but non-zero)
//! guarantee than a hand-authored arm's code review + regression test
//! -- worth keeping visibly separate, hence a distinct file and a
//! distinct, append-only audit log (`promotions.log` next to the cache
//! file) rather than folding straight into `self_repair_hint`'s own
//! match arms.
//!
//! Scoped globally (one file under the user's home directory), not per
//! project: a hint about how `acquire`/`RoleView` actually work is a
//! fact about the LANGUAGE, true in every project, and
//! `generate_from_task_prompt` (the bench harness) has no project
//! directory at all to scope one to.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// If `s` starts with a bare `<line>:<col>: ` location prefix (both
/// fields plain ASCII digits, a real column present, not `N::`), returns
/// everything after it; `None` for anything else, including a message
/// with no location at all -- the caller decides what "no match" means.
fn try_strip_leading_line_col(s: &str) -> Option<&str> {
    let mut chars = s.char_indices();
    let mut first_colon = None;
    let mut second_colon = None;
    for (i, c) in chars.by_ref() {
        if c == ':' {
            first_colon = Some(i);
            break;
        }
        if !c.is_ascii_digit() {
            return None;
        }
    }
    let fc = first_colon?;
    if fc == 0 {
        return None;
    }
    for (i, c) in chars.by_ref() {
        if c == ':' {
            second_colon = Some(i);
            break;
        }
        if !c.is_ascii_digit() {
            return None;
        }
    }
    let sc = second_colon?;
    if sc == fc + 1 {
        // `N::` -- an empty column field, not the `N:N:` shape this
        // targets. Leave it alone rather than guess.
        return None;
    }
    Some(s[sc + 1..].trim_start())
}

/// Strips a `<line>:<col>: ` location marker, if present, so the same
/// underlying mistake at two different call sites (`16:58: ...` vs
/// `26:62: ...`) normalizes to one cache key. Mirrors what
/// `self_repair_hint`'s own `.contains(...)` substring checks already
/// do implicitly by matching text *after* the location -- this just
/// makes that same location-independence explicit for a key used in
/// exact lookups, where a substring match isn't available.
///
/// Checks two shapes, in order:
/// 1. The message starts with the bare marker (`typecheck`/`ownership`
///    diagnostics already look like this -- `"16:58: expected ..."`).
/// 2. The message contains `" at <line>:<col>: "` anywhere, and
///    everything after it is stripped (a lex/parse error's raw text
///    looks like `"lex error in <path> at 28:69: unexpected character
///    ...\`"` -- issue #64: the `<path>` component is a per-self-repair-
///    attempt-unique scratch file, so leaving it in the cache key meant
///    a lex/parse-stage hint could never be looked up again, and
///    `promote_validated_hints`'s "did this fix it" check was
///    comparing two paths that always differ regardless of whether the
///    actual mistake was fixed). Splits on the FIRST `" at "`, not the
///    last -- same reasoning as `hi_llm.rs::first_span_in`: a message
///    whose own prose or a quoted path contains a *later* `" at "` must
///    not steal this one.
///
/// A message matching neither shape (no location at all -- e.g.
/// `"codegen doesn't support \`print\` on a Vector argument"`) is
/// returned unchanged: it has nothing volatile to strip, and is already
/// a stable, reusable cache key as-is.
pub fn normalize_pattern(message: &str) -> String {
    let trimmed = message.trim();
    if let Some(stripped) = try_strip_leading_line_col(trimmed) {
        return stripped.to_string();
    }
    if let Some((_, rest)) = trimmed.split_once(" at ") {
        if let Some(stripped) = try_strip_leading_line_col(rest) {
            return stripped.to_string();
        }
    }
    trimmed.to_string()
}

/// Where the cache (and its sibling audit log) lives. Overridable via
/// `NIRDOSHA_HINT_CACHE_PATH` so tests, and the bench harness, never
/// touch a real developer's `$HOME` -- unset, corrupt, or unreadable
/// all fall back to a fresh, empty, process-local cache rather than
/// erroring: this is a nice-to-have accelerator for the self-repair
/// loop, never a dependency the loop can fail over.
fn cache_path() -> PathBuf {
    if let Ok(p) = std::env::var("NIRDOSHA_HINT_CACHE_PATH") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    PathBuf::from(home).join(".nirdosha").join("self_repair_hint_cache.json")
}

fn log_path(cache: &std::path::Path) -> PathBuf {
    cache.with_file_name("self_repair_hint_promotions.log")
}

/// The cache itself: pattern -> hint. Kept as a plain `HashMap` in
/// memory for the lifetime of one generate call; `load`/`save` are the
/// only points that touch disk, both best-effort.
pub struct HintCache {
    path: PathBuf,
    entries: HashMap<String, String>,
}

impl HintCache {
    /// Never fails: a missing, corrupt, or unreadable file all resolve
    /// to an empty cache (with a log line explaining which, for
    /// visibility) -- the self-repair loop must never be blocked by
    /// this being unavailable.
    pub fn load(on_log: &mut dyn FnMut(&str)) -> Self {
        let path = cache_path();
        let entries = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<HashMap<String, String>>(&text) {
                Ok(map) => map,
                Err(e) => {
                    on_log(&format!("hint cache at {} is not valid JSON ({e}) -- starting empty", path.display()));
                    HashMap::new()
                }
            },
            Err(_) => HashMap::new(),
        };
        HintCache { path, entries }
    }

    pub fn lookup(&self, pattern: &str) -> Option<&str> {
        self.entries.get(pattern).map(String::as_str)
    }

    /// Writes the hint in (only reachable after the caller has already
    /// confirmed it cleared the pattern on a real attempt -- see the
    /// module doc comment) and persists immediately: a long-running
    /// `hi_window`/`hi_api` process serves many independent generate
    /// calls, and a promotion from one should be visible to the next
    /// without restarting. Write-temp-then-rename keeps a concurrent
    /// reader (another in-flight generate call loading its own
    /// `HintCache`) from ever observing a half-written file; two
    /// concurrent promotions can still race and one can lose its entry
    /// to the other's overwrite of the whole map, which is fine -- the
    /// losing entry just gets re-synthesized and re-proven next time it
    /// comes up, the same as any other cache miss.
    pub fn record_success(&mut self, pattern: &str, hint: &str) {
        self.entries.insert(pattern.to_string(), hint.to_string());
        if let Some(parent) = self.path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let Ok(json) = serde_json::to_string_pretty(&self.entries) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, &json).is_err() {
            return;
        }
        let _ = std::fs::rename(&tmp, &self.path);
        // Best-effort audit trail: unlike a hand-authored `self_repair_
        // hint` arm, nobody reviewed this hint before it started being
        // served to future runs -- an append-only, human-readable log
        // of every promotion (pattern + the hint text + when) is the
        // minimum needed for someone to periodically skim what this
        // loop has been teaching itself and spot a bad one.
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path(&self.path)) {
            let _ = writeln!(f, "{{\"promoted_at\": {}, \"pattern\": {}, \"hint\": {}}}", now_unix(), json_escape(pattern), json_escape(hint));
        }
    }
}

/// Process-wide, lazily-loaded-once shared cache. `hi_window.rs` runs
/// every `:generate`/`:prompt` call on its own thread inside one long-
/// running process, so this module's own promise -- a promotion from
/// one call is visible to the next without restarting -- previously
/// still meant every call re-read and re-parsed the same file from
/// disk from scratch (`hi_llm.rs`'s `generate_program`/
/// `generate_from_task_prompt` each called `HintCache::load` fresh).
/// A `Mutex`-guarded singleton gives that same visibility within one
/// process for free, and turns "load from disk" into a once-per-
/// process cost instead of a once-per-call one. `on_log` is only
/// consulted by whichever call actually wins the race to initialize
/// it (a corrupt-file warning, say) -- later callers share the
/// already-loaded cache silently, same as any other `OnceLock`.
///
/// Deliberately not used by this module's own tests, which each want
/// a fresh, isolated cache scoped to their own `NIRDOSHA_HINT_CACHE_
/// PATH` -- a `shared` singleton loaded once per process would ignore
/// that env var for every test after the first one to touch it. Tests
/// construct their own local `Mutex::new(HintCache::load(...))`
/// instead; `corrective_hint_for`/`promote_validated_hints` only need
/// `&Mutex<HintCache>`, not `&'static`, so both shapes work.
pub fn shared(on_log: &mut dyn FnMut(&str)) -> &'static Mutex<HintCache> {
    static CACHE: OnceLock<Mutex<HintCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HintCache::load(on_log)))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn json_escape(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// A lesson learned from a REAL, observed runtime incident (an
/// isolation anomaly, a crossed NFR threshold) -- Phase 4 item 2 of
/// `docs/research/2026-09-pending-verification-differentiation-
/// work.md`: "a production runtime violation should be able to become
/// a taught lesson for `hi_llm`'s self-repair loop the same way a
/// `:generate`-time failure already does." Built 2026-09-15.
///
/// **A structurally different mechanism from `HintCache` above, on
/// purpose, not a missed chance to reuse it.** `HintCache` is keyed by
/// COMPILER DIAGNOSTIC TEXT, consulted only when a real compile attempt
/// fails with that exact diagnostic, and a hint only ever gets
/// persisted after proving it clears that diagnostic on the very next
/// attempt (`promote_validated_hints`). A runtime incident has no
/// diagnostic text at all -- the code compiled and ran; the bug showed
/// up in *behavior*, not in `nirdosha build`'s own output -- and there
/// is no "next compile attempt" to prove a lesson against. So this is
/// keyed by a small, fixed incident-kind vocabulary instead
/// (`"transact_isolation_anomaly"`, `"nfr_drift"`, ...; see
/// `main.rs::cmd_check_isolation`/`cmd_check_drift`'s own `--teach`
/// flag, the only writer), consulted PROACTIVELY at prompt-
/// construction time rather than reactively after a compile failure
/// (`hi_llm.rs::generate_from_task_prompt`'s own call site has the
/// real, disclosed matching rule -- task-prompt keyword matching, a
/// real v1 simplification, not AST-shape similarity, which is a
/// bigger, separate problem this pass doesn't attempt), and written
/// only on deliberate human/operator confirmation via `--teach`, never
/// automatically -- there is no automatic "proof" step for a runtime
/// lesson the way there is for a compile-diagnostic one, so promotion
/// requires an explicit act instead. Kept in its own sibling file
/// (`self_repair_runtime_lessons.json`), not folded into `HintCache`'s
/// own cache file, for that same "visibly different provenance,
/// visibly separate store" reason `promotions.log` is already a
/// sibling file rather than inline.
pub struct RuntimeLessons {
    path: PathBuf,
    entries: HashMap<String, String>,
}

fn runtime_lessons_path() -> PathBuf {
    if let Ok(p) = std::env::var("NIRDOSHA_RUNTIME_LESSONS_PATH") {
        return PathBuf::from(p);
    }
    cache_path().with_file_name("self_repair_runtime_lessons.json")
}

impl RuntimeLessons {
    /// Same "never fails" contract as `HintCache::load` -- a missing,
    /// corrupt, or unreadable file resolves to an empty store rather
    /// than blocking anything that reads from it.
    pub fn load() -> Self {
        let path = runtime_lessons_path();
        let entries = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str::<HashMap<String, String>>(&text).ok()).unwrap_or_default();
        RuntimeLessons { path, entries }
    }

    pub fn lookup(&self, incident_kind: &str) -> Option<&str> {
        self.entries.get(incident_kind).map(String::as_str)
    }

    /// Writes (or overwrites) the lesson for `incident_kind` and
    /// persists immediately -- same write-temp-then-rename discipline
    /// as `HintCache::record_success`, same reasoning (a concurrent
    /// reader never observes a half-written file). Overwriting is
    /// deliberate, not a limitation: this store's own granularity is
    /// coarse by design (a handful of incident kinds, not one entry per
    /// function or per anomaly), so a fresh `--teach` for the same kind
    /// is meant to replace, not accumulate.
    pub fn record(&mut self, incident_kind: &str, hint: &str) {
        self.entries.insert(incident_kind.to_string(), hint.to_string());
        if let Some(parent) = self.path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let Ok(json) = serde_json::to_string_pretty(&self.entries) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, &json).is_err() {
            return;
        }
        let _ = std::fs::rename(&tmp, &self.path);
    }
}

/// Process-wide, lazily-loaded-once shared store -- same reasoning as
/// `shared()` above (one load-from-disk per process, not per call).
/// Separate `OnceLock` from `shared()`'s own: the two stores have
/// independent lifetimes and independent env-var overrides
/// (`NIRDOSHA_RUNTIME_LESSONS_PATH` vs `NIRDOSHA_HINT_CACHE_PATH`), and
/// tests want to construct their own local, isolated instance the same
/// way `HintCache`'s own tests do rather than share this singleton.
pub fn shared_runtime_lessons() -> &'static Mutex<RuntimeLessons> {
    static LESSONS: OnceLock<Mutex<RuntimeLessons>> = OnceLock::new();
    LESSONS.get_or_init(|| Mutex::new(RuntimeLessons::load()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pattern_strips_line_col_prefix() {
        assert_eq!(normalize_pattern("16:58: expected `RoleView`, found `str`"), "expected `RoleView`, found `str`");
        assert_eq!(normalize_pattern("145:33: expected `fn() -> i64`, found `Result(fn() -> i64, str)`"), "expected `fn() -> i64`, found `Result(fn() -> i64, str)`");
    }

    #[test]
    fn normalize_pattern_leaves_prefix_free_messages_alone() {
        assert_eq!(normalize_pattern("codegen doesn't support `print` on a Vector argument"), "codegen doesn't support `print` on a Vector argument");
        // A leading word that merely contains digits/colons in an
        // unrelated shape must not be mistaken for a location prefix.
        assert_eq!(normalize_pattern("unknown variable `RequestStatus`"), "unknown variable `RequestStatus`");
    }

    /// Issue #64: a lex/parse error's raw text is `"lex error in <path>
    /// at <line>:<col>: <description>"` -- the leading digit-prefix
    /// check alone can't strip this (the string starts with `l`, not a
    /// digit), so before this fix the embedded scratch path (unique on
    /// every self-repair attempt, `hi_llm.rs::typecheck_and_build_check`'s
    /// own `SCRATCH_COUNTER`) rode straight through into the cache key,
    /// meaning the SAME real mistake at two different attempts/runs
    /// normalized to two DIFFERENT patterns and could never share a
    /// cache entry. This pins that two such messages, differing only in
    /// their embedded path, now normalize identically.
    #[test]
    fn normalize_pattern_strips_a_lex_error_prefix_regardless_of_the_embedded_scratch_path() {
        let a = normalize_pattern("lex error in /tmp/nirdosha_hi_generate_check_2739298_3.nir at 28:69: unexpected character `?`");
        let b = normalize_pattern("lex error in /tmp/nirdosha_hi_generate_check_9911205_0.nir at 3:1: unexpected character `?`");
        assert_eq!(a, "unexpected character `?`", "got: {a}");
        assert_eq!(a, b, "the same underlying mistake at two different scratch paths must normalize to one cache key");
    }

    /// Same shape for `parse error in <path> at ...`, not just `lex
    /// error in <path> at ...` -- both loader error kinds share this
    /// text convention (`hi_llm.rs::typecheck_and_build_check`'s own
    /// error-mapping closure builds both from the same `&e`).
    #[test]
    fn normalize_pattern_strips_a_parse_error_prefix_regardless_of_the_embedded_scratch_path() {
        let a = normalize_pattern("parse error in /tmp/nirdosha_hi_generate_check_111_1.nir at 154:19: expected an expression, found the reserved keyword `return`");
        let b = normalize_pattern("parse error in /tmp/nirdosha_hi_generate_check_222_7.nir at 9:4: expected an expression, found the reserved keyword `return`");
        assert_eq!(a, "expected an expression, found the reserved keyword `return`", "got: {a}");
        assert_eq!(a, b);
    }

    /// The "first ` at `, not the last" rule matters: a message whose
    /// own description contains a later `" at "` (quoted prose, a
    /// second location mentioned in passing) must still resolve using
    /// the FIRST one, matching `hi_llm.rs::first_span_in`'s identical
    /// reasoning for the identical raw text.
    #[test]
    fn normalize_pattern_uses_the_first_at_marker_not_a_later_one() {
        let got = normalize_pattern("lex error in /tmp/x_1.nir at 5:2: unexpected token, expected the keyword at line 9");
        assert_eq!(got, "unexpected token, expected the keyword at line 9");
    }

    #[test]
    fn record_success_then_lookup_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("nir_hint_cache_test_{}", std::process::id()));
        let path = dir.join("cache.json");
        // SAFETY (test-only): `HintCache` reads this env var once per
        // `cache_path()` call; scoping every read to right after the
        // write and inside this single test keeps it from racing other
        // tests in this same process, which `cargo test`'s default
        // multi-threaded runner would otherwise allow.
        unsafe { std::env::set_var("NIRDOSHA_HINT_CACHE_PATH", &path) };
        let mut log = |_: &str| {};
        let mut cache = HintCache::load(&mut log);
        assert_eq!(cache.lookup("expected `RoleView`, found `str`"), None);
        cache.record_success("expected `RoleView`, found `str`", "use check_role, not a string literal");
        let reloaded = HintCache::load(&mut log);
        assert_eq!(reloaded.lookup("expected `RoleView`, found `str`"), Some("use check_role, not a string literal"));
        unsafe { std::env::remove_var("NIRDOSHA_HINT_CACHE_PATH") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runtime_lessons_record_then_lookup_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("nir_runtime_lessons_test_{}", std::process::id()));
        let path = dir.join("runtime_lessons.json");
        // SAFETY (test-only): same single-test-scoped env var discipline
        // `record_success_then_lookup_round_trips_through_disk` above
        // already uses for `HintCache`'s own env var.
        unsafe { std::env::set_var("NIRDOSHA_RUNTIME_LESSONS_PATH", &path) };
        let mut lessons = RuntimeLessons::load();
        assert_eq!(lessons.lookup("transact_isolation_anomaly"), None);
        lessons.record("transact_isolation_anomaly", "add a serializing guard around this transact's commit slot");
        let reloaded = RuntimeLessons::load();
        assert_eq!(reloaded.lookup("transact_isolation_anomaly"), Some("add a serializing guard around this transact's commit slot"));
        unsafe { std::env::remove_var("NIRDOSHA_RUNTIME_LESSONS_PATH") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runtime_lessons_record_overwrites_the_same_incident_kind() {
        let dir = std::env::temp_dir().join(format!("nir_runtime_lessons_overwrite_test_{}", std::process::id()));
        let path = dir.join("runtime_lessons.json");
        unsafe { std::env::set_var("NIRDOSHA_RUNTIME_LESSONS_PATH", &path) };
        let mut lessons = RuntimeLessons::load();
        lessons.record("nfr_drift", "first lesson");
        lessons.record("nfr_drift", "second, updated lesson");
        assert_eq!(lessons.lookup("nfr_drift"), Some("second, updated lesson"), "a fresh --teach for the same incident kind must replace, not accumulate");
        unsafe { std::env::remove_var("NIRDOSHA_RUNTIME_LESSONS_PATH") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runtime_lessons_load_is_never_fails_on_a_missing_or_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("nir_runtime_lessons_missing_test_{}", std::process::id()));
        unsafe { std::env::set_var("NIRDOSHA_RUNTIME_LESSONS_PATH", dir.join("does_not_exist.json")) };
        let missing = RuntimeLessons::load();
        assert_eq!(missing.lookup("anything"), None);

        std::fs::create_dir_all(&dir).unwrap();
        let corrupt_path = dir.join("corrupt.json");
        std::fs::write(&corrupt_path, "not valid json at all").unwrap();
        unsafe { std::env::set_var("NIRDOSHA_RUNTIME_LESSONS_PATH", &corrupt_path) };
        let corrupt = RuntimeLessons::load();
        assert_eq!(corrupt.lookup("anything"), None, "a corrupt file must resolve to an empty store, not a panic");

        unsafe { std::env::remove_var("NIRDOSHA_RUNTIME_LESSONS_PATH") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
