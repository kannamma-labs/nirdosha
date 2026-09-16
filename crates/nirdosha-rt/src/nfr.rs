//! NFR guards — the runtime enforcement for `nfr(latency_ms = N,
//! concurrency_max = N)` claims.
//!
//! Honesty note: NFRs are **enforced, not proven**. `latency_ms` is
//! measured per call and recorded against the declared limit;
//! `concurrency_max` is a hard gate — the (max+1)-th concurrent call
//! blocks until a slot frees. The Stage-2 driver additionally turns
//! these records into certificate fields and bench obligations
//! (`nirdosha bench`); Stage 1 keeps every event in the flight recorder
//! so a violation is always observable.
//!
//! The guard is Drop-based, so recording fires on every exit path:
//! early return, `?`, and panic.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;

/// Declared NFR limits for one function. Injected by the
/// `#[contract]` macro from `nfr(latency_ms = .., concurrency_max = ..)`.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub latency_ms: Option<f64>,
    pub concurrency_max: Option<u64>,
}

impl Limits {
    pub const NONE: Limits = Limits {
        latency_ms: None,
        concurrency_max: None,
    };
}

/// One flight-recorder entry per guarded call.
#[derive(Debug, Clone, Serialize)]
pub struct NfrEvent {
    pub function: String,
    pub latency_ms: f64,
    pub limit_ms: Option<f64>,
    pub latency_ok: bool,
    pub concurrency_max: Option<u64>,
}

struct Slot {
    count: Mutex<u64>,
    free: Condvar,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<Slot>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<Slot>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn flight_recorder() -> &'static Mutex<Vec<NfrEvent>> {
    static EVENTS: OnceLock<Mutex<Vec<NfrEvent>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Enter a guarded function. Called only by code the `#[contract]`
/// macro injected.
pub fn enter(function: impl Into<String>, limits: &Limits) -> Guard {
    let function = function.into();
    let slot = limits.concurrency_max.map(|max| {
        let slot = {
            let mut reg = registry().lock().unwrap();
            reg.entry(function.clone())
                .or_insert_with(|| Arc::new(Slot {
                    count: Mutex::new(0),
                    free: Condvar::new(),
                }))
                .clone()
        };
        let mut count = slot.count.lock().unwrap();
        while *count >= max {
            count = slot.free.wait(count).unwrap();
        }
        *count += 1;
        drop(count);
        slot
    });
    Guard {
        function,
        slot,
        started: Instant::now(),
        latency_limit: limits.latency_ms,
        concurrency_max: limits.concurrency_max,
    }
}

/// The Drop-based guard injected into contract functions. Nothing to
/// call — recording happens when the scope exits.
pub struct Guard {
    function: String,
    slot: Option<Arc<Slot>>,
    started: Instant,
    latency_limit: Option<f64>,
    concurrency_max: Option<u64>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            let mut count = slot.count.lock().unwrap();
            *count -= 1;
            slot.free.notify_all();
        }
        let latency_ms = self.started.elapsed().as_secs_f64() * 1_000.0;
        let latency_ok = self.latency_limit.is_none_or(|limit| latency_ms <= limit);
        let event = NfrEvent {
            function: self.function.clone(),
            latency_ms,
            limit_ms: self.latency_limit,
            latency_ok,
            concurrency_max: self.concurrency_max,
        };
        let mut log = flight_recorder().lock().unwrap();
        // Flight recorder, two sinks (both best-effort):
        // - NIRDOSHA_NFR_LOG=1      JSON lines on stderr (human watch)
        // - NIRDOSHA_NFR_LOG_FILE=p append JSON lines to a file (harness sink:
        //   `cargo nirdosha bench` points this at target/nirdosha/ and
        //   gates the SLA from what it reads back)
        let line = serde_json::to_string(&event).ok();
        if let Some(line) = &line {
            if std::env::var_os("NIRDOSHA_NFR_LOG").is_some_and(|v| !v.is_empty()) {
                eprintln!("{line}");
            }
            if let Some(path) = std::env::var_os("NIRDOSHA_NFR_LOG_FILE") {
                if !path.is_empty() {
                    use std::io::Write as _;
                    if let Ok(mut file) =
                        std::fs::OpenOptions::new().create(true).append(true).open(path)
                    {
                        let _ = writeln!(file, "{line}");
                    }
                }
            }
        }
        log.push(event);
    }
}

/// All recorded events so far (this process). Tests and `cargo nirdosha`
/// Stage-2 certificate generation consume this.
pub fn events() -> Vec<NfrEvent> {
    flight_recorder().lock().unwrap().clone()
}

/// Clear the recorder. Test-only.
pub fn reset_events() {
    flight_recorder().lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn latency_violations_are_recorded() {
        {
            let _g = enter("slow_fn", &Limits {
                latency_ms: Some(0.0),
                concurrency_max: None,
            });
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // Tests run in parallel on one shared recorder: always filter
        // by function name, never assert on total counts.
        let log = events();
        let mine: Vec<_> = log.iter().filter(|e| e.function == "slow_fn").collect();
        assert!(!mine.is_empty(), "guarding slow_fn must record an event");
        assert!(!mine[0].latency_ok, "0ms limit + 2ms sleep must violate");
    }

    #[test]
    fn concurrency_gate_caps_parallelism() {
        let name = "gated_fn";
        let peak = Arc::new(AtomicU64::new(0));
        let inside = Arc::new(AtomicU64::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let peak = peak.clone();
            let inside = inside.clone();
            handles.push(std::thread::spawn(move || {
                let _g = enter(name, &Limits {
                    latency_ms: None,
                    concurrency_max: Some(2),
                });
                let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(5));
                inside.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(
            peak.load(Ordering::SeqCst) <= 2,
            "concurrency_max=2 was exceeded: {}",
            peak.load(Ordering::SeqCst)
        );
    }
}