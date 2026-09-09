//! The compiled-path resource-control kernel — `rfcs/0007-apm-runtime-
//! kernel.md`'s "local admission plane" (§3), built for real for the
//! first time, deliberately scoped to the smallest useful version of
//! that RFC's mechanism rather than the whole five-plane design at
//! once. Two decisions this file makes, both taken directly from that
//! RFC's own findings and from the fresh, independent performance
//! review that fed into it:
//!
//! 1. **Admission happens once per resource-*creation* call
//!    (`connect`/`listen`/`accept`/`open`), never on `send`/`recv`/
//!    `read`/`write`.** RFC 0007 §4.2 already identifies "an accepted
//!    TCP connection or an opened file" as the compiled path's real
//!    admission boundary (there's no classified-request boundary yet —
//!    compiled `serve` doesn't exist); the independent performance
//!    review sharpened this further: checking on every syscall, not
//!    every resource-acquisition event, was the wrong granularity in
//!    the original design. `kernel_bench` (`rfcs/evidence/0007-apm-
//!    runtime-kernel/kernel_bench/`) measured the *hot-path* calls
//!    (`send`/`recv`/`read`/`write`) at 0.8-1.8µs with zero admission
//!    logic — a per-call check there was the one real risk that
//!    review flagged. Gating only at creation avoids it entirely.
//! 2. **The check itself is one atomic compare-and-swap, no lock, no
//!    allocation, no syscall** — the "O(1) local lease check" §3 calls
//!    for. The same review argued the original ~100ns hot-path budget
//!    was the wrong order of magnitude for a true per-call gate (it
//!    should cost single-digit nanoseconds); by only running at
//!    creation calls, which already cost 2.4-27µs per `kernel_bench`'s
//!    own measurements, this doesn't need to hit that bar at all — but
//!    it's a plain atomic op regardless, cheap enough to not need the
//!    exemption.
//!
//! **Telemetry is a real, working data plane now, not just counters.**
//! [`recorder`] double-buffers every `acquire`/`release`/denial event
//! into pages, flushes a full page to disk asynchronously (via
//! [`thread_pool::ThreadPool`], gzip-compressed) without ever blocking
//! `acquire`/`release` themselves, and flushes whatever's left on exit.
//! Still genuinely smaller than RFC 0007 §6's full design: no OTLP
//! export, no sampling policy, one file rather than a real sink/pipeline
//! — but this is a real double-buffered recorder, not a placeholder for
//! one. See [`recorder`]'s own module doc. No cross-shard wait-for
//! sweep (only two domains exist; nothing to deadlock across yet), no
//! NFR declarations — real future work, left for when a real need shows
//! up, per this session's own "add capability through the kernel as we
//! go" plan.
//!
//! [`HandleTable`] exists now, wired to nothing yet, specifically so the
//! next resource domain this project adds (`json`/`db`/`mq` — see the
//! "how many features could we bring back" discussion this was built
//! from) gets a real opaque-handle mechanism from its very first line of
//! code instead of a fourth independently invented one.
//!
//! **Two more primitives live alongside this one, also unwired**:
//! [`pool`] (generic, backend-agnostic connection *pooling* — reuse, not
//! just admission — ported from the interpreter's own real `pool.rs`,
//! which had zero interpreter dependency) and [`thread_pool`] (an
//! eager-growth, reused-worker OS thread pool — ported from the
//! interpreter's own `thread_pool.rs`, same story, with a real
//! panic-containment question this crate's `panic = "abort"` release
//! profile used to raise — resolved: this crate now builds with `panic
//! = "unwind"` instead, verified against the real production link path,
//! not assumed — see `thread_pool`'s own module doc and
//! `rfcs/evidence/0007-apm-runtime-kernel/panic_containment/`).
//! Admission (this module) answers "are we under the ceiling"; pooling
//! answers "reuse what's already open instead of paying to create a new
//! one" — a future `db`/`mq` domain plausibly wants both at the same
//! call site, and a future `spawn` codegen effort wants the worker pool
//! specifically (not connection pooling — threads aren't connections).
//!
//! **Flight recorder, not a query interface.** A compiled `.nir`
//! program never asks the kernel anything — there is deliberately no
//! builtin a `.nir` author can call to read admission stats mid-run.
//! Instead, [`dump_report`] is invoked exactly once, automatically, by
//! `codegen.rs`'s generated `main` wrapper, right before the process
//! exits — every compiled binary silently records what it did and
//! reports it on the way out, the same way a flight recorder doesn't
//! answer queries mid-flight, it just captures and hands over the tape
//! afterward. This keeps the recording passive and uniform (nothing a
//! `.nir` author writes can suppress or distort it) instead of opt-in
//! and queryable.
//!
//! **Two of RFC 0006's five pillars, wired in for real now, hidden
//! behind keywords that already existed — no new syntax, no new
//! developer-facing API.** [`mailbox`] is Pillar 2 (non-blocking `send`)
//! and Pillar 3 (blocking, multi-consumer `receive`) — `chan`/`send`/
//! `recv` compile to `runtime-kernels/src/lib.rs`'s `nir_chan_new`/
//! `nir_chan_send`/`nir_chan_recv`, which is this module's `mailbox`
//! exactly, not a parallel API a `.nir` author opts into.
//! [`thread_pool::Scope`] is Pillar 4 (structured spawn, no orphan
//! threads) — `spawn`/`join` compile to `nir_thread_spawn`/
//! `nir_thread_join`, each spawn getting its own dedicated one-job
//! `Scope`, auto-joined by `codegen.rs`'s `emit_affine_free` if the
//! spawning function itself never consumes the `thread` handle (see
//! `lib.rs`'s "chan/spawn/join kernels" section for the one real,
//! disclosed gap versus the RFC prototype's own lexical-`Scope`-per-
//! function mechanism, and `codegen.rs`'s `is_word_sized` for the other
//! one: word-sized payloads/arguments/results only, so far — no
//! `str`/`dec128`/struct/enum yet).
//!
//! **Pillars 1 and 5 are honestly still open, and nothing in this crate
//! substitutes for them.** Pillar 1 (`iso`/`froze`/`lend` capability
//! types on `Box<T>`) and Pillar 5 (the lexical-level deadlock-freedom
//! proof via scope levels) are type-system properties the compiler must
//! check statically — they need real work in `ownership.rs` and a
//! level-typing extension to scopes, not a runtime primitive. A kernel
//! module can host the mechanics Pillars 2-4 need at runtime; it cannot
//! retroactively make `.nir`'s type checker prove something it doesn't
//! check yet.

// `stats`/`HandleTable` aren't called from anywhere in `lib.rs` yet --
// deliberately: `stats` is for a future telemetry export point, and
// `HandleTable` is for the next resource domain to be added (see this
// module's own doc comment). Both are proven by this file's own unit
// tests, not dead in any real sense; `#[allow(dead_code)]` just says so
// to the compiler until a real caller exists.
#![allow(dead_code)]

pub mod db;
pub mod http;
pub mod identity;
pub mod instance_lock;
pub mod mailbox;
pub mod nfr;
pub mod plugin_provider;
pub mod pool;
pub mod reaper;
pub mod recorder;
pub mod thread_pool;
pub mod transact;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// A resource domain the kernel admits/tracks, identified by a plain
/// `u32` index into [`DOMAIN_SLOTS`] rather than a closed enum variant —
/// **RFC 0011 §3's open registry**, replacing the old closed `enum
/// Domain` (7 hardcoded variants) so a future plugin-registered domain
/// (a `db`/`call`-shape provider's own scheme, RFC 0011 §2) can mint a
/// domain at runtime instead of needing a recompile of this crate. `0`
/// is a valid id here (unlike [`HandleTable`]'s reserved-`0` convention)
/// — the first domain [`domain::register_builtin_domains`] registers
/// (`tcp`) legitimately gets id `0`.
pub type DomainId = u32;

/// Hard ceiling on how many domains this process can ever register —
/// 7 built-ins plus however many plugin providers a program links in
/// (RFC 0011 §2/§4). 4096 is deliberately generous headroom over any
/// realistic plugin count, chosen so [`recorder::Event`]'s `u16` domain
/// field (65536 range) stays comfortably larger than this ceiling with
/// room to spare, per this phase's own recorder-widening rationale.
/// Exceeding it is a loud startup-time panic in [`register_domain`],
/// never silent truncation or wraparound.
const MAX_DOMAINS: usize = 4096;

static NEXT_DOMAIN: AtomicU32 = AtomicU32::new(0);

/// Per-domain metadata captured at registration time — the env-var name
/// and default ceiling `max_for` needs to lazily resolve a domain's
/// ceiling on first `acquire`, now that these aren't compile-time enum
/// methods (`Domain::env_var`/`Domain::default_max`) anymore.
struct DomainMeta {
    env_var: &'static str,
    default_max: i64,
}

static DOMAIN_META: [OnceLock<DomainMeta>; MAX_DOMAINS] = [const { OnceLock::new() }; MAX_DOMAINS];
static DOMAIN_NAMES: [OnceLock<&'static str>; MAX_DOMAINS] = [const { OnceLock::new() }; MAX_DOMAINS];

/// Name → id, so [`register_domain`] can be idempotent by name (calling
/// it twice for the same name — e.g. `domain::register_builtin_domains()`
/// running more than once, see that function's own doc comment — returns
/// the same [`DomainId`] rather than minting a second one).
static NAME_INDEX: OnceLock<Mutex<HashMap<&'static str, DomainId>>> = OnceLock::new();

/// Registers one resource domain, minting a fresh [`DomainId`] the first
/// time `name` is seen and returning the existing one on any later call
/// with the same `name` — registration is rare (startup-time only, never
/// on any hot path), so this serializes on one lock for the whole
/// operation rather than trying to be lock-free, unlike [`acquire`]/
/// [`release`] which very much need to be.
///
/// `env_var`/`default_max` follow the same override-by-env-var
/// convention `crates/compiler/src/pool.rs`'s `PoolConfig` already
/// establishes: `env_var` is read once, lazily, on this domain's first
/// `acquire` (`max_for`), falling back to `default_max` if unset,
/// unparseable, or `<= 0`.
///
/// Panics loudly if registering a genuinely new name would exceed
/// [`MAX_DOMAINS`] — a startup-time configuration error, never something
/// a well-behaved running program should silently truncate or wrap
/// around on.
pub fn register_domain(name: &'static str, env_var: &'static str, default_max: i64) -> DomainId {
    let index = NAME_INDEX.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = index.lock().unwrap();
    if let Some(&existing) = map.get(name) {
        return existing;
    }
    let id = NEXT_DOMAIN.fetch_add(1, Ordering::Relaxed);
    if (id as usize) >= MAX_DOMAINS {
        // Drop the lock *before* panicking, deliberately -- panicking
        // while still holding a `std::sync::Mutex` guard poisons it
        // permanently, which would make every future `register_domain`
        // call (even for an already-registered name, whose lookup above
        // also needs this same lock) panic too, forever, for the rest of
        // the process. In real production use this panic is meant to be
        // fatal anyway (a startup-time configuration error), so it
        // wouldn't matter -- but it does matter to anything that catches
        // this panic and keeps running (this module's own test for this
        // exact case), so this is cleaned up regardless of who's asking.
        drop(map);
        panic!(
            "nirdosha kernel: MAX_DOMAINS ({MAX_DOMAINS}) exceeded registering domain {name:?} -- \
             raise MAX_DOMAINS or investigate a domain-registration loop/leak"
        );
    }
    DOMAIN_META[id as usize].set(DomainMeta { env_var, default_max }).ok().expect("a freshly-minted DomainId's slot must be unclaimed");
    DOMAIN_NAMES[id as usize].set(name).ok().expect("a freshly-minted DomainId's slot must be unclaimed");
    map.insert(name, id);
    id
}

/// Looks up an already-registered domain's id by name, without
/// registering anything — RFC 0011 §2's plugin-provider dispatch table
/// registration (`plugin_provider::register`) needs a provider's
/// `DomainId` to store alongside its function pointers, and by the time
/// that call happens (`codegen.rs` always emits `nir_kernel_register_
/// domain` for a given provider immediately before `nir_kernel_register_
/// plugin_provider` for the same one, in the same preamble loop
/// iteration), the domain is already registered — so this is a plain
/// lookup, not a second registration path. `None` would mean codegen
/// emitted these calls out of order or for mismatched providers, a
/// compiler bug, not a runtime condition to recover from gracefully.
pub fn domain_id_for_name(name: &str) -> Option<DomainId> {
    let index = NAME_INDEX.get_or_init(|| Mutex::new(HashMap::new()));
    index.lock().unwrap_or_else(|e| e.into_inner()).get(name).copied()
}

/// Domain ids/names actually registered so far, in registration order —
/// [`dump_report`] and [`recorder::domain_name`] both need this instead
/// of a fixed 7-entry list now that the domain set is open.
fn registered_domains() -> Vec<(DomainId, &'static str)> {
    let n = (NEXT_DOMAIN.load(Ordering::Relaxed) as usize).min(MAX_DOMAINS) as u32;
    (0..n).filter_map(|id| DOMAIN_NAMES[id as usize].get().map(|&name| (id, name))).collect()
}

/// Looks up the name for one domain id — `None` for an id past whatever
/// has been registered so far (e.g. a corrupt/foreign value), in which
/// case the caller (`recorder::domain_name`) falls back to `"unknown"`
/// the same way it always has.
fn registered_domain_name(id: DomainId) -> Option<&'static str> {
    DOMAIN_NAMES.get(id as usize).and_then(|cell| cell.get().copied())
}

/// The 7 built-in resource domains this kernel has always tracked —
/// `(name, env_var, default_max)`, in the fixed order `tcp, file,
/// thread, db, mq, http, serve_http`. **This one array is the only
/// place that order is written down** — it both drives
/// [`domain::register_builtin_domains`]'s registration loop and, via
/// that same loop, is what fills [`domain`]'s 7 `OnceLock<DomainId>`
/// cells. `codegen.rs` emits exactly one call to
/// `nir_kernel_register_builtin_domains` (→ `domain::
/// register_builtin_domains`) in every compiled program's `main`
/// preamble, strictly before any user code — it carries zero knowledge
/// of this set or its order, unlike an earlier design this phase
/// replaced (codegen emitting one `register_domain(...)` call per
/// built-in, a second, driftable copy of this exact list).
const BUILTIN_DOMAINS: [(&str, &str, i64); 7] = [
    ("tcp", "NIRDOSHA_KERNEL_MAX_TCP", 10_000),
    ("file", "NIRDOSHA_KERNEL_MAX_FILE", 10_000),
    ("thread", "NIRDOSHA_KERNEL_MAX_THREAD", 10_000),
    ("db", "NIRDOSHA_KERNEL_MAX_DB", 10_000),
    ("mq", "NIRDOSHA_KERNEL_MAX_MQ", 10_000),
    ("http", "NIRDOSHA_KERNEL_MAX_HTTP", 10_000),
    ("serve_http", "NIRDOSHA_KERNEL_MAX_SERVE_HTTP", 10_000),
];

/// Stable, non-compile-time-constant handles to the 7 built-in domains
/// — replaces the old closed `enum Domain`'s variants. Every accessor
/// here (`tcp()`, `file()`, ...) reads an `OnceLock<DomainId>` cell
/// filled once by [`register_builtin_domains`], instead of being a bare
/// compile-time constant the way `domain::tcp()` used to be — a real,
/// documented precondition this phase introduces (see each accessor's
/// own doc comment) where none existed before.
pub mod domain {
    use super::{register_domain, DomainId, BUILTIN_DOMAINS};
    use std::sync::{Once, OnceLock};

    static TCP: OnceLock<DomainId> = OnceLock::new();
    static FILE: OnceLock<DomainId> = OnceLock::new();
    static THREAD: OnceLock<DomainId> = OnceLock::new();
    static DB: OnceLock<DomainId> = OnceLock::new();
    static MQ: OnceLock<DomainId> = OnceLock::new();
    static HTTP: OnceLock<DomainId> = OnceLock::new();
    static SERVE_HTTP: OnceLock<DomainId> = OnceLock::new();

    static INIT: Once = Once::new();

    /// Registers all 7 built-in domains, in [`BUILTIN_DOMAINS`]'s fixed
    /// order, filling this module's 7 cells so `tcp()`/`file()`/...
    /// resolve to stable ids for the rest of the process's life.
    /// Idempotent (`Once`-guarded): safe to call more than once, from
    /// more than one thread — later calls are no-ops. `codegen.rs`
    /// emits exactly one call to this (via `nir_kernel_register_builtin_
    /// domains`) at the very top of every compiled program's `main`,
    /// strictly before any user code, which is what actually gives the
    /// 7 built-ins their historical, stable ids 0-6 in a compiled
    /// binary (nothing else can have registered a domain first). Each
    /// accessor below also calls this itself before reading its cell —
    /// a deliberate self-healing safety net for callers that reach a
    /// domain accessor outside a compiled program's `main` (this
    /// crate's own unit tests, `compiled-serve`'s tests, any future
    /// direct library use) rather than a hard "you forgot to call this"
    /// panic — `Once` makes the net free after the first real call.
    pub fn register_builtin_domains() {
        INIT.call_once(|| {
            let cells = [&TCP, &FILE, &THREAD, &DB, &MQ, &HTTP, &SERVE_HTTP];
            for (cell, (name, env_var, default_max)) in cells.into_iter().zip(BUILTIN_DOMAINS.iter()) {
                let id = register_domain(name, env_var, *default_max);
                cell.set(id).ok().expect("register_builtin_domains must only ever run its registration body once (Once-guarded)");
            }
        });
    }

    macro_rules! accessor {
        ($name:ident, $cell:ident, $doc:literal) => {
            #[doc = $doc]
            ///
            /// Precondition: [`register_builtin_domains`] must have run
            /// at least once in this process before this is called (it
            /// self-registers on demand if not, so this is a safety net
            /// rather than a hard requirement — see that function's own
            /// doc comment).
            pub fn $name() -> DomainId {
                register_builtin_domains();
                *$cell.get().expect("register_builtin_domains just ran unconditionally above; the cell must be set")
            }
        };
    }
    accessor!(tcp, TCP, "The built-in `tcp` domain.");
    accessor!(file, FILE, "The built-in `file` domain.");
    accessor!(thread, THREAD, "The built-in `thread` domain.");
    accessor!(db, DB, "The built-in `db` domain.");
    accessor!(mq, MQ, "The built-in `mq` domain.");
    accessor!(http, HTTP, "The built-in `http` domain.");
    accessor!(serve_http, SERVE_HTTP, "The built-in `serve_http` domain.");
}

struct DomainCounters {
    held: AtomicI64,
    grants: AtomicU64,
    denials: AtomicU64,
    /// A pooled checkout (`kernel::db`, and later `kernel::http`) that
    /// failed its `ManageConnection::is_valid` round-trip and was
    /// evicted + transparently replaced before the caller ever saw the
    /// staleness — this phase's own "APM rehydrates stale pooled
    /// connections" design. Always `0` for a domain with no pooling
    /// behind it (`Tcp`/`File`/`Thread`/`Mq` today) — same "the number
    /// just stays honestly zero" posture `held`/`grants`/`denials`
    /// already have for a domain that never denies anything.
    stale_rehydrated: AtomicU64,
    /// Unix seconds of the most recent [`record_stale_rehydrated`] call
    /// for this domain, `0` meaning "never" — red-team report A13
    /// (`scratch/red-team-report-main-d7fae42.md`): `stale_rehydrated`
    /// alone can't distinguish "the pool is healthy, one connection went
    /// stale a month ago" from "every connection just went stale at
    /// once" (e.g. a server restart, a network partition) — both look
    /// identical as a raw incrementing count. A last-seen timestamp,
    /// surfaced alongside the count in [`dump_report`], answers *when*,
    /// not just *how many*.
    last_stale_rehydrated_at: AtomicI64,
    max: OnceLock<i64>,
}

impl DomainCounters {
    const fn new() -> Self {
        DomainCounters {
            held: AtomicI64::new(0),
            grants: AtomicU64::new(0),
            denials: AtomicU64::new(0),
            stale_rehydrated: AtomicU64::new(0),
            last_stale_rehydrated_at: AtomicI64::new(0),
            max: OnceLock::new(),
        }
    }
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Records one pooled checkout that had to evict a stale connection and
/// open a fresh one before returning — called from `kernel::db`'s (and
/// later `kernel::http`'s) `connect` path, never from `.nir`-visible
/// code. Deliberately separate from [`acquire`]/[`release`]: rehydration
/// is a pool-level event, not an admission decision — a rehydrated
/// checkout still went through a real `acquire` either way.
pub fn record_stale_rehydrated(domain: DomainId) {
    let counters = counters_for(domain);
    counters.stale_rehydrated.fetch_add(1, Ordering::Relaxed);
    counters.last_stale_rehydrated_at.store(now_unix_secs(), Ordering::Relaxed);
}

/// `0` if [`record_stale_rehydrated`] has never fired for `domain`,
/// otherwise the unix-seconds timestamp of its most recent call — see
/// [`DomainCounters::last_stale_rehydrated_at`]'s own doc comment.
pub fn last_stale_rehydrated_at(domain: DomainId) -> i64 {
    counters_for(domain).last_stale_rehydrated_at.load(Ordering::Relaxed)
}

/// Every domain's counters, indexed directly by [`DomainId`] — the open-
/// registry replacement for the old 7 named `static`s + `counters_for`
/// match. A plain array index, same as before: this is the load-bearing
/// non-blocking property (this module's own doc comment, point 2) —
/// `acquire`/`release` must stay zero-lock, zero-allocation, zero-syscall
/// reads/writes into this array, exactly as they were against the old
/// named statics.
static DOMAIN_SLOTS: [DomainCounters; MAX_DOMAINS] = [const { DomainCounters::new() }; MAX_DOMAINS];

fn counters_for(domain: DomainId) -> &'static DomainCounters {
    &DOMAIN_SLOTS[domain as usize]
}

fn max_for(domain: DomainId, counters: &DomainCounters) -> i64 {
    *counters.max.get_or_init(|| {
        let meta = DOMAIN_META[domain as usize].get().expect("max_for called for a DomainId that was never registered");
        std::env::var(meta.env_var).ok().and_then(|s| s.parse::<i64>().ok()).filter(|&n| n > 0).unwrap_or(meta.default_max)
    })
}

/// Public wrapper around a domain's resolved admission ceiling —
/// RFC 0011 §5's "what the ceiling actually bounds": `pool.rs`'s
/// registration-time enforcement (`get_or_create_within_ceiling`) reads
/// this to refuse building a provider's pool if its own
/// `PoolConfig::max_size` would exceed it, rather than letting r2d2
/// silently grow physical connections past a ceiling nothing consults.
/// Reuses `max_for`'s own env-var-resolution/caching rather than a
/// second copy of that logic.
pub fn ceiling_for(domain: DomainId) -> i64 {
    max_for(domain, counters_for(domain))
}

/// Attempts to admit one more concurrently-held resource in `domain`.
/// `true` if granted (the caller must eventually call [`release`]
/// exactly once); `false` if `domain` is already at its ceiling, in
/// which case the caller should fail the same way any other resource-
/// creation error already does (`nir_tcp_connect`/`nir_file_open`
/// return `-1` uniformly today — this is deliberately not yet a
/// distinct error code; that's real future work, not a gap to route
/// around today, see this module's own doc comment).
pub fn acquire(domain: DomainId) -> bool {
    let counters = counters_for(domain);
    let max = max_for(domain, counters);
    let mut current = counters.held.load(Ordering::Relaxed);
    loop {
        if current >= max {
            counters.denials.fetch_add(1, Ordering::Relaxed);
            recorder::record(domain, recorder::EventKind::Denial);
            return false;
        }
        match counters.held.compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => {
                counters.grants.fetch_add(1, Ordering::Relaxed);
                recorder::record(domain, recorder::EventKind::Grant);
                return true;
            }
            Err(observed) => current = observed,
        }
    }
}

/// Releases one previously-[`acquire`]d resource in `domain`. Must be
/// called exactly once per successful `acquire` — `ownership.rs`'s
/// affine typing already guarantees a `tcp`/`file` handle's own `stop`
/// runs at most once per handle in a well-typed program (this crate's
/// own "the checker is the real gate" convention), so a matched
/// acquire/release pair per handle is a real invariant, not a hope.
pub fn release(domain: DomainId) {
    counters_for(domain).held.fetch_sub(1, Ordering::AcqRel);
    recorder::record(domain, recorder::EventKind::Release);
}

/// Raw self-metrics — RFC 0007 §7's "kernel self-metrics are first-class"
/// principle, in its smallest form: no exporter, no aggregation, just
/// the numbers. `(currently_held, total_grants, total_denials,
/// total_stale_rehydrated)`.
pub fn stats(domain: DomainId) -> (i64, u64, u64, u64) {
    let c = counters_for(domain);
    (c.held.load(Ordering::Relaxed), c.grants.load(Ordering::Relaxed), c.denials.load(Ordering::Relaxed), c.stale_rehydrated.load(Ordering::Relaxed))
}

/// The flight recorder's one output: every domain's final counters,
/// formatted as one line each. Called exactly once, automatically, from
/// `codegen.rs`'s generated `main` wrapper (via `nir_kernel_flight_
/// recorder_dump` in `lib.rs`) — never from anything a `.nir` author
/// writes (this module's own doc comment). Deliberately plain text, not
/// JSON — this is the smallest possible version of "the tape gets
/// handed over," not a structured export pipeline (RFC 0007 §6's own
/// rings/aggregator/exporter design is the real future version of this,
/// if a real need for one ever shows up).
///
/// `held` should always print `0` for every domain by the time this
/// runs, in a well-typed program — `ownership.rs`'s affine checking
/// already proves every `tcp`/`file` handle's `stop` runs before
/// `main` returns (nothing currently open at exit is holdable at all,
/// since a live handle would need a live binding, and every binding's
/// scope has already ended). A nonzero `held` here would mean this
/// kernel's own acquire/release bookkeeping has drifted from what
/// `ownership.rs` guarantees — worth treating as a real bug report if
/// it's ever seen, not just a diagnostic curiosity.
pub fn dump_report() -> String {
    let mut out = String::from("nirdosha kernel flight recorder:\n");
    for (id, name) in registered_domains() {
        let (held, grants, denials, stale_rehydrated) = stats(id);
        // `last_stale_rehydrated_at` (red-team report A13) appended
        // after `stale_rehydrated`, not replacing it — existing
        // consumers asserting on `held=...`/`grants=...`/`denials=...`/
        // `stale_rehydrated=...` substrings (this module's own
        // regression tests, `nirdosha_ops_console.rs`) match a prefix of
        // this line, unaffected by trailing fields.
        let last_stale = last_stale_rehydrated_at(id);
        out.push_str(&format!(
            "  {name}: held={held} grants={grants} denials={denials} stale_rehydrated={stale_rehydrated} last_stale_rehydrated_at={last_stale}\n"
        ));
    }
    // RFC 0011 §5: "a broken reaper is a visible number going up, not a
    // silent absence" — appended as a trailing line, not folded into the
    // per-domain loop above (it isn't a domain), so existing consumers
    // asserting on a `{name}: held=... grants=...` substring (this
    // module's own regression tests, `nirdosha_ops_console.rs`) are
    // unaffected by this new line's presence.
    out.push_str(&format!("  reaper_panics={}\n", reaper::reaper_panics()));
    // Red-team report A9: the reaper's actually-in-effect interval
    // (post floor/soft-ceiling resolution), so a misconfiguration that
    // used to be visible only via a stderr `eprintln!` at startup is
    // queryable post-incident too. `0` means the reaper hasn't started
    // in this process yet (see `reaper::reaper_configured_interval_secs`'s
    // own doc comment).
    out.push_str(&format!("  reaper_interval_secs={}\n", reaper::reaper_configured_interval_secs()));
    out
}

/// A generic, process-wide table mapping an opaque, mint-once `i64`
/// handle to a live Rust value `T` — for a resource domain whose handle
/// isn't already a raw OS fd (`json`'s parsed document, a `db`
/// connection, an `mq` subscription). Same shape the now-removed
/// `nirdosha-plugin-support::HandleRegistry` used, minus its
/// interpreter-specific error-construction helpers (`Value`/
/// `RuntimeError` don't exist on this side of the ABI boundary — this
/// crate can't depend on the compiler crate at all, this file's own
/// module doc). **Doc-drift fix**: this comment used to say "not wired
/// to any `nir_*` kernel yet" — `lib.rs`'s `db_table()` is a real,
/// working `HandleTable<...>` today.
pub struct HandleTable<T> {
    next_id: AtomicI64,
    handles: Mutex<HashMap<i64, T>>,
}

impl<T> Default for HandleTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> HandleTable<T> {
    pub fn new() -> Self {
        Self::new_starting_at(1)
    }

    /// Same as [`HandleTable::new`], except minted ids start at `start`
    /// instead of `1`. RFC 0011 §2's plugin-conn `HandleTable` needs
    /// this: `.nir`'s `handle(Db)` type is minted by either `lib.rs`'s
    /// core `db_table()` (a separate `HandleTable<DbConn>`) or
    /// `plugin_provider::plugin_conn_table()` (`HandleTable<PluginConn>`)
    /// — two independent `next_id` counters that, both starting at `1`,
    /// could otherwise both mint the same numeric id, and step 2's
    /// "check `db_table()` first, then `HandleTable<PluginConn>` on a
    /// miss" priority would then resolve a plugin-table id that
    /// collides with an unrelated, still-live core `db` id to the
    /// *wrong* connection instead of falling through correctly. Starting
    /// the plugin table at a disjoint, high range (`1 << 62`) makes that
    /// collision structurally impossible rather than merely unlikely —
    /// the same "closed by construction, not by convention" posture §2
    /// already argues for `next_id`'s own never-reused-once-removed
    /// property within a single table.
    pub fn new_starting_at(start: i64) -> Self {
        HandleTable { next_id: AtomicI64::new(start), handles: Mutex::new(HashMap::new()) }
    }

    /// Takes ownership of `value`, mints a fresh id (never `0` —
    /// reserve that as a caller-chosen "no handle"/invalid sentinel),
    /// and returns it as the plain `i64` a `nir_*_open`-style kernel
    /// hands back across the ABI boundary.
    pub fn insert(&self, value: T) -> i64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.handles.lock().unwrap().insert(id, value);
        id
    }

    /// Runs `f` against the live resource for `id` without removing it.
    /// `None` if `id` isn't currently open (already closed, or never
    /// existed) — the caller turns that into `-1`, the same uniform
    /// failure convention every other kernel here already uses.
    pub fn with<R>(&self, id: i64, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.handles.lock().unwrap().get_mut(&id).map(f)
    }

    /// Removes and returns the resource for `id` — what a `_stop`
    /// kernel calls; the returned `T` is dropped at the call site.
    /// `None` on a double-close, not a panic.
    pub fn remove(&self, id: i64) -> Option<T> {
        self.handles.lock().unwrap().remove(&id)
    }

    pub fn len(&self) -> usize {
        self.handles.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A cheap, **exact** dynamic backstop for `rfcs/0006-structured-
/// concurrency.md`'s still-unbuilt, still-deferred Pillar 5 (the
/// compile-time, lexical-scope-level deadlock-freedom proof) — the same
/// relationship this whole module has to admission control in general:
/// a runtime mechanism standing in for a static guarantee the type
/// checker doesn't make yet, honestly scoped rather than oversold.
///
/// **The technique**: the same one Go's own runtime uses ("fatal error:
/// all goroutines are asleep - deadlock!"), not a general wait-for-graph
/// cycle detector. Track how many `.nir`-level concurrent participants
/// currently exist (`live` — the main thread, plus one per outstanding
/// `spawn` job) against how many are, right now, blocked in exactly the
/// two operations that can only ever be unblocked by *another* one of
/// those participants (`join`, `recv` — never `tcp`/`file` I/O, which
/// can always still resolve from outside the process). If every live
/// participant is simultaneously in that state, nothing left in the
/// process could ever run the `send`/return that would unblock any of
/// them — a certain, permanent stall, not a heuristic guess.
///
/// **Why this is exact, not probabilistic**, unlike a timeout-based
/// guess: `live`/`blocked` only ever change while holding `STALL`'s own
/// lock, so a thread cannot be "about to do something that would
/// unblock everyone" without that fact already being reflected in
/// `live` before the check runs (a not-yet-submitted `spawn` hasn't
/// incremented `live` yet, so it correctly doesn't count as a possible
/// rescuer; once submitted, it does, before its job could possibly
/// reach a `join`/`recv` of its own).
///
/// **What this deliberately doesn't catch**: a *local* deadlock — two
/// threads cyclically stuck on each other while a third, unrelated
/// thread keeps making progress on something else. Pillar 5's own real
/// proof-by-construction (once built) would catch that too, ahead of
/// time; this only fires once the *whole* program can never move again,
/// same as Go's detector, and for the same reason (correctly telling
/// "some of it is still running" from "literally none of it can ever
/// run again" needs exactly the global count this uses, not a partial
/// one). See `rfcs/0007-apm-runtime-kernel.md` §8 for why this and
/// Pillar 5 are genuinely separate deadlock classes to begin with
/// (resource-acquisition cycles vs. reply-obligation cycles) — this
/// mechanism is a backstop for the *reply-obligation* class only.
struct StallTracker {
    live: usize,
    blocked: usize,
}

static STALL: Mutex<StallTracker> = Mutex::new(StallTracker { live: 1, blocked: 0 });

/// What one currently-blocked thread is actually waiting on — purely
/// diagnostic (`concurrency_wait_begin`'s abort message), never
/// consulted by the deadlock decision itself (`register_wait_and_check_
/// stall` only ever looks at `STALL`'s plain counts). Naming the real
/// stuck handle/call kind is the difference between "all 2 threads are
/// blocked" and "thread A is blocked in `recv` on channel 3, thread B
/// is blocked in `join` on thread 5" — the latter is what actually lets
/// a `.nir` author find the two call sites that made a cycle, without
/// this crate attempting anything like real prevention (this section's
/// own doc comment on why a general wait-for graph doesn't fit `chan`'s
/// no-single-owner semantics).
#[derive(Clone, Copy, Debug)]
pub enum WaitTarget {
    ChanRecv(i64),
    ThreadJoin(i64),
}

impl std::fmt::Display for WaitTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaitTarget::ChanRecv(handle) => write!(f, "`recv` on chan handle {handle}"),
            WaitTarget::ThreadJoin(handle) => write!(f, "`join` on thread handle {handle}"),
        }
    }
}

/// A separate lock from `STALL`, deliberately — this map is read only
/// for the abort message itself (after `STALL` has already decided a
/// deadlock is real), never as part of the decision, so it doesn't need
/// to share `STALL`'s own ordering guarantees. A brief window where this
/// map and `STALL.blocked` disagree by one entry (registered just before
/// vs. just after the count) only affects the diagnostic's completeness,
/// never whether a deadlock is correctly detected.
fn waiting_registry() -> &'static Mutex<HashMap<std::thread::ThreadId, WaitTarget>> {
    static REGISTRY: OnceLock<Mutex<HashMap<std::thread::ThreadId, WaitTarget>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Call exactly once, synchronously, at the point a `spawn` job is
/// actually handed off to run (`nir_thread_spawn`, after a successful
/// submit — never on a failed one, which never runs at all) — this
/// job is now a real concurrent participant that could, in principle,
/// be the one to unblock someone else.
pub fn concurrency_thread_started() {
    STALL.lock().unwrap().live += 1;
}

/// Call exactly once, synchronously, once `nir_thread_join` has
/// confirmed (via `thread_pool::Scope::already_done`/`join`) that a
/// spawned job's own code has fully finished running — it can no
/// longer unblock anyone.
///
/// **Deliberately not called from inside the spawned job itself, at the
/// instant its own code returns** — that would create a real, if
/// narrow, race: this counter and `thread_pool::Scope`'s own completion
/// state are two separate locks with no ordering relationship between
/// them, so a joiner's non-blocking "is it done yet" check could
/// observe "not yet" a few instructions after this counter had already
/// dropped, undercounting `live` for a job that (from the joiner's own
/// perspective) hasn't finished. Decrementing only once `join` itself
/// has *confirmed* completion ties this counter to the one fact a
/// caller can already prove, closing that gap — at the cost of a
/// finished-but-not-yet-joined job still counting as `live` (a real,
/// narrower detection gap, not a correctness one: it can only make this
/// detector miss a deadlock it otherwise would have caught, never
/// report one that isn't real).
pub fn concurrency_thread_finished() {
    STALL.lock().unwrap().live -= 1;
}

/// The actual decision, factored out from the real `nir_*` entry points
/// (`concurrency_wait_begin`) so it's unit-testable without triggering
/// a real `process::abort()` inside the test binary itself. `true`
/// means this call is the one that made every live participant blocked
/// at once — a certain deadlock, already registered (the caller is
/// still counted as blocked either way; there is no valid "un-register"
/// for a call that's about to abort the process).
fn register_wait_and_check_stall() -> (bool, usize) {
    let mut s = STALL.lock().unwrap();
    s.blocked += 1;
    (s.blocked >= s.live, s.live)
}

/// Call immediately before the real blocking wait inside `nir_thread_
/// join`/`nir_chan_recv`, naming what this specific call is about to
/// wait on (purely for the abort message — see `waiting_registry`'s own
/// doc comment) — see this section's own doc comment for the exact
/// detection guarantee. Aborts immediately on a detected deadlock
/// rather than letting the calling thread actually block forever: the
/// same "trap now, with a real diagnostic, rather than hang silently"
/// contract every other unrecoverable condition in this backend already
/// has (division-by-zero, out-of-bounds, narrow-type overflow — see
/// `codegen.rs`'s `guard_io_ok`/`guard_recv_ok`), extended here to a
/// failure kind only this runtime kernel, not generated LLVM IR, can
/// actually observe.
/// Guards the print-and-abort sequence below against a real, observed
/// race (a CI failure on `build-macos`, not a hypothetical): `STALL`'s
/// lock is only held for the `blocked >= live` check itself
/// (`register_wait_and_check_stall`), released well before this
/// function's `eprintln!`/`process::abort()` actually run. If a thread's
/// wait ends (`concurrency_wait_end`) and it immediately begins a *new*
/// one before the first detector's `abort()` has actually taken effect
/// — `eprintln!`, a real syscall, is not instantaneous — that second
/// `concurrency_wait_begin` call can independently cross the same
/// `blocked >= live` threshold a second time and race this same branch,
/// printing its own, by-then-stale snapshot of `waiting_registry`
/// (observed on CI: the same thread id reported against two different
/// channel handles across two separate printed reports, since its wait
/// target had already changed between them) before the *first* thread's
/// `abort()` actually terminates the process. Only the thread that wins
/// this compare-exchange gets to print and abort; a losing thread parks
/// briefly instead of racing to print a second, possibly-inconsistent
/// report — the winner's `abort()` will end the process very shortly
/// either way, so there's nothing else useful for the loser to do.
static DEADLOCK_REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn concurrency_wait_begin(target: WaitTarget) {
    waiting_registry().lock().unwrap().insert(std::thread::current().id(), target);
    let (deadlocked, live) = register_wait_and_check_stall();
    if deadlocked {
        if DEADLOCK_REPORTED.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            // Another thread already won the race to report and abort —
            // park here rather than exit this function normally (which
            // would let generated code proceed as if nothing were
            // wrong); the winner's `abort()` ends the whole process
            // imminently.
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
        let mut lines: Vec<String> =
            waiting_registry().lock().unwrap().iter().map(|(tid, target)| format!("  {tid:?} is blocked in {target}")).collect();
        lines.sort();
        eprintln!(
            "nirdosha: deadlock detected -- all {live} concurrently-running thread(s) are \
             blocked, with nothing left in the process that could ever unblock any of them:\n{}",
            lines.join("\n")
        );
        recorder::flush_remaining();
        std::process::abort();
    }
}

/// Call immediately after a `join`/`recv` call actually completes
/// (successfully or not — see `concurrency_wait_begin`'s doc comment,
/// there is no "unblock" path once a deadlock has already been
/// reported, since the process has already aborted by then).
pub fn concurrency_wait_end() {
    waiting_registry().lock().unwrap().remove(&std::thread::current().id());
    STALL.lock().unwrap().blocked -= 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test picks a domain the others don't touch, so they can run
    // concurrently (`cargo test`'s default) without one test's grants
    // skewing another's -- these are real `static` counters, process-
    // wide, not reset between tests.

    // `STALL`, unlike the per-domain counters above, has no such "each
    // test picks its own" escape hatch -- there's only one of it, for
    // the same reason there's only one real deadlock question for a
    // whole process. The two tests that touch it share this lock so
    // `cargo test`'s default parallelism can't interleave their
    // increments/decrements into a spurious pass or failure.
    static STALL_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn acquire_then_release_returns_to_zero_held() {
        let (held_before, _, _, _) = stats(domain::file());
        assert!(acquire(domain::file()));
        let (held_after_acquire, _, _, _) = stats(domain::file());
        assert_eq!(held_after_acquire, held_before + 1);
        release(domain::file());
        let (held_after_release, _, _, _) = stats(domain::file());
        assert_eq!(held_after_release, held_before);
    }

    #[test]
    fn denial_past_the_ceiling_does_not_increment_held() {
        // SAFETY: test-only env mutation, single-threaded within this
        // process's env (std::env::set_var is process-global) --
        // chosen a domain (Tcp) no other test in this file touches, and
        // this must run before anything else calls `acquire(Tcp)` for
        // the first time, since the ceiling is resolved once and cached
        // (`max_for`'s `OnceLock`). This is the only test touching Tcp.
        unsafe { std::env::set_var("NIRDOSHA_KERNEL_MAX_TCP", "2") };
        assert!(acquire(domain::tcp()));
        assert!(acquire(domain::tcp()));
        let (_, _, denials_before, _) = stats(domain::tcp());
        assert!(!acquire(domain::tcp()), "third acquire must be denied at a ceiling of 2");
        let (held, _, denials_after, _) = stats(domain::tcp());
        assert_eq!(held, 2, "a denied acquire must not increment held");
        assert_eq!(denials_after, denials_before + 1);
        release(domain::tcp());
        release(domain::tcp());
    }

    #[test]
    fn handle_table_mint_use_remove_is_a_clean_lifecycle() {
        let table: HandleTable<String> = HandleTable::new();
        let id = table.insert("hello".to_string());
        assert_ne!(id, 0, "0 is reserved as a caller-chosen invalid sentinel");
        assert_eq!(table.with(id, |v| v.clone()), Some("hello".to_string()));
        assert_eq!(table.remove(id), Some("hello".to_string()));
        assert_eq!(table.remove(id), None, "a double-remove must be a clean None, not a panic");
        assert_eq!(table.with(id, |v| v.clone()), None, "use-after-remove must be visible as None too");
    }

    #[test]
    fn handle_table_ids_are_unique() {
        let table: HandleTable<i32> = HandleTable::new();
        let a = table.insert(1);
        let b = table.insert(2);
        assert_ne!(a, b);
        assert_eq!(table.len(), 2);
        table.remove(a);
        assert_eq!(table.len(), 1);
    }

    // `STALL` is one process-wide static, and nothing else in this test
    // binary touches it (only `lib.rs`'s `nir_thread_*`/`nir_chan_recv`
    // do, exercised by the compiler crate's own integration tests, not
    // here) -- still balanced back to its starting point explicitly,
    // the same discipline `recorder.rs`'s own shared-state tests use,
    // so this test's own effect on global state is self-contained.
    // Deliberately only ever calls the pure `register_wait_and_check_
    // stall` -- never `concurrency_wait_begin`, which calls
    // `std::process::abort()` on a real detection and would kill this
    // whole test binary, not just fail one test.
    #[test]
    fn every_live_participant_blocked_at_once_is_detected_exactly_once() {
        let _guard = STALL_TEST_LOCK.lock().unwrap();
        // Two more participants join (three "live" total, including the
        // baseline "main thread" `STALL` starts with).
        concurrency_thread_started();
        concurrency_thread_started();

        // Two of the three block -- not everyone yet, so no detection.
        let (deadlocked_1, live_1) = register_wait_and_check_stall();
        let (deadlocked_2, live_2) = register_wait_and_check_stall();
        assert!(!deadlocked_1, "only 1 of {live_1} live participants is blocked so far");
        assert!(!deadlocked_2, "only 2 of {live_2} live participants is blocked so far");

        // The third (and last) blocks too -- now every live participant
        // is blocked at once, with nothing left that could ever run the
        // send/return that would unblock any of them.
        let (deadlocked_3, live_3) = register_wait_and_check_stall();
        assert!(deadlocked_3, "all {live_3} live participants are now blocked -- this must be reported as a real deadlock");

        // Restore the exact starting state -- three waits registered,
        // three ended; two participants started, two finished.
        concurrency_wait_end();
        concurrency_wait_end();
        concurrency_wait_end();
        concurrency_thread_finished();
        concurrency_thread_finished();

        let s = STALL.lock().unwrap();
        assert_eq!((s.live, s.blocked), (1, 0), "must return to the exact baseline once every registered wait/spawn is matched");
    }

    // A thread that's still running real work (not blocked at all) is
    // never mistaken for a rescuer that's "about to" unblock someone --
    // `live` only ever counts participants that *exist*, `blocked` only
    // ever counts ones that are actually stuck; a live-but-unblocked
    // participant keeps `blocked < live` correctly, with no detection.
    #[test]
    fn a_still_running_participant_prevents_a_false_positive() {
        let _guard = STALL_TEST_LOCK.lock().unwrap();
        concurrency_thread_started(); // two live: main + this one
        let (deadlocked, _) = register_wait_and_check_stall(); // only main blocks
        assert!(!deadlocked, "the second participant is still running, not blocked -- must not be reported as a deadlock");
        concurrency_wait_end();
        concurrency_thread_finished();

        let s = STALL.lock().unwrap();
        assert_eq!((s.live, s.blocked), (1, 0));
    }

    /// One combined test for the open domain registry, deliberately not
    /// split into separate `#[test]`s: `cargo test`'s default
    /// parallelism gives no ordering guarantee between two independent
    /// tests even behind a shared lock (a lock only prevents them
    /// *overlapping*, not which acquires it first) — and the last part
    /// of this test deliberately exhausts the process-wide registry
    /// (see its own comment below), which would make a *separately*
    /// scheduled idempotence check that happens to run afterward fail
    /// for an unrelated reason. Keeping idempotence → round-trip →
    /// exhaustion as ordered steps inside one function body is what
    /// actually guarantees that order.
    ///
    /// Covers: registering the same name twice is idempotent (returns
    /// the same [`DomainId`], doesn't mint a second one); the 7
    /// built-ins keep their historical ids 0-6 through registration and
    /// well past 300 additional registrations; a domain id past 255
    /// still resolves to its real name (proving the recorder's widened
    /// `u16` field doesn't truncate the way the old `u8` field would);
    /// and registering brand-new names all the way to [`MAX_DOMAINS`]
    /// is a loud panic, never silent truncation or wraparound.
    ///
    /// **Deliberately exhausts the process-wide domain registry** in its
    /// last step. Every other test in this crate only ever resolves
    /// already-registered names (the 7 built-ins via
    /// `domain::tcp()`/`domain::db()`/etc., idempotent lookups, not new
    /// registrations), so this doesn't break anything else in this test
    /// binary today. A future phase adding a same-process (not a
    /// spawned compiled-binary child process, which gets a fresh
    /// registry automatically) unit test that registers new domain
    /// names in *this* crate's test binary will need to account for
    /// that — flagged here rather than left as a silent trap.
    #[test]
    fn open_registry_idempotence_255_plus_domains_and_max_domains_overflow() {
        let a = register_domain("registry_idempotence_test_domain", "NIRDOSHA_KERNEL_MAX_REGISTRY_IDEMPOTENCE_TEST", 1);
        let b = register_domain("registry_idempotence_test_domain", "NIRDOSHA_KERNEL_MAX_REGISTRY_IDEMPOTENCE_TEST", 1);
        assert_eq!(a, b, "registering the same domain name twice must return the same DomainId, not mint a second one");

        domain::register_builtin_domains();
        let builtins: [(DomainId, &str); 7] =
            [(0, "tcp"), (1, "file"), (2, "thread"), (3, "db"), (4, "mq"), (5, "http"), (6, "serve_http")];
        for (id, name) in builtins {
            assert_eq!(registered_domain_name(id), Some(name), "built-in domain ids must be stable at 0-6");
        }

        let mut synthetic_ids = Vec::new();
        for i in 0..300 {
            let name: &'static str = Box::leak(format!("registry_round_trip_synthetic_domain_{i}").into_boxed_str());
            let env_var: &'static str = Box::leak(format!("NIRDOSHA_KERNEL_MAX_SYNTH_{i}").into_boxed_str());
            synthetic_ids.push(register_domain(name, env_var, 1));
        }
        let high_id = *synthetic_ids.last().unwrap();
        assert!(high_id > 255, "must have registered at least one domain with an id past the old u8 ceiling, got {high_id}");
        assert_eq!(
            registered_domain_name(high_id),
            Some("registry_round_trip_synthetic_domain_299"),
            "a domain id past 255 must still resolve to its real name, not silently truncate the way a u8 field would"
        );

        // The built-ins' ids must still read back correctly after 300
        // more domains registered after them -- registration is
        // append-only, so ids already minted never move.
        for (id, name) in builtins {
            assert_eq!(registered_domain_name(id), Some(name), "built-in domain ids must stay stable even after later registrations");
        }

        let overflowed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for i in 0..(MAX_DOMAINS + 16) {
                let name: &'static str = Box::leak(format!("registry_overflow_synthetic_domain_{i}").into_boxed_str());
                let env_var: &'static str = Box::leak(format!("NIRDOSHA_KERNEL_MAX_OVERFLOW_{i}").into_boxed_str());
                register_domain(name, env_var, 1);
            }
        }));
        assert!(overflowed.is_err(), "registering far more than MAX_DOMAINS distinct new domains must panic, not silently succeed or wrap around");
    }
}
