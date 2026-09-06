# RFC 0007: A compiled-path resource-control kernel — boundary-leased admission, fail-open telemetry, and NFRs-as-language

> **Cross-references**: read alongside `rfcs/0006-structured-concurrency.md`
> — both now bear directly on native codegen for `spawn`/`chan`, and
> §"Impact on the rest of the system" spells out where they overlap.

> **Provenance note (read this before anything else in the document).**
> This RFC has been rewritten twice already, and this version merges a
> third input:
> 1. The first draft proposed a mandatory request/grant round trip
>    through APM for every effect, grounded in `interpreter.rs`.
> 2. A scope correction re-grounded the whole document in the
>    AOT-compiled path (`codegen.rs`/`runtime-kernels`) instead, because
>    the interpreter was explicitly ruled out of scope.
> 3. A separate architecture-decision document (`apm_kernel_decision.md`)
>    was produced against draft #1 — **before the scope correction** —
>    and states its own scope as *"interpreter runtime first; compiled
>    path explicitly deferred,"* the opposite of this RFC's actual scope.
>    Its mechanism critique (reject mandatory per-effect arbitration,
>    replace with boundary-leased admission across five planes with
>    distinct failure semantics) is kept in full — it's sound
>    engineering regardless of which path it targets, and if anything
>    applies *more* forcefully to compiled code, where every nanosecond
>    of `runtime-kernels`' raw-syscall calls is the entire performance
>    case for compiling in the first place. What's been re-derived below
>    is everything that document assumed about the substrate: the
>    request boundary (no `tiny_http`/`serve.rs` in the compiled path —
>    compiled `serve` is Track B item B8, blocked), the data plane (no
>    `pool.rs`/`thread_pool.rs` to call — `runtime-kernels` is a hard,
>    separate compilation unit with none of that), which resource
>    domains exist to admit at all (only `tcp`/`file` compile today —
>    `db`/`spawn`/`chan`/`sandbox` are hard rejections), and the plugin
>    boundary (compiled plugins use a different, even more direct ABI).
>    **This version is that re-derivation, not a copy-paste.**

## Motivation

The interpreted path already has partial, optional resource management
built ad hoc, one subsystem at a time: `pool.rs` for DB connections,
`thread_pool.rs` for `spawn`, `observability.rs` as an optional tracer.
None of it is reachable from the compiled path — `runtime-kernels` is
its own Cargo workspace, statically linked as a `.a` staticlib into a
compiled binary, and its own module doc states it "cannot `use`
anything from `interpreter.rs` directly across that compilation-unit
boundary." A binary from `nirdosha build` today has, for the only two
effects that compile at all (`tcp`, `file`), a handful of raw `extern
"C"` syscall wrappers — one `connect()`/`open()` per call, no pooling,
no scheduling, no admission control, no telemetry, no audit hook.

As `db` (Track B item B2) and `spawn`/`chan`/`sandbox` (item B6) gain
codegen over time, should `runtime-kernels` grow the same way the
interpreter did — independent per-effect resource managers, never
unified, discovering their coupling only after each ships separately —
or should a single, deliberately designed resource-control kernel exist
from the start? This RFC proposes the latter, and specifies its shape
using the boundary-leased, five-plane mechanism the decision document
worked out (§1-§7 below), re-grounded entirely in what the compiled
path actually has and doesn't have.

This is an RFC because it shapes what a compiled Nirdosha binary's
execution model is (a public interface), is cross-cutting (touches
`codegen.rs`, `runtime-kernels`, and directly overlaps RFC 0006's
unbuilt concurrency codegen), and would be breaking for any `.nir`
program that already compiles and uses `tcp`/`file` — `GOVERNANCE.md`'s
bar for this process.

**This document is a design capture, not a committed plan or a
prototype.** Every numeric SLO below is an initial target to validate
with a Phase 0 harness against the compiled path's actual baseline
(raw `nir_tcp_connect`/`nir_file_open` call cost), not a number carried
over unchanged from interpreter-side or RFC-0006-prototype benchmarks —
those measured a different execution model and don't directly transfer.

## 1. Executive decision

**Reject mandatory per-effect arbitration for the compiled path, same
as for the interpreter, for the same reasons and one additional one.**
A synchronous request/grant round trip in front of every `nir_tcp_*`/
`nir_file_*` call (and, eventually, `nir_db_*`/`nir_spawn_*`) puts a
control-plane decision on the hot path of calls that exist specifically
*because* they're supposed to be cheap, unmediated syscalls — that is
the entire point of compiling instead of interpreting. Regressing that
is a more direct contradiction of the compiled path's purpose than the
same mistake would be in the tree-walking interpreter, which already
pays AST-dispatch overhead per operation.

**The goals survive, re-scoped to what actually exists to admit today.**
Static resource manifests, admission control, NFRs-as-language,
wait-for-graph deadlock handling (narrowed to conditional
prevention + bounded detection, not blanket freedom — see §8), unified
resource accounting, and fail-open telemetry are all worth designing in
from the start of `runtime-kernels`'s growth, rather than retrofitted
once `db`/`spawn` already exist as independent, uncoordinated kernels.

**The replacement, one paragraph, re-grounded**: admission happens once
per **classified boundary unit** — for the compiled path today, that's
an accepted TCP connection or an opened file, not an HTTP request (no
compiled `serve` exists yet; see §4.2) — for every resource domain the
static manifest can bound. Admission is owned by a sharded
resource-control plane built inside `runtime-kernels` in the same
`extern "C"`/opaque-integer-handle idiom its existing `nir_tcp_*`/
`nir_file_*` functions already use, **not** by exposing Rust types
across the compilation-unit boundary. Telemetry is passive, fail-open,
non-authoritative — and, unlike the interpreter's `observability.rs`,
there is no existing zero-cost-when-disabled contract to violate here,
because nothing like it exists in the compiled path yet. That's a real
advantage of this scope: whatever overhead this introduces is new,
deliberately chosen overhead, not a broken promise.

**Ship immediately, unrelated to this RFC's scope**: the original
draft's §6 finding — `TCP_NODELAY` never set in `crates/compiler`, the
interpreter-side APM handler's double `write_all` per span — is
interpreter-side (`observability.rs::handle_otel_client`), entirely
outside this RFC's now-compiled-only scope, but still a real, small,
independent fix worth its own PR regardless.

## 2. What's missing, re-prioritized for the compiled-path baseline

The decision document's own P0/P1/P2 gap list is kept, with two new
top-priority items this scope correction surfaces and one item resolved
outright by the correction:

| Priority | Item | Status under compiled-only scope |
|---|---|---|
| **P0 (new)** | **What is the admission boundary, absent compiled `serve`?** | Not answered by the decision doc — it assumed `tiny_http`. Resolved provisionally in §4.2: accepted connection / opened file is the boundary until B8 ships. |
| **P0 (new)** | **What does the data plane call, given there's no `pool.rs`/`thread_pool.rs` to wrap?** | Resolved in §3: new C-ABI primitives inside `runtime-kernels`, built new for `db`/`spawn` in lockstep with their codegen (B2/B6), not a lease check bolted onto existing Rust modules. |
| P0 | Mechanism alternative to per-effect arbitration | Kept from the decision doc — boundary leasing, §3-§4. |
| P0 | Hot path / slow path definition | Kept — §4, re-baselined against `runtime-kernels`' raw-syscall cost, not the interpreter's. |
| P0 | Failure semantics | Kept — §3's failure table, unchanged in structure. |
| P0 | Telemetry architecture | Kept in principle (§6-§7) but the pipeline must live inside `runtime-kernels`'s own Cargo workspace and export via a mechanism a standalone compiled binary can actually configure — there's no `--otel-port` CLI flag for a `nirdosha build` output. New open item, not in the original decision doc. |
| P1 | Multi-tenancy and fairness | **Substantially moot for now** — multi-tenant admission presumes multiple concurrent principals arriving at a shared boundary, which is what compiled `serve` (B8, blocked) would provide. A standalone compiled binary today is closer to single-tenant. Deferred, not designed away — revisit at B8. |
| P1 | Narrowed deadlock claims | Kept unchanged — §8. |
| P1 | NFR governance | Kept as open work — §9, with the CLI-knob question replaced (compiled binaries have no `NIRDOSHA_{PREFIX}_POOL_*`-equivalent surface today; a new one would need designing, not migrating). |
| P1 | Spawn-admission blocking hazard | **Reframed, not just kept**: there is no `thread_pool.rs` in the compiled path to protect from re-introduced deadlock — a compiled spawn scheduler doesn't exist yet. The hazard is real but prospective: whatever compiled thread scheduler eventually gets built (as part of B6) must have this property from its first line of code, not retrofitted. See §8. |
| P2 | Authorization/admission error taxonomy | **Resolved as inapplicable, for now**: `acquire`/`requires(...)` don't compile (`codegen.rs:946-949` rejects `Expr::Acquire`) — there's no `PrivilegedFnNotAcquired`-equivalent in the compiled surface to conflict with `AdmissionDenied`. Reopens if `acquire` ever gains codegen. |
| P2 | Numeric SLOs and rollout plan | Kept, re-baselined — §5, §10. |
| P2 | Global sequencing as a serialization point | Kept — replay stays opt-in debug mode, §6. |

## 3. Recommended target architecture: five planes, re-grounded in `runtime-kernels`

Same five planes, same rule (no plane synchronously depends on a plane
to its right) — re-specified against the actual substrate:

```text
Compiler / static plane (crates/compiler, frontend — unchanged by path)
  -> manifests, NFR validation, composition checks
Local admission plane  (NEW: inside crates/runtime-kernels)
  -> sharded lease issuers; O(1) grant/deny; per-domain failure policy
Data plane (hot path)  (crates/runtime-kernels' existing + new kernels)
  -> nir_tcp_*/nir_file_* today; nir_db_*/nir_spawn_* once B2/B6 land;
     O(1) local lease check inserted at each call site by codegen.rs
Telemetry plane  (NEW: inside crates/runtime-kernels)
  -> rings -> aggregator -> exporter; fail-open, drop-with-accounting;
     export/config mechanism TBD (no CLI surface exists for a
     compiled binary today — see §2's new P0 item)
Global coordination plane  (NEW, likely deferred past Phase 4 for now)
  -> only meaningful once there are 2+ real resource domains to
     coordinate across; today that's tcp+file, which barely needs it
```

**Compiler / static plane.** Unchanged from either prior draft: a new
compiler-frontend pass walks each handler's/function's call graph and
emits a manifest of symbolic bounds (resource domains touched, upper
bounds, unknown/dynamic markers, declared NFRs). This runs identically
whether the program is later interpreted or compiled — it is genuinely
path-agnostic, unlike everything below it.

**Local admission plane.** The one plane this RFC's compiled-only scope
changes the most. It cannot be `Arc`-based Rust shared with the
compiler crate — `runtime-kernels`'s existing kernels are already
`extern "C"` functions over `i64` handles (`nir_tcp_connect`, etc.),
and any lease-issuing code added alongside them must match that idiom
at the **call boundary** codegen.rs emits into. Internally, though,
`runtime-kernels` is free to use ordinary Rust (its own Cargo
dependencies, atomics, threads) — the compilation-unit boundary only
constrains what crosses *into* generated LLVM IR, not how the crate
implements itself. Concretely: `codegen.rs` would emit a call to
something like `nir_lease_acquire(domain: i64, count: i64) -> i64`
(returning a lease token or `-1`) immediately before the existing
`nir_tcp_connect`/`nir_file_open` call, and `nir_lease_release(token:
i64)` after `nir_tcp_stop`/`nir_file_stop` — the same "thin C-ABI
wrapper over a real Rust implementation" pattern `runtime-kernels`
already uses for everything else.

**Data plane.** Today: the existing `nir_tcp_*`/`nir_file_*` kernels,
unchanged, called directly after a lease is granted. Once B2/B6 land:
whatever `nir_db_*`/`nir_spawn_*` kernels get built as part of that
codegen work — **built together with the lease-check wrapper from day
one**, not shipped first and retrofitted later, which is the
interpreter's actual history (`pool.rs`, `thread_pool.rs`, and
`observability.rs` were all built independently, at different times,
and are still not unified today — see Motivation).

**Telemetry plane.** Same rings → aggregator → exporter design as the
decision document (§6-§7 below carry it over largely unchanged, since
it's an internal-to-`runtime-kernels` implementation detail, not
something that crosses the ABI boundary at every call site — only the
`try_send` in step 3 of the hot path does). What's new: a compiled
binary has no existing config/lifecycle surface (`--otel-port` is a
`nirdosha run`/`serve` CLI flag, meaningless for a `nirdosha build`
output binary). Exporting telemetry from a compiled binary needs new
plumbing — environment variables read at process startup, or values
baked in at `nirdosha build` time — not decided here, flagged as new
open work the decision document didn't need to solve.

**Global coordination plane.** Named for completeness, but **its exit
criteria are close to vacuous today**: cross-shard wait-for sweeps
matter once there are multiple real, contended resource domains. With
only `tcp` and `file` compiling, and no DB/spawn contention possible
yet, there's very little for a coordinator to coordinate. This plane's
real design work should wait for B2/B6, not be speculatively built now.

**Failure behavior table** — kept unchanged from the decision document;
it's a general design principle (fail-open telemetry, fail-closed hard
safety, local shards continue on last-known policy when a coordinator
is unreachable) that doesn't depend on interpreted-vs-compiled.

## 4. Exact paths, re-grounded

### 4.1 Hot path (synchronous, per effect, at the `nir_*` call site)

1. **Local lease check** — a new `extern "C"` call `codegen.rs` emits
   immediately before each `nir_tcp_*`/`nir_file_*`/(future)
   `nir_db_*`/`nir_spawn_*` call. O(1), reads task-local lease state,
   decrements an atomic, no allocation, no lock.
2. **Direct resource call** — the existing kernel function, unchanged,
   called with no indirection.
3. **Nonblocking event emission** — `try_send` into a preallocated ring
   inside `runtime-kernels`'s own telemetry state. A full ring
   increments a drop counter and continues.

**Forbidden on the hot path** — unchanged from the decision document:
global mutexes, cross-thread wakeups, remote/control-plane calls,
telemetry backpressure, wait-for-graph cycle checks, allocation on the
telemetry channel.

**Exempt from admission**: `chan` send/recv (memory, not an external
resource — see §8) and pure compute — moot today since `chan` doesn't
compile at all yet, but worth deciding now so it isn't accidentally
included once it does.

**Budget**: added latency per effect of at most ~100ns is the decision
document's target; it needs re-validation against `runtime-kernels`'s
actual current call cost (a raw `nir_tcp_connect` is one syscall — the
relevant question is what fraction of that this adds, not an absolute
number assumed to transfer from a different substrate).

### 4.2 Boundary path — redefined, since compiled `serve` doesn't exist

The decision document's §4.2 said: *"`tiny_http` already surfaces
method, path, and headers before the body is read, so classification
precedes body consumption."* **This is entirely inapplicable to the
compiled path** — `tiny_http`/`serve.rs` is interpreter-only, and
compiled `serve` is Track B item B8, explicitly `[BLOCKED: B1-B7]`.
There is no HTTP request to classify in a compiled binary today.

What the compiled path actually has as a boundary, today:

1. **An accepted TCP connection** (`nir_tcp_accept`) or **an opened
   file** (`nir_file_open`) is the only unit available to reserve a
   lease against — this is exactly the decision document's §4.3 point
   3 fallback ("raw socket entry points... no classifiable request
   head... coarse per-tenant/per-connection budget at accept time").
   **For the compiled path, that fallback is the primary and, for now,
   only case** — not a corner case of a request-classification system
   that mostly doesn't apply here.
2. Once `spawn` compiles (B6), a spawned task's entry point becomes a
   second, more natural boundary unit — closer in spirit to "a request"
   — since a task has a definable start and a manifest of what it will
   do, the way a request handler does in the original design.
3. Once compiled `serve` ships (B8, after B1-B7), the original
   request-classification design becomes applicable again in close to
   its originally proposed form, and this section should be revisited
   rather than assumed to transfer unchanged — a compiled HTTP server
   might reuse `tiny_http`'s crate directly, or something else; not
   knowable from here.

### 4.3 Dynamic/unknown-effect path

Kept from the decision document, re-ordered to reflect what's actually
common in the compiled path today: point 3 ("raw socket entry points...
coarse per-tenant/per-connection budget") is promoted to the default
case per §4.2 above, not the exception. Point 4 (plugin-internal IO
charged as `unknown` to the caller's coarse budget) is unchanged in
substance but the plugin ABI it refers to is different — see §8.

### 4.4 Slow path

Unchanged in principle from the decision document — everything
asynchronous runs off whatever worker mechanism ends up executing
compiled `.nir` code (today: the OS threads a compiled binary's `main`
and any raw `tcp`/`file` handling run on directly; once B6 lands,
whatever thread scheduler that codegen work produces).

## 5. How this avoids becoming a bottleneck

The decision document's "bounded-impact contract, not an absolute
guarantee" framing is kept in full — it's the right framing regardless
of path. The five clauses transfer unchanged. **The numeric SLO table
does not transfer unchanged** — every number in the decision document
was proposed against an unspecified or interpreter-flavored baseline.

**This has now been measured.** The Phase 0 harness this RFC called for
exists at `rfcs/evidence/0007-apm-runtime-kernel/kernel_bench/` (see its
README for full methodology and both runs' numbers) — it links against
the *actual compiled* `runtime-kernels` staticlib, the same way
`crates/compiler/build.rs` builds it for a real `nirdosha build`
binary, and times each `nir_tcp_*`/`nir_file_*` kernel against the raw
`std` call it wraps. Results, i7-8550U/Linux, best of 3:

| Call | Measured (zero admission) |
|---|---:|
| `nir_tcp_connect`+`stop` (boundary) | ~26–27 µs |
| `nir_tcp_accept`+`stop` (boundary, CAVEAT: backlog kept warm) | ~23–25 µs |
| `nir_file_open`("w")+`stop` (boundary) | ~2.4–2.6 µs |
| `nir_tcp_send` (hot path, 8B) | ~1.3 µs |
| `nir_tcp_recv` (hot path, 64B, CAVEAT: warmed by a writer) | ~0.8–0.9 µs |
| `nir_file_write`/`read` (hot path, 4096B) | ~0.9–1.7 µs |

Two findings this resolves:

1. **The `extern "C"` wrapper itself adds no measurable overhead** —
   every kernel lands within normal run-to-run noise of its raw `std`
   equivalent. §1's premise (these really are thin, near-zero-cost
   wrappers) holds.
2. **The two SLO targets are not equally at risk.** The ~50µs
   boundary-reservation target has real headroom — `connect`/`accept`/
   `open` cost ~2.4–27µs today, so a lease check would need to roughly
   double the slowest of these before threatening the target. The
   ~100ns hot-path target is the one to watch: `send`/`recv`/`write`/
   `read` cost **0.8–1.8µs with zero admission logic today**, so a
   100ns lease check would be a real, non-trivial **5–15% overhead** on
   exactly the calls that exist because they're supposed to be cheap —
   not negligible the way it is for the boundary calls.

**Still unmeasured, and unmeasurable until Track B lands**: `db`
(B2) and `spawn`/`chan`/`sandbox` (B6) don't compile, so this harness
says nothing about those domains. Whatever kernels eventually back
them need this same measurement repeated against their own baseline.

## 6. Telemetry/data transmission design

Kept close to the decision document's design — this is genuinely an
internal-to-`runtime-kernels` implementation question (ring buffers,
aggregator thread, batching, OTLP export, spill, sampling, redaction
vs. replay-fidelity separation) that doesn't change shape based on
interpreted-vs-compiled, since none of it crosses the ABI boundary
except the single `try_send` per effect. The one addition: since
`runtime-kernels` has no existing debug-only JSON-lines/TCP listener
(that's `observability.rs`'s Layer 2a, interpreter-only), there's no
legacy interim transport to keep around as "debug only, known
architectural problem" the way the decision document frames it for the
interpreter — a compiled-path telemetry exporter can be designed
without that particular piece of debt from day one, if this is built
after that finding is internalized.

## 7. Metrics design

Kept essentially unchanged from the decision document — cgroup v2/PSI-
based OS metric collection describes a Linux process, and a compiled
`.nir` binary is exactly as much an OS process as an interpreter
invocation is. The one substantive difference: "thread-pool state" and
"DB-pool state" rows in the decision document's signal table refer to
`thread_pool.rs`/`pool.rs`'s internal counters, which don't exist in
the compiled path. Those rows apply only once B2/B6 land and the
compiled-path equivalents (§3's new `nir_db_*`/`nir_spawn_*`-adjacent
lease issuers) exist to instrument. Everything else in the table (CPU,
IO, memory, network, host steal time) is unaffected by this RFC's
scope correction.

## 8. Deadlock, race, spawn, plugin, and RFC 0006 decisions — re-grounded

**Deadlock — narrowed claims, unchanged from the decision document.**
Conditional prevention for statically known plans, bounded
detection/recovery otherwise — not blanket freedom, doesn't solve RFC
0006's nested reply-obligation cycles, doesn't handle unknown future
claims. This holds regardless of path.

**Race-freedom.** Also unchanged in principle: races are the
compiler's job (`ownership.rs`, and RFC 0006's capability types if
adopted); leases cover external scarce resources only. Re-stated for
this scope: since `ownership.rs` already runs identically for compiled
and interpreted programs (it's a frontend pass), this guarantee is
exactly as strong or weak for compiled `.nir` as for interpreted `.nir`
today — this RFC's kernel doesn't change that either way.

**Spawn — the one place this RFC's compiled-only scope changes the
substance of the decision, not just its wording.** The decision
document's spawn design (admission never blocks executor workers,
waiters park as continuations, preserves `thread_pool.rs`'s
eager-growth protection, parent/child priority inheritance) describes
properties a compiled thread scheduler *should* have — but that
scheduler doesn't exist. `spawn`/`thread` are hard `codegen.rs`
rejections today, and RFC 0006's own Pillars 1-4 prototype is pure
Rust with no native-codegen story (its own critic section: compiling
this from `.nir` source means either linking a real Rust concurrency
runtime into the compiled binary, or hand-rolling structured
concurrency from raw OS threads inside `codegen.rs`/`runtime-kernels`
— "neither attempted"). **This RFC's compiled-path spawn-admission work
and RFC 0006's unbuilt compiled-concurrency codegen are the same
missing piece, not two that can be sequenced independently** — you
cannot design lease-aware spawn admission for a scheduler that hasn't
been designed yet. The decision document's spawn-admission properties
should be treated as **requirements RFC 0006's eventual compiled-path
design must satisfy**, not as something this RFC can specify in
isolation.

**Plugin boundary — same conclusion, different (and harder) ABI.** The
decision document's rule ("IO inside a native plugin's own code is not
mediated, declared `unknown`, charged to the caller's coarse budget")
holds, but the compiled path's plugin mechanism is
`NativePluginBuiltin` (`plugin.rs:125-180`) — `#[no_mangle] extern "C"`
symbols linked directly into the compiled binary's `.a` staticlib, not
the interpreter's `HashMap<String, PluginFn>` runtime dispatch. This is
if anything a **harder** case to intercept: a compiled call to a native
plugin builtin is a direct linked-symbol call with no dispatch point to
insert a lease check into, short of either changing the plugin ABI
itself or having `codegen.rs` wrap every call site to a
`NativePluginBuiltin` symbol the same way it would wrap
`nir_tcp_connect`. Whether that wrapping is feasible without plugin
cooperation is an open question this document doesn't resolve.

**RFC 0006 relationship — tighter than the decision document assumed.**
The decision document says "Pillars 1-4 land first" as if RFC 0006's
work and this RFC's spawn-admission work were sequential but separable.
Under the compiled-only scope, they aren't separable in the way just
described above — RFC 0006's compiled-path story and this RFC's
data-plane story for `spawn` are one piece of work. The two deadlock
graphs (resource-acquisition here, reply-obligation in RFC 0006's
Pillar 5) remain genuinely separate concerns within that combined
effort, and neither subsumes the other. `chan` stays out of kernel
mediation regardless, once it compiles.

**Update, now that `spawn`/`join`/`chan`/`send`/`recv` are genuinely
compiled**: the paragraph above was written when neither existed in the
compiled path at all — there was no real blocking wait for either
deadlock graph's kernel-side half to observe yet. That's no longer
true, and it opens a real, additive option this RFC's admission
mechanism can now offer *the reply-obligation graph specifically*,
without waiting for Pillar 5's own static proof (still deferred,
`rfcs/0006-structured-concurrency.md`'s own recommendation, unchanged):
a **dynamic stall detector**, `runtime-kernels/src/kernel/mod.rs`'s
`concurrency_wait_begin`/`_end`/`concurrency_thread_started`/`_finished`,
wired into `nir_thread_join`/`nir_chan_recv`. It tracks two counts —
how many `.nir`-level concurrent participants exist (`live`: main, plus
one per outstanding `spawn`) against how many are, right now, blocked
in one of exactly the two operations that can only ever be unblocked by
*another* one of those participants (never `tcp`/`file` I/O, which can
always still resolve from outside the process). If every live
participant is simultaneously in that state, nothing left in the
process could ever run the `send`/return that would unblock any of
them — reported and aborted immediately (this crate's own "trap now,
don't hang silently" convention — `codegen.rs`'s `guard_io_ok`/
`guard_recv_ok`), not left to hang.

This is deliberately the same technique Go's own runtime uses ("fatal
error: all goroutines are asleep - deadlock!"), not a general wait-for-
graph cycle detector, and it inherits that technique's one real
limitation honestly: it only fires once the *whole* program can never
move again, not a *local* cycle between two threads while a third,
unrelated one keeps making progress. Getting this exact — no false
positives, ever, for a mechanism that is otherwise invisible to a
`.nir` author and could otherwise silently kill a correct program — took
one real correctness bug, found by actually running the existing
`channels.nir` example repeatedly, not by inspection: naively
incrementing `blocked` and checking it *before* confirming the wait
would actually block raced a fast-finishing `spawn` (a producer that
already sent everything and returned before its consumer's first
`recv` even ran) into a spurious detection. Fixed by trying the
non-blocking path first (`Receiver::try_recv`/`Scope::already_done`)
and only registering a wait — ever — once that's confirmed empty/not-
done, plus moving the `live` decrement for a finished job to the point
its own `join` actually confirms completion (not the instant the job's
code returns), closing a second, narrower race between two separately-
locked counters. See `kernel::concurrency_thread_finished`'s own doc
comment for the exact ordering argument.

**How this relates to Pillar 5, precisely**: this is a real, working
backstop for the reply-obligation deadlock class specifically —
`tests/codegen.rs`'s `a_nested_reply_obligation_deadlock_is_detected_
and_aborted_not_hung` compiles and runs `fixtures/deadlock.nir` (the
exact shape RFC 0006's own Pillar 5 evidence names: A sends a request
and blocks on the reply; B needs one more answer from A to compute it,
which A is no longer running any code to provide) and confirms it
aborts in under a second instead of hanging. It is *detection*, not
*prevention* — it does not earn Pillar 5's actual proof-by-construction
claim (a well-typed `.nir` program can express this shape perfectly
well today, precisely because Pillar 5 doesn't exist yet to reject it
at compile time), only makes hitting it fail fast and diagnosably —
naming the actual stuck call sites (`kernel::WaitTarget`, e.g. "thread
X is blocked in `recv` on chan handle 3") — instead of hanging forever
with no signal at all. True *prevention* (refuse the specific request
that would create the cycle, before blocking) doesn't fit `chan`
either: unlike a mutex, a channel has no single "owner" to check a
request against (any of potentially many senders could satisfy a
`recv`), so there is no well-defined wait-for edge to test ahead of
time the way there would be for an exclusive lock — this is exactly
why Pillar 5's own answer is a *type-level* constraint (levels), not a
runtime graph. `join`'s own graph has no cycles to prevent in the first
place: affine `thread` handles already form a DAG by construction
(`rfcs/0006-structured-concurrency.md`'s own claim). The resource-
acquisition graph (tcp/file/thread ceilings, admission control) remains
this RFC's own, separate concern, unaffected by any of this.

**Housekeeping alongside it, same commit**: `thread` now has its own
admission `Domain` (`Domain::Thread`, `NIRDOSHA_KERNEL_MAX_THREAD`),
the same concurrently-held ceiling `tcp`/`file` already enforce,
acquired at `spawn` and released at the `join` that closes it — `chan`
deliberately still has none (a channel handle is never released, so a
concurrently-*held* ceiling doesn't fit its lifecycle; a total-ever-
created cap would be a different kind of limit, not built here). `db`/
`mq` are explicitly out of scope for the stall detector even once they
compile: both can block on a genuinely external system that might
still resolve from outside the process, which is exactly the property
that makes `tcp`/`file` safe to exclude today — counting them toward
"can only ever be unblocked by another concurrent participant" would
reintroduce false positives, not close a gap.

## 9. Security, tenancy, NFR governance, and operations

**Multi-tenancy — deferred, not designed.** The decision document's
tenant-identity-at-ingress design presumes a request-serving boundary
(compiled `serve`, B8) that doesn't exist. A standalone compiled binary
today has no natural multiple-tenant concept. This section is
explicitly parked until B8 unblocks it, rather than force-fit onto a
substrate that doesn't have the concept it needs.

**Authorization-oracle risk — moot today, reopens later.** The
decision document's concern (don't let `AdmissionDenied` leak
authorization facts distinguishable from `PrivilegedFnNotAcquired`)
doesn't apply because `acquire`/`requires(...)` don't compile
(`codegen.rs:946-949`). Reopens exactly if/when `acquire` gains codegen
— worth remembering rather than assuming permanently resolved.

**NFR governance — partially landed, 2026-09 — validation ranges done,
precedence/composition/per-principal ceilings/staged rollout still
open.** `nfr(latency_ms:, error_rate_max:, throughput_min_per_sec:,
concurrency_max:)` (`docs/LANGUAGE.md` §6f) is now a real, compiled
fn-level annotation — the single-function-scoped slice of "NFRs-as-
language" this section used to describe entirely in the future tense.
Per-field validation ranges are decided and enforced at parse time
(each threshold positive, `error_rate_max` in `[0.0, 1.0]`); tracking is
O(1) state per function via `runtime-kernels::kernel::nfr`, with
async, fail-open escalation to `NIRDOSHA_OBSERVABILITY_URL` on a
crossed threshold — the "configuration surface a compiled binary gets
at all" question below is answered, at least for this one purpose, by
that one env var. What's still genuinely open, unchanged from the
decision document: **composition** (nothing checks two `nfr(...)`
declarations across a call chain for contradiction, e.g. a callee's
`latency_ms` budget exceeding its caller's), **precedence** (no notion
of an env/deployment override beneath a declared `nfr(...)` value —
today the source annotation is the only value, unconditionally),
**per-principal ceilings** (a `concurrency_max` is global to the
function, not scoped per caller/tenant — multi-tenancy is still parked,
above), and **staged rollout** (no shadow/observe-only mode for a new
`nfr(...)` — day one is already live enforcement + escalation, the same
"ship live rather than shadow-first" choice §10's Phase 2/3 admission
mechanism made). The one now-answered half of the decision document's
own question: its CLI/env-knob framing (`--otel-*`,
`NIRDOSHA_{PREFIX}_POOL_*`) still doesn't apply (those remain
`nirdosha run`/`serve` flags with no compiled-binary equivalent), but
"what configuration surface does a compiled binary get at all" now has
one concrete instance, `NIRDOSHA_OBSERVABILITY_URL` — not a general
answer, just proof the question is answerable.

**Operations.** Observe-only-first rollout, `AdmissionDenied` as a new
failure mode requiring compatibility/migration tests, kernel health as
part of the operational surface from Phase 1 — all unchanged in
principle. Non-loopback telemetry export requiring TLS/auth is
unchanged and, if anything, more clearly necessary here since a
compiled binary is more likely to be a standalone deployed artifact
than a `nirdosha serve` process an operator already controls.

## 10. Phased roadmap, re-sequenced for what the compiled path actually has

| Phase | Content | Exit criteria | What changed from the decision document |
|---|---|---|---|
| **0 — Foundations** | ✅ **Done.** Phase 0 harness measures the zero-admission baseline — see §5. | ✅ Done. | The decision doc's Phase 0 assumed an interpreter/RFC-0006 baseline; this measured the actual compiled-path substrate instead, and found the two SLOs are not equally at risk (§5). |
| **2/3 — Admission mechanism, live** | ✅ **Built and measured**, ahead of the sequencing this table originally proposed. `crates/runtime-kernels/src/kernel/`: one atomic compare-and-swap per domain (`Tcp`, `File`), gating `nir_tcp_connect`/`nir_tcp_listen`/`nir_tcp_accept`/`nir_file_open` only — never `send`/`recv`/`read`/`write`, resolving §5's "which SLO is at risk" finding by not putting admission on that path at all, rather than by hitting an aggressive nanosecond budget on it. `kernel_bench` re-run against it: every boundary call within run-to-run noise of the original baseline (`rfcs/evidence/0007-apm-runtime-kernel/README.md`'s "Update" section) — no measurable regression. A generic `HandleTable<T>` also now exists, unwired, for the next resource domain (`json`/`db`/`mq`) to use instead of inventing its own table. **Also now built, both unwired, both ported near-verbatim from the interpreter-side modules of the same name (real design, zero interpreter dependency, confirmed by actually checking before porting)**: `kernel::pool` (generic `r2d2`-backed connection *pooling* — reuse, distinct from admission) and `kernel::thread_pool` (eager-growth reused-worker OS thread pool, the exact deadlock-avoidance design `rfcs/0006`'s spawn/join concerns need) — 17 total unit tests across all three modules, all passing, including `thread_pool`'s own adversarial suite (panic containment, flaky-spawner injection, deep spawn/join chains that would deadlock a bounded pool). **One real, disclosed gap found while porting, not before**: `thread_pool`'s panic containment (`catch_unwind`) only works under a profile with unwinding enabled; this crate's `[profile.release]` sets `panic = "abort"` (the profile a real compiled `.nir` binary actually ships as), where a panicking spawned job would abort the whole process — flagged prominently in `thread_pool`'s own module doc as unresolved, not silently carried over. **Still open**: the compiler-side manifest pass (frontend, path-agnostic) hasn't been built; `AdmissionDenied` as a distinct, surfaced error kind (today a denial folds into the same `-1` every other failure returns); neither `pool` nor `thread_pool` is wired to any `nir_*` kernel yet, since `db`/`mq`/`spawn` don't compile. | Real code now exists in the tree, not just a measured baseline — the two originally-separate phases (build a lease-check stub; measure its cost) happened together, and the mechanism landed as live enforcement (generous ceiling) rather than pure shadow/observe-only mode first. The pooling/worker-reuse primitives are new relative to the original table entirely — added after explicitly re-checking whether anything from the deleted interpreter-side modules should be resurrected into the kernel's foundation now, rather than reinvented per-domain later. |
| **1 — Telemetry data plane** | `kernel.rs`'s own self-metrics (`stats()` — grants/denials/currently-held per domain) exist now, in the smallest possible form (§7's "kernel self-metrics are first-class" principle) — no exporter, no rings, no aggregation, not yet called from anywhere. Still open: rings/aggregator/batching/exporter, and a new compiled-binary config/export mechanism (env-var or build-time, undesigned). | Telemetry overhead within re-derived targets; 100% drop accounting; a decided answer for how a compiled binary is told where to export telemetry. | Added the config-mechanism exit criterion — the decision doc didn't need one, since `nirdosha serve` already has `--otel-*` flags (and `serve` no longer exists at all as of this session's separate interpreter-removal work). |
| **Manifests** | Compiler manifest pass (frontend, path-agnostic) — not started. | Manifests emitted for classifiable call sites; declared bounds feed the kernel's ceilings instead of the current hardcoded default. | Unchanged from the original table's Phase 2 half. |
| **`db`/`spawn` enforcement** | `db` once B2 lands; `spawn` budget only once B6's compiled scheduler exists (see §8's spawn note) — both still fully blocked on codegen that doesn't exist. | `AdmissionDenied` compatibility tests pass for each as it lands. | Unchanged — the decision doc's "DB pools first" ordering is still inverted here, since DB doesn't compile yet. |
| **4 — Global coordination and deadlock recovery** | Likely light/deferred until 2+ real contended domains exist (post-B2) — moot with only `Tcp`/`File` today. | Cross-shard detection SLOs, but only meaningful once there's more than one shard worth coordinating. | Unchanged. |
| **5 — NFR composition, replay** | Single-function `nfr(...)` declaration, O(1) tracking, and threshold escalation are now ✅ **done** (`docs/LANGUAGE.md` §6f, §9 above) — ahead of this row's original sequencing, same pattern as Phase 2/3's admission mechanism landing early. Still open, as originally scoped: composition checks across a call chain, precedence/override semantics, per-principal ceilings, and opt-in replay. The decision document's "interpreter-parity revisit" is now moot outright, not just deferred: the interpreter was removed entirely in a separate pass this session (`run`/`serve`, `interpreter.rs`, and every module that only existed to serve it are gone from the tree) — there is no interpreter left to reach parity with. | Composition checks reject contradictory declarations; replay artifacts meet access-control rules. | The parity question this row used to defer is now closed by events, not by decision; single-function declaration/tracking/escalation moved from "not started" to "done" ahead of the rest of this row. |

**Ordering constraint, tightened**: RFC 0006's compiled-path concurrency
codegen and this RFC's `spawn` admission are not sequential — they are
the same work item (§8). Phase 3's `spawn` budget cannot start before
that combined design exists, not merely "before Pillars 1-4 land" as
the decision document phrased it for the interpreter.

## 11. Final recommendation

**Adopt the decision document's mechanism (reject mandatory per-effect
arbitration; boundary-leased admission across five planes with
independent failure semantics; "never a bottleneck" replaced by a
bounded-impact contract with numeric SLOs) — for the compiled path,
re-derived rather than copied.** The reasoning that killed mandatory
per-effect arbitration for the interpreter applies at least as strongly
here: `runtime-kernels`'s entire reason to exist is being a thin,
unmediated layer over real syscalls, and a synchronous control-plane
hop on every call defeats that purpose more directly than it would for
an already-slower interpreter dispatch loop.

**What's genuinely different from just "the same design, compiled":**
almost nothing to admit exists yet (`tcp`/`file` only), the natural
admission boundary isn't a classified HTTP request (compiled `serve` is
blocked on B1-B7), the data plane has to be built from scratch rather
than wrapped (no `pool.rs`/`thread_pool.rs` equivalent), the plugin
boundary is a harder, more direct linked-symbol case, and — most
importantly — the `spawn` admission piece cannot be designed
independently of RFC 0006's still-unbuilt compiled concurrency codegen;
they are one project, not two sequenced ones.

**Sequencing**: Phase 0's compiled-path benchmark harness is
**done** (`rfcs/evidence/0007-apm-runtime-kernel/kernel_bench/`) —
every SLO is now checked against real `runtime-kernels` costs, not
borrowed numbers, and the finding that matters most for what comes next
is that the ~100ns hot-path target has far less headroom than the
~50µs boundary target (§5). Next: prototype a minimal lease-check stub
and measure its actual added cost against this baseline, before
building the telemetry plane or manifest/observe-only admission for
`tcp`/`file`; `db` enforcement once B2 lands; `spawn` enforcement only
as part of RFC 0006's compiled-concurrency design, not before or
independently of it; global coordination and multi-tenancy deferred
until there's enough real domain contention (post-B2) and a real
multi-principal boundary (post-B8) to justify them.

**Assumptions needing confirmation**: that a lease-check `extern "C"`
call at each `nir_*` call site is cheap enough relative to the syscall
it guards to be worth doing before `db`/`spawn` exist to justify the
investment (only the Phase 0 harness can answer this); that
`NativePluginBuiltin` call sites can be wrapped by `codegen.rs` without
changing the plugin ABI itself; that deferring multi-tenancy and global
coordination until B2/B6/B8 land is acceptable rather than a reason to
not start this work yet at all. This document is design-only; no
source files are changed by it.
