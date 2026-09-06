//! The flight recorder's actual event log — double-buffered pages,
//! per the real production pattern this was built to match: a page
//! fills up, gets atomically switched out for the other (empty) page,
//! and the full page is handed off for disk flushing while recording
//! continues immediately into the new active page. `record` (the only
//! function [`super::acquire`]/[`super::release`] call) never blocks on
//! disk I/O — the same "never block, never grow, drop-with-accounting
//! if truly overwhelmed" principle RFC 0007 §6 lays out for the bigger
//! telemetry pipeline this is the smallest real version of.
//!
//! Flushing runs on [`super::thread_pool::ThreadPool`] — the first real
//! caller that pool has had since it was ported in, unwired, alongside
//! this module. A page-full event is exactly the kind of small,
//! occasional, latency-insensitive background job that pool exists for.

use super::thread_pool::ThreadPool;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// One admission-relevant thing that happened. Deliberately tiny and
/// allocation-free (10 bytes) — an event is recorded once per
/// `acquire`/`release` call (boundary-only, never on the `send`/`recv`
/// hot path, same scoping [`super::acquire`] itself uses), so the
/// volume is inherently low, but the representation stays cheap anyway
/// rather than assuming that.
#[derive(Clone, Copy)]
struct Event {
    seq: u64,
    domain: u8,
    kind: u8,
}

/// What `record` is reporting — a grant, a release, or a denial. Kept
/// as a real enum (not a bare `u8` at the call site) so `acquire`/
/// `release` can't accidentally record the wrong kind; converted to
/// `u8` only at the point an `Event` is actually built.
#[derive(Clone, Copy)]
pub enum EventKind {
    Grant = 0,
    Release = 1,
    Denial = 2,
}

fn page_capacity() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("NIRDOSHA_KERNEL_RECORDER_PAGE_CAPACITY").ok().and_then(|s| s.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(1024)
    })
}

fn recorder_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| std::env::var("NIRDOSHA_KERNEL_RECORDER_PATH").unwrap_or_else(|_| "nirdosha_kernel_flight_recorder.log.gz".to_string()))
}

struct Recorder {
    pages: [Mutex<Vec<Event>>; 2],
    active: AtomicUsize,
    seq: AtomicU64,
    flush_pool: OnceLock<Arc<ThreadPool>>,
    write_lock: Mutex<()>,
}

static RECORDER: Recorder =
    Recorder { pages: [Mutex::new(Vec::new()), Mutex::new(Vec::new())], active: AtomicUsize::new(0), seq: AtomicU64::new(0), flush_pool: OnceLock::new(), write_lock: Mutex::new(()) };

/// Records one event into the currently-active page. If this call fills
/// the page, the full page is atomically swapped out (the other page
/// becomes active immediately, before the full one is handed off) and
/// queued for background flushing — this function itself never touches
/// disk and never waits on anything but the page's own short-lived
/// lock.
pub fn record(domain: super::Domain, kind: EventKind) {
    let seq = RECORDER.seq.fetch_add(1, Ordering::Relaxed);
    let event = Event { seq, domain: domain as u8, kind: kind as u8 };
    let active = RECORDER.active.load(Ordering::Acquire);
    let full = {
        let mut page = RECORDER.pages[active].lock().unwrap();
        page.push(event);
        if page.len() >= page_capacity() { Some(std::mem::take(&mut *page)) } else { None }
    };
    if let Some(full_page) = full {
        RECORDER.active.store(1 - active, Ordering::Release);
        flush_async(full_page);
    }
}

/// Flushes whatever's sitting in the currently-active page right now —
/// called exactly once, from the flight recorder's exit/trap hook
/// (`nir_kernel_flight_recorder_dump`, `lib.rs`), so a run that never
/// fills a full page still gets its partial page written instead of
/// silently dropped on exit.
pub fn flush_remaining() {
    let active = RECORDER.active.load(Ordering::Acquire);
    let remaining = {
        let mut page = RECORDER.pages[active].lock().unwrap();
        std::mem::take(&mut *page)
    };
    if !remaining.is_empty() {
        // Synchronous here, deliberately, unlike `flush_async` below:
        // this runs once, at process exit/trap, where "block briefly to
        // make sure it's really on disk before the process ends" is
        // exactly the right tradeoff. `record`'s own hot path is what
        // must never block -- not this.
        write_page(&remaining);
    }
}

fn flush_async(page: Vec<Event>) {
    if page.is_empty() {
        return;
    }
    let pool = RECORDER.flush_pool.get_or_init(ThreadPool::new);
    // A cheap clone (at most `page_capacity()` small `Copy` structs,
    // and only paid once per page-full, not per event) kept for the
    // fallback path below -- `submit` takes the job by value, and on a
    // real submit failure the job (and whatever it captured, including
    // `page` itself) is dropped, never handed back. Without this clone
    // a failed submit would silently lose the whole page instead of
    // falling back to writing it here.
    let fallback = page.clone();
    if pool.submit(Box::new(move || write_page(&page))).is_err() {
        // OS refused to create a thread for the flush worker -- rare
        // (real OS resource exhaustion), and correctness (never silently
        // lose a full page) matters more than "never block" in that one
        // degraded case.
        write_page(&fallback);
    }
}

fn domain_name(d: u8) -> &'static str {
    match d {
        0 => "tcp",
        1 => "file",
        _ => "unknown",
    }
}

fn kind_name(k: u8) -> &'static str {
    match k {
        0 => "grant",
        1 => "release",
        2 => "denial",
        _ => "unknown",
    }
}

/// Formats a page as one line per event, then gzip-compresses it as one
/// independent gzip member (`flate2::write::GzEncoder` over an in-memory
/// buffer — never touches disk itself, so this runs entirely off
/// `record`'s hot path with no I/O cost, only CPU, and only on the
/// background flush worker). Gzip members concatenate validly (RFC
/// 1952) — appending one compressed member per flush to the same file
/// produces a file any gzip-aware reader (`flate2::read::MultiGzDecoder`,
/// or plain `zcat`/`gunzip`) decodes as the full, in-order event log,
/// without needing to buffer whole-file contents in memory to compress
/// it as one giant stream.
fn compress_page(page: &[Event]) -> Option<Vec<u8>> {
    let mut buf = String::with_capacity(page.len() * 24);
    for e in page {
        buf.push_str(&format!("{},{},{}\n", e.seq, domain_name(e.domain), kind_name(e.kind)));
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    // Writing to/finishing an in-memory `Vec` sink is not a real I/O
    // operation -- this essentially cannot fail in practice. Treated as
    // a real `Option`, not unwrapped, anyway: a page that fails to
    // compress is dropped rather than falling back to writing
    // uncompressed bytes into what must otherwise be a clean sequence
    // of gzip members (mixing formats would corrupt every later read of
    // this file, worse than losing one page — the same "fail open,
    // never corrupt" principle RFC 0007 §6 states for telemetry).
    encoder.write_all(buf.as_bytes()).ok()?;
    encoder.finish().ok()
}

fn write_page(page: &[Event]) {
    let Some(compressed) = compress_page(page) else {
        return;
    };
    // One process-wide lock around the actual file write -- more than
    // one flush job can run concurrently once two pages fill close
    // together, and `O_APPEND`'s own atomicity guarantee is about where
    // a write lands, not that arbitrarily large concurrent writes never
    // interleave. A short lock around a single `write_all` per flush is
    // cheap and simple; this runs on a background worker, never on
    // `record`'s own hot path.
    let _guard = RECORDER.write_lock.lock().unwrap();
    // A failure to open/write the recorder file is deliberately
    // swallowed here, not propagated -- the same fail-open telemetry
    // principle RFC 0007 §6 states explicitly: a broken flight recorder
    // must never be the reason a `.nir` program itself fails.
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(recorder_path()) {
        let _ = f.write_all(&compressed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::MultiGzDecoder;
    use std::io::Read;

    // These tests share the one process-wide `RECORDER`/`NIRDOSHA_
    // KERNEL_RECORDER_*` env vars with each other -- serialized via
    // this lock so they don't interleave (`cargo test`'s default
    // parallelism would otherwise let two tests race on the same
    // static state and the same env vars).
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Reads a recorder file back as plain text -- `MultiGzDecoder`,
    /// not a plain `GzDecoder`, because the file is a *sequence* of
    /// independently-compressed gzip members (one per flushed page,
    /// `compress_page`'s own doc comment), not one continuous stream.
    fn read_decompressed(path: &str) -> String {
        let file = std::fs::File::open(path).expect("recorder file should exist");
        let mut decoder = MultiGzDecoder::new(file);
        let mut out = String::new();
        decoder.read_to_string(&mut out).expect("recorder file should be valid gzip");
        out
    }

    // `page_capacity()`/`recorder_path()` cache themselves in a
    // process-wide `OnceLock` on first call, in *this whole test
    // binary* -- a real production process only ever sets these env
    // vars once, at startup, so that's the right design for the real
    // thing, but it means two `#[test]`s in this module cannot each set
    // a *different* path/capacity and expect their own to win: whichever
    // test's first `record()`/`page_capacity()` call runs first (`cargo
    // test`'s default parallelism makes that nondeterministic) decides
    // the value for every test in the binary after it. Fixed by testing
    // both behaviors (partial-page flush-on-exit, full-page auto-flush)
    // in ONE test against ONE configuration, set before anything in this
    // module has been called at all, rather than fighting the cache.
    #[test]
    fn partial_and_full_page_flushes_both_work_against_one_configuration() {
        let _guard = TEST_LOCK.lock().unwrap();
        const PATH: &str = "kernel_recorder_test.log.gz";
        const CAP: usize = 8;
        // SAFETY (test-only): set once, before this module's first real
        // call, and never changed again -- matches how a real process
        // actually uses these (set once at startup).
        unsafe {
            std::env::set_var("NIRDOSHA_KERNEL_RECORDER_PAGE_CAPACITY", CAP.to_string());
            std::env::set_var("NIRDOSHA_KERNEL_RECORDER_PATH", PATH);
        }
        let _ = std::fs::remove_file(PATH);
        assert_eq!(page_capacity(), CAP, "this test must run before any other caller has resolved page_capacity() differently");

        // Part 1: fewer events than a page holds -- nothing flushes on
        // its own; `flush_remaining()` is what a real run's exit/trap
        // hook calls, and it's the only thing that should produce a
        // file here.
        for _ in 0..5 {
            record(super::super::Domain::File, EventKind::Grant);
        }
        assert!(!std::path::Path::new(PATH).exists(), "a page nowhere near full must not have flushed anything yet");
        flush_remaining();
        let after_partial = read_decompressed(PATH);
        assert_eq!(after_partial.lines().count(), 5, "expected exactly the 5 events just recorded, got:\n{after_partial}");

        // Part 2: filling a full page (CAP more events) must flush
        // itself automatically, with no further `flush_remaining()`
        // call -- appended as a second gzip member after part 1's.
        for _ in 0..CAP {
            record(super::super::Domain::Tcp, EventKind::Release);
        }
        // The flush runs on a background worker -- give it a real
        // moment to land before checking.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let after_full = read_decompressed(PATH);
        assert_eq!(after_full.lines().count(), 5 + CAP, "expected the original 5 lines plus one full page's worth, got:\n{after_full}");

        let _ = std::fs::remove_file(PATH);
    }
}
