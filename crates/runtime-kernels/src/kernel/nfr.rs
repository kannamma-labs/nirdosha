//! Non-functional requirements as a first-class, `nfr(...)`-declared
//! language feature — the "NFRs-as-language" half of
//! `rfcs/0007-apm-runtime-kernel.md`'s own title, genuinely unbuilt
//! until now (that RFC's own §9 parks NFR governance as open work with
//! no concrete design). `nfr(latency_ms: 100, error_rate_max: 0.01,
//! throughput_min_per_sec: 50, concurrency_max: 10)` on a `fn`
//! declaration (`ast::NfrSpec`) makes every call to that function
//! tracked automatically — no separate opt-in call, the same "hidden
//! behind a keyword" pattern every other kernel feature this project
//! has added this cycle already uses.
//!
//! **Four real, disclosed simplifications versus a full APM system —
//! see `ast::NfrSpec`'s own doc comment for the honest reasoning behind
//! each one**: `latency_ms` is a per-call max, not a true p99;
//! `error_rate_max`/`throughput_min_per_sec` are cumulative-since-start,
//! not a sliding window; only `concurrency_max` is exact. All four are
//! O(1) state per tracked function — no histogram, no ring buffer.
//!
//! **Escalation, only on violation, never on the hot path.** A
//! violated threshold fires one JSON `POST` to `NIRDOSHA_OBSERVABILITY_
//! URL` (unset — the default — means NFRs are still tracked locally,
//! nothing ever touches the network) — on `thread_pool::ThreadPool`, the
//! same "never block the caller" discipline `recorder::flush_async`
//! already established for the flight recorder. Plain `std::net::
//! TcpStream` and a hand-written HTTP/1.1 request line, not a routed
//! call through `nir_tcp_*` (this is the kernel's own internal
//! telemetry, not a `.nir`-visible resource — the same reason
//! `recorder.rs`'s own file writes don't go through `nir_file_*`
//! either) and not a real HTTP/JSON/OTLP client library (a real
//! dependency this project has deliberately avoided all cycle — see
//! `docs/PUBLIC_ROADMAP.md`'s still-open `http`/`https` Track B item).
//! No TLS, no retry, no batching, no debouncing — a single plaintext
//! `POST` per violation, fire-and-forget.

use super::thread_pool::ThreadPool;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// A sample smaller than this is too noisy to judge an error *rate*
/// from — one early failure out of one call would otherwise read as a
/// "100% error rate" violation on the very first call.
const MIN_SAMPLE_FOR_ERROR_RATE: u64 = 10;
/// Same reasoning for throughput: an average taken over the first
/// fraction of a second is dominated by startup noise, not a real rate.
const MIN_ELAPSED_SECS_FOR_THROUGHPUT: f64 = 1.0;

struct NfrTracker {
    name: String,
    latency_ms: Option<i64>,
    error_rate_max: Option<f64>,
    throughput_min_per_sec: Option<i64>,
    concurrency_max: Option<i64>,
    in_flight: AtomicI64,
    total_calls: AtomicU64,
    total_errors: AtomicU64,
    first_call_at: OnceLock<Instant>,
}

// A `Mutex<Vec<_>>` for both registration (once per tracked fn, at
// process start, single-threaded — `emit_c_main`'s own prologue) and
// every later `call_begin`/`call_end` lookup by index. Real, small lock
// overhead on every tracked call, deliberately not optimized away here:
// `nfr(...)` is opt-in per function, so this cost is scoped to exactly
// the functions a `.nir` author asked to monitor, not paid by every
// call in the program — a reasonable place to stop for a first slice.
static TRACKERS: OnceLock<Mutex<Vec<NfrTracker>>> = OnceLock::new();
fn trackers() -> &'static Mutex<Vec<NfrTracker>> {
    TRACKERS.get_or_init(|| Mutex::new(Vec::new()))
}

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
fn process_start() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

/// Registers one `nfr(...)`-declared function, called exactly once per
/// such function, at program start (`codegen.rs` emits this into
/// `emit_c_main`'s own prologue, before the `.nir` program's own `main`
/// runs). A negative `i64` field / negative `f64` field means "this NFR
/// wasn't declared" — the same sentinel-for-absence convention already
/// used at this ABI boundary elsewhere (`nir_tcp_connect`'s `-1`, etc.),
/// chosen here so `codegen.rs` never needs a variable-arity call for a
/// variable-arity set of declared fields. Returns the id every later
/// `call_begin`/`call_end` for this function passes back.
pub fn register(name: String, latency_ms: i64, error_rate_max: f64, throughput_min_per_sec: i64, concurrency_max: i64) -> i64 {
    let mut t = trackers().lock().unwrap();
    let id = t.len() as i64;
    t.push(NfrTracker {
        name,
        latency_ms: (latency_ms >= 0).then_some(latency_ms),
        error_rate_max: (error_rate_max >= 0.0).then_some(error_rate_max),
        throughput_min_per_sec: (throughput_min_per_sec >= 0).then_some(throughput_min_per_sec),
        concurrency_max: (concurrency_max >= 0).then_some(concurrency_max),
        in_flight: AtomicI64::new(0),
        total_calls: AtomicU64::new(0),
        total_errors: AtomicU64::new(0),
        first_call_at: OnceLock::new(),
    });
    id
}

/// Call immediately before a tracked function's own body runs. Returns
/// a start timestamp (nanoseconds since this process's own first
/// `nfr`-related call, an arbitrary but fixed and monotonic epoch — the
/// only thing that matters is that the same value comes back out of
/// `call_end` unchanged) for `call_end` to compute elapsed latency from.
pub fn call_begin(id: i64) -> i64 {
    let start_ns = process_start().elapsed().as_nanos() as i64;
    let t = trackers().lock().unwrap();
    if let Some(tracker) = t.get(id as usize) {
        let in_flight = tracker.in_flight.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(max) = tracker.concurrency_max
            && in_flight > max
        {
            let name = tracker.name.clone();
            drop(t);
            escalate(name, "concurrency_max", max as f64, in_flight as f64);
        }
    }
    start_ns
}

/// Call immediately after a tracked function's own body finishes
/// (every return path — `codegen.rs`'s own doc comment on where these
/// calls land). `was_err` is only ever meaningfully `true` for a
/// `Result(_, _)`-returning function whose `nfr(...)` declares
/// `error_rate_max` (`typeck.rs` rejects declaring it on any other
/// return type) — every other caller passes `false` unconditionally,
/// which is exactly correct: there is no "error" to record.
pub fn call_end(id: i64, start_ns: i64, was_err: bool) {
    let end_ns = process_start().elapsed().as_nanos() as i64;
    let elapsed_ms = (end_ns - start_ns).max(0) / 1_000_000;

    // Collect at most one violation per NFR kind under the lock (cheap:
    // a few atomic ops and a couple of field reads), then drop it before
    // ever touching the network — `escalate`'s own doc comment on why.
    let mut violations: Vec<(String, &'static str, f64, f64)> = Vec::new();
    {
        let t = trackers().lock().unwrap();
        let Some(tracker) = t.get(id as usize) else { return };
        tracker.in_flight.fetch_sub(1, Ordering::Relaxed);
        let total_calls = tracker.total_calls.fetch_add(1, Ordering::Relaxed) + 1;
        let total_errors = if was_err { tracker.total_errors.fetch_add(1, Ordering::Relaxed) + 1 } else { tracker.total_errors.load(Ordering::Relaxed) };
        let first_call_at = *tracker.first_call_at.get_or_init(Instant::now);

        if let Some(max_ms) = tracker.latency_ms
            && elapsed_ms > max_ms
        {
            violations.push((tracker.name.clone(), "latency_ms", max_ms as f64, elapsed_ms as f64));
        }
        if let Some(max_rate) = tracker.error_rate_max
            && total_calls >= MIN_SAMPLE_FOR_ERROR_RATE
        {
            let rate = total_errors as f64 / total_calls as f64;
            if rate > max_rate {
                violations.push((tracker.name.clone(), "error_rate_max", max_rate, rate));
            }
        }
        if let Some(min_rate) = tracker.throughput_min_per_sec {
            let elapsed_secs = first_call_at.elapsed().as_secs_f64();
            if elapsed_secs >= MIN_ELAPSED_SECS_FOR_THROUGHPUT {
                let rate = total_calls as f64 / elapsed_secs;
                if rate < min_rate as f64 {
                    violations.push((tracker.name.clone(), "throughput_min_per_sec", min_rate as f64, rate));
                }
            }
        }
    }
    for (name, kind, threshold, actual) in violations {
        escalate(name, kind, threshold, actual);
    }
}

/// `NIRDOSHA_OBSERVABILITY_URL`, parsed once and cached — plain
/// `http://host[:port][/path]`, no TLS (the same limitation `connect()`
/// already has). Unset, or unparseable, means escalation is silently a
/// no-op forever — NFRs are still tracked locally either way, the
/// "fail open, telemetry never breaks the program" principle RFC 0007
/// §6 already states for the flight recorder, applied here too.
fn observability_target() -> Option<&'static (String, u16, String)> {
    static TARGET: OnceLock<Option<(String, u16, String)>> = OnceLock::new();
    TARGET.get_or_init(|| std::env::var("NIRDOSHA_OBSERVABILITY_URL").ok().and_then(|raw| parse_http_url(&raw))).as_ref()
}

fn parse_http_url(raw: &str) -> Option<(String, u16, String)> {
    let rest = raw.strip_prefix("http://")?;
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match hostport.split_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok()?),
        None => (hostport, 80),
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port, path))
}

fn escalation_pool() -> &'static std::sync::Arc<ThreadPool> {
    static POOL: OnceLock<std::sync::Arc<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(ThreadPool::new)
}

/// Fires one escalation, asynchronously — never on the calling thread,
/// so a slow or unreachable observability server can never add latency
/// to the very call whose latency it's being told about. A no-op,
/// immediately, if no `NIRDOSHA_OBSERVABILITY_URL` is configured —
/// checked *before* touching the thread pool, so the common "not
/// configured" case costs one cached env lookup, not a job submission.
fn escalate(fn_name: String, nfr_kind: &'static str, threshold: f64, actual: f64) {
    let Some((host, port, path)) = observability_target() else { return };
    let host = host.clone();
    let port = *port;
    let path = path.clone();
    let _ = escalation_pool().submit(Box::new(move || {
        send_escalation(&host, port, &path, &fn_name, nfr_kind, threshold, actual);
    }));
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// The actual network call — a hand-written HTTP/1.1 request, not a
/// library, for a fixed, tiny, known JSON shape (this module's own doc
/// comment explains why no HTTP/JSON client is linked in for this).
/// Best-effort: any failure (refused connection, write error, timeout)
/// is silently swallowed, the same fail-open posture every other
/// telemetry path in this crate already takes — a broken observability
/// server must never be the reason a `.nir` program itself fails.
fn send_escalation(host: &str, port: u16, path: &str, fn_name: &str, nfr_kind: &str, threshold: f64, actual: f64) {
    use std::io::Write;
    let body = format!(
        r#"{{"function":"{}","nfr":"{}","threshold":{},"actual":{},"timestamp_ms":{}}}"#,
        json_escape(fn_name),
        nfr_kind,
        threshold,
        actual,
        unix_time_ms()
    );
    let request = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n\
         {body}",
        body.len()
    );
    if let Ok(mut stream) = std::net::TcpStream::connect((host, port)) {
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(2)));
        let _ = stream.write_all(request.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_http_url_handles_host_port_and_path() {
        assert_eq!(parse_http_url("http://localhost:9000/ingest"), Some(("localhost".to_string(), 9000, "/ingest".to_string())));
        assert_eq!(parse_http_url("http://example.com"), Some(("example.com".to_string(), 80, "/".to_string())));
        assert_eq!(parse_http_url("http://example.com/"), Some(("example.com".to_string(), 80, "/".to_string())));
        assert_eq!(parse_http_url("https://example.com"), None, "no TLS support -- an https:// URL must not be silently treated as http");
        assert_eq!(parse_http_url("not a url"), None);
        assert_eq!(parse_http_url("http://"), None);
    }

    #[test]
    fn json_escape_handles_quotes_and_backslashes() {
        assert_eq!(json_escape(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(json_escape(r"back\slash"), r"back\\slash");
        assert_eq!(json_escape("plain"), "plain");
    }

    #[test]
    fn register_assigns_sequential_ids_and_none_sentinels_correctly() {
        let a = register("fn_a".to_string(), 100, -1.0, -1, -1);
        let b = register("fn_b".to_string(), -1, 0.5, 10, -1);
        assert_ne!(a, b);
        let t = trackers().lock().unwrap();
        assert_eq!(t[a as usize].latency_ms, Some(100));
        assert_eq!(t[a as usize].error_rate_max, None);
        assert_eq!(t[b as usize].latency_ms, None);
        assert_eq!(t[b as usize].error_rate_max, Some(0.5));
        assert_eq!(t[b as usize].throughput_min_per_sec, Some(10));
    }

    #[test]
    fn call_begin_end_updates_in_flight_and_call_counts() {
        let id = register("tracked_fn".to_string(), -1, -1.0, -1, -1);
        let start = call_begin(id);
        {
            let t = trackers().lock().unwrap();
            assert_eq!(t[id as usize].in_flight.load(Ordering::Relaxed), 1);
        }
        call_end(id, start, false);
        let t = trackers().lock().unwrap();
        assert_eq!(t[id as usize].in_flight.load(Ordering::Relaxed), 0);
        assert_eq!(t[id as usize].total_calls.load(Ordering::Relaxed), 1);
        assert_eq!(t[id as usize].total_errors.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn call_end_counts_errors_only_when_was_err_is_true() {
        let id = register("maybe_fails".to_string(), -1, 0.9, -1, -1);
        let s1 = call_begin(id);
        call_end(id, s1, true);
        let s2 = call_begin(id);
        call_end(id, s2, false);
        let t = trackers().lock().unwrap();
        assert_eq!(t[id as usize].total_calls.load(Ordering::Relaxed), 2);
        assert_eq!(t[id as usize].total_errors.load(Ordering::Relaxed), 1);
    }
}
