# Evidence for RFC 0007: Phase 0 benchmark harness

`kernel_bench/` measures the actual, current, zero-admission cost of
`crates/runtime-kernels`'s `nir_tcp_*`/`nir_file_*` kernels — the only
two effects that compile at all today (`db`/`spawn`/`chan`/`sandbox`
are still hard `codegen.rs` rejections). RFC 0007 §5 needs this before
any of its numeric SLOs can be treated as more than a guess.

## What it does

`kernel_bench` links against the *real* compiled `runtime-kernels`
staticlib — built the same way `crates/compiler/build.rs` builds it for
an actual `nirdosha build` binary (`cargo rustc --release
--target-dir <private dir> -- --print=native-static-libs`), not a
reimplementation of its logic — and calls each `nir_tcp_*`/`nir_file_*`
function directly via `extern "C"`, timed with the same "best of 3,
ns/iter" methodology `rfcs/evidence/0006-structured-concurrency`'s
`bench.rs` uses. Each kernel is benchmarked side by side with the raw
`std` call it wraps, to separate two different numbers:

- the **syscall floor** (connect/accept/open/send/recv/read/write) —
  nothing about RFC 0007 can reduce this;
- the **`extern "C"` wrapper's own overhead** on top of that floor —
  the actual proxy for "what would one more `extern "C"` lease-check
  call of similar shape cost," since that's exactly what RFC 0007's
  local admission plane would add at each call site.

One build wrinkle worth recording: the staticlib bundles its own
complete copy of `std` by design (a real compiled `.nir` binary has no
Rust runtime of its own — `runtime-kernels/Cargo.toml`'s own doc
comment), but this harness's own driver code needs `std` too (threads,
`Instant`, `TcpStream`), so linking produces two copies of `std`'s
internals and the linker rejects the duplicate symbols. Both copies
come from the same toolchain invocation moments apart, so they're
ABI-identical — `build.rs` passes `-Wl,--allow-multiple-definition` to
tolerate the duplication, which is only safe *because* of that
identity, not as a general fix for linking two arbitrary `std` copies
together.

## Running it

```
cd rfcs/evidence/0007-apm-runtime-kernel/kernel_bench
cargo build --release
./target/release/kernel_bench
```

Two benchmarks (`2`, accept; `4b`, recv) need a concurrent peer thread
to keep work available so the timed call doesn't block waiting on the
network — each is labeled `CAVEAT` in its own output line, following
the same disclosure norm RFC 0006's `bench.rs` sets for its own
64MB-clone caveat, rather than presenting a number that looks cleaner
than what it actually measures.

## Results (i7-8550U, Linux 7.0.10-zen1, best of 3, two runs)

| # | What | Run 1 | Run 2 |
|---|---|---:|---:|
| 1a | raw `TcpStream::connect` + drop | 22.9–25.7 µs | 24.2 µs |
| 1b | `nir_tcp_connect` + `nir_tcp_stop` | 27.0 µs | 26.3 µs |
| 2 | `nir_tcp_accept` + `nir_tcp_stop` (CAVEAT: backlog kept warm) | 22.3 µs | 24.7 µs |
| 3a | raw `write_all` (8 bytes) | 1.35 µs | 1.42 µs |
| 3b | `nir_tcp_send` (8 bytes) | 1.32 µs | 1.32 µs |
| 4a | raw `read` (64B, CAVEAT: warmed by writer) | 0.80 µs | 0.90 µs |
| 4b | `nir_tcp_recv` (64B, CAVEAT: warmed by writer) | 0.77 µs | 0.90 µs |
| 5a | raw `File::create` + drop | 2.23 µs | 2.68 µs |
| 5b | `nir_file_open("w")` + `nir_file_stop` | 2.56 µs | 2.45 µs |
| 6a | raw `write_all` (4096 bytes) | 1.79 µs | 1.81 µs |
| 6b | raw `read` (4096 bytes) | 1.36 µs | 1.38 µs |
| 6c | `nir_file_write` (4096 bytes) | 1.70 µs | 1.64 µs |
| 6d | `nir_file_read` (4096 bytes, single call/trial — see harness comment) | 0.94 µs | 0.92 µs |

## What this means for RFC 0007 §5's SLOs

**The `extern "C"` wrapper itself adds no measurable overhead.** Every
`nir_*` row sits within normal run-to-run noise (~5–10% here) of its
raw `std` counterpart — sometimes faster, sometimes slower, never
systematically worse. This confirms RFC 0007 §1's premise that
`runtime-kernels`'s existing kernels really are thin, near-zero-cost
wrappers, not a hidden source of overhead a lease check would be
competing with.

**The boundary-reservation SLO (§5's ~50µs target) has real headroom.**
`connect`/`accept`/`open` — the compiled path's only available
admission boundaries today (RFC 0007 §4.2) — cost ~22–27µs on their
own. A lease check would need to roughly double that cost before
threatening the 50µs target; a well-implemented O(1) atomic-based
check (RFC 0007 §3's design) should land nowhere near that.

**The hot-path SLO (§5's ~100ns target) is the one to watch closely.**
`send`/`recv`/`write`/`read` cost **0.8–1.8µs** today with zero
admission logic in front of them. A 100ns lease check on top of that
is a real, non-trivial **5–15% overhead** on the fastest, most
frequently-called operations — not negligible the way it is for the
boundary calls. This is the number future phases need to protect most
carefully: anything beyond a couple of atomic operations in the local
admission plane's hot-path check risks eating a double-digit percentage
of the call it's guarding, for effects that exist specifically because
they're supposed to be cheap.

**Caveat on `db`/`spawn`.** This harness cannot say anything about
those domains — they don't compile yet (`codegen.rs` hard-rejects
them). Whatever `nir_db_*`/`nir_spawn_*` kernels eventually get built
per Track B items B2/B6 will need this same measurement repeated
against their own baseline before RFC 0007's SLOs can be considered
validated for those domains too.

## Update: the admission mechanism now exists, and this harness measured it live

`crates/runtime-kernels/src/kernel.rs` implements RFC 0007 §3's local
admission plane for real — one atomic compare-and-swap per resource
domain (`Tcp`, `File`), gating only `nir_tcp_connect`/`nir_tcp_listen`/
`nir_tcp_accept`/`nir_file_open` (resource-*creation* calls), never
`send`/`recv`/`read`/`write`. Re-running this harness against that
change (same machine, same methodology) is the real measurement §5's
Phase 0 gap called for:

| # | With admission live | Without (original baseline) |
|---|---:|---:|
| 1b. `nir_tcp_connect`+`stop` | 26.7 µs | 26.0–27.0 µs |
| 2. `nir_tcp_accept`+`stop` | 21.8 µs | 22.3–24.7 µs |
| 5b. `nir_file_open`+`stop` | 2.27 µs | 2.4–2.6 µs |
| 3b. `nir_tcp_send` (not gated) | 1.34 µs | 1.3 µs |
| 4b. `nir_tcp_recv` (not gated) | 0.78 µs | 0.8–0.9 µs |

Every boundary call is within the same run-to-run noise as before —
no measurable regression. This confirms §5's own prediction: an atomic
CAS pair costs low single-digit nanoseconds, invisible against a
22–27µs syscall. The hot-path calls (`send`/`recv`) are unaffected
because they were deliberately never gated — the actual, live
resolution of the tension this document's §5 flagged as the one number
to watch, resolved by not putting admission on that path at all rather
than by hitting an aggressive nanosecond budget on it.

