//! Freestanding kernels for the six Vector/Matrix builtins with genuine
//! data-dependent control flow (`det`/`inv`/`solve`/`rank`/
//! `kf_update_state`/`kf_update_cov` — partial-pivot row selection is a
//! runtime `if v > max_val` inside a loop, not something whose trip count
//! or instruction shape is known at codegen time the way every other
//! Vector/Matrix builtin's is). Rather than hand-emit branchy LLVM IR for
//! that control flow, `codegen.rs` calls these — compiled once, ahead of
//! time, into a linked static library — exactly the way it already calls
//! `@printf`/`@abort`/the libm intrinsics. A native `call` to `-O2`-
//! compiled code costs exactly what inlined IR would; this is an
//! implementation-risk choice, not a performance one.
//!
//! **This crate is its own, separate Cargo workspace** (`Cargo.toml`'s
//! own doc comment), built by `crates/compiler/build.rs` via `cargo
//! rustc` into a `.a` at `nirdosha`'s own build time — not part of the
//! `nirdosha` lib's own crate graph, so it still cannot `use` anything
//! from `interpreter.rs` directly across that compilation-unit
//! boundary. Every Vector/Matrix algorithm here is therefore a
//! deliberate line-for-line mirror of the corresponding `&[f64]`-taking
//! function in `interpreter.rs` (`matrix_det`/`matrix_inv`/
//! `matrix_solve`/`matrix_rank`/`kf_update`/`mat_mul_f64`/
//! `mat_vec_mul_f64`/`mat_transpose_f64`/`vec_add_f64`/`vec_sub_f64`) —
//! if you change the algorithm in one place, change it in the other, and
//! `crates/compiler/tests/codegen.rs`'s interpreter-parity tests will catch a
//! divergence immediately if you forget.
//!
//! **Unlike before, this crate is a genuine Cargo package with real
//! dependencies** (`Cargo.toml`'s `[dependencies]` — `rust_decimal`,
//! used by this file's `nir_dec128_*` kernels): the old bare-`rustc`
//! invocation had no dependency resolution at all, which is exactly
//! why `Ty::Dec128` stayed interpreter-only long after `tcp`/`file`
//! were compiled — `rust_decimal` simply wasn't reachable from a
//! dependency-free `rustc` call. `cargo rustc`, not `cargo build`, is
//! what `build.rs` actually invokes: it both compiles this crate *and*
//! forwards `--print=native-static-libs` to the one real `rustc`
//! invocation that produces the final artifact, in a single command —
//! the same two facts the old bare-`rustc` call captured together, now
//! captured through Cargo's own dependency-resolved build instead of
//! around it.
#![allow(clippy::missing_safety_doc)]

// Made `pub`, not `mod`, specifically for `crates/compiled-serve`
// (ROADMAP B8) -- the first real consumer of this crate as an ordinary
// Rust dependency rather than only via the `extern "C"`/staticlib-
// embedding path every compiled `.nir` binary uses. Every compiled
// `.nir` binary still only ever calls the `#[no_mangle] extern "C"`
// `nir_*` functions below (unaffected by this); `compiled-serve` is a
// second, additive consumer needing real Rust-level access (`domain`,
// `acquire`/`release`, `dump_report`) for things a `.nir` program has
// no reason to ever touch directly, matching this module's own "no
// query interface for `.nir` code" doc comment.
pub mod kernel;

/// Not a real language builtin — no `.nir` program can call this
/// (`codegen.rs` never emits a `declare`/`call` for it). Proves
/// `kernel::thread_pool`'s panic containment (`catch_unwind`) survives
/// being called via `extern "C"` from a host with **no Rust runtime of
/// its own** — the exact scenario a real compiled `.nir` binary is
/// (raw LLVM IR + this staticlib, linked by a bare `clang` invocation,
/// `codegen.rs::build`'s own convention), once `spawn` gets real
/// codegen. This function is the actual evidence behind the decision to
/// change this crate's `[profile.release]` from `panic = "abort"` to
/// `"unwind"` — see `rfcs/evidence/0007-apm-runtime-kernel/panic_containment/`
/// for the hand-written C program (zero Rust runtime except this
/// staticlib) that calls this and checks the result, the same rigor
/// `kernel_bench` already applies to the admission mechanism itself.
///
/// Submits a job that panics, waits for it to actually run, then
/// submits a normal job — returns `1` if the pool survived the panic
/// and ran the second job, `0` if the pool became unusable. If panic
/// containment does NOT actually work in the calling binary's
/// environment, this function never returns at all (the process aborts
/// first) — a `0` is not the only failure signal; a caller that gets no
/// output whatsoever from the process this ran in has also learned the
/// answer.
#[unsafe(no_mangle)]
pub extern "C" fn nir_kernel_self_test_panic_containment() -> i32 {
    let pool = kernel::thread_pool::ThreadPool::new();
    if pool.submit(Box::new(|| panic!("nir_kernel_self_test_panic_containment: expected panic, containment under test"))).is_err() {
        return 0;
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    let (tx, rx) = std::sync::mpsc::channel();
    if pool.submit(Box::new(move || {
        let _ = tx.send(());
    })).is_err() {
        return 0;
    }
    match rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

/// The open domain registry's one bootstrap point — `codegen.rs`
/// (`emit_c_main`) emits exactly one call to this, at the very top of
/// every compiled program's `main`, strictly before any user code
/// (`nir_main`) or any other kernel-preamble setup runs. This is what
/// gives the 7 built-in domains their historical, stable ids 0-6 in a
/// compiled binary: nothing else has had a chance to register a domain
/// first. `kernel::domain::register_builtin_domains` is itself
/// idempotent (`Once`-guarded) and every domain accessor
/// (`kernel::domain::db()`, etc.) also self-registers on first use, so
/// this call is a deliberate, redundant belt-and-suspenders bootstrap,
/// not the only path that can make registration happen — see that
/// function's own doc comment.
#[unsafe(no_mangle)]
pub extern "C" fn nir_kernel_register_builtin_domains() {
    kernel::domain::register_builtin_domains();
}

/// RFC 0011 §4's per-provider registration point — `codegen.rs` emits
/// one call to this per distinct validated plugin provider in
/// `build_with_native_plugins`'s list, in the order given, immediately
/// after the single `nir_kernel_register_builtin_domains()` bootstrap
/// call above, still inside `main`'s preamble before any user code
/// runs. `name`/`env_var` arrive as `(ptr, len)` pairs pointing at
/// LLVM string-constant globals `codegen.rs` emits alongside this
/// call — those globals live for the whole process (they're `.rodata`,
/// never freed), so leaking a heap copy here to satisfy
/// `register_domain`'s `&'static str` requirement is the correct
/// permanent-leak posture (same one `NativePluginBuiltin`'s own str
/// return-value convention already documents), not a real leak in any
/// sense that matters for a process that's about to eagerly register
/// every provider it will ever have, once, at startup.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_kernel_register_domain(
    name_ptr: *const u8,
    name_len: i64,
    env_var_ptr: *const u8,
    env_var_len: i64,
    default_max: i64,
) {
    let name = unsafe { str_from_raw(name_ptr, name_len) }.expect("codegen only ever emits valid UTF-8 domain-name globals");
    let env_var = unsafe { str_from_raw(env_var_ptr, env_var_len) }.expect("codegen only ever emits valid UTF-8 env-var-name globals");
    let name: &'static str = Box::leak(name.to_string().into_boxed_str());
    let env_var: &'static str = Box::leak(env_var.to_string().into_boxed_str());
    kernel::register_domain(name, env_var, default_max);
}

/// RFC 0011 §2/§4's dispatch-table registration — `codegen.rs` emits
/// one call to this immediately after this same provider's
/// `nir_kernel_register_domain` call above (same preamble loop
/// iteration), so `domain_name`/`domain_name_len` name an
/// already-registered domain here: `kernel::domain_id_for_name` is a
/// plain lookup, not a second registration path. `scheme` is the
/// compiler's own already-normalized scheme identifier
/// (`plugin::normalize_scheme`'s output) — this call does not
/// re-normalize it.
///
/// The four function-pointer parameters are opaque addresses
/// (`*const ()`, matching LLVM's untyped `ptr` at this boundary) rather
/// than their real `extern "C" fn` types, because `_op`'s and
/// `_request`'s real signatures differ in arity (one `str` arg vs two) —
/// `is_call_shape` says which one `op_or_request_fn` actually is, so
/// this function can transmute it back to the correct type on this side
/// rather than `codegen.rs` needing two mutually-exclusive parameters,
/// one always unused. Sound because a function pointer and a data
/// pointer share the same representation on every platform this
/// backend targets (the identical assumption every other `ptr`-typed
/// `declare` in `codegen.rs` already makes for its own extern calls),
/// and because `codegen.rs` only ever passes the address of a real,
/// `declare`d symbol with the shape this call expects — never a value
/// `.nir` code could influence.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_kernel_register_plugin_provider(
    domain_name_ptr: *const u8,
    domain_name_len: i64,
    scheme_ptr: *const u8,
    scheme_len: i64,
    connect_fn: *const (),
    op_or_request_fn: *const (),
    is_valid_fn: *const (),
    close_fn: *const (),
    is_call_shape: i32,
) {
    let domain_name = unsafe { str_from_raw(domain_name_ptr, domain_name_len) }.expect("codegen only ever emits valid UTF-8 domain-name globals");
    let scheme = unsafe { str_from_raw(scheme_ptr, scheme_len) }.expect("codegen only ever emits valid UTF-8 scheme globals");
    let domain = kernel::domain_id_for_name(domain_name)
        .expect("codegen emits nir_kernel_register_domain for this exact provider immediately before this call, in the same preamble loop iteration");
    let connect_fn: extern "C" fn(kernel::pool::NirStr) -> i64 = unsafe { std::mem::transmute(connect_fn) };
    let is_valid_fn: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(is_valid_fn) };
    let close_fn: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(close_fn) };
    let op = if is_call_shape != 0 {
        let request_fn: extern "C" fn(i64, kernel::pool::NirStr, kernel::pool::NirStr) -> kernel::pool::NirStr = unsafe { std::mem::transmute(op_or_request_fn) };
        kernel::plugin_provider::ProviderOp::Call { request_fn }
    } else {
        let op_fn: extern "C" fn(i64, kernel::pool::NirStr) -> kernel::pool::NirStr = unsafe { std::mem::transmute(op_or_request_fn) };
        kernel::plugin_provider::ProviderOp::ConnStream { op_fn }
    };
    kernel::plugin_provider::register(scheme.to_string(), kernel::plugin_provider::ProviderFns { connect_fn, is_valid_fn, close_fn, op, domain });
}

/// RFC 0011 §5's reaper startup point — `codegen.rs` emits exactly one
/// call to this, in every compiled program's `main` preamble,
/// immediately after the domain/plugin-provider registration calls
/// above. `kernel::reaper::start` is itself `Once`-guarded and directly
/// callable (and tested) from plain Rust with no dependency on this FFI
/// wrapper existing at all — this function exists only so a compiled
/// `.nir` binary actually starts the reaper thread once, the same
/// "codegen carries zero knowledge of the mechanism, just calls the one
/// bootstrap entrypoint" shape `nir_kernel_register_builtin_domains`
/// already established.
#[unsafe(no_mangle)]
pub extern "C" fn nir_kernel_start_reaper() {
    kernel::reaper::start();
}

/// The flight recorder's one exit point — `codegen.rs`'s generated
/// `main` wrapper calls this exactly once, automatically, immediately
/// before every `ret` in `emit_c_main` (every exit path: `unit`, `str`,
/// and the generic numeric case), regardless of what the `.nir` program
/// itself did or does. No `.nir` source can call this (it's not
/// registered in `ast::BUILTIN_NAMES` at all) — this is a compiler-
/// inserted hook, not a language feature, matching `kernel::dump_report`'s
/// own "the program never queries the kernel" design (see that
/// function's doc comment). Prints to stderr so it's always visible
/// after a run without needing a file to manage.
#[unsafe(no_mangle)]
pub extern "C" fn nir_kernel_flight_recorder_dump() {
    // Flush whatever's left in the currently-active event page first
    // (a run that never filled a page would otherwise have its whole
    // history silently dropped, since `kernel::recorder::record` only
    // flushes automatically when a page actually fills) — then print
    // the plain-counter summary, same as before.
    kernel::recorder::flush_remaining();
    eprint!("{}", kernel::dump_report());
}

const SINGULAR_EPSILON: f64 = 1e-10;

fn matrix_det(elems: &[f64], n: usize) -> f64 {
    let mut a: Vec<f64> = elems.to_vec();
    let mut det = 1.0;
    for col in 0..n {
        let mut pivot_row = col;
        let mut max_val = a[col * n + col].abs();
        for row in (col + 1)..n {
            let v = a[row * n + col].abs();
            if v > max_val {
                max_val = v;
                pivot_row = row;
            }
        }
        if max_val == 0.0 {
            return 0.0;
        }
        if pivot_row != col {
            for k in 0..n {
                a.swap(col * n + k, pivot_row * n + k);
            }
            det = -det;
        }
        det *= a[col * n + col];
        for row in (col + 1)..n {
            let factor = a[row * n + col] / a[col * n + col];
            for k in col..n {
                a[row * n + k] -= factor * a[col * n + k];
            }
        }
    }
    det
}

fn matrix_inv(elems: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut a: Vec<f64> = elems.to_vec();
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    for col in 0..n {
        let mut pivot_row = col;
        let mut max_val = a[col * n + col].abs();
        for row in (col + 1)..n {
            let v = a[row * n + col].abs();
            if v > max_val {
                max_val = v;
                pivot_row = row;
            }
        }
        if max_val < SINGULAR_EPSILON {
            return None;
        }
        if pivot_row != col {
            for k in 0..n {
                a.swap(col * n + k, pivot_row * n + k);
                inv.swap(col * n + k, pivot_row * n + k);
            }
        }
        let pivot = a[col * n + col];
        for k in 0..n {
            a[col * n + k] /= pivot;
            inv[col * n + k] /= pivot;
        }
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = a[row * n + col];
            if factor != 0.0 {
                for k in 0..n {
                    a[row * n + k] -= factor * a[col * n + k];
                    inv[row * n + k] -= factor * inv[col * n + k];
                }
            }
        }
    }
    Some(inv)
}

fn matrix_solve(a_elems: &[f64], n: usize, b_elems: &[f64]) -> Option<Vec<f64>> {
    let mut a: Vec<f64> = a_elems.to_vec();
    let mut b: Vec<f64> = b_elems.to_vec();
    for col in 0..n {
        let mut pivot_row = col;
        let mut max_val = a[col * n + col].abs();
        for row in (col + 1)..n {
            let v = a[row * n + col].abs();
            if v > max_val {
                max_val = v;
                pivot_row = row;
            }
        }
        if max_val < SINGULAR_EPSILON {
            return None;
        }
        if pivot_row != col {
            for k in 0..n {
                a.swap(col * n + k, pivot_row * n + k);
            }
            b.swap(col, pivot_row);
        }
        for row in (col + 1)..n {
            let factor = a[row * n + col] / a[col * n + col];
            for k in col..n {
                a[row * n + k] -= factor * a[col * n + k];
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let mut sum = b[row];
        for k in (row + 1)..n {
            sum -= a[row * n + k] * x[k];
        }
        x[row] = sum / a[row * n + row];
    }
    Some(x)
}

fn matrix_rank(elems: &[f64], rows: usize, cols: usize) -> usize {
    let mut a: Vec<f64> = elems.to_vec();
    let mut rank = 0;
    let mut pivot_row = 0;
    for col in 0..cols {
        if pivot_row >= rows {
            break;
        }
        let mut best_row = pivot_row;
        let mut max_val = a[pivot_row * cols + col].abs();
        for row in (pivot_row + 1)..rows {
            let v = a[row * cols + col].abs();
            if v > max_val {
                max_val = v;
                best_row = row;
            }
        }
        if max_val < SINGULAR_EPSILON {
            continue;
        }
        if best_row != pivot_row {
            for k in 0..cols {
                a.swap(pivot_row * cols + k, best_row * cols + k);
            }
        }
        for row in (pivot_row + 1)..rows {
            let factor = a[row * cols + col] / a[pivot_row * cols + col];
            for k in col..cols {
                a[row * cols + k] -= factor * a[pivot_row * cols + k];
            }
        }
        pivot_row += 1;
        rank += 1;
    }
    rank
}

fn mat_mul_f64(a: &[f64], ar: usize, ac: usize, b: &[f64], bc: usize) -> Vec<f64> {
    let mut out = vec![0.0; ar * bc];
    for i in 0..ar {
        for j in 0..bc {
            out[i * bc + j] = (0..ac).map(|k| a[i * ac + k] * b[k * bc + j]).sum();
        }
    }
    out
}

fn mat_vec_mul_f64(a: &[f64], ar: usize, ac: usize, v: &[f64]) -> Vec<f64> {
    (0..ar).map(|i| (0..ac).map(|k| a[i * ac + k] * v[k]).sum()).collect()
}

fn mat_transpose_f64(a: &[f64], r: usize, c: usize) -> Vec<f64> {
    let mut out = vec![0.0; r * c];
    for i in 0..r {
        for j in 0..c {
            out[j * r + i] = a[i * c + j];
        }
    }
    out
}

fn vec_add_f64(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter().zip(b).map(|(x, y)| x + y).collect()
}

fn vec_sub_f64(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter().zip(b).map(|(x, y)| x - y).collect()
}

fn kf_update(
    x: &[f64],
    p: &[f64],
    z: &[f64],
    h: &[f64],
    r: &[f64],
    n: usize,
    m: usize,
) -> Option<(Vec<f64>, Vec<f64>)> {
    let hx = mat_vec_mul_f64(h, m, n, x);
    let y = vec_sub_f64(z, &hx);
    let ht = mat_transpose_f64(h, m, n);
    let hp = mat_mul_f64(h, m, n, p, n);
    let hpht = mat_mul_f64(&hp, m, n, &ht, m);
    let s = vec_add_f64(&hpht, r);
    let s_inv = matrix_inv(&s, m)?;
    let pht = mat_mul_f64(p, n, n, &ht, m);
    let k = mat_mul_f64(&pht, n, m, &s_inv, m);
    let ky = mat_vec_mul_f64(&k, n, m, &y);
    let x_new = vec_add_f64(x, &ky);
    let kh = mat_mul_f64(&k, n, m, h, n);
    let mut i_minus_kh = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            i_minus_kh[i * n + j] = if i == j { 1.0 } else { 0.0 } - kh[i * n + j];
        }
    }
    let p_new = mat_mul_f64(&i_minus_kh, n, n, p, n);
    Some((x_new, p_new))
}

// ---- sha256_hex kernel ----------------------------------------------------
//
// A from-scratch FIPS 180-4 SHA-256 implementation, not a binding to the
// `sha2` crate `interpreter.rs`'s own `sha256_hex`/`sha256_hex_chain`
// use — this file is compiled as an isolated `rustc --crate-type
// staticlib` invocation with no `--extern` flags (`build.rs`'s doc
// comment), so it has no access to Cargo dependencies at all, only
// `std`. Verified bit-for-bit against `interpreter.rs`'s `sha2`-backed
// output for the empty string, ASCII text, and the exact two-part
// chained form `sha256_hex_chain` uses (`crates/compiler/tests/sha256_hex.rs`),
// not just against the standard's own published test vectors.

const SHA256_H0: [u32; 8] =
    [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

/// One 64-byte block's worth of compression, updating `state` in place —
/// the algorithm's actual core (message-schedule expansion, 64 mixing
/// rounds), everything else in this section is padding/framing around
/// this.
fn sha256_compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// Hashes `a` followed by `b` as one continuous message (`b` empty is
/// the 1-arg `sha256_hex(s)` case; `b` non-empty is the 2-arg
/// `sha256_hex(prev_hash, payload)` chained form) — streaming both
/// buffers through the same padding/block state the way multiple
/// `Sha256::update` calls on one hasher already do in
/// `interpreter.rs`, since hashing "a then b" one block at a time is
/// mathematically identical to hashing the concatenation `a ++ b` in
/// one pass; there's no need to actually concatenate them into a new
/// buffer first (which `str`'s lack of a concatenation operator
/// wouldn't let calling Nirdosha code do anyway — this streaming
/// approach is what makes that a non-issue at the kernel level too).
fn sha256(a: &[u8], b: &[u8]) -> [u8; 32] {
    let mut state = SHA256_H0;
    let total_len = (a.len() + b.len()) as u64;

    let mut block = [0u8; 64];
    let mut filled = 0usize;
    for &byte in a.iter().chain(b.iter()) {
        block[filled] = byte;
        filled += 1;
        if filled == 64 {
            sha256_compress(&mut state, &block);
            filled = 0;
        }
    }

    // Padding: a single `1` bit (0x80, since messages here are always a
    // whole number of bytes), then zero bits, then the original message
    // length in bits as a big-endian 64-bit integer -- padded so the
    // total is a multiple of 64 bytes, with the length always the final
    // 8 bytes of the final block, same as every other SHA-256
    // implementation's framing (FIPS 180-4 §5.1.1).
    block[filled] = 0x80;
    filled += 1;
    if filled > 56 {
        for b in &mut block[filled..64] {
            *b = 0;
        }
        sha256_compress(&mut state, &block);
        filled = 0;
    }
    for b in &mut block[filled..56] {
        *b = 0;
    }
    let bit_len = total_len.wrapping_mul(8);
    block[56..64].copy_from_slice(&bit_len.to_be_bytes());
    sha256_compress(&mut state, &block);

    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// RFC 2104 HMAC-SHA256, built on the [`sha256`] primitive above —
/// added as red-team report A26's own recommended defense-in-depth
/// convention (`scratch/red-team-report-main-d7fae42.md`): "hash a
/// secret for storage" should reach for HMAC by default, plain
/// `sha256`/`sha256_hex` for content hashing only.
///
/// **Not wired into `nir_validate_api_key`'s existing hashing in this
/// same fix, deliberately disclosed rather than silently done** — the
/// report's own text already concedes length extension doesn't
/// meaningfully weaken that specific call site (a high-entropy API key
/// hashed keylessly, not a low-entropy secret an attacker could feasibly
/// extend against). Actually using HMAC there would need a *separate*
/// server-side secret key (an HMAC "pepper") to hash the API key
/// against — a real, unreviewed piece of config/secret-management
/// surface (where does that key live? how does it rotate?) that doesn't
/// exist anywhere in this codebase today, the same class of judgment
/// call A1's fix already declined to invent unilaterally for JWKS
/// config. This function exists so the *next* piece of code that
/// genuinely needs to hash a secret with a real HMAC key has the right
/// primitive ready, rather than reaching for plain `sha256` by default.
///
/// `key` longer than one block (64 bytes) is hashed down first per the
/// spec (RFC 2104 §2, step "If K is longer than B... hash it"); `sha256`
/// is reused for that, not a second SHA-256 entry point.
#[allow(dead_code)] // intentionally unused in production today -- see this fn's own doc comment (A26)
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    const IPAD: u8 = 0x36;
    const OPAD: u8 = 0x5c;

    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = sha256(key, &[]);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad_key = [0u8; BLOCK_SIZE];
    let mut opad_key = [0u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad_key[i] = key_block[i] ^ IPAD;
        opad_key[i] = key_block[i] ^ OPAD;
    }

    let inner = sha256(&ipad_key, message);
    sha256(&opad_key, &inner)
}

#[cfg(test)]
mod hmac_sha256_tests {
    use super::hmac_sha256;

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// RFC 4231 §4.2, Test Case 1 -- a known-answer test for this
    /// hand-rolled implementation, not just internal self-consistency.
    #[test]
    fn matches_rfc_4231_test_case_1() {
        let key = [0x0bu8; 20];
        let data = b"Hi There";
        let expected = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";
        assert_eq!(to_hex(&hmac_sha256(&key, data)), expected);
    }

    /// RFC 4231 §4.3, Test Case 2 -- a short, ASCII key and message,
    /// different shape from Test Case 1's binary key.
    #[test]
    fn matches_rfc_4231_test_case_2() {
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let expected = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
        assert_eq!(to_hex(&hmac_sha256(key, data)), expected);
    }

    /// RFC 4231 §4.6, Test Case 6 -- a key longer than SHA-256's own
    /// 64-byte block size, exercising the "hash the key down first"
    /// branch (`hmac_sha256`'s own doc comment, RFC 2104 §2) that
    /// neither test case above touches.
    #[test]
    fn matches_rfc_4231_test_case_6_key_longer_than_one_block() {
        let key = [0xaau8; 131];
        let data = b"Test Using Larger Than Block-Size Key - Hash Key First";
        let expected = "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54";
        assert_eq!(to_hex(&hmac_sha256(&key, data)), expected);
    }
}

/// Lowercase hex encoding, matching `interpreter.rs`'s own
/// `format!("{b:02x}")` per byte exactly.
fn hex_encode(bytes: &[u8], out: &mut [u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (i, &b) in bytes.iter().enumerate() {
        out[2 * i] = HEX[(b >> 4) as usize];
        out[2 * i + 1] = HEX[(b & 0x0f) as usize];
    }
}

/// The interpreter's own `constant_time_eq`, line-for-line: length
/// mismatch is a real, immediate difference (a real, accepted timing
/// leak of *length* — the property this function actually protects is
/// "don't leak *which byte* differs"), otherwise XOR-accumulate every
/// byte pair with no early exit.
///
/// `pub` (red-team report A20, `scratch/red-team-report-main-d7fae42.md`):
/// `crates/compiled-serve`'s own `metrics_token` comparison used a plain
/// `==`, a real timing oracle against `/metrics` for a short or
/// guessable token — reusing this function there instead of a second,
/// independent implementation.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---- extern "C" boundary -----------------------------------------------
//
// Every pointer here is trusted, not validated: typeck.rs already proved
// every call site passes correctly-shaped, correctly-sized buffers before
// codegen.rs ever emits the `call` instruction that reaches these — the
// same "the checker is the real gate" convention interpreter.rs's own
// `unreachable!()`s already follow for builtin dispatch.

/// Determinant of an `n x n` matrix. Never fails — `0.0` for singular is
/// a real, legitimate answer for `det` specifically.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_det(a: *const f64, n: i64) -> f64 {
    let n = n as usize;
    let a = unsafe { std::slice::from_raw_parts(a, n * n) };
    matrix_det(a, n)
}

/// Inverse of an `n x n` matrix into `out` (also `n x n`). Returns `1` on
/// success, `0` if singular (caller traps on `0`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_inv(a: *const f64, n: i64, out: *mut f64) -> i32 {
    let n = n as usize;
    let a = unsafe { std::slice::from_raw_parts(a, n * n) };
    match matrix_inv(a, n) {
        Some(v) => {
            let out = unsafe { std::slice::from_raw_parts_mut(out, n * n) };
            out.copy_from_slice(&v);
            1
        }
        None => 0,
    }
}

/// Solves `A x = b` for an `n x n` `A` and length-`n` `b`, into `out`
/// (length `n`). Returns `1` on success, `0` if `A` is singular.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_solve(a: *const f64, n: i64, b: *const f64, out: *mut f64) -> i32 {
    let n = n as usize;
    let a = unsafe { std::slice::from_raw_parts(a, n * n) };
    let b = unsafe { std::slice::from_raw_parts(b, n) };
    match matrix_solve(a, n, b) {
        Some(x) => {
            let out = unsafe { std::slice::from_raw_parts_mut(out, n) };
            out.copy_from_slice(&x);
            1
        }
        None => 0,
    }
}

/// Rank of a `rows x cols` matrix. Never fails.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_rank(a: *const f64, rows: i64, cols: i64) -> i64 {
    let rows = rows as usize;
    let cols = cols as usize;
    let a = unsafe { std::slice::from_raw_parts(a, rows * cols) };
    matrix_rank(a, rows, cols) as i64
}

/// Linear Kalman filter update step's state output, into `out` (length
/// `n`). `x`/`p`/`z`/`h`/`r` are the state vector (len `n`), state
/// covariance (`n x n`), measurement (len `m`), measurement matrix
/// (`m x n`), and measurement-noise covariance (`m x m`). Returns `1` on
/// success, `0` if the innovation covariance is singular.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_kf_update_state(
    x: *const f64,
    p: *const f64,
    z: *const f64,
    h: *const f64,
    r: *const f64,
    n: i64,
    m: i64,
    out: *mut f64,
) -> i32 {
    let n = n as usize;
    let m = m as usize;
    let x = unsafe { std::slice::from_raw_parts(x, n) };
    let p = unsafe { std::slice::from_raw_parts(p, n * n) };
    let z = unsafe { std::slice::from_raw_parts(z, m) };
    let h = unsafe { std::slice::from_raw_parts(h, m * n) };
    let r = unsafe { std::slice::from_raw_parts(r, m * m) };
    match kf_update(x, p, z, h, r, n, m) {
        Some((x_new, _)) => {
            let out = unsafe { std::slice::from_raw_parts_mut(out, n) };
            out.copy_from_slice(&x_new);
            1
        }
        None => 0,
    }
}

/// Same update step's covariance output, into `out` (`n x n`). Same
/// shapes/return convention as `nir_kf_update_state`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_kf_update_cov(
    x: *const f64,
    p: *const f64,
    z: *const f64,
    h: *const f64,
    r: *const f64,
    n: i64,
    m: i64,
    out: *mut f64,
) -> i32 {
    let n = n as usize;
    let m = m as usize;
    let x = unsafe { std::slice::from_raw_parts(x, n) };
    let p = unsafe { std::slice::from_raw_parts(p, n * n) };
    let z = unsafe { std::slice::from_raw_parts(z, m) };
    let h = unsafe { std::slice::from_raw_parts(h, m * n) };
    let r = unsafe { std::slice::from_raw_parts(r, m * m) };
    match kf_update(x, p, z, h, r, n, m) {
        Some((_, p_new)) => {
            let out = unsafe { std::slice::from_raw_parts_mut(out, n * n) };
            out.copy_from_slice(&p_new);
            1
        }
        None => 0,
    }
}

/// `str`'s `==`/`!=` — length check, then a byte-for-byte compare.
/// Returns `1` if equal, `0` otherwise. `codegen.rs`'s `str_eq` is the
/// only caller — it already only ever passes buffers `{ptr, i64}`-typed
/// `str` values actually own, matching every other kernel's "the checker
/// is the real gate" trust convention (this file's module doc).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_str_eq(a: *const u8, a_len: i64, b: *const u8, b_len: i64) -> i32 {
    if a_len != b_len {
        return 0;
    }
    let n = a_len as usize;
    let a = unsafe { std::slice::from_raw_parts(a, n) };
    let b = unsafe { std::slice::from_raw_parts(b, n) };
    (a == b) as i32
}

/// Backs the `str_index_of(haystack, needle)` builtin — the one string
/// primitive that's a genuine byte-scan (`str_slice`/`len(str)` are pure
/// pointer-arithmetic in `codegen.rs`, no kernel call at all). Returns the
/// byte offset of the first occurrence of `needle` in `haystack`, or `-1`
/// if it's not found. An empty `needle` always matches at offset `0`
/// (mirrors `str::find`'s own convention for an empty pattern).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_str_index_of(
    hay_ptr: *const u8,
    hay_len: i64,
    needle_ptr: *const u8,
    needle_len: i64,
) -> i64 {
    if needle_len == 0 {
        return 0;
    }
    if needle_len > hay_len {
        return -1;
    }
    let hay = unsafe { std::slice::from_raw_parts(hay_ptr, hay_len as usize) };
    let needle = unsafe { std::slice::from_raw_parts(needle_ptr, needle_len as usize) };
    match hay.windows(needle.len()).position(|w| w == needle) {
        Some(i) => i as i64,
        None => -1,
    }
}

#[cfg(test)]
mod str_index_of_tests {
    use super::nir_str_index_of;

    fn index_of(hay: &str, needle: &str) -> i64 {
        unsafe {
            nir_str_index_of(
                hay.as_ptr(),
                hay.len() as i64,
                needle.as_ptr(),
                needle.len() as i64,
            )
        }
    }

    #[test]
    fn finds_a_substring_in_the_middle() {
        assert_eq!(index_of("GET /api/hello HTTP/1.1", " "), 3);
    }

    #[test]
    fn returns_negative_one_when_not_found() {
        assert_eq!(index_of("hello", "xyz"), -1);
    }

    #[test]
    fn empty_needle_matches_at_offset_zero() {
        assert_eq!(index_of("hello", ""), 0);
    }

    #[test]
    fn needle_at_offset_zero() {
        assert_eq!(index_of("hello world", "hello"), 0);
    }

    #[test]
    fn needle_at_the_end() {
        assert_eq!(index_of("hello world", "world"), 6);
    }

    #[test]
    fn needle_longer_than_haystack_is_not_found() {
        assert_eq!(index_of("hi", "hello"), -1);
    }
}

// ---- tcp/tcp_listener kernels --------------------------------------------
//
// A `tcp`/`tcp_listener` handle is a raw OS socket handle, not a Rust
// `TcpStream`/`TcpListener` kept alive across calls the way
// `interpreter.rs`'s `Value::Tcp`'s `Arc<Mutex<Option<..>>>` does — the
// kernel already tracks everything a "handle" needs, so `codegen.rs`
// lowers `Ty::Tcp`/`Ty::TcpListener` straight to `i64`. Every kernel below
// reconstructs a `std`-level view of that handle for the duration of one
// call via the platform's own `from_raw_*`, wrapped in `ManuallyDrop`
// wherever the handle must stay open afterward (only `nir_tcp_stop`
// actually wants the real `Drop`/close to run). This mirrors
// `interpreter.rs`'s exact error/port-validation behavior
// (`Expr::Connect`/`Expr::Listen`/`read_tcp`/`write_tcp`) — see each fn's
// doc comment for the specific line it matches.
//
// Unix represents a socket as a `RawFd` (`i32`); Windows represents it as
// a `RawSocket` (`u64`), a structurally different type with a differently
// named conversion trait (`IntoRawSocket`/`FromRawSocket` vs.
// `IntoRawFd`/`FromRawFd`). The four tiny `handle_*` helpers below are the
// only platform-conditional surface — every kernel fn's own body is
// platform-agnostic, calling only these. **The Windows path is untested**
// (no Windows machine available to this project — see README.md's
// "Honest scope"): it's a direct, believed-correct port of the Unix path
// using the equivalent stdlib API, not verified end-to-end against a real
// Windows TCP round-trip. Report a bug if it doesn't work.

use std::io::{Read, Write};
use std::mem::ManuallyDrop;
use std::net::{TcpListener, TcpStream};

#[cfg(unix)]
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket, OwnedSocket, RawSocket};

#[cfg(unix)]
fn handle_of_stream(s: TcpStream) -> i64 {
    s.into_raw_fd() as i64
}
#[cfg(windows)]
fn handle_of_stream(s: TcpStream) -> i64 {
    s.into_raw_socket() as i64
}

#[cfg(unix)]
fn handle_of_listener(l: TcpListener) -> i64 {
    l.into_raw_fd() as i64
}
#[cfg(windows)]
fn handle_of_listener(l: TcpListener) -> i64 {
    l.into_raw_socket() as i64
}

#[cfg(unix)]
unsafe fn stream_from_handle(h: i64) -> TcpStream {
    unsafe { TcpStream::from_raw_fd(h as RawFd) }
}
#[cfg(windows)]
unsafe fn stream_from_handle(h: i64) -> TcpStream {
    unsafe { TcpStream::from_raw_socket(h as RawSocket) }
}

#[cfg(unix)]
unsafe fn listener_from_handle(h: i64) -> TcpListener {
    unsafe { TcpListener::from_raw_fd(h as RawFd) }
}
#[cfg(windows)]
unsafe fn listener_from_handle(h: i64) -> TcpListener {
    unsafe { TcpListener::from_raw_socket(h as RawSocket) }
}

/// Connects to `host:port` (`host` a `{ptr, len}` UTF-8 buffer). Returns
/// the new connection's handle on success, `-1` on failure — mirrors
/// `interpreter.rs`'s `Expr::Connect`: `u16::try_from(port)` (an
/// out-of-range port is a failure, not a silent truncation) then
/// `TcpStream::connect`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_tcp_connect(host_ptr: *const u8, host_len: i64, port: i64) -> i64 {
    let host = unsafe { std::slice::from_raw_parts(host_ptr, host_len as usize) };
    let Ok(host) = std::str::from_utf8(host) else { return -1 };
    let Ok(port) = u16::try_from(port) else { return -1 };
    // Admission first, at the resource-creation call only -- never on
    // send/recv (kernel.rs's own module doc). A denial is folded into
    // the same `-1` every other connect failure already returns; a
    // distinct error code is real future work, not a gap to route
    // around here.
    if !kernel::acquire(kernel::domain::tcp()) {
        return -1;
    }
    match TcpStream::connect((host, port)) {
        Ok(stream) => handle_of_stream(stream),
        Err(_) => {
            kernel::release(kernel::domain::tcp());
            -1
        }
    }
}

/// Binds `0.0.0.0:port` (all interfaces, matching `interpreter.rs`'s
/// `Expr::Listen` — not just loopback). Returns the listener's handle on
/// success, `-1` on failure (including an out-of-range port).
#[unsafe(no_mangle)]
pub extern "C" fn nir_tcp_listen(port: i64) -> i64 {
    let Ok(port) = u16::try_from(port) else { return -1 };
    if !kernel::acquire(kernel::domain::tcp()) {
        return -1;
    }
    match TcpListener::bind(("0.0.0.0", port)) {
        Ok(listener) => handle_of_listener(listener),
        Err(_) => {
            kernel::release(kernel::domain::tcp());
            -1
        }
    }
}

/// Blocks for the next connection on `listener_handle`. Returns the
/// accepted connection's own handle on success, `-1` on failure.
/// `listener_handle` itself is left open and reusable (`accept` doesn't
/// consume the listener, `ownership.rs`'s `touch_expr(listener, false)`)
/// — `ManuallyDrop` stops the temporary `TcpListener` view constructed
/// here from closing it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_tcp_accept(listener_handle: i64) -> i64 {
    let listener = ManuallyDrop::new(unsafe { listener_from_handle(listener_handle) });
    if !kernel::acquire(kernel::domain::tcp()) {
        return -1;
    }
    match listener.accept() {
        Ok((stream, _addr)) => handle_of_stream(stream),
        Err(_) => {
            kernel::release(kernel::domain::tcp());
            -1
        }
    }
}

/// Sends `buf` in full over `handle` — `write_all`, matching
/// `interpreter.rs`'s `write_tcp` exactly (it loops internally until
/// every byte is written, not a single partial-write return). Returns
/// `buf_len` on success, `-1` on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_tcp_send(handle: i64, buf_ptr: *const u8, buf_len: i64) -> i64 {
    let mut stream = ManuallyDrop::new(unsafe { stream_from_handle(handle) });
    let buf = unsafe { std::slice::from_raw_parts(buf_ptr, buf_len as usize) };
    match stream.write_all(buf) {
        Ok(()) => buf_len,
        Err(_) => -1,
    }
}

/// One read syscall into `buf_cap` bytes of caller-provided `buf_ptr` —
/// matches `interpreter.rs`'s `read_tcp`: one chunk, not a loop until a
/// message boundary. Returns bytes read, or `-1` on error. Note: unlike a
/// typical Unix `read`, a `0` return (peer closed) is *not* distinguished
/// from a short read here — `codegen.rs`'s caller (`guard_recv_ok`) traps
/// on `<= 0` the same way `read_tcp` treats `n == 0` as an error, not a
/// valid empty read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_tcp_recv(handle: i64, buf_ptr: *mut u8, buf_cap: i64) -> i64 {
    let mut stream = ManuallyDrop::new(unsafe { stream_from_handle(handle) });
    let buf = unsafe { std::slice::from_raw_parts_mut(buf_ptr, buf_cap as usize) };
    match stream.read(buf) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Closes `handle` — serves both `tcp` and `tcp_listener` uniformly (both
/// are plain sockets at the OS level, so one raw-handle close path is
/// correct for either). `ownership.rs`'s affine-typing already proves
/// this runs at most once per handle in a well-typed program (this
/// file's module doc's "the checker is the real gate" convention) —
/// reconstructing an *owned* handle (not `ManuallyDrop`) and letting it
/// drop is what actually closes the socket.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_tcp_stop(handle: i64) -> i32 {
    #[cfg(unix)]
    drop(unsafe { OwnedFd::from_raw_fd(handle as RawFd) });
    #[cfg(windows)]
    drop(unsafe { OwnedSocket::from_raw_socket(handle as RawSocket) });
    // One release per handle, regardless of which of connect/listen/
    // accept originally admitted it -- all three fold into this one
    // close path (this fn's own doc comment), so the acquire:release
    // ratio stays 1:1 either way.
    kernel::release(kernel::domain::tcp());
    0
}

// ---- file kernels ---------------------------------------------------------
//
// `open`/`file` (docs/PROTOLANG_PORT.md's "Locked design 2") reuses `send`/
// `recv`/`stop` verbatim, the same way `tcp` itself reuses them from `chan`
// (`examples/file_io.nir`'s own doc comment) -- so this section mirrors the
// `tcp` kernels above almost exactly, differing only where a file's real
// semantics genuinely differ from a socket's: `nir_file_read`'s `0` return
// is valid EOF (`interpreter.rs::read_file`'s own doc comment: "a file
// simply running out of bytes to read is the normal, expected way a file
// ends"), never a trap the way `nir_tcp_recv`'s `0` (peer closed) is —
// `codegen.rs` dispatches to `guard_io_ok` (traps only on negative) for
// `Ty::File`'s `recv`, not `guard_recv_ok` (traps on `<= 0`), to match.
//
// On Unix a file descriptor and a socket descriptor are the same `RawFd`
// type, so `handle_of_stream`/`stream_from_handle`'s *traits*
// (`IntoRawFd`/`FromRawFd`, already imported above) apply to
// `std::fs::File` unchanged -- only Windows genuinely needs its own
// conversion, since a Win32 file `HANDLE` and a `SOCKET` are different,
// differently-APIed types (`IntoRawHandle`/`FromRawHandle`, not
// `IntoRawSocket`/`FromRawSocket`). Same "believed-correct, untested on
// real Windows hardware" disclosure as every other Windows-conditional
// kernel in this file.
#[cfg(unix)]
fn handle_of_file(f: std::fs::File) -> i64 {
    f.into_raw_fd() as i64
}
#[cfg(windows)]
fn handle_of_file(f: std::fs::File) -> i64 {
    use std::os::windows::io::IntoRawHandle;
    f.into_raw_handle() as i64
}

#[cfg(unix)]
unsafe fn file_from_handle(h: i64) -> std::fs::File {
    unsafe { std::fs::File::from_raw_fd(h as RawFd) }
}
#[cfg(windows)]
unsafe fn file_from_handle(h: i64) -> std::fs::File {
    use std::os::windows::io::{FromRawHandle, RawHandle};
    unsafe { std::fs::File::from_raw_handle(h as RawHandle) }
}

/// `open(path, mode)` — matches `interpreter.rs`'s `Expr::Open` exactly:
/// `"r"` opens for reading, `"w"` creates/truncates for writing, `"a"`
/// creates/appends; any other mode string is `-1`, the same "invalid mode"
/// failure a real I/O error already collapses into (`codegen.rs`'s
/// `guard_io_ok` can't distinguish *why* `open` failed, only that it did —
/// same limitation `nir_tcp_connect`'s own bad-port/bad-host cases already
/// have). `path`/`mode` are `{ptr, len}` UTF-8 buffers, same convention
/// `nir_tcp_connect`'s `host` argument already uses.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_file_open(path_ptr: *const u8, path_len: i64, mode_ptr: *const u8, mode_len: i64) -> i64 {
    let path = unsafe { std::slice::from_raw_parts(path_ptr, path_len as usize) };
    let Ok(path) = std::str::from_utf8(path) else { return -1 };
    let mode = unsafe { std::slice::from_raw_parts(mode_ptr, mode_len as usize) };
    let Ok(mode) = std::str::from_utf8(mode) else { return -1 };
    if !kernel::acquire(kernel::domain::file()) {
        return -1;
    }
    let opened = match mode {
        "r" => std::fs::File::open(path),
        "w" => std::fs::File::create(path),
        "a" => std::fs::OpenOptions::new().append(true).create(true).open(path),
        _ => {
            kernel::release(kernel::domain::file());
            return -1;
        }
    };
    match opened {
        Ok(file) => handle_of_file(file),
        Err(_) => {
            kernel::release(kernel::domain::file());
            -1
        }
    }
}

/// `send(file, s)` — `write_all`, matching `interpreter.rs::write_file`
/// exactly (loops internally until every byte is written). Returns
/// `buf_len` on success, `-1` on failure — same convention as
/// `nir_tcp_send`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_file_write(handle: i64, buf_ptr: *const u8, buf_len: i64) -> i64 {
    let mut file = ManuallyDrop::new(unsafe { file_from_handle(handle) });
    let buf = unsafe { std::slice::from_raw_parts(buf_ptr, buf_len as usize) };
    match file.write_all(buf) {
        Ok(()) => buf_len,
        Err(_) => -1,
    }
}

/// `recv(file)` — one read syscall into a fixed 64KiB buffer, matching
/// `interpreter.rs::read_file` exactly (same buffer size, same "one
/// chunk" scope). Returns bytes read (`0` is valid EOF, not an error —
/// see this section's own module doc), or `-1` on a real I/O failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_file_read(handle: i64, buf_ptr: *mut u8, buf_cap: i64) -> i64 {
    let mut file = ManuallyDrop::new(unsafe { file_from_handle(handle) });
    let buf = unsafe { std::slice::from_raw_parts_mut(buf_ptr, buf_cap as usize) };
    match file.read(buf) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Closes `handle` — `ownership.rs`'s affine typing already proves this
/// runs at most once per handle in a well-typed program, same "the
/// checker is the real gate" convention `nir_tcp_stop` already documents.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_file_stop(handle: i64) -> i32 {
    #[cfg(unix)]
    drop(unsafe { OwnedFd::from_raw_fd(handle as RawFd) });
    #[cfg(windows)]
    {
        use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
        drop(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) });
    }
    kernel::release(kernel::domain::file());
    0
}

// ---- sha256_hex / constant_time_str_eq extern boundary --------------------
//
// Backs the `sha256_hex(s)`/`sha256_hex(prev_hash, payload)` and
// `constant_time_str_eq(a, b)` builtins. `codegen.rs`'s caller passes
// `b_len: 0` for the 1-arg `sha256_hex` form -- `b_len == 0` never
// dereferences `b_ptr` (`&[]` instead of `slice::from_raw_parts` on it),
// so whatever codegen happens to pass as `b_ptr` in that case (a null
// pointer constant is fine) never has to be a real, valid pointer.
//
// **`sha256_hex`'s output buffer is heap-allocated via `nir_alloc` and
// never freed.** `Ty::Str` isn't affine (`Ty::is_affine`'s doc comment)
// -- nothing in `ownership.rs`'s `FreeMap` tracks a `str` binding's last
// use the way it does for `box`, so there's no scope-closing point for
// codegen to hook a matching `nir_free` onto even if this function
// wanted one. A real, disclosed, permanent leak (one 64-byte allocation
// per `sha256_hex` call), not a silent one -- the same "state it here
// rather than leave it implicit" discipline `box`'s own allocator used
// before its own free-hookup phase existed, except here there is no
// later phase that closes this one: making `str` affine to fix it would
// be a real, unrelated language change (every existing `str` use --
// literals, params, returns, `print` -- currently assumes freely-
// copyable, unowned `str` values), not a small follow-up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_sha256_hex(a_ptr: *const u8, a_len: i64, b_ptr: *const u8, b_len: i64, out: *mut u8) {
    let a = unsafe { std::slice::from_raw_parts(a_ptr, a_len as usize) };
    let b: &[u8] = if b_len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(b_ptr, b_len as usize) } };
    let digest = sha256(a, b);
    let out = unsafe { std::slice::from_raw_parts_mut(out, 64) };
    hex_encode(&digest, out);
}

/// `1` if the two buffers are equal, `0` otherwise -- see
/// `constant_time_eq`'s own doc comment for exactly what "constant-time"
/// does and doesn't mean here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_constant_time_str_eq(a_ptr: *const u8, a_len: i64, b_ptr: *const u8, b_len: i64) -> i32 {
    let a = unsafe { std::slice::from_raw_parts(a_ptr, a_len as usize) };
    let b = unsafe { std::slice::from_raw_parts(b_ptr, b_len as usize) };
    constant_time_eq(a, b) as i32
}

// ---- rand_seed/rand_f64/rand_gaussian kernel -------------------------------
//
// `interpreter.rs`'s own `RngState`, line-for-line (SplitMix64 for the
// underlying stream, Box-Muller for `rand_gaussian`) -- deliberately
// re-derived here rather than shared, for the same reason every other
// kernel in this file is: no access to `interpreter.rs` across the
// isolated-staticlib compilation boundary (`build.rs`'s doc comment).
// Verified bit-for-bit against the interpreter's actual output for a
// fixed seed, not just against a re-reading of the same algorithm
// description (`crates/compiler/tests/codegen.rs`'s `rand_*` tests).
//
// **Per-thread state, not process-wide — fixed (2026-09), not just
// disclosed.** This used to be a `static AtomicU64`, justified as
// "`thread`/`spawn` aren't compiled yet, so there's only ever one
// thread to own this state regardless" — true when written, false once
// they compiled (this crate's "chan/spawn/join kernels" section), and
// genuinely broken even for a single caller: the old `splitmix64_next`
// was an atomic *load* then a separate atomic *store*, not one
// compare-and-swap, so two threads calling `nir_rand_f64`/
// `nir_rand_gaussian` at the same instant could both read the same
// state before either wrote back, silently drawing the same value or
// corrupting the stream's period. `thread_local!` below closes both
// problems at once, not just the race: each OS thread gets its own
// independent `Cell` (no atomics needed at all — nothing outside this
// thread ever touches it, so there's nothing to race), and a freshly
// spawned thread's stream starts **unseeded**, restoring the
// interpreter's own documented "independent, unseeded RNG per spawn"
// behavior (`Interpreter::rng`'s doc comment) exactly, rather than
// silently inheriting the spawning thread's seed/position.
thread_local! {
    static RAND_SEEDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static RAND_STATE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn splitmix64_next() -> u64 {
    RAND_STATE.with(|state| {
        let mut s = state.get().wrapping_add(0x9E3779B97F4A7C15);
        state.set(s);
        s = (s ^ (s >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        s = (s ^ (s >> 27)).wrapping_mul(0x94D049BB133111EB);
        s ^ (s >> 31)
    })
}

fn rand_next_f64() -> f64 {
    (splitmix64_next() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// Seeds the *calling thread's* stream only — `codegen.rs` compiles
/// `rand_seed(n)` for any integer type `n` (`typeck.rs`'s
/// `t.is_integer()` check) -- every one of them arrives here already
/// widened to `i64` (this backend's own internal convention,
/// `widen_to_i64`'s doc comment), so this only ever needs to accept one
/// width. A thread spawned after this call does **not** inherit the
/// seed — see this section's own doc comment for why that's the
/// intended behavior, not a gap.
#[unsafe(no_mangle)]
pub extern "C" fn nir_rand_seed(seed: i64) {
    RAND_STATE.with(|s| s.set(seed as u64));
    RAND_SEEDED.with(|s| s.set(true));
}

/// Aborts if called before `nir_rand_seed` **on this same thread** --
/// the same "the checker can't catch this statically, so trap at
/// runtime rather than return a silently-wrong value" treatment every
/// other unrecoverable runtime condition in this backend gets
/// (div-by-zero, integer overflow, `nir_alloc` failure), enforced in
/// Rust here rather than threading an extra codegen-side branch-and-trap
/// sequence through every call site: `interpreter.rs`'s own
/// `ErrorKind::RngNotSeeded` is a real, catchable `RuntimeError` there
/// because the interpreter *can* return one; a compiled binary's
/// equivalent of "stop, this precondition was violated" is `abort()`,
/// same as `nir_alloc`'s allocation-failure path already uses.
#[unsafe(no_mangle)]
pub extern "C" fn nir_rand_f64() -> f64 {
    if !RAND_SEEDED.with(|s| s.get()) {
        std::process::abort();
    }
    rand_next_f64()
}

/// Same not-yet-seeded guard as `nir_rand_f64` (per-thread, same
/// caveat), then the interpreter's exact Box-Muller transform
/// (`next_f64()` clamped away from `0.0` before `.ln()`, same
/// sharp-edge note as `RngState::next_gaussian`'s own doc comment).
#[unsafe(no_mangle)]
pub extern "C" fn nir_rand_gaussian(mean: f64, stddev: f64) -> f64 {
    if !RAND_SEEDED.with(|s| s.get()) {
        std::process::abort();
    }
    let u1 = rand_next_f64().max(f64::MIN_POSITIVE);
    let u2 = rand_next_f64();
    let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    mean + stddev * z0
}

// ---- box's heap allocator -------------------------------------------------
//
// `nir_free(ptr)` takes only a pointer, no size — `codegen.rs`'s
// `ty_byte_size` computes a box's allocation size at the `box e` call
// site, but by the time (a later phase's) `nir_free` runs at last-use, the
// *static* type is still known there too in principle, but threading it
// through would mean every free call site needs to redo that computation
// and match it exactly against what the alloc site used. Simpler and more
// robust: `nir_alloc` writes its own size into a small header immediately
// before the returned pointer, and `nir_free` reads it back — the
// allocator is the only thing that ever needs to agree with itself.
// `size + HEADER_BYTES` is over-allocated by exactly enough to fit that
// header; `align(16)` is generous enough for every `Ty` this backend can
// box (nothing here needs more than 8-byte alignment, `f64`/`ptr`
// included, but 16 costs nothing and leaves headroom).
const NIR_ALLOC_HEADER_BYTES: usize = 16;
const NIR_ALLOC_ALIGN: usize = 16;

/// Heap-allocates `size` bytes for `box e`, returning a pointer to the
/// usable region (the header lives just before it, invisible to the
/// caller). Aborts on allocation failure — `panic=abort` (`build.rs`)
/// turns `handle_alloc_error`'s abort into the same "the process just
/// stops" behavior every other unrecoverable condition in this backend
/// already has (the div-by-zero/overflow/bounds traps), not a new failure
/// mode.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_alloc(size: i64) -> *mut u8 {
    let size = size as usize;
    let total = size + NIR_ALLOC_HEADER_BYTES;
    let layout = std::alloc::Layout::from_size_align(total, NIR_ALLOC_ALIGN)
        .expect("box allocation size is always a small, codegen-computed constant");
    let base = unsafe { std::alloc::alloc(layout) };
    if base.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    unsafe {
        (base as *mut usize).write(size);
        base.add(NIR_ALLOC_HEADER_BYTES)
    }
}

/// Frees a pointer previously returned by `nir_alloc`. Called for real
/// by every compiled program that boxes a value: `codegen.rs`'s
/// `emit_frees_for_names`/`emit_box_free`, driven by `ownership.rs`'s
/// `FreeMap` (which binding's last use is where), emit this at every
/// scope-closing point a boxed binding's last use falls in — confirmed
/// in generated IR (`nirdosha emit-llvm`) for a simple `let`-bound box,
/// not just assumed from this comment.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let base = ptr.sub(NIR_ALLOC_HEADER_BYTES);
        let size = (base as *const usize).read();
        let layout = std::alloc::Layout::from_size_align(size + NIR_ALLOC_HEADER_BYTES, NIR_ALLOC_ALIGN)
            .expect("matches the layout nir_alloc used to allocate this same pointer");
        std::alloc::dealloc(base, layout);
    }
}

// ---- chan/spawn/join kernels (RFC 0006 pillars 2-4, wired for real) -------
//
// `codegen.rs` lowers both `Ty::Channel(_)` and `Ty::Thread(_)` to a plain
// `i64` handle, exactly like `Ty::Tcp`/`Ty::File` above — the same "the
// kernel already tracks everything a handle needs" story, so there's no
// separate handle-table type per resource, just [`kernel::HandleTable`]
// used twice. What crosses this ABI boundary is always one `i64` machine
// word per value (`codegen.rs`'s `word_to_i64`/`word_from_i64` bitcast/
// ptrtoint a narrower scalar into that shape at the call site) — a real,
// disclosed narrower scope than `chan`/`spawn`'s full type-level generality
// (`str`/`dec128`/struct/enum payloads aren't supported yet, the same
// "type-oblivious pre-pass, real check at IR-gen time" gap every other
// partial feature here discloses rather than silently mishandles).
//
// A `chan` handle is never closed (`typeck.rs` gives `Ty::Channel` no
// `stop` case — a channel is meant to be held by more than one concurrent
// computation, `mailbox`'s own doc comment) — its `HandleTable` entry, and
// the `crossbeam_channel` pair inside it, simply live for the process's
// whole lifetime. `nir_chan_recv` clones the `Receiver` out from under the
// table's lock before blocking on it: `Receiver` is legally cloneable
// (`kernel::mailbox`'s whole point) and blocking while holding the one
// lock shared by every channel in the process would serialize every other
// channel's `new`/`send`/`recv` behind it, defeating "multi-consumer"
// before it even starts.
//
// A `thread` handle's `HandleTable` entry holds the one-job [`Scope`] that
// call site's `spawn` created (so `join` blocks on exactly that job, not
// every job any `Scope` anywhere has ever spawned) plus a raw `result_ptr`
// this file itself owns (a `Box<i64>` converted to a raw pointer with
// `Box::into_raw`, so its heap address survives being moved into the
// table). The spawned job writes through `result_ptr` and then drops its
// `DecrementOnDrop` guard (`thread_pool::Scope::spawn`'s own doc comment)
// — a plain `Mutex` lock/unlock inside `Scope::join` already gives that
// write a real happens-before edge to whatever thread later calls
// `nir_thread_join` and reads it back, so `result_ptr` needs no atomic or
// lock of its own.
//
// **What this deliberately does not attempt**: Pillar 4's full promise —
// "every spawned thread is tracked by the `Scope` covering its spawning
// function body" — would need `codegen.rs` to thread a per-function
// `Scope` through every frame that can spawn. What's real today instead:
// every `spawn` gets its own dedicated one-job `Scope` (so a `join` really
// does wait for, and only for, that one spawn — not an accidental wait on
// some unrelated concurrent spawn sharing the same `Scope`), and
// `codegen.rs`'s `emit_affine_free` auto-`join`s any `thread` handle a
// function forgot to consume before its scope ends (the same `FreeMap`-
// driven auto-close `box`/`tcp` already get) — so an orphan, never-joined
// thread is structurally impossible in a well-typed program, even though
// it isn't the exact lexical-scope mechanism the RFC's own prototype uses.

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use kernel::HandleTable;
use kernel::thread_pool::{Scope, ThreadPool};
use std::sync::{Arc, OnceLock};

fn global_thread_pool() -> &'static Arc<ThreadPool> {
    static POOL: OnceLock<Arc<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(ThreadPool::new)
}

fn channel_table() -> &'static HandleTable<(Sender<i64>, Receiver<i64>)> {
    static TABLE: OnceLock<HandleTable<(Sender<i64>, Receiver<i64>)>> = OnceLock::new();
    TABLE.get_or_init(HandleTable::new)
}

/// `chan T`'s own construction — same handle for every `T` (the payload's
/// shape only matters at `send`/`recv`, never at creation), so this needs
/// no type information at all.
#[unsafe(no_mangle)]
pub extern "C" fn nir_chan_new() -> i64 {
    let (tx, rx) = kernel::mailbox::mailbox::<i64>();
    channel_table().insert((tx, rx))
}

/// Pillar 2: enqueues `value` and returns immediately — `0` always,
/// unless every receiver for `handle` has already been dropped (never
/// happens today, since nothing ever removes a channel's table entry —
/// kept as a real, checked `-1` rather than an `unwrap`, so a future
/// caller that *does* add a close path fails cleanly instead of
/// panicking).
#[unsafe(no_mangle)]
pub extern "C" fn nir_chan_send(handle: i64, value: i64) -> i64 {
    match channel_table().with(handle, |(tx, _rx)| kernel::mailbox::send(tx, value)) {
        Some(Ok(())) => 0,
        _ => -1,
    }
}

/// Pillar 3: blocks until a message is available. `0` on a closed channel
/// (see `nir_chan_send`'s doc comment — not reachable today, but an inert
/// `0` rather than a panic if it ever is).
///
/// Tries a non-blocking `try_recv` first, deliberately — this is one of
/// exactly two operations (`nir_thread_join` is the other) the deadlock
/// detector in `kernel::concurrency_wait_begin`'s own doc comment treats
/// as "can only ever be unblocked by another concurrent participant,"
/// and it must never register a wait for a call that was never actually
/// going to block: a message already sitting in the mailbox (the
/// overwhelmingly common case — a producer that already ran to
/// completion before this `recv` even started) must be returned without
/// ever touching the wait counters, or a fast-finishing sender racing a
/// slower receiver could look indistinguishable from a real stall.
#[unsafe(no_mangle)]
pub extern "C" fn nir_chan_recv(handle: i64) -> i64 {
    let Some(rx) = channel_table().with(handle, |(_tx, rx)| rx.clone()) else {
        return 0;
    };
    match rx.try_recv() {
        Ok(v) => return v,
        Err(TryRecvError::Disconnected) => return 0,
        Err(TryRecvError::Empty) => {}
    }
    kernel::concurrency_wait_begin(kernel::WaitTarget::ChanRecv(handle));
    let result = kernel::mailbox::receive(&rx).unwrap_or(0);
    kernel::concurrency_wait_end();
    result
}

/// One spawned computation's kernel-owned bookkeeping — see this
/// section's own doc comment for why both fields live here rather than on
/// the `.nir`-side `thread` handle itself (which stays a bare `i64`).
struct ThreadHandle {
    scope: Scope,
    result_ptr: *mut i64,
}
// SAFETY: `result_ptr` is written by exactly one spawned job and read
// back by exactly one `nir_thread_join` call, synchronized through
// `Scope::join`'s own `Mutex` (this section's doc comment) — never
// accessed concurrently from two threads at once, so moving the whole
// `ThreadHandle` (raw pointer included) into the table's `Mutex`-guarded
// map from a different thread than the one that eventually joins it is
// sound.
unsafe impl Send for ThreadHandle {}

fn thread_table() -> &'static HandleTable<ThreadHandle> {
    static TABLE: OnceLock<HandleTable<ThreadHandle>> = OnceLock::new();
    TABLE.get_or_init(HandleTable::new)
}

/// `spawn name(args)`'s real implementation. `trampoline` is a function
/// `codegen.rs` generates once per call site — it unpacks `ctx` (a
/// `nir_alloc`-ed block holding `args`, freed by the trampoline itself
/// once it's copied them out), calls the actual spawned function, and
/// writes its result (widened/bitcast to one `i64` word, or left
/// untouched for a `unit`-returning spawn) through `result_slot`. Passing
/// a raw function pointer across this boundary needs no cast on either
/// side: LLVM's opaque `ptr` and Rust's `extern "C" fn(...)` are the same
/// calling-convention shape.
///
/// Returns the new thread's handle immediately — the job runs
/// concurrently; `nir_thread_join` is what actually waits for it. `-1`
/// only if the OS itself refused to create a thread
/// (`thread_pool::SpawnError` — real, not-happened-in-practice resource
/// exhaustion), the same uniform failure convention every other
/// resource-creation kernel here already uses.
#[unsafe(no_mangle)]
pub extern "C" fn nir_thread_spawn(trampoline: extern "C" fn(*mut u8, *mut i64), ctx: *mut u8) -> i64 {
    // Admission first, at the resource-creation call only (this
    // module's own "chan/spawn/join kernels" doc comment) -- one
    // concurrently-outstanding `thread` handle held between `spawn` and
    // its matching `join`, the same ceiling `nir_tcp_connect`/
    // `nir_file_open` already enforce for their own domains.
    if !kernel::acquire(kernel::domain::thread()) {
        unsafe {
            if !ctx.is_null() {
                nir_free(ctx);
            }
        }
        return -1;
    }
    let result_ptr = Box::into_raw(Box::new(0i64));
    let scope = Scope::new(global_thread_pool());
    // A tiny `Send` wrapper around the two raw pointers and the function
    // pointer -- all three are used exactly once, entirely on the
    // spawned job's own thread, never touched again by the thread that
    // called `nir_thread_spawn` until (if ever) it later calls
    // `nir_thread_join`.
    struct SpawnPayload(*mut u8, *mut i64, extern "C" fn(*mut u8, *mut i64));
    unsafe impl Send for SpawnPayload {}
    impl SpawnPayload {
        // A method call's receiver is the *whole* value, not a field
        // projection -- unlike `payload.0`/`let SpawnPayload(a, b, c) =
        // payload` (both of which Rust's disjoint-closure-capture
        // analysis, RFC 2229, decomposes into per-field captures even
        // through a full-struct pattern), this is the one access shape
        // that forces the closure below to capture `payload` as one
        // `Send`-wrapped value instead of three individually-non-`Send`
        // raw pointers/fn pointer.
        fn call(self) {
            (self.2)(self.0, self.1);
        }
    }
    let payload = SpawnPayload(ctx, result_ptr, trampoline);
    // Incremented *before* the job is submitted, not after -- a worker
    // thread can start running the job the instant `scope.spawn` returns
    // `Ok`, and that job's own code could reach a `join`/`recv` (and so
    // `concurrency_wait_begin`'s live-count check) before this calling
    // thread gets to run another instruction. Registering "this
    // participant now exists" strictly before it could possibly wait on
    // anything is what makes `concurrency_wait_begin`'s check exact
    // rather than racy (see `kernel::concurrency_thread_started`'s own
    // doc comment).
    kernel::concurrency_thread_started();
    // Deliberately *not* calling `kernel::concurrency_thread_finished()`
    // from inside this closure once `payload.call()` returns -- see
    // `concurrency_thread_finished`'s own doc comment for the race that
    // would reopen (this counter and `Scope`'s own completion state are
    // two separate locks with no ordering between them). `nir_thread_
    // join` calls it instead, only once `Scope::already_done`/`join`
    // has itself confirmed completion.
    let submitted = scope.spawn(Box::new(move || payload.call()));
    if submitted.is_err() {
        // The OS refused to create a worker thread -- the job never ran
        // at all, so the optimistic increment above has to be rolled
        // back here (nothing else ever will: this handle is never
        // inserted into `thread_table`, so `nir_thread_join` will never
        // run for it either).
        kernel::concurrency_thread_finished();
        kernel::release(kernel::domain::thread());
        unsafe {
            drop(Box::from_raw(result_ptr));
            if !ctx.is_null() {
                nir_free(ctx);
            }
        }
        return -1;
    }
    thread_table().insert(ThreadHandle { scope, result_ptr })
}

/// `join`'s real implementation — blocks until `handle`'s one spawned job
/// completes (whether it returned normally or panicked; `thread_pool`'s
/// own panic containment, this section's doc comment), then returns its
/// result word. A double-join or an already-consumed handle returns `0`
/// rather than panicking — `ownership.rs`'s affine typing already proves
/// this doesn't happen in a well-typed program (the same "the checker is
/// the real gate" trust convention `nir_tcp_stop` documents), including
/// the implicit auto-join `codegen.rs::emit_affine_free` emits for a
/// `thread` handle a function forgot to consume itself.
#[unsafe(no_mangle)]
pub extern "C" fn nir_thread_join(handle: i64) -> i64 {
    let Some(entry) = thread_table().remove(handle) else {
        return 0;
    };
    // Checked non-blockingly first, for the identical reason
    // `nir_chan_recv`'s own `try_recv`-first does: a `join` on a job
    // that already finished was never actually going to block, and must
    // never be counted as a real wait (`concurrency_wait_begin`'s doc
    // comment, and `Scope::already_done`'s own doc comment for exactly
    // this hazard). Only when genuinely still outstanding does this
    // become one of the two operations (`nir_chan_recv` is the other)
    // the deadlock detector treats as "can only ever be unblocked by
    // another concurrent participant."
    if !entry.scope.already_done() {
        kernel::concurrency_wait_begin(kernel::WaitTarget::ThreadJoin(handle));
        entry.scope.join();
        kernel::concurrency_wait_end();
    }
    // This job can no longer run any more code that could unblock
    // someone else, whichever branch above got here — see `kernel::
    // concurrency_thread_finished`'s own doc comment for why this is
    // the one place that's called, rather than the job itself.
    kernel::concurrency_thread_finished();
    let result = unsafe { *entry.result_ptr };
    unsafe { drop(Box::from_raw(entry.result_ptr)) };
    // The `thread` handle's own admission slot (`nir_thread_spawn`'s own
    // doc comment) is released here, at the one-time consuming `join`
    // that closes it — the same acquire-at-creation/release-at-close
    // pairing `nir_tcp_stop`/`nir_file_stop` already use for their own
    // domains.
    kernel::release(kernel::domain::thread());
    result
}

// ---- nfr kernels (NFRs as a first-class, compiled-runtime feature) -------
//
// See `kernel::nfr`'s own module doc for the real design (four O(1)
// tracked metrics, escalation only on violation, never on the hot
// path). This is the `extern "C"` boundary `codegen.rs` compiles
// `nfr(...)` to: `nir_nfr_register` once per tracked function at
// program start, `nir_nfr_call_begin`/`nir_nfr_call_end` bracketing
// every call to it.

/// Registers one `nfr(...)`-declared function — `name_ptr`/`name_len`
/// is a `{ptr, i64}` `str` value's own two halves (same convention
/// every other kernel here taking a `str` argument uses). A negative
/// `i64`/`f64` field means that NFR wasn't declared — `kernel::nfr::
/// register`'s own doc comment. Returns the id `nir_nfr_call_begin`/
/// `_end` pass back on every call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_nfr_register(
    name_ptr: *const u8,
    name_len: i64,
    latency_ms: i64,
    error_rate_max: f64,
    throughput_min_per_sec: i64,
    concurrency_max: i64,
) -> i64 {
    let name = unsafe { std::slice::from_raw_parts(name_ptr, name_len as usize) };
    let name = String::from_utf8_lossy(name).into_owned();
    kernel::nfr::register(name, latency_ms, error_rate_max, throughput_min_per_sec, concurrency_max)
}

#[unsafe(no_mangle)]
pub extern "C" fn nir_nfr_call_begin(id: i64) -> i64 {
    kernel::nfr::call_begin(id)
}

/// `was_err` is `0`/`1`, not a real `i1`/`bool` at this ABI boundary —
/// the same "every boolean-shaped flag crossing this file's `extern
/// "C"` edge is a plain integer" convention this crate already uses
/// throughout (`nir_str_eq`'s `i32` return, etc.), not an LLVM-`i1`-
/// specific assumption.
#[unsafe(no_mangle)]
pub extern "C" fn nir_nfr_call_end(id: i64, start_ns: i64, was_err: i32) {
    kernel::nfr::call_end(id, start_ns, was_err != 0)
}

// ---- check_role / oidc_validate_token / extract_claim kernels -----------
// (identity/RBAC, real authorization pipeline)
//
// `codegen.rs`'s `IDENTITY_BUILTINS`/`emit_check_role` doc comments have
// the full scope. `check_role` was previously a plain comma-separated
// role list, deliberately narrower than the interpreter's own JSON-based
// one, disclosed as such — "no JSON parser is linked into this crate".
// 2026-09: that's no longer true (`Cargo.toml`'s new `serde_json`
// dependency, added for `oidc_validate_token`'s own claims below), so
// `check_role` is upgraded here to real JSON-array parsing too, **with a
// fallback to the original comma-separated matching** so a
// `VerifiedIdentity` built directly in `.nir` source with a plain
// `claims_json = "admin,editor"` string (not real JSON — the existing
// `check_role_produces_real_role_view_that_drives_field_masking` test
// fixture does exactly this) still works unchanged.
//
// `oidc_validate_token`/`extract_claim` port
// `crates/presence-gateway/src/jwt.rs`'s exact JWKS-verification shape
// (same dependency set, same doc comment's own reasoning) — including its
// one load-bearing security detail: a JWK's `kty` *locks* which `alg` it
// may verify under (RSA→RS256, EC/P-256→ES256, oct→HS256), never derived
// from the token's own `alg` header, closing the classic algorithm-
// confusion attack (an RSA public key replayed as an HMAC secret).
// Deliberately **not** validating `exp` against the real wall clock here,
// unlike `jwt.rs::verify` — that module is a live network boundary and
// checking real-time expiry there is the right split for its job; this
// compiled builtin stays a pure function of its inputs
// (`docs/LANGUAGE.md` §9's determinism story), leaving expiry to
// `identity_expired(identity, now)`'s own explicit `now` instead of an
// ambient clock read.

#[derive(serde::Deserialize)]
struct RawJwks {
    keys: Vec<RawJwk>,
}

#[derive(serde::Deserialize)]
struct RawJwk {
    kid: String,
    kty: String,
    // RSA (`kty: "RSA"`)
    n: Option<String>,
    e: Option<String>,
    // EC (`kty: "EC"`, `crv: "P-256"` only — ES256)
    crv: Option<String>,
    x: Option<String>,
    y: Option<String>,
    // oct (`kty: "oct"`, symmetric — HS256 only).
    k: Option<String>,
}

fn decoding_key_for(jwk: &RawJwk) -> Result<(jsonwebtoken::DecodingKey, jsonwebtoken::Algorithm), String> {
    use base64::Engine as _;
    match jwk.kty.as_str() {
        "RSA" => {
            let n = jwk.n.as_deref().ok_or("JWK is missing required field `n`")?;
            let e = jwk.e.as_deref().ok_or("JWK is missing required field `e`")?;
            let decoding_key = jsonwebtoken::DecodingKey::from_rsa_components(n, e).map_err(|err| format!("invalid RSA key material: {err}"))?;
            Ok((decoding_key, jsonwebtoken::Algorithm::RS256))
        }
        "EC" => {
            let crv = jwk.crv.clone().unwrap_or_default();
            if crv != "P-256" {
                return Err(format!("unsupported EC curve `{crv}` (only P-256/ES256 is supported)"));
            }
            let x = jwk.x.as_deref().ok_or("JWK is missing required field `x`")?;
            let y = jwk.y.as_deref().ok_or("JWK is missing required field `y`")?;
            let decoding_key = jsonwebtoken::DecodingKey::from_ec_components(x, y).map_err(|err| format!("invalid EC key material: {err}"))?;
            Ok((decoding_key, jsonwebtoken::Algorithm::ES256))
        }
        "oct" => {
            let k = jwk.k.as_deref().ok_or("JWK is missing required field `k`")?;
            let raw_secret = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(k).map_err(|_| "JWK field `k` is not valid base64url".to_string())?;
            Ok((jsonwebtoken::DecodingKey::from_secret(&raw_secret), jsonwebtoken::Algorithm::HS256))
        }
        other => Err(format!("unsupported JWK `kty`: `{other}` (only RSA, EC/P-256, and oct are supported)")),
    }
}

struct VerifiedClaims {
    subject: String,
    issuer: String,
    audience: String,
    expires_at: i64,
    issued_at: i64,
    claims_json: String,
}

fn validate_oidc_token_inner(token: &str, expected_issuer: &str, expected_audience: &str, jwks_json: &str) -> Result<VerifiedClaims, String> {
    let jwks: RawJwks = serde_json::from_str(jwks_json).map_err(|e| format!("malformed JWKS: {e}"))?;
    let header = jsonwebtoken::decode_header(token).map_err(|e| format!("malformed token: {e}"))?;
    let kid = header.kid.ok_or_else(|| "token header has no `kid`".to_string())?;
    let jwk = jwks.keys.iter().find(|k| k.kid == kid).ok_or_else(|| format!("token references unknown key id `{kid}`"))?;
    let (decoding_key, algorithm) = decoding_key_for(jwk)?;

    // Locked to exactly the one algorithm this `kid`'s key material is
    // valid for (never the token's own `alg` header) — the actual
    // algorithm-confusion guard, decided entirely by `decoding_key_for`'s
    // `kty` match above, same as `jwt.rs::verify`.
    let mut validation = jsonwebtoken::Validation::new(algorithm);
    validation.set_issuer(&[expected_issuer]);
    validation.set_audience(&[expected_audience]);
    validation.validate_exp = false; // see this section's own doc comment

    let data = jsonwebtoken::decode::<serde_json::Value>(token, &decoding_key, &validation).map_err(|e| format!("token verification failed: {e}"))?;
    let claims = data.claims;

    let subject = claims.get("sub").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let issuer = claims.get("iss").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let audience = claims.get("aud").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let expires_at = claims.get("exp").and_then(|v| v.as_i64()).unwrap_or(0);
    let issued_at = claims.get("iat").and_then(|v| v.as_i64()).unwrap_or(0);
    // The full decoded claims, re-serialized — this is what `check_role`/
    // `extract_claim` read back out of `VerifiedIdentity.claims_json`.
    let claims_json = serde_json::to_string(&claims).unwrap_or_else(|_| "{}".to_string());

    Ok(VerifiedClaims { subject, issuer, audience, expires_at, issued_at, claims_json })
}

/// `{ptr, i64}` — matches `Ty::Str`'s own codegen layout exactly (same
/// idea as the RFC 0008 native-plugin ABI's `NirStr`), so `codegen.rs`
/// can load one of these straight into a `str` value with a plain
/// `insertvalue` pair, no marshaling. Every `NirStrOut` this file
/// produces is heap-allocated via `nir_alloc` and never freed — the same
/// disclosed, permanent-leak convention `nir_sha256_hex` already uses
/// (`Ty::Str` isn't affine, so there's no scope-closing point to hook a
/// matching `nir_free` onto).
#[repr(C)]
pub struct NirStrOut {
    pub ptr: *const u8,
    pub len: i64,
}

unsafe fn write_str_out(out: *mut NirStrOut, s: String) {
    let leaked: &'static [u8] = Box::leak(s.into_boxed_str().into_boxed_bytes());
    unsafe {
        *out = NirStrOut { ptr: leaked.as_ptr(), len: leaked.len() as i64 };
    }
}

unsafe fn str_from_raw<'a>(ptr: *const u8, len: i64) -> Option<&'a str> {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    std::str::from_utf8(bytes).ok()
}

/// `oidc_validate_token`'s real, compiled implementation. `1` (with every
/// `out_*` param populated) if `token`'s signature verifies against
/// `jwks_json` and its `iss`/`aud` match, `0` (with `out_err` populated,
/// everything else untouched) otherwise — a malformed token/JWKS is a
/// real `Err`, never a trap, same as every other identity check in this
/// codebase.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_oidc_validate_token(
    token_ptr: *const u8,
    token_len: i64,
    issuer_ptr: *const u8,
    issuer_len: i64,
    audience_ptr: *const u8,
    audience_len: i64,
    jwks_ptr: *const u8,
    jwks_len: i64,
    out_subject: *mut NirStrOut,
    out_issuer: *mut NirStrOut,
    out_audience: *mut NirStrOut,
    out_expires_at: *mut i64,
    out_issued_at: *mut i64,
    out_claims_json: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let (Some(token), Some(expected_issuer), Some(expected_audience), Some(jwks_json)) = (
        unsafe { str_from_raw(token_ptr, token_len) },
        unsafe { str_from_raw(issuer_ptr, issuer_len) },
        unsafe { str_from_raw(audience_ptr, audience_len) },
        unsafe { str_from_raw(jwks_ptr, jwks_len) },
    ) else {
        unsafe { write_str_out(out_err, "token/issuer/audience/jwks is not valid UTF-8".to_string()) };
        return 0;
    };
    match validate_oidc_token_inner(token, expected_issuer, expected_audience, jwks_json) {
        Ok(claims) => unsafe {
            write_str_out(out_subject, claims.subject);
            write_str_out(out_issuer, claims.issuer);
            write_str_out(out_audience, claims.audience);
            *out_expires_at = claims.expires_at;
            *out_issued_at = claims.issued_at;
            write_str_out(out_claims_json, claims.claims_json);
            1
        },
        Err(msg) => unsafe {
            write_str_out(out_err, msg);
            0
        },
    }
}

/// `mock_issue_token`'s real implementation — the inverse of
/// `oidc_validate_token` above: signs a token instead of verifying one.
/// `mock_` is load-bearing, not decorative (the builtin's own typeck doc
/// comment) — this issues a token from key material the caller supplies
/// directly, standing in for a real IdP's own signing endpoint, never a
/// substitute for one.
///
/// **HS256 (symmetric) only, deliberately.** `jwks_json` is the exact
/// same shape `oidc_validate_token`/`decoding_key_for` already parse
/// (`RawJwks`/`RawJwk` above); this looks up the first `kty: "oct"` entry
/// and signs with it. RSA/EC issuance would need real private-key
/// material (a JWK's `d` parameter and friends) this first pass doesn't
/// handle — a real, disclosed follow-up, not attempted here. Verifying
/// the token this produces against the *same* `jwks_json` already works
/// today, unchanged, via `oidc_validate_token`'s own existing `"oct"` arm.
fn issue_mock_token_inner(
    subject: &str,
    issuer: &str,
    audience: &str,
    issued_at: i64,
    ttl_secs: i64,
    claims_json: &str,
    jwks_json: &str,
) -> Result<String, String> {
    use base64::Engine as _;

    let jwks: RawJwks = serde_json::from_str(jwks_json).map_err(|e| format!("malformed JWKS: {e}"))?;
    let jwk = jwks
        .keys
        .iter()
        .find(|k| k.kty == "oct")
        .ok_or_else(|| "no `oct` (HMAC) signing key found in jwks_json -- mock_issue_token only supports HS256".to_string())?;
    let k = jwk.k.as_deref().ok_or("JWK is missing required field `k`")?;
    let raw_secret = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(k)
        .map_err(|_| "JWK field `k` is not valid base64url".to_string())?;
    let encoding_key = jsonwebtoken::EncodingKey::from_secret(&raw_secret);

    // `claims_json`'s own fields (e.g. a `"roles"` array `check_role`
    // reads back out later) are preserved -- only the six standard claims
    // this builtin itself owns are overwritten, same "caller-supplied
    // extras survive" convention `oidc_validate_token`'s own `claims_json`
    // output already has.
    let mut claims: serde_json::Value = serde_json::from_str(claims_json).unwrap_or_default();
    if !claims.is_object() {
        claims = serde_json::Value::Object(serde_json::Map::new());
    }
    let obj = claims.as_object_mut().expect("just ensured this is an object");
    obj.insert("sub".to_string(), serde_json::Value::String(subject.to_string()));
    obj.insert("iss".to_string(), serde_json::Value::String(issuer.to_string()));
    obj.insert("aud".to_string(), serde_json::Value::String(audience.to_string()));
    obj.insert("iat".to_string(), serde_json::Value::from(issued_at));
    obj.insert("exp".to_string(), serde_json::Value::from(issued_at.saturating_add(ttl_secs)));

    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    header.kid = Some(jwk.kid.clone());

    jsonwebtoken::encode(&header, &claims, &encoding_key).map_err(|e| format!("failed to sign token: {e}"))
}

/// `1` (with `out_token` populated) on success, `0` (with `out_err`
/// populated) otherwise — a malformed JWKS or a JWKS with no usable
/// signing key is a real `Err`, never a trap, same as every other
/// identity check in this codebase.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_mock_issue_token(
    subject_ptr: *const u8,
    subject_len: i64,
    issuer_ptr: *const u8,
    issuer_len: i64,
    audience_ptr: *const u8,
    audience_len: i64,
    issued_at: i64,
    ttl_secs: i64,
    claims_json_ptr: *const u8,
    claims_json_len: i64,
    jwks_ptr: *const u8,
    jwks_len: i64,
    out_token: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let (Some(subject), Some(issuer), Some(audience), Some(claims_json), Some(jwks_json)) = (
        unsafe { str_from_raw(subject_ptr, subject_len) },
        unsafe { str_from_raw(issuer_ptr, issuer_len) },
        unsafe { str_from_raw(audience_ptr, audience_len) },
        unsafe { str_from_raw(claims_json_ptr, claims_json_len) },
        unsafe { str_from_raw(jwks_ptr, jwks_len) },
    ) else {
        unsafe { write_str_out(out_err, "subject/issuer/audience/claims_json/jwks is not valid UTF-8".to_string()) };
        return 0;
    };
    match issue_mock_token_inner(subject, issuer, audience, issued_at, ttl_secs, claims_json, jwks_json) {
        Ok(token) => unsafe {
            write_str_out(out_token, token);
            1
        },
        Err(msg) => unsafe {
            write_str_out(out_err, msg);
            0
        },
    }
}

/// `1` if `role` (as UTF-8 bytes) is present in `claims`'s roles, `0`
/// otherwise — including if either buffer isn't valid UTF-8, the same
/// "fail closed on malformed input" posture every other identity check
/// in this codebase already has. Tries `claims` as real JSON first (a
/// top-level `"roles"` array of strings — `nir_oidc_validate_token`'s own
/// `claims_json` output shape); on JSON-parse failure, falls back to the
/// original plain comma-separated-list matching (each entry's
/// surrounding whitespace trimmed) — exact per-entry matching either way,
/// never a substring search (`"adm"` must never match `"admin"`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_check_role(claims_ptr: *const u8, claims_len: i64, role_ptr: *const u8, role_len: i64) -> i32 {
    let claims = unsafe { std::slice::from_raw_parts(claims_ptr, claims_len as usize) };
    let role = unsafe { std::slice::from_raw_parts(role_ptr, role_len as usize) };
    let (Ok(claims), Ok(role)) = (std::str::from_utf8(claims), std::str::from_utf8(role)) else {
        return 0;
    };
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(claims) {
        if let Some(roles) = parsed.get("roles").and_then(|v| v.as_array()) {
            return roles.iter().any(|r| r.as_str() == Some(role)) as i32;
        }
    }
    claims.split(',').any(|entry| entry.trim() == role) as i32
}

/// `extract_claim`'s real, compiled implementation. `1` (with `out_value`
/// populated) if `claims_json` parses as JSON and has a top-level
/// string-valued claim named `name`, `0` otherwise. Array/object-valued
/// claims (e.g. `"roles"`) are out of scope here — that's `check_role`'s
/// job, not `extract_claim`'s, per this exact split in
/// `examples/features/30_identity_oidc.nir`'s own fixture.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_extract_claim(claims_json_ptr: *const u8, claims_json_len: i64, name_ptr: *const u8, name_len: i64, out_value: *mut NirStrOut) -> i32 {
    let (Some(claims_json), Some(name)) = (unsafe { str_from_raw(claims_json_ptr, claims_json_len) }, unsafe { str_from_raw(name_ptr, name_len) }) else {
        return 0;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(claims_json) else {
        return 0;
    };
    match parsed.get(name).and_then(|v| v.as_str()) {
        Some(s) => unsafe {
            write_str_out(out_value, s.to_string());
            1
        },
        None => 0,
    }
}

#[cfg(test)]
mod identity_kernel_tests {
    use super::*;

    // Same fixture as `examples/features/30_identity_oidc.nir`/
    // `31_mock_identity_provider.nir`: header `{"alg":"HS256","kid":"key1"}`,
    // payload `{"sub":"alice","iss":"https://example.com","aud":"my-app",
    // "exp":2000000000,"iat":1700000000,"roles":["physician"],
    // "department":"cardiology"}`, secret `"my-secret-key"` (JWKS `k` is
    // that string's base64url encoding).
    const FIXTURE_TOKEN: &str = "eyJhbGciOiAiSFMyNTYiLCAia2lkIjogImtleTEifQ.eyJzdWIiOiAiYWxpY2UiLCAiaXNzIjogImh0dHBzOi8vZXhhbXBsZS5jb20iLCAiYXVkIjogIm15LWFwcCIsICJleHAiOiAyMDAwMDAwMDAwLCAiaWF0IjogMTcwMDAwMDAwMCwgInJvbGVzIjogWyJwaHlzaWNpYW4iXSwgImRlcGFydG1lbnQiOiAiY2FyZGlvbG9neSJ9.nrFdeqNDwXWLeGzud6X9Q4ITzCXULzZBBK8y51LGYXs";
    const FIXTURE_JWKS: &str = r#"{"keys":[{"kid":"key1","kty":"oct","k":"bXktc2VjcmV0LWtleQ"}]}"#;

    #[test]
    fn valid_token_verifies_and_extracts_real_claims() {
        let claims = validate_oidc_token_inner(FIXTURE_TOKEN, "https://example.com", "my-app", FIXTURE_JWKS).expect("fixture token should verify");
        assert_eq!(claims.subject, "alice");
        assert_eq!(claims.issuer, "https://example.com");
        assert_eq!(claims.audience, "my-app");
        assert_eq!(claims.expires_at, 2000000000);
        assert_eq!(claims.issued_at, 1700000000);
        let parsed: serde_json::Value = serde_json::from_str(&claims.claims_json).unwrap();
        assert_eq!(parsed["department"], "cardiology");
        assert_eq!(parsed["roles"][0], "physician");
    }

    #[test]
    fn wrong_issuer_is_rejected() {
        assert!(validate_oidc_token_inner(FIXTURE_TOKEN, "https://wrong-issuer.example", "my-app", FIXTURE_JWKS).is_err());
    }

    #[test]
    fn wrong_audience_is_rejected() {
        assert!(validate_oidc_token_inner(FIXTURE_TOKEN, "https://example.com", "wrong-app", FIXTURE_JWKS).is_err());
    }

    #[test]
    fn tampered_signature_is_rejected() {
        // Flip the last character of the signature segment.
        let mut tampered = FIXTURE_TOKEN.to_string();
        tampered.pop();
        tampered.push('x');
        assert!(validate_oidc_token_inner(&tampered, "https://example.com", "my-app", FIXTURE_JWKS).is_err());
    }

    #[test]
    fn unknown_kid_is_rejected() {
        let jwks = r#"{"keys":[{"kid":"some-other-key","kty":"oct","k":"bXktc2VjcmV0LWtleQ"}]}"#;
        assert!(validate_oidc_token_inner(FIXTURE_TOKEN, "https://example.com", "my-app", jwks).is_err());
    }

    #[test]
    fn malformed_jwks_is_rejected_not_a_panic() {
        assert!(validate_oidc_token_inner(FIXTURE_TOKEN, "https://example.com", "my-app", "not json").is_err());
    }

    fn check_role(claims_json: &str, role: &str) -> i32 {
        unsafe { nir_check_role(claims_json.as_ptr(), claims_json.len() as i64, role.as_ptr(), role.len() as i64) }
    }

    #[test]
    fn check_role_finds_a_role_in_a_real_json_roles_array() {
        let claims = r#"{"roles":["physician","nurse"]}"#;
        assert_eq!(check_role(claims, "physician"), 1);
        assert_eq!(check_role(claims, "admin"), 0);
    }

    #[test]
    fn check_role_falls_back_to_comma_separated_for_non_json_claims() {
        // The existing `check_role_produces_real_role_view_that_drives_
        // field_masking` test's own fixture shape -- must keep working.
        assert_eq!(check_role("admin,editor", "admin"), 1);
        assert_eq!(check_role("admin,editor", "adm"), 0); // exact match, not substring
    }

    #[test]
    fn extract_claim_reads_a_string_claim_from_real_json() {
        let claims_json = r#"{"department":"cardiology","roles":["physician"]}"#;
        let name = "department";
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let found = unsafe { nir_extract_claim(claims_json.as_ptr(), claims_json.len() as i64, name.as_ptr(), name.len() as i64, &mut out) };
        assert_eq!(found, 1);
        let value = unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap() };
        assert_eq!(value, "cardiology");
    }

    #[test]
    fn extract_claim_not_found_returns_zero() {
        let claims_json = r#"{"department":"cardiology"}"#;
        let name = "missing";
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let found = unsafe { nir_extract_claim(claims_json.as_ptr(), claims_json.len() as i64, name.as_ptr(), name.len() as i64, &mut out) };
        assert_eq!(found, 0);
    }
}

// ---- db kernels (`db_connect`/`db_query`/`db_execute`/`stop`) -----------
//
// `Ty::Db`'s own doc comment (`ast.rs`) has the full design: layer 1
// targets SQLite specifically, via `rusqlite`'s `bundled` feature
// (statically linked, no system `libsqlite3`, same reason this crate's
// own `Cargo.toml` gives). Postgres (`postgres://`/`postgresql://`) is a
// real, separate, deferred follow-up (dynamic TLS linking) — not
// silently dropped, `docs/ROADMAP.md`'s own B2 note already says so.
//
// A `db` handle rides through `kernel::HandleTable<rusqlite::Connection>`
// (`kernel/mod.rs`'s own "not wired to any `nir_*` kernel yet" table,
// built exactly for this), not a raw OS fd like `tcp`/`file` — a
// `rusqlite::Connection` isn't `Copy`/reconstructable from a bare
// integer the way a fd is, so the table (not `Box::into_raw`/`from_raw`
// pointer arithmetic) is the right fit here, same as `channel_table`/
// `thread_table` above already use it for their own non-fd resources.

fn db_table() -> &'static HandleTable<kernel::db::DbConn> {
    static TABLE: OnceLock<HandleTable<kernel::db::DbConn>> = OnceLock::new();
    TABLE.get_or_init(HandleTable::new)
}

/// One bind value for `db_execute`/`db_query`'s trailing `?`-placeholder
/// arguments — `codegen.rs`'s `emit_db_binds` builds an array of these
/// (one per trailing arg, tag chosen by that arg's own static type),
/// passed as a plain `(ptr, i64 len)` pair like any other buffer here.
/// `#[repr(C)]`, field order/types matched exactly by the anonymous LLVM
/// struct type `codegen.rs` GEPs into (`{ i32, i64, double, ptr, i64 }`)
/// — non-packed, so ordinary C/LLVM natural-alignment layout applies on
/// both sides, the same "trust the target's own layout rules, don't
/// hand-replicate them" stance `agg_byte_size_operand`'s sizeof trick
/// already takes.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NirBindValue {
    pub tag: i32, // 0 = i64, 1 = f64, 2 = str, 3 = bool
    pub i: i64,
    pub f: f64,
    pub s_ptr: *const u8,
    pub s_len: i64,
}

unsafe fn bind_values_from_raw(ptr: *const NirBindValue, len: i64) -> Vec<rusqlite::types::Value> {
    if len == 0 {
        return Vec::new();
    }
    let raw = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    raw.iter()
        .map(|b| match b.tag {
            0 => rusqlite::types::Value::Integer(b.i),
            1 => rusqlite::types::Value::Real(b.f),
            2 => {
                let s = unsafe { str_from_raw(b.s_ptr, b.s_len) }.unwrap_or("").to_string();
                rusqlite::types::Value::Text(s)
            }
            // SQLite has no native boolean column type — bound as 0/1,
            // same convention every `i32`-as-bool return in this file
            // already uses on the way back out.
            3 => rusqlite::types::Value::Integer(b.i),
            _ => rusqlite::types::Value::Null,
        })
        .collect()
}

fn sqlite_row_to_json(row: &rusqlite::Row, column_names: &[String]) -> serde_json::Value {
    let mut obj = serde_json::Map::with_capacity(column_names.len());
    for (i, name) in column_names.iter().enumerate() {
        let value: rusqlite::types::Value = row.get(i).unwrap_or(rusqlite::types::Value::Null);
        let json_val = match value {
            rusqlite::types::Value::Null => serde_json::Value::Null,
            rusqlite::types::Value::Integer(n) => serde_json::Value::from(n),
            rusqlite::types::Value::Real(f) => serde_json::Number::from_f64(f).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
            rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
            // `BLOB` has no first-class Nirdosha type to carry it (no
            // `bytes` type — the same gap `Ty::File`'s own doc comment
            // already names for file I/O) — a disclosed, narrower cut,
            // not a silent drop: every other SQLite type round-trips
            // exactly.
            rusqlite::types::Value::Blob(_) => serde_json::Value::Null,
        };
        obj.insert(name.clone(), json_val);
    }
    serde_json::Value::Object(obj)
}

/// `db_connect(path) -> Result(db, str)`. `path` is really "connection
/// string" per `Ty::Db`'s own doc comment — a bare path or `":memory:"`
/// opens (pooled, except `:memory:`) SQLite; a `postgres://`/
/// `postgresql://` URL opens a real, pooled Postgres connection
/// (`kernel::db::connect`'s own doc comment has the full scheme-dispatch
/// and pooling design).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_db_connect(path_ptr: *const u8, path_len: i64, out_handle: *mut i64, out_err: *mut NirStrOut) -> i32 {
    let Some(path) = (unsafe { str_from_raw(path_ptr, path_len) }) else {
        unsafe { write_str_out(out_err, "connection string is not valid UTF-8".to_string()) };
        return 0;
    };
    // RFC 0011 §2 step 1: which domain ends up admission-gated depends
    // on whether `path`'s scheme is built-in -- decided *before* either
    // path acquires anything (each path's own callee -- `kernel::db::
    // connect` below, or `plugin_provider::connect_conn_shape` -- does
    // its own acquiring internally, A8 fix), so a plugin-routed connect
    // never consumes the built-in `db` domain's ceiling (and vice versa).
    if !kernel::db::is_builtin_scheme(path) {
        return match kernel::plugin_provider::connect_conn_shape(path) {
            Ok(conn) => {
                let id = kernel::plugin_provider::plugin_conn_table().insert(conn);
                unsafe { *out_handle = id };
                1
            }
            Err(e) => {
                unsafe { write_str_out(out_err, e) };
                0
            }
        };
    }
    // `kernel::db::connect` itself calls `kernel::acquire(domain::db())`
    // at its own start and releases on any `Err` path internally (A8
    // fix) — this call site holds no admission of its own to release on
    // failure; a successful `Ok(_)` return here means one admission
    // slot is now owned by `db_table()`'s newly inserted handle, released
    // by `nir_db_stop` below when that handle closes.
    match kernel::db::connect(path) {
        Ok(conn) => {
            let id = db_table().insert(conn);
            unsafe { *out_handle = id };
            1
        }
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
    }
}

/// Closes `handle` — `ownership.rs`'s affine typing already proves this
/// runs at most once per handle in a well-typed program, same "the
/// checker is the real gate" convention `nir_tcp_stop`/`nir_file_stop`
/// already document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_db_stop(handle: i64) -> i32 {
    if db_table().remove(handle).is_some() {
        kernel::release(kernel::domain::db());
        return 0;
    }
    // RFC 0011 §2 step 2: core `db_table()` first, then `HandleTable<
    // PluginConn>` on a miss -- `PluginConn`'s own `Drop` releases its
    // provider's domain (the `Pooled` variant explicitly, the
    // `Unpooled` variant via its own guard), so nothing else is needed
    // here beyond removing it from the table and letting it drop.
    kernel::plugin_provider::plugin_conn_table().remove(handle);
    0
}

/// `db_execute(conn, sql, ...binds) -> Result(i64, str)` — everything
/// except `SELECT` (`INSERT`/`UPDATE`/`DELETE`/DDL); returns the
/// affected-row count.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_db_execute(
    handle: i64,
    sql_ptr: *const u8,
    sql_len: i64,
    binds_ptr: *const NirBindValue,
    binds_len: i64,
    out_affected: *mut i64,
    out_err: *mut NirStrOut,
) -> i32 {
    let Some(sql) = (unsafe { str_from_raw(sql_ptr, sql_len) }) else {
        unsafe { write_str_out(out_err, "sql is not valid UTF-8".to_string()) };
        return 0;
    };
    let result: Option<Result<i64, String>> = db_table().with(handle, |conn| {
        if let Some(sqlite) = conn.as_sqlite_mut() {
            let binds = unsafe { bind_values_from_raw(binds_ptr, binds_len) };
            return sqlite.execute(sql, rusqlite::params_from_iter(binds.iter())).map(|n| n as i64).map_err(|e| e.to_string());
        }
        let pg = conn.as_postgres_mut().expect("DbConn is either SQLite or Postgres");
        let binds = unsafe { kernel::db::pg_bind_values_from_raw(binds_ptr, binds_len) };
        let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = binds.iter().map(|b| b as &(dyn postgres::types::ToSql + Sync)).collect();
        let rewritten = kernel::db::rewrite_placeholders(sql);
        pg.execute(&rewritten, &refs).map(|n| n as i64).map_err(|e| e.to_string())
    });
    match result {
        Some(Ok(n)) => {
            unsafe { *out_affected = n };
            1
        }
        Some(Err(e)) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
        // RFC 0011 §2 step 2: a miss in `db_table()` falls through to
        // `HandleTable<PluginConn>` before giving up -- "a miss in
        // *both* is the same invalid-handle error every kernel boundary
        // already returns," not a new error kind.
        None => match kernel::plugin_provider::plugin_conn_table().with(handle, |conn| kernel::plugin_provider::op(conn, sql, binds_len > 0)) {
            Some(Ok(affected_str)) => match affected_str.trim().parse::<i64>() {
                Ok(n) => {
                    unsafe { *out_affected = n };
                    1
                }
                Err(_) => {
                    unsafe {
                        write_str_out(
                            out_err,
                            format!("plugin _op returned {affected_str:?} for db_execute, which is not a decimal affected-row count"),
                        )
                    };
                    0
                }
            },
            Some(Err(e)) => {
                unsafe { write_str_out(out_err, e) };
                0
            }
            None => {
                unsafe { write_str_out(out_err, "db handle is not open".to_string()) };
                0
            }
        },
    }
}

/// `db_query(conn, sql, ...binds) -> Result(json, str)` — `SELECT`
/// statements. Every row comes back as one JSON object (column name ->
/// value); the whole result set is a JSON array (`Ty::Json`'s own doc
/// comment). This compiled path represents `Ty::Json` as the raw text
/// itself (see `codegen.rs`'s `Ty::Json` `llvm_ty` arm), re-parsed by
/// each `json_get_*` accessor below rather than a persisted parsed-tree
/// handle — the simplest thing that reuses `str`'s existing
/// `{ptr, i64}` representation with zero new runtime value type,
/// matching this crate's own established "ship the real, narrower
/// slice, disclose the gap" pattern (`nir_check_role`'s own doc comment
/// is the precedent for this exact discipline).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_db_query(
    handle: i64,
    sql_ptr: *const u8,
    sql_len: i64,
    binds_ptr: *const NirBindValue,
    binds_len: i64,
    out_json: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let Some(sql) = (unsafe { str_from_raw(sql_ptr, sql_len) }) else {
        unsafe { write_str_out(out_err, "sql is not valid UTF-8".to_string()) };
        return 0;
    };
    let result: Option<Result<String, String>> = db_table().with(handle, |conn| {
        if let Some(sqlite) = conn.as_sqlite_mut() {
            let binds = unsafe { bind_values_from_raw(binds_ptr, binds_len) };
            let mut stmt = sqlite.prepare(sql).map_err(|e| e.to_string())?;
            let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
            let mut rows = stmt.query(rusqlite::params_from_iter(binds.iter())).map_err(|e| e.to_string())?;
            let mut out_rows = Vec::new();
            loop {
                let next = rows.next().map_err(|e| e.to_string())?;
                let Some(row) = next else { break };
                out_rows.push(sqlite_row_to_json(row, &column_names));
            }
            return Ok(serde_json::to_string(&serde_json::Value::Array(out_rows)).unwrap_or_else(|_| "[]".to_string()));
        }
        let pg = conn.as_postgres_mut().expect("DbConn is either SQLite or Postgres");
        let binds = unsafe { kernel::db::pg_bind_values_from_raw(binds_ptr, binds_len) };
        let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = binds.iter().map(|b| b as &(dyn postgres::types::ToSql + Sync)).collect();
        let rewritten = kernel::db::rewrite_placeholders(sql);
        let rows = pg.query(&rewritten, &refs).map_err(|e| e.to_string())?;
        let out_rows: Vec<serde_json::Value> = rows.iter().map(kernel::db::pg_row_to_json).collect();
        Ok(serde_json::to_string(&serde_json::Value::Array(out_rows)).unwrap_or_else(|_| "[]".to_string()))
    });
    match result {
        Some(Ok(json)) => {
            unsafe { write_str_out(out_json, json) };
            1
        }
        Some(Err(e)) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
        // RFC 0011 §2 step 2: same fallback priority as `nir_db_execute`
        // above -- `_op`'s returned string is passed straight through as
        // the JSON payload (this module's own doc comment: `Ty::Json` is
        // already represented as raw text, so a plugin author's `_op`
        // simply has to return well-formed JSON text by convention).
        None => match kernel::plugin_provider::plugin_conn_table().with(handle, |conn| kernel::plugin_provider::op(conn, sql, binds_len > 0)) {
            Some(Ok(json)) => {
                unsafe { write_str_out(out_json, json) };
                1
            }
            Some(Err(e)) => {
                unsafe { write_str_out(out_err, e) };
                0
            }
            None => {
                unsafe { write_str_out(out_err, "db handle is not open".to_string()) };
                0
            }
        },
    }
}

#[cfg(test)]
mod db_kernel_tests {
    use super::*;

    unsafe fn connect(path: &str) -> Result<i64, String> {
        let mut handle = 0i64;
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_db_connect(path.as_ptr(), path.len() as i64, &mut handle, &mut err) };
        if ok != 0 {
            Ok(handle)
        } else {
            Err(unsafe { std::str::from_utf8(std::slice::from_raw_parts(err.ptr, err.len as usize)).unwrap().to_string() })
        }
    }

    unsafe fn execute(handle: i64, sql: &str, binds: &[NirBindValue]) -> Result<i64, String> {
        let mut affected = 0i64;
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_db_execute(handle, sql.as_ptr(), sql.len() as i64, binds.as_ptr(), binds.len() as i64, &mut affected, &mut err) };
        if ok != 0 {
            Ok(affected)
        } else {
            Err(unsafe { std::str::from_utf8(std::slice::from_raw_parts(err.ptr, err.len as usize)).unwrap().to_string() })
        }
    }

    unsafe fn query(handle: i64, sql: &str, binds: &[NirBindValue]) -> Result<String, String> {
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_db_query(handle, sql.as_ptr(), sql.len() as i64, binds.as_ptr(), binds.len() as i64, &mut out, &mut err) };
        if ok != 0 {
            Ok(unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap().to_string() })
        } else {
            Err(unsafe { std::str::from_utf8(std::slice::from_raw_parts(err.ptr, err.len as usize)).unwrap().to_string() })
        }
    }

    fn str_bind(s: &'static str) -> NirBindValue {
        NirBindValue { tag: 2, i: 0, f: 0.0, s_ptr: s.as_ptr(), s_len: s.len() as i64 }
    }
    fn i64_bind(n: i64) -> NirBindValue {
        NirBindValue { tag: 0, i: n, f: 0.0, s_ptr: std::ptr::null(), s_len: 0 }
    }

    #[test]
    fn connect_execute_query_round_trips_real_rows_in_memory() {
        unsafe {
            let conn = connect(":memory:").expect("in-memory sqlite should always open");
            execute(conn, "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, rating INTEGER)", &[]).expect("DDL should succeed");
            let inserted = execute(conn, "INSERT INTO users (name, rating) VALUES (?, ?)", &[str_bind("ada"), i64_bind(5)]).expect("insert should succeed");
            assert_eq!(inserted, 1);
            execute(conn, "INSERT INTO users (name, rating) VALUES (?, ?)", &[str_bind("grace"), i64_bind(4)]).expect("insert should succeed");

            let rows_json = query(conn, "SELECT name, rating FROM users WHERE rating >= ? ORDER BY id", &[i64_bind(5)]).expect("query should succeed");
            let rows: serde_json::Value = serde_json::from_str(&rows_json).unwrap();
            assert_eq!(rows[0]["name"], "ada");
            assert_eq!(rows[0]["rating"], 5);
            assert_eq!(rows.as_array().unwrap().len(), 1);

            let updated = execute(conn, "UPDATE users SET rating = ? WHERE name = ?", &[i64_bind(5), str_bind("grace")]).expect("update should succeed");
            assert_eq!(updated, 1);

            assert_eq!(nir_db_stop(conn), 0);
        }
    }

    /// Proves the `db` domain's admission ceiling is real, not
    /// decorative — `nir_db_connect`/`nir_db_stop` already bracket every
    /// connect-to-stop session with `kernel::acquire`/`release`
    /// (confirmed pre-existing, not new to this phase; `kernel::db::connect`
    /// itself never calls `acquire`, see that function's own doc comment).
    /// `#[ignore]`d for the same reason `db.rs`'s own
    /// `postgres_checkout_rehydrates_a_connection_killed_out_from_under_the_pool`
    /// is: `NIRDOSHA_KERNEL_MAX_DB`'s ceiling is resolved once and cached
    /// process-wide (`kernel::mod.rs`'s `max_for`), so lowering it here
    /// would corrupt every other concurrently-running test in this binary
    /// that also opens a `db` connection — safe only run alone
    /// (`cargo test -- --ignored connect_past_the_db_ceiling...`).
    #[test]
    #[ignore]
    fn connect_past_the_db_ceiling_is_denied_fast_not_blocked() {
        unsafe { std::env::set_var("NIRDOSHA_KERNEL_MAX_DB", "2") };
        unsafe {
            let a = connect(":memory:").expect("first connect under the ceiling must succeed");
            let b = connect(":memory:").expect("second connect under the ceiling must succeed");
            let denied = connect(":memory:");
            assert!(denied.is_err(), "third connect past a ceiling of 2 must be denied, not blocked");
            assert_eq!(denied.unwrap_err(), "too many open db connections");
            nir_db_stop(a);
            nir_db_stop(b);
            let c = connect(":memory:").expect("connect after release must succeed again");
            nir_db_stop(c);
        }
    }

    #[test]
    fn connect_to_an_invalid_path_is_a_real_err_not_a_panic() {
        unsafe {
            assert!(connect("/no/such/directory/at/all/db.sqlite").is_err());
        }
    }

    #[test]
    fn bad_sql_is_a_real_err_not_a_panic() {
        unsafe {
            let conn = connect(":memory:").unwrap();
            assert!(execute(conn, "NOT VALID SQL AT ALL", &[]).is_err());
            nir_db_stop(conn);
        }
    }

    /// Two `db_connect` calls to the same non-`:memory:` path see the
    /// same underlying database — true regardless of pooling (it's the
    /// same file either way), but a real regression guard that the
    /// pooled path didn't break ordinary persistence semantics.
    /// `kernel::db::tests` (module-private, sees `PoolRegistry::pool_count`
    /// under `#[cfg(test)]`) is where pooling *itself* — same key reuses
    /// one pool, `:memory:` never gets one at all — is actually proven.
    #[test]
    fn two_connects_to_the_same_file_path_see_the_same_data() {
        let dir = std::env::temp_dir().join(format!("nirdosha_db_pool_test_{}.sqlite", std::process::id()));
        let path = dir.to_str().unwrap().to_string();
        let _ = std::fs::remove_file(&path);
        unsafe {
            let conn1 = connect(&path).expect("file-backed sqlite should open");
            execute(conn1, "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)", &[]).expect("DDL should succeed");
            execute(conn1, "INSERT INTO t (name) VALUES (?)", &[str_bind("first")]).expect("insert should succeed");
            nir_db_stop(conn1);

            let conn2 = connect(&path).expect("reconnecting to the same path should open");
            let rows_json = query(conn2, "SELECT name FROM t", &[]).expect("query should succeed");
            let rows: serde_json::Value = serde_json::from_str(&rows_json).unwrap();
            assert_eq!(rows[0]["name"], "first");
            nir_db_stop(conn2);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Real, opt-in, `#[ignore]`d Postgres coverage — same convention
    /// `NIRDOSHA_TEST_POSTGRES_URL`-gated tests use throughout this
    /// repo (`docker-compose.dev.yml` at the repo root stands up a real
    /// local server for this). Run with:
    ///   NIRDOSHA_TEST_POSTGRES_URL=postgres://nirdosha:nirdosha@localhost:5432/nirdosha_dev \
    ///     cargo test --release -- --ignored
    fn test_postgres_url() -> String {
        std::env::var("NIRDOSHA_TEST_POSTGRES_URL").unwrap_or_else(|_| "postgres://postgres@127.0.0.1:5432/postgres".to_string())
    }

    #[test]
    #[ignore]
    fn postgres_connect_execute_query_round_trips_real_rows() {
        let url = test_postgres_url();
        unsafe {
            let conn = connect(&url).expect("real postgres server must be reachable (see NIRDOSHA_TEST_POSTGRES_URL)");
            let _ = execute(conn, "DROP TABLE IF EXISTS nirdosha_pg_kernel_test", &[]);
            execute(conn, "CREATE TABLE nirdosha_pg_kernel_test (id BIGINT PRIMARY KEY, name TEXT, rating INTEGER)", &[]).expect("DDL should succeed");
            let inserted = execute(conn, "INSERT INTO nirdosha_pg_kernel_test (id, name, rating) VALUES (1, ?, ?)", &[str_bind("ada"), i64_bind(5)]).expect("insert should succeed");
            assert_eq!(inserted, 1);

            let rows_json = query(conn, "SELECT name, rating FROM nirdosha_pg_kernel_test WHERE rating >= ?", &[i64_bind(5)]).expect("query should succeed");
            let rows: serde_json::Value = serde_json::from_str(&rows_json).unwrap();
            assert_eq!(rows[0]["name"], "ada");
            assert_eq!(rows[0]["rating"], 5);

            execute(conn, "DROP TABLE nirdosha_pg_kernel_test", &[]).expect("cleanup DDL should succeed");
            nir_db_stop(conn);
        }
    }

    /// Proves pooling for real against a live server, not just SQLite's
    /// same-file coincidence above: sequential `db_connect`/`db_stop`
    /// pairs against the same Postgres connection string must reuse a
    /// pooled connection rather than opening a fresh TCP/TLS handshake
    /// every time — indirectly observable here as "many sequential
    /// connects complete quickly," the direct pool-identity assertion
    /// lives in `kernel::db::tests` where `PoolRegistry` internals are
    /// actually visible.
    #[test]
    #[ignore]
    fn postgres_sequential_connects_are_fast_meaning_pooled() {
        let url = test_postgres_url();
        let start = std::time::Instant::now();
        unsafe {
            for _ in 0..20 {
                let conn = connect(&url).expect("real postgres server must be reachable");
                query(conn, "SELECT 1", &[]).expect("query should succeed");
                nir_db_stop(conn);
            }
        }
        let elapsed = start.elapsed();
        assert!(elapsed < std::time::Duration::from_secs(5), "20 sequential connects took {elapsed:?} -- pooling should make this fast, not one fresh handshake each time");
    }
}

// ---- json kernels (`Ty::Json` as raw text, re-parsed per accessor) ------
//
// See `nir_db_query`'s own doc comment for the representation choice.
// Every accessor is fallible (`Result(_, str)`, `Ty::Json`'s own doc
// comment) — a malformed document, a missing key, or a type mismatch is
// a real `Err`, never a trap.

unsafe fn parse_json(ptr: *const u8, len: i64) -> Result<serde_json::Value, String> {
    let s = unsafe { str_from_raw(ptr, len) }.ok_or_else(|| "json text is not valid UTF-8".to_string())?;
    serde_json::from_str(s).map_err(|e| format!("malformed JSON: {e}"))
}

/// `json_parse(s) -> Result(json, str)` — validates `s` parses as JSON;
/// `codegen.rs`'s `emit_json_parse` reuses `s`'s own already-computed
/// `{ptr, i64}` value as the `Ok` payload directly (this representation
/// makes `json_parse` an identity function on success), so this kernel
/// only needs to report validity, not produce a value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_validate(json_ptr: *const u8, json_len: i64, out_err: *mut NirStrOut) -> i32 {
    match unsafe { parse_json(json_ptr, json_len) } {
        Ok(_) => 1,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
    }
}

/// `json_get(doc, key) -> Result(json, str)` — the sub-value at `key`,
/// re-serialized (this representation's `Ty::Json` is text, not a
/// persisted tree, so navigating one level means re-emitting the
/// sub-tree as its own JSON text).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_get(doc_ptr: *const u8, doc_len: i64, key_ptr: *const u8, key_len: i64, out_json: *mut NirStrOut, out_err: *mut NirStrOut) -> i32 {
    let parsed = match unsafe { parse_json(doc_ptr, doc_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            return 0;
        }
    };
    let Some(key) = (unsafe { str_from_raw(key_ptr, key_len) }) else {
        unsafe { write_str_out(out_err, "key is not valid UTF-8".to_string()) };
        return 0;
    };
    match parsed.get(key) {
        Some(v) => {
            unsafe { write_str_out(out_json, serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())) };
            1
        }
        None => {
            unsafe { write_str_out(out_err, format!("key `{key}` not found")) };
            0
        }
    }
}

/// `json_array_get(doc, idx) -> Result(json, str)` — same shape as
/// `json_get`, indexed by position instead of key.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_array_get(doc_ptr: *const u8, doc_len: i64, idx: i64, out_json: *mut NirStrOut, out_err: *mut NirStrOut) -> i32 {
    let parsed = match unsafe { parse_json(doc_ptr, doc_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            return 0;
        }
    };
    let found = if idx < 0 { None } else { parsed.as_array().and_then(|a| a.get(idx as usize)) };
    match found {
        Some(v) => {
            unsafe { write_str_out(out_json, serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())) };
            1
        }
        None => {
            unsafe { write_str_out(out_err, format!("index {idx} out of range, or not an array")) };
            0
        }
    }
}

/// `json_array_len(doc) -> Result(i64, str)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_array_len(doc_ptr: *const u8, doc_len: i64, out_len: *mut i64, out_err: *mut NirStrOut) -> i32 {
    let parsed = match unsafe { parse_json(doc_ptr, doc_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            return 0;
        }
    };
    match parsed.as_array() {
        Some(a) => {
            unsafe { *out_len = a.len() as i64 };
            1
        }
        None => {
            unsafe { write_str_out(out_err, "not a JSON array".to_string()) };
            0
        }
    }
}

/// `json_get_str(doc, key) -> Result(str, str)` — a leaf accessor: the
/// value at `key` must itself be a JSON string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_get_str(doc_ptr: *const u8, doc_len: i64, key_ptr: *const u8, key_len: i64, out_value: *mut NirStrOut, out_err: *mut NirStrOut) -> i32 {
    let parsed = match unsafe { parse_json(doc_ptr, doc_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            return 0;
        }
    };
    let Some(key) = (unsafe { str_from_raw(key_ptr, key_len) }) else {
        unsafe { write_str_out(out_err, "key is not valid UTF-8".to_string()) };
        return 0;
    };
    match parsed.get(key).and_then(|v| v.as_str()) {
        Some(s) => {
            unsafe { write_str_out(out_value, s.to_string()) };
            1
        }
        None => {
            unsafe { write_str_out(out_err, format!("key `{key}` not found, or not a string")) };
            0
        }
    }
}

/// `env(name) -> Result(str, str)` (RFC 0011 §1) — `Ok(value)` when the
/// process environment variable is set, `Err(_)` when unset or not valid
/// Unicode. Not resource-gated: no `kernel::acquire`/`Domain` involved,
/// unlike `nir_db_connect` above — reading process environment state
/// isn't a pooled/ceiling-bound resource.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_env_get(name_ptr: *const u8, name_len: i64, out_value: *mut NirStrOut, out_err: *mut NirStrOut) -> i32 {
    let Some(name) = (unsafe { str_from_raw(name_ptr, name_len) }) else {
        unsafe { write_str_out(out_err, "variable name is not valid UTF-8".to_string()) };
        return 0;
    };
    match std::env::var(name) {
        Ok(value) => {
            unsafe { write_str_out(out_value, value) };
            1
        }
        Err(e) => {
            unsafe { write_str_out(out_err, format!("{name}: {e}")) };
            0
        }
    }
}

/// `json_get_i64(doc, key) -> Result(i64, str)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_get_i64(doc_ptr: *const u8, doc_len: i64, key_ptr: *const u8, key_len: i64, out_value: *mut i64, out_err: *mut NirStrOut) -> i32 {
    let parsed = match unsafe { parse_json(doc_ptr, doc_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            return 0;
        }
    };
    let Some(key) = (unsafe { str_from_raw(key_ptr, key_len) }) else {
        unsafe { write_str_out(out_err, "key is not valid UTF-8".to_string()) };
        return 0;
    };
    match parsed.get(key).and_then(|v| v.as_i64()) {
        Some(n) => {
            unsafe { *out_value = n };
            1
        }
        None => {
            unsafe { write_str_out(out_err, format!("key `{key}` not found, or not an integer")) };
            0
        }
    }
}

/// `json_get_f64(doc, key) -> Result(f64, str)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_get_f64(doc_ptr: *const u8, doc_len: i64, key_ptr: *const u8, key_len: i64, out_value: *mut f64, out_err: *mut NirStrOut) -> i32 {
    let parsed = match unsafe { parse_json(doc_ptr, doc_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            return 0;
        }
    };
    let Some(key) = (unsafe { str_from_raw(key_ptr, key_len) }) else {
        unsafe { write_str_out(out_err, "key is not valid UTF-8".to_string()) };
        return 0;
    };
    match parsed.get(key).and_then(|v| v.as_f64()) {
        Some(f) => {
            unsafe { *out_value = f };
            1
        }
        None => {
            unsafe { write_str_out(out_err, format!("key `{key}` not found, or not a number")) };
            0
        }
    }
}

/// `json_get_bool(doc, key) -> Result(bool, str)`. `out_value` is `i32`
/// (`0`/`1`), the same boolean-as-int convention every other kernel here
/// already uses.
///
/// Accepts a real JSON boolean (`true`/`false`) *or* the JSON integers
/// `0`/`1` — not just the former. SQLite has no native boolean storage
/// class (`NirBindValue`'s own doc comment: a bound `bool` is stored as
/// SQLite `INTEGER` `0`/`1`), so `nir_db_query`'s `sqlite_row_to_json`
/// re-serializes that column back as a plain JSON *number*, never a JSON
/// boolean — there is no schema-level "this integer column is really a
/// bool" signal available to make it re-serialize any other way. Without
/// this fallback, `json_get_bool` would never succeed on any `db`-sourced
/// boolean column, which would make the two builtins genuinely unusable
/// together — found by testing a real `db_execute`/`db_query` round trip
/// of a bound `bool`, not by reasoning about the representations in
/// advance. Any other JSON number (not `0`/`1`) is still a real `Err`,
/// not silently coerced.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_get_bool(doc_ptr: *const u8, doc_len: i64, key_ptr: *const u8, key_len: i64, out_value: *mut i32, out_err: *mut NirStrOut) -> i32 {
    let parsed = match unsafe { parse_json(doc_ptr, doc_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            return 0;
        }
    };
    let Some(key) = (unsafe { str_from_raw(key_ptr, key_len) }) else {
        unsafe { write_str_out(out_err, "key is not valid UTF-8".to_string()) };
        return 0;
    };
    let found = parsed.get(key).and_then(|v| match v {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::Number(n) if n.as_i64() == Some(0) => Some(false),
        serde_json::Value::Number(n) if n.as_i64() == Some(1) => Some(true),
        _ => None,
    });
    match found {
        Some(b) => {
            unsafe { *out_value = b as i32 };
            1
        }
        None => {
            unsafe { write_str_out(out_err, format!("key `{key}` not found, or not a boolean (or 0/1 integer)")) };
            0
        }
    }
}

/// `json_set_str(doc, key, value) -> Result(json, str)` — `json_get_str`'s
/// inverse: sets `key` to a string value on a JSON object, or starts a
/// fresh object if `doc` is JSON `null` (the shape `json_parse("{}")`/
/// `json_parse("null")` both already produce). Any other JSON shape (an
/// array, a scalar) is a real `Err`, not a type error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_set_str(
    doc_ptr: *const u8,
    doc_len: i64,
    key_ptr: *const u8,
    key_len: i64,
    value_ptr: *const u8,
    value_len: i64,
    out_json: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let mut parsed = match unsafe { parse_json(doc_ptr, doc_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            return 0;
        }
    };
    let (Some(key), Some(value)) = (unsafe { str_from_raw(key_ptr, key_len) }, unsafe { str_from_raw(value_ptr, value_len) }) else {
        unsafe { write_str_out(out_err, "key/value is not valid UTF-8".to_string()) };
        return 0;
    };
    if parsed.is_null() {
        parsed = serde_json::Value::Object(serde_json::Map::new());
    }
    match parsed.as_object_mut() {
        Some(map) => {
            map.insert(key.to_string(), serde_json::Value::String(value.to_string()));
            unsafe { write_str_out(out_json, serde_json::to_string(&parsed).unwrap_or_else(|_| "null".to_string())) };
            1
        }
        None => {
            unsafe { write_str_out(out_err, "not a JSON object (and not null)".to_string()) };
            0
        }
    }
}

/// `codegen.rs`'s generic `Ty`<->JSON walk (Stage 1 of reviving compiled
/// `serve`, `rfcs/0010-landing-and-serve-exposure.md`) needs a shape
/// none of the builtins above provide: encoding one bare scalar as JSON
/// *text* on its own, not keyed inside an object (`nir_json_get_*`'s own
/// shape) and not building one (`nir_json_set_str`'s). A struct's own
/// field values -- and a `Result`'s inner payload -- are computed one at
/// a time by walking a compiled value whose `Ty` is known statically at
/// codegen time, long before there's an object to put them in; these are
/// the leaves that walk produces, combined back into a real object by
/// `nir_json_set_raw` below. Always succeed on a well-typed input (an
/// `i64`/`f64`/`bool`/`str` value out of already-typechecked compiled
/// code cannot fail to become JSON text), so -- unlike every
/// `nir_json_get_*`/`_decode_*` sibling here, which has to handle
/// attacker-controlled request bytes -- none of these four take an
/// `out_err` or return a status.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_encode_i64(value: i64, out_json: *mut NirStrOut) {
    unsafe { write_str_out(out_json, value.to_string()) };
}

/// Non-finite (`NaN`/`+-inf`) encodes as JSON `null` -- JSON has no
/// literal for either. Same choice the deleted interpreter's own
/// `encode_value` made (`Value::Float(f) => if f.is_finite() {...} else
/// { JsonVal::Null }`, recovered via `git show
/// 05a747c~1:crates/compiler/src/serve.rs` as this rewrite's own ground
/// truth for wire-compatible behavior) -- kept for exact parity with
/// what `ui_gen_template.html`'s client already expects, not re-derived.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_encode_f64(value: f64, out_json: *mut NirStrOut) {
    let text = if value.is_finite() { serde_json::Number::from_f64(value).map(|n| n.to_string()).unwrap_or_else(|| "null".to_string()) } else { "null".to_string() };
    unsafe { write_str_out(out_json, text) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_encode_bool(value: i32, out_json: *mut NirStrOut) {
    unsafe { write_str_out(out_json, if value != 0 { "true" } else { "false" }.to_string()) };
}

/// Real JSON string quoting/escaping (`serde_json`'s own `Display`),
/// not hand-rolled -- the one encoder of the four that can't just
/// `to_string()` the raw Rust value, since a `str` value can contain
/// quotes/backslashes/control characters JSON text must escape.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_encode_str(value_ptr: *const u8, value_len: i64, out_json: *mut NirStrOut) {
    let text = match unsafe { str_from_raw(value_ptr, value_len) } {
        Some(s) => serde_json::Value::String(s.to_string()).to_string(),
        None => "null".to_string(),
    };
    unsafe { write_str_out(out_json, text) };
}

/// The decode-side counterpart to the four encoders above: parses `json`
/// as a **bare** value (an array element, not a keyed object field --
/// `nir_json_get_i64`'s own shape) and requires it to be a JSON integer.
/// Used when a route argument's own declared `Ty` is a plain scalar, not
/// a struct -- `nir_json_array_get` hands codegen that argument's raw
/// JSON text with no key to look it up by, so decoding it needs this
/// bare-value shape, not `nir_json_get_i64`'s keyed one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_decode_i64(json_ptr: *const u8, json_len: i64, out_value: *mut i64, out_err: *mut NirStrOut) -> i32 {
    match unsafe { parse_json(json_ptr, json_len) } {
        Ok(v) => match v.as_i64() {
            Some(n) => {
                unsafe { *out_value = n };
                1
            }
            None => {
                unsafe { write_str_out(out_err, "expected a JSON integer".to_string()) };
                0
            }
        },
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_decode_f64(json_ptr: *const u8, json_len: i64, out_value: *mut f64, out_err: *mut NirStrOut) -> i32 {
    match unsafe { parse_json(json_ptr, json_len) } {
        Ok(v) => match v.as_f64() {
            Some(f) => {
                unsafe { *out_value = f };
                1
            }
            None => {
                unsafe { write_str_out(out_err, "expected a JSON number".to_string()) };
                0
            }
        },
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
    }
}

/// Same permissive "a real JSON boolean, or the integers `0`/`1`"
/// acceptance `nir_json_get_bool`'s own doc comment establishes and
/// justifies (a `db`-sourced boolean re-serializes as a plain integer,
/// no schema-level way to know otherwise) -- kept identical here rather
/// than a stricter decoder, so a value that already round-tripped
/// through `db`/`json_get_bool` once keeps round-tripping through this
/// bare-value path too.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_decode_bool(json_ptr: *const u8, json_len: i64, out_value: *mut i32, out_err: *mut NirStrOut) -> i32 {
    match unsafe { parse_json(json_ptr, json_len) } {
        Ok(v) => {
            let found = match v {
                serde_json::Value::Bool(b) => Some(b),
                serde_json::Value::Number(ref n) if n.as_i64() == Some(0) => Some(false),
                serde_json::Value::Number(ref n) if n.as_i64() == Some(1) => Some(true),
                _ => None,
            };
            match found {
                Some(b) => {
                    unsafe { *out_value = b as i32 };
                    1
                }
                None => {
                    unsafe { write_str_out(out_err, "expected a JSON boolean (or 0/1 integer)".to_string()) };
                    0
                }
            }
        }
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_decode_str(json_ptr: *const u8, json_len: i64, out_value: *mut NirStrOut, out_err: *mut NirStrOut) -> i32 {
    match unsafe { parse_json(json_ptr, json_len) } {
        Ok(v) => match v.as_str() {
            Some(s) => {
                unsafe { write_str_out(out_value, s.to_string()) };
                1
            }
            None => {
                unsafe { write_str_out(out_err, "expected a JSON string".to_string()) };
                0
            }
        },
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
    }
}

/// `json_set_str`'s sibling for a value that's already JSON text (a
/// nested struct's own already-encoded object, a `nir_json_encode_*`
/// scalar's output, a `Result`'s wrapped payload) rather than a bare
/// Rust string to be quoted -- the object-building primitive the struct/
/// `Result` encode walk in `codegen.rs` folds over one field at a time,
/// starting from `"{}"`. `raw` must itself already be valid JSON (it was
/// produced by one of this same walk's own encode calls, or is the
/// literal `"{}"` starting point) -- a real `Err`, not a garbage splice,
/// if it somehow isn't.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_json_set_raw(
    doc_ptr: *const u8,
    doc_len: i64,
    key_ptr: *const u8,
    key_len: i64,
    raw_ptr: *const u8,
    raw_len: i64,
    out_json: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let mut parsed = match unsafe { parse_json(doc_ptr, doc_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            return 0;
        }
    };
    let Some(key) = (unsafe { str_from_raw(key_ptr, key_len) }) else {
        unsafe { write_str_out(out_err, "key is not valid UTF-8".to_string()) };
        return 0;
    };
    let raw_value = match unsafe { parse_json(raw_ptr, raw_len) } {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, format!("value for key `{key}` is not valid JSON: {e}")) };
            return 0;
        }
    };
    if parsed.is_null() {
        parsed = serde_json::Value::Object(serde_json::Map::new());
    }
    match parsed.as_object_mut() {
        Some(map) => {
            map.insert(key.to_string(), raw_value);
            unsafe { write_str_out(out_json, serde_json::to_string(&parsed).unwrap_or_else(|_| "null".to_string())) };
            1
        }
        None => {
            unsafe { write_str_out(out_err, "not a JSON object (and not null)".to_string()) };
            0
        }
    }
}

#[cfg(test)]
mod json_kernel_tests {
    use super::*;

    fn to_str(out: &NirStrOut) -> String {
        unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap().to_string() }
    }

    #[test]
    fn get_str_reads_a_string_field() {
        let doc = r#"{"name":"ada","rating":5}"#;
        let key = "name";
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_json_get_str(doc.as_ptr(), doc.len() as i64, key.as_ptr(), key.len() as i64, &mut out, &mut err) };
        assert_eq!(ok, 1);
        assert_eq!(to_str(&out), "ada");
    }

    #[test]
    fn array_get_then_get_str_navigates_a_row() {
        let doc = r#"[{"name":"ada","rating":5}]"#;
        let mut row = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_json_array_get(doc.as_ptr(), doc.len() as i64, 0, &mut row, &mut err) };
        assert_eq!(ok, 1);
        let row_text = to_str(&row);
        let key = "name";
        let mut name = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok2 = unsafe { nir_json_get_str(row_text.as_ptr(), row_text.len() as i64, key.as_ptr(), key.len() as i64, &mut name, &mut err) };
        assert_eq!(ok2, 1);
        assert_eq!(to_str(&name), "ada");
    }

    #[test]
    fn missing_key_is_a_real_err_not_a_panic() {
        let doc = "{}";
        let key = "missing";
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_json_get_str(doc.as_ptr(), doc.len() as i64, key.as_ptr(), key.len() as i64, &mut out, &mut err) };
        assert_eq!(ok, 0);
    }

    #[test]
    fn malformed_json_is_a_real_err_not_a_panic() {
        let doc = "not json";
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        assert_eq!(unsafe { nir_json_validate(doc.as_ptr(), doc.len() as i64, &mut err) }, 0);
    }

    #[test]
    fn set_str_on_null_starts_a_fresh_object() {
        let doc = "null";
        let key = "department";
        let value = "cardiology";
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_json_set_str(doc.as_ptr(), doc.len() as i64, key.as_ptr(), key.len() as i64, value.as_ptr(), value.len() as i64, &mut out, &mut err) };
        assert_eq!(ok, 1);
        let parsed: serde_json::Value = serde_json::from_str(&to_str(&out)).unwrap();
        assert_eq!(parsed["department"], "cardiology");
    }

    fn get_bool(doc: &str, key: &str) -> Option<i32> {
        let mut out = 0i32;
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_json_get_bool(doc.as_ptr(), doc.len() as i64, key.as_ptr(), key.len() as i64, &mut out, &mut err) };
        if ok != 0 { Some(out) } else { None }
    }

    #[test]
    fn get_bool_accepts_a_real_json_boolean() {
        assert_eq!(get_bool(r#"{"active":true}"#, "active"), Some(1));
        assert_eq!(get_bool(r#"{"active":false}"#, "active"), Some(0));
    }

    #[test]
    fn get_bool_also_accepts_sqlite_shaped_0_and_1_integers() {
        // `db_query`'s own re-serialization of a bound `bool` column --
        // SQLite has no native boolean storage class, so it comes back
        // as a plain JSON number, not a JSON boolean.
        assert_eq!(get_bool(r#"{"active":1}"#, "active"), Some(1));
        assert_eq!(get_bool(r#"{"active":0}"#, "active"), Some(0));
    }

    #[test]
    fn get_bool_rejects_any_other_integer() {
        assert_eq!(get_bool(r#"{"active":2}"#, "active"), None);
    }

    #[test]
    fn encode_scalars_round_trip_through_decode() {
        let mut json = NirStrOut { ptr: std::ptr::null(), len: 0 };
        unsafe { nir_json_encode_i64(42, &mut json) };
        assert_eq!(to_str(&json), "42");
        let text = to_str(&json);
        let mut value = 0i64;
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        assert_eq!(unsafe { nir_json_decode_i64(text.as_ptr(), text.len() as i64, &mut value, &mut err) }, 1);
        assert_eq!(value, 42);

        unsafe { nir_json_encode_bool(1, &mut json) };
        assert_eq!(to_str(&json), "true");
        let mut b = 0i32;
        let text = to_str(&json);
        assert_eq!(unsafe { nir_json_decode_bool(text.as_ptr(), text.len() as i64, &mut b, &mut err) }, 1);
        assert_eq!(b, 1);

        unsafe { nir_json_encode_str(b"hi \"there\"".as_ptr(), 10, &mut json) };
        assert_eq!(to_str(&json), r#""hi \"there\"""#);
        let text = to_str(&json);
        let mut s = NirStrOut { ptr: std::ptr::null(), len: 0 };
        assert_eq!(unsafe { nir_json_decode_str(text.as_ptr(), text.len() as i64, &mut s, &mut err) }, 1);
        assert_eq!(to_str(&s), "hi \"there\"");
    }

    #[test]
    fn encode_f64_non_finite_becomes_null() {
        let mut json = NirStrOut { ptr: std::ptr::null(), len: 0 };
        unsafe { nir_json_encode_f64(f64::NAN, &mut json) };
        assert_eq!(to_str(&json), "null");
        unsafe { nir_json_encode_f64(1.5, &mut json) };
        assert_eq!(to_str(&json), "1.5");
    }

    #[test]
    fn set_raw_builds_an_object_from_pre_encoded_fields() {
        let mut doc = "{}".to_string();
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        for (key, raw) in [("id", "1"), ("active", "true"), ("nested", r#"{"x":1}"#)] {
            let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
            let ok = unsafe { nir_json_set_raw(doc.as_ptr(), doc.len() as i64, key.as_ptr(), key.len() as i64, raw.as_ptr(), raw.len() as i64, &mut out, &mut err) };
            assert_eq!(ok, 1, "set_raw(`{key}`) failed: {}", to_str(&err));
            doc = to_str(&out);
        }
        let parsed: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["active"], true);
        assert_eq!(parsed["nested"]["x"], 1);
    }

    #[test]
    fn set_raw_rejects_a_malformed_raw_value() {
        let doc = "{}";
        let key = "x";
        let raw = "not json";
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_json_set_raw(doc.as_ptr(), doc.len() as i64, key.as_ptr(), key.len() as i64, raw.as_ptr(), raw.len() as i64, &mut out, &mut err) };
        assert_eq!(ok, 0);
    }
}

// ---- transact kernels (`docs/TRANSACT.md`) --------------------------------
//
// Layer 1 (in-process control flow) plus retry/backoff, reinterpreted for
// the compiled backend's own trap model: the now-deleted interpreter could
// catch its own internal `RuntimeError` inside `transact`'s retry loop
// before it ever unwound out of the interpreter; a compiled trap calls
// `abort()` directly (`codegen.rs`'s `guard_io_ok` and every other guard),
// an unrecoverable process exit, not a catchable error. So retry here
// reacts only to a slot's own declared `Result(_, _)` return coming back
// `Err` — the same rule `docs/TRANSACT.md`'s own §1b already uses for
// `commit`/`compensate` (a trap *there* was always going to retry the same
// way a trap anywhere else in this language traps: it doesn't, it aborts).
// Durability logging and crash replay (`transact_log.rs`, deleted with the
// interpreter) are a real, separate, disclosed follow-up, not attempted
// here — `codegen::emit_transact`'s own doc comment names the gap.

/// `txn_id`'s real, compiled implementation — an always-unique (not
/// cryptographically unpredictable, but not guessable in any way that
/// matters for its actual job: deduping a replayed resend) idempotency
/// key: process id + a coarse monotonic timestamp + a process-wide atomic
/// sequence number, hex-formatted. No crash-replay mechanism exists yet to
/// actually need this to survive a restart (this update's own disclosed
/// gap) — it only has to be unique *within* one process's lifetime today,
/// same as the now-deleted interpreter's own scope note for a local
/// SQLite-file-backed log.
static TXN_ID_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_transact_gen_txn_id(out: *mut NirStrOut) {
    use std::sync::atomic::Ordering;
    let seq = TXN_ID_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let pid = std::process::id();
    let id = format!("txn-{pid:x}-{nanos:x}-{seq:x}");
    unsafe { write_str_out(out, id) };
}

/// `sleep_ms(ms)` — a real wall-clock sleep (`docs/ROADMAP.md`'s own B9,
/// "small, currently omitted... found this session"). `transact`'s own
/// `commit`/`compensate` bounded-backoff retry loop
/// (`codegen::emit_transact`) is what actually needed this to exist;
/// `ms <= 0` is a no-op, not a panic (mirrors `std::thread::sleep`'s own
/// "a zero duration returns immediately" behavior for negative input too,
/// which `Duration::from_millis` can't represent directly).
#[unsafe(no_mangle)]
pub extern "C" fn nir_sleep_ms(ms: i64) {
    if ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
    }
}

#[cfg(test)]
mod transact_kernel_tests {
    use super::*;

    #[test]
    fn gen_txn_id_produces_distinct_ids() {
        let mut a = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut b = NirStrOut { ptr: std::ptr::null(), len: 0 };
        unsafe {
            nir_transact_gen_txn_id(&mut a);
            nir_transact_gen_txn_id(&mut b);
        }
        let a_str = unsafe { std::str::from_utf8(std::slice::from_raw_parts(a.ptr, a.len as usize)).unwrap() };
        let b_str = unsafe { std::str::from_utf8(std::slice::from_raw_parts(b.ptr, b.len as usize)).unwrap() };
        assert_ne!(a_str, b_str);
        assert!(a_str.starts_with("txn-"));
    }

    #[test]
    fn sleep_ms_zero_or_negative_returns_immediately() {
        let start = std::time::Instant::now();
        nir_sleep_ms(0);
        nir_sleep_ms(-5);
        assert!(start.elapsed() < std::time::Duration::from_millis(50));
    }
}

// ---- mq kernels (`mq_connect`/`mq_publish`/`mq_consume`, Redis) ---------
//
// `Ty::Mq`'s own doc comment has the full design: layer 1 targets Redis
// specifically, `LPUSH`/`BLPOP` backing `mq_publish`/`mq_consume` (same
// choice the now-deleted interpreter made). Same `HandleTable` shape
// `db_table()` already uses — a `redis::Connection` isn't reconstructible
// from a bare integer either.

fn mq_table() -> &'static HandleTable<redis::Connection> {
    static TABLE: OnceLock<HandleTable<redis::Connection>> = OnceLock::new();
    TABLE.get_or_init(HandleTable::new)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_mq_connect(host_ptr: *const u8, host_len: i64, port: i64, out_handle: *mut i64, out_err: *mut NirStrOut) -> i32 {
    let Some(host) = (unsafe { str_from_raw(host_ptr, host_len) }) else {
        unsafe { write_str_out(out_err, "host is not valid UTF-8".to_string()) };
        return 0;
    };
    if !kernel::acquire(kernel::domain::mq()) {
        unsafe { write_str_out(out_err, "too many open mq connections".to_string()) };
        return 0;
    }
    let url = format!("redis://{host}:{port}");
    let opened = redis::Client::open(url).and_then(|client| client.get_connection());
    match opened {
        Ok(conn) => {
            let id = mq_table().insert(conn);
            unsafe { *out_handle = id };
            1
        }
        Err(e) => {
            kernel::release(kernel::domain::mq());
            unsafe { write_str_out(out_err, e.to_string()) };
            0
        }
    }
}

/// Closes `handle` — same "the checker is the real gate" convention
/// `nir_db_stop`/`nir_tcp_stop` already document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_mq_stop(handle: i64) -> i32 {
    if mq_table().remove(handle).is_some() {
        kernel::release(kernel::domain::mq());
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_mq_publish(handle: i64, queue_ptr: *const u8, queue_len: i64, msg_ptr: *const u8, msg_len: i64, out_err: *mut NirStrOut) -> i32 {
    let (Some(queue), Some(msg)) = (unsafe { str_from_raw(queue_ptr, queue_len) }, unsafe { str_from_raw(msg_ptr, msg_len) }) else {
        unsafe { write_str_out(out_err, "queue/message is not valid UTF-8".to_string()) };
        return 0;
    };
    let result = mq_table().with(handle, |conn| redis::cmd("LPUSH").arg(queue).arg(msg).query::<i64>(conn));
    match result {
        Some(Ok(_)) => 1,
        Some(Err(e)) => {
            unsafe { write_str_out(out_err, e.to_string()) };
            0
        }
        None => {
            unsafe { write_str_out(out_err, "mq handle is not open".to_string()) };
            0
        }
    }
}

/// `mq_consume(conn, queue, timeout_secs) -> Result(str, str)` — `BLPOP`,
/// blocking up to `timeout_secs` (`0` blocks forever, Redis's own
/// convention, unchanged here). A timeout with nothing published is a
/// real `Err`, not a trap — indistinguishable at this layer from any
/// other Redis error, both just a message string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_mq_consume(handle: i64, queue_ptr: *const u8, queue_len: i64, timeout_secs: i64, out_msg: *mut NirStrOut, out_err: *mut NirStrOut) -> i32 {
    let Some(queue) = (unsafe { str_from_raw(queue_ptr, queue_len) }) else {
        unsafe { write_str_out(out_err, "queue is not valid UTF-8".to_string()) };
        return 0;
    };
    let result = mq_table().with(handle, |conn| redis::cmd("BLPOP").arg(queue).arg(timeout_secs).query::<Option<(String, String)>>(conn));
    match result {
        Some(Ok(Some((_key, value)))) => {
            unsafe { write_str_out(out_msg, value) };
            1
        }
        Some(Ok(None)) => {
            unsafe { write_str_out(out_err, "timed out waiting for a message".to_string()) };
            0
        }
        Some(Err(e)) => {
            unsafe { write_str_out(out_err, e.to_string()) };
            0
        }
        None => {
            unsafe { write_str_out(out_err, "mq handle is not open".to_string()) };
            0
        }
    }
}

#[cfg(test)]
mod mq_kernel_tests {
    use super::*;

    // Real Redis, not a mock -- `examples/features/28_message_queue.nir`'s
    // own header comment already documents this file's tests degrade
    // gracefully when Redis isn't reachable; these do the same, skipping
    // (not failing) rather than asserting against an environment this
    // crate doesn't control the availability of.
    fn connect() -> Option<i64> {
        let host = "127.0.0.1";
        let mut handle = 0i64;
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe { nir_mq_connect(host.as_ptr(), host.len() as i64, 6379, &mut handle, &mut err) };
        if ok != 0 { Some(handle) } else { None }
    }

    #[test]
    fn publish_then_consume_round_trips_a_real_message() {
        let Some(conn) = connect() else {
            eprintln!("skipping: no Redis reachable at 127.0.0.1:6379");
            return;
        };
        let queue = "nirdosha_test_queue_publish_then_consume";
        let msg = "hello from mq_kernel_tests";
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let published = unsafe { nir_mq_publish(conn, queue.as_ptr(), queue.len() as i64, msg.as_ptr(), msg.len() as i64, &mut err) };
        assert_eq!(published, 1);

        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let consumed = unsafe { nir_mq_consume(conn, queue.as_ptr(), queue.len() as i64, 2, &mut out, &mut err) };
        assert_eq!(consumed, 1);
        let value = unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap() };
        assert_eq!(value, msg);

        assert_eq!(unsafe { nir_mq_stop(conn) }, 0);
    }

    #[test]
    fn consume_with_nothing_published_times_out_as_a_real_err() {
        let Some(conn) = connect() else {
            eprintln!("skipping: no Redis reachable at 127.0.0.1:6379");
            return;
        };
        let queue = "nirdosha_test_queue_never_published_to";
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let consumed = unsafe { nir_mq_consume(conn, queue.as_ptr(), queue.len() as i64, 1, &mut out, &mut err) };
        assert_eq!(consumed, 0);
        unsafe { nir_mq_stop(conn) };
    }

    #[test]
    fn connect_to_an_unreachable_host_is_a_real_err_not_a_panic() {
        let host = "127.0.0.1";
        let mut handle = 0i64;
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        // Port 1 is never a real Redis server.
        let ok = unsafe { nir_mq_connect(host.as_ptr(), host.len() as i64, 1, &mut handle, &mut err) };
        assert_eq!(ok, 0);
    }
}

// ---- http/https kernels (`http_get`/`http_post`/`https_get`/`https_post`) --
//
// Real, pooled HTTP/1.1 keep-alive connections plus real admission
// control (`domain::http()`) — `kernel::http` has the full design and the
// protocol rewrite this required (real `Content-Length`/chunked framing,
// replacing the original connection-per-call `Connection: close` +
// read-to-EOF cut, which was correct for its own scope but structurally
// incompatible with pooling: a pool only has value if a connection
// survives past one request). A network failure, a malformed status
// line, or a non-UTF-8 body are all a real `Err`, never a trap.

type HttpParsed = kernel::http::HttpParsed;

fn do_http(host: &str, port: i64, path: &str, method: &str, body: Option<&str>) -> Result<HttpParsed, String> {
    do_http_with_auth(host, port, path, method, body, None)
}

/// `do_http`'s own generalization — `bearer_token`, when present, adds
/// an `Authorization: Bearer <token>` header. `send_email`/`send_sms`/
/// `send_push`'s own generic-provider POST (`send_via_provider`) is the
/// one caller that needs this; `http_post`/`https_post` (client-facing
/// builtins) never pass one, matching their own already-locked design
/// (no auth header in that surface).
fn do_http_with_auth(host: &str, port: i64, path: &str, method: &str, body: Option<&str>, bearer_token: Option<&str>) -> Result<HttpParsed, String> {
    kernel::http::request_http(host, port, path, method, body, bearer_token)
}

fn do_https(host: &str, port: i64, path: &str, method: &str, body: Option<&str>) -> Result<HttpParsed, String> {
    kernel::http::request_https(host, port, path, method, body)
}

unsafe fn write_http_result(result: Result<HttpParsed, String>, out_status: *mut i64, out_body: *mut NirStrOut, out_err: *mut NirStrOut) -> i32 {
    match result {
        Ok(r) => {
            unsafe {
                *out_status = r.status;
                write_str_out(out_body, r.body);
            }
            1
        }
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_http_get(
    host_ptr: *const u8,
    host_len: i64,
    port: i64,
    path_ptr: *const u8,
    path_len: i64,
    out_status: *mut i64,
    out_body: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let (Some(host), Some(path)) = (unsafe { str_from_raw(host_ptr, host_len) }, unsafe { str_from_raw(path_ptr, path_len) }) else {
        unsafe { write_str_out(out_err, "host/path is not valid UTF-8".to_string()) };
        return 0;
    };
    let result = do_http(host, port, path, "GET", None);
    unsafe { write_http_result(result, out_status, out_body, out_err) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_http_post(
    host_ptr: *const u8,
    host_len: i64,
    port: i64,
    path_ptr: *const u8,
    path_len: i64,
    body_ptr: *const u8,
    body_len: i64,
    out_status: *mut i64,
    out_body: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let (Some(host), Some(path), Some(body)) =
        (unsafe { str_from_raw(host_ptr, host_len) }, unsafe { str_from_raw(path_ptr, path_len) }, unsafe { str_from_raw(body_ptr, body_len) })
    else {
        unsafe { write_str_out(out_err, "host/path/body is not valid UTF-8".to_string()) };
        return 0;
    };
    let result = do_http(host, port, path, "POST", Some(body));
    unsafe { write_http_result(result, out_status, out_body, out_err) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_https_get(
    host_ptr: *const u8,
    host_len: i64,
    port: i64,
    path_ptr: *const u8,
    path_len: i64,
    out_status: *mut i64,
    out_body: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let (Some(host), Some(path)) = (unsafe { str_from_raw(host_ptr, host_len) }, unsafe { str_from_raw(path_ptr, path_len) }) else {
        unsafe { write_str_out(out_err, "host/path is not valid UTF-8".to_string()) };
        return 0;
    };
    let result = do_https(host, port, path, "GET", None);
    unsafe { write_http_result(result, out_status, out_body, out_err) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_https_post(
    host_ptr: *const u8,
    host_len: i64,
    port: i64,
    path_ptr: *const u8,
    path_len: i64,
    body_ptr: *const u8,
    body_len: i64,
    out_status: *mut i64,
    out_body: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let (Some(host), Some(path), Some(body)) =
        (unsafe { str_from_raw(host_ptr, host_len) }, unsafe { str_from_raw(path_ptr, path_len) }, unsafe { str_from_raw(body_ptr, body_len) })
    else {
        unsafe { write_str_out(out_err, "host/path/body is not valid UTF-8".to_string()) };
        return 0;
    };
    let result = do_https(host, port, path, "POST", Some(body));
    unsafe { write_http_result(result, out_status, out_body, out_err) }
}

/// `rest` is everything after `scheme://` -- `host` or `host:port`,
/// optionally followed by a path/query this function ignores (`call_via`
/// takes `path` as its own separate argument, matching `http_get`/
/// `http_post`'s existing `(host, port, path)` split rather than a
/// single combined URL).
fn split_host_port(rest: &str, default_port: i64) -> (&str, i64) {
    let host_part = rest.split(['/', '?']).next().unwrap_or(rest);
    match host_part.rsplit_once(':') {
        Some((host, port_str)) => match port_str.parse::<i64>() {
            Ok(port) => (host, port),
            Err(_) => (host_part, default_port),
        },
        None => (host_part, default_port),
    }
}

/// `call_via(url, path, body) -> Result(HttpResponse, str)` —
/// rfcs/0011-uniform-service-provider-model.md §1/§2's `call`-shape
/// dispatch entrypoint. `url`'s scheme decides everything: `http://`/
/// `https://` are served by the exact same pooled, admission-controlled
/// core path `http_post`/`https_post` already use (this function is a
/// thin scheme-sniff in front of `do_http`/`do_https`, not a second
/// implementation of either); anything else falls through to
/// `kernel::plugin_provider::call_via`.
///
/// **Plugin-routed status code, disclosed rather than left implicit**:
/// RFC 0011 §2 pins `_request`'s ABI as a single `str` return with no
/// separate status code — a plugin has no way to hand back a numeric
/// status at all. A successful plugin-routed call therefore always
/// reports `status: 200`; a plugin wanting to signal a non-2xx-shaped
/// outcome has to encode that in its own returned body text (or fail
/// the call outright, which surfaces as `Err`, not a status code).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_call_via(
    url_ptr: *const u8,
    url_len: i64,
    path_ptr: *const u8,
    path_len: i64,
    body_ptr: *const u8,
    body_len: i64,
    out_status: *mut i64,
    out_body: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let (Some(url), Some(path), Some(body)) =
        (unsafe { str_from_raw(url_ptr, url_len) }, unsafe { str_from_raw(path_ptr, path_len) }, unsafe { str_from_raw(body_ptr, body_len) })
    else {
        unsafe { write_str_out(out_err, "url/path/body is not valid UTF-8".to_string()) };
        return 0;
    };
    if let Some(rest) = url.strip_prefix("http://") {
        let (host, port) = split_host_port(rest, 80);
        let result = do_http(host, port, path, "POST", Some(body));
        return unsafe { write_http_result(result, out_status, out_body, out_err) };
    }
    if let Some(rest) = url.strip_prefix("https://") {
        let (host, port) = split_host_port(rest, 443);
        let result = do_https(host, port, path, "POST", Some(body));
        return unsafe { write_http_result(result, out_status, out_body, out_err) };
    }
    match kernel::plugin_provider::call_via(url, path, body) {
        Ok(response_body) => {
            unsafe {
                *out_status = 200;
                write_str_out(out_body, response_body);
            }
            1
        }
        Err(e) => {
            unsafe { write_str_out(out_err, e) };
            0
        }
    }
}

#[cfg(test)]
mod http_kernel_tests {
    use super::*;

    // Response-parsing/chunked-decoding unit tests moved to
    // `kernel::http::tests` -- they test that module's own
    // `read_http_response`/`read_chunked_body` directly now, since the
    // buffer-based `parse_http_response`/`decode_chunked_body` they used
    // to test no longer exist (replaced by a real streaming reader, the
    // only way keep-alive framing can work at all -- see `kernel::http`'s
    // own module doc). What's left here is real-server, real-socket
    // round-trip coverage through the public `do_http`/`nir_http_get`
    // surface, now exercising the full pooled/keep-alive path for real.

    #[test]
    fn http_get_round_trips_against_a_real_local_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nhello from server").unwrap();
        });
        let result = do_http("127.0.0.1", port as i64, "/", "GET", None);
        server.join().unwrap();
        let parsed = result.unwrap();
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.body, "hello from server");
    }

    #[test]
    fn http_post_sends_a_real_body_with_content_length() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let received = String::from_utf8_lossy(&buf[..n]).into_owned();
            assert!(received.contains("Content-Length: 11"));
            assert!(received.ends_with("hello world"));
            stream.write_all(b"HTTP/1.1 201 Created\r\nConnection: close\r\n\r\ncreated").unwrap();
        });
        let result = do_http("127.0.0.1", port as i64, "/submit", "POST", Some("hello world"));
        server.join().unwrap();
        let parsed = result.unwrap();
        assert_eq!(parsed.status, 201);
        assert_eq!(parsed.body, "created");
    }

    #[test]
    fn http_get_connection_refused_is_a_real_err_not_a_panic() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // bound then immediately dropped -- nothing listens on it
        assert!(do_http("127.0.0.1", port as i64, "/", "GET", None).is_err());
    }
}

// ---- workflow kernels (`docs/WORKFLOW.md`, Layer 1 only) -----------------
//
// The now-deleted interpreter's own `workflow_log.rs` (1294 lines) was a
// real, file-backed-SQLite-by-default durability store, modeled on
// `transact_log.rs` — instance state, an append-only history log, magic-
// link tokens, an identity/presence directory for `notify()`. None of
// that is rebuilt here. This is deliberately the *smallest* real slice,
// the same "Layer 1: in-process, no durability" cut `transact` itself
// shipped first (`docs/TRANSACT.md`'s own layering) — a process-wide,
// in-memory instance table (`instance_id -> (workflow_name,
// current_state)`), real state transitions, real `on_entry`/`on_exit`
// action calls. Lost on process exit, same as `transact`'s own Layer 1
// durability posture — a real, disclosed, much narrower gap than the
// full design's cross-restart/multi-instance guarantee.

struct WorkflowInstance {
    workflow_name: String,
    state: String,
    /// Unix seconds this instance entered `state` — reset on every
    /// transition (`nir_workflow_set_state`), stamped fresh on create.
    /// The one piece of data `nir_workflow_list_overdue` needs to answer
    /// "how long has this instance sat here" — an SLA/escalation clock,
    /// not a general audit timestamp (no history of *previous* states'
    /// own dwell times is kept, matching this whole track's Layer-1
    /// "narrower, disclosed" scope).
    entered_at: i64,
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn workflow_instances() -> &'static std::sync::Mutex<std::collections::HashMap<i64, WorkflowInstance>> {
    static TABLE: OnceLock<std::sync::Mutex<std::collections::HashMap<i64, WorkflowInstance>>> = OnceLock::new();
    TABLE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

static WORKFLOW_INSTANCE_SEQ: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

/// Creates a new instance in `initial_state`, returns its fresh
/// `instance_id`. Only fails on malformed UTF-8 input — in practice
/// never, since `codegen::emit_workflow_start` only ever passes
/// compile-time-known `str` literals (the workflow's own name, its
/// first-declared state's name) here, never a runtime value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_workflow_create_instance(workflow_name_ptr: *const u8, workflow_name_len: i64, initial_state_ptr: *const u8, initial_state_len: i64, out_instance_id: *mut i64) -> i32 {
    let (Some(workflow_name), Some(initial_state)) =
        (unsafe { str_from_raw(workflow_name_ptr, workflow_name_len) }, unsafe { str_from_raw(initial_state_ptr, initial_state_len) })
    else {
        return 0;
    };
    let id = WORKFLOW_INSTANCE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    workflow_instances()
        .lock()
        .unwrap()
        .insert(id, WorkflowInstance { workflow_name: workflow_name.to_string(), state: initial_state.to_string(), entered_at: now_unix_secs() });
    unsafe { *out_instance_id = id };
    1
}

/// `1` (with `out_state` populated) if `instance_id` exists and belongs
/// to `workflow_name`, `0` otherwise (unknown instance, or an
/// `instance_id` that belongs to a *different* workflow — the same
/// "each workflow's own instance space" isolation a real per-workflow
/// table would give for free).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_workflow_get_state(workflow_name_ptr: *const u8, workflow_name_len: i64, instance_id: i64, out_state: *mut NirStrOut) -> i32 {
    let Some(workflow_name) = (unsafe { str_from_raw(workflow_name_ptr, workflow_name_len) }) else {
        return 0;
    };
    let table = workflow_instances().lock().unwrap();
    match table.get(&instance_id) {
        Some(inst) if inst.workflow_name == workflow_name => {
            unsafe { write_str_out(out_state, inst.state.clone()) };
            1
        }
        _ => 0,
    }
}

/// Moves `instance_id` to `new_state`, resetting its SLA clock
/// (`entered_at`) to now.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_workflow_set_state(workflow_name_ptr: *const u8, workflow_name_len: i64, instance_id: i64, new_state_ptr: *const u8, new_state_len: i64) -> i32 {
    let (Some(workflow_name), Some(new_state)) =
        (unsafe { str_from_raw(workflow_name_ptr, workflow_name_len) }, unsafe { str_from_raw(new_state_ptr, new_state_len) })
    else {
        return 0;
    };
    let mut table = workflow_instances().lock().unwrap();
    match table.get_mut(&instance_id) {
        Some(inst) if inst.workflow_name == workflow_name => {
            inst.state = new_state.to_string();
            inst.entered_at = now_unix_secs();
            1
        }
        _ => 0,
    }
}

/// `list_<workflow>_overdue()`'s real implementation
/// (`codegen::emit_workflow_overdue`) — SLA/escalation, the queryable
/// half (`docs/ROADMAP.md` A15's own proposed, more tractable design:
/// a `list_<workflow>_overdue()` read fn an external scheduler polls,
/// not an in-process timer — the same "real cross-thread callback"
/// complexity already deferred for `transact`'s `network` timeout would
/// be needed to fire escalations *automatically* with no caller
/// involved at all, not attempted here). `sla_config_json` is a
/// compile-time-built `{"StateName": seconds, ...}` object, one entry
/// per state that declared `sla_seconds` — states with no entry are
/// never SLA-tracked, matching `docs/WORKFLOW.md`'s own "sla is a real,
/// scoped proposed design, not sketched in the original doc" framing.
/// Returns a JSON array of `{"instance_id":N,"state":"...","age_seconds":N}`,
/// oldest-dwelling first.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_workflow_list_overdue(
    workflow_name_ptr: *const u8,
    workflow_name_len: i64,
    sla_config_json_ptr: *const u8,
    sla_config_json_len: i64,
    out_json: *mut NirStrOut,
    out_err: *mut NirStrOut,
) -> i32 {
    let (Some(workflow_name), Some(sla_config_json)) =
        (unsafe { str_from_raw(workflow_name_ptr, workflow_name_len) }, unsafe { str_from_raw(sla_config_json_ptr, sla_config_json_len) })
    else {
        unsafe { write_str_out(out_err, "workflow_name/sla_config is not valid UTF-8".to_string()) };
        return 0;
    };
    let sla_config: serde_json::Value = match serde_json::from_str(sla_config_json) {
        Ok(v) => v,
        Err(e) => {
            unsafe { write_str_out(out_err, format!("malformed sla config: {e}")) };
            return 0;
        }
    };
    let now = now_unix_secs();
    let table = workflow_instances().lock().unwrap();
    let mut overdue: Vec<(i64, &String, i64)> = Vec::new();
    for (id, inst) in table.iter() {
        if inst.workflow_name != workflow_name {
            continue;
        }
        if let Some(sla) = sla_config.get(&inst.state).and_then(|v| v.as_i64()) {
            let age = now - inst.entered_at;
            if age >= sla {
                overdue.push((*id, &inst.state, age));
            }
        }
    }
    overdue.sort_by(|a, b| b.2.cmp(&a.2));
    let rows: Vec<serde_json::Value> =
        overdue.into_iter().map(|(id, state, age)| serde_json::json!({"instance_id": id, "state": state, "age_seconds": age})).collect();
    unsafe { write_str_out(out_json, serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())) };
    1
}

#[cfg(test)]
mod workflow_overdue_tests {
    use super::*;

    #[test]
    fn an_instance_past_its_sla_is_reported_overdue() {
        let name = "OverdueTestWorkflow";
        let state = "Waiting";
        let mut id = 0i64;
        unsafe { nir_workflow_create_instance(name.as_ptr(), name.len() as i64, state.as_ptr(), state.len() as i64, &mut id) };
        // Backdate `entered_at` directly (real time would need a real
        // sleep) -- same "poke the table, don't wait on a real clock"
        // convention this crate's own other timing-adjacent tests avoid
        // needing at all elsewhere.
        workflow_instances().lock().unwrap().get_mut(&id).unwrap().entered_at -= 1000;

        let sla_config = r#"{"Waiting":60}"#;
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let ok = unsafe {
            nir_workflow_list_overdue(name.as_ptr(), name.len() as i64, sla_config.as_ptr(), sla_config.len() as i64, &mut out, &mut err)
        };
        assert_eq!(ok, 1);
        let json_text = unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap() };
        let parsed: serde_json::Value = serde_json::from_str(json_text).unwrap();
        assert_eq!(parsed[0]["instance_id"], id);
        assert_eq!(parsed[0]["state"], "Waiting");
    }

    #[test]
    fn an_instance_within_its_sla_is_not_overdue() {
        let name = "NotOverdueTestWorkflow";
        let state = "Waiting";
        let mut id = 0i64;
        unsafe { nir_workflow_create_instance(name.as_ptr(), name.len() as i64, state.as_ptr(), state.len() as i64, &mut id) };

        let sla_config = r#"{"Waiting":600}"#;
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        unsafe { nir_workflow_list_overdue(name.as_ptr(), name.len() as i64, sla_config.as_ptr(), sla_config.len() as i64, &mut out, &mut err) };
        let json_text = unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap() };
        assert_eq!(json_text, "[]");
    }

    #[test]
    fn a_state_with_no_sla_entry_is_never_overdue() {
        let name = "NoSlaTestWorkflow";
        let state = "Untracked";
        let mut id = 0i64;
        unsafe { nir_workflow_create_instance(name.as_ptr(), name.len() as i64, state.as_ptr(), state.len() as i64, &mut id) };
        workflow_instances().lock().unwrap().get_mut(&id).unwrap().entered_at -= 100_000;

        let sla_config = r#"{"SomeOtherState":1}"#;
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        unsafe { nir_workflow_list_overdue(name.as_ptr(), name.len() as i64, sla_config.as_ptr(), sla_config.len() as i64, &mut out, &mut err) };
        let json_text = unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap() };
        assert_eq!(json_text, "[]");
    }
}

// ---- send_email/send_sms/send_push/notify (`docs/WORKFLOW.md`) ----------
//
// A real, generic, provider-agnostic authenticated HTTPS POST — not any
// specific vendor's own exact API schema (SendGrid/Twilio/FCM), exactly
// the already-locked design. Reads the first `active = 1` row of a
// fixed-name table (`email_provider_config`/`sms_provider_config`/
// `push_provider_config`) via the caller's own `db` handle — no implicit/
// global connection, matching every `db_query`/`db_execute` convention
// already established.
//
// **`notify`'s presence bridge is not wired up in this round** — a real
// `identity_presence` table only ever gets populated by two `serve.rs`
// routes (`_presence_connect`/`_disconnect`), and `serve.rs` itself is
// gone. Rather than build an unreachable presence table nothing can ever
// populate, `notify` here always takes the documented *offline* path
// (falls back to `send_email`) — a real, disclosed narrowing of an
// already-narrow design, not a fake "sometimes online" simulation.
// `crates/presence-gateway/`'s own real Redis-`PUBLISH`-relay protocol
// (`docs/WORKFLOW.md`'s own §"notify's presence bridge") is exactly what
// a future session should wire this up to.

/// `active = 1` row's five fixed columns, in `docs/WORKFLOW.md`'s own
/// declared order (`EmailProviderConfig`'s doc comment).
struct ProviderConfig {
    host: String,
    port: i64,
    path: String,
    api_key: String,
    from_address: String,
}

fn provider_table_name(channel: &str) -> &'static str {
    match channel {
        "email" => "email_provider_config",
        "sms" => "sms_provider_config",
        "push" => "push_provider_config",
        _ => unreachable!("codegen only ever passes email/sms/push"),
    }
}

fn load_active_provider_config(conn_handle: i64, channel: &str) -> Result<ProviderConfig, String> {
    let table = provider_table_name(channel);
    let sql = format!("SELECT host, port, path, api_key, from_address FROM {table} WHERE active = 1 LIMIT 1");
    // SQLite-only for now — this predates Phase 1's Postgres support and
    // isn't in that phase's own scope to extend; a Postgres `conn_handle`
    // here is a disclosed, narrow `Err`, not a silent wrong answer.
    let row = db_table().with(conn_handle, |conn| {
        let Some(sqlite) = conn.as_sqlite_mut() else {
            return Err(rusqlite::Error::InvalidQuery);
        };
        sqlite.query_row(&sql, [], |r| {
            Ok(ProviderConfig {
                host: r.get(0)?,
                port: r.get(1)?,
                path: r.get(2)?,
                api_key: r.get(3)?,
                from_address: r.get(4)?,
            })
        })
    });
    match row {
        Some(Ok(cfg)) => Ok(cfg),
        Some(Err(_)) => Err("provider_not_configured".to_string()),
        None => Err("db handle is not open".to_string()),
    }
}

/// The actual authenticated POST — `{"to","from","template","vars"}` as
/// JSON, `Authorization: Bearer <api_key>`. A non-2xx status or a
/// connection failure is `Err(message)`; success is `Ok(())`.
fn send_via_provider(cfg: &ProviderConfig, to: &str, template: &str, vars_json: &str) -> Result<(), String> {
    let body = serde_json::json!({"to": to, "from": cfg.from_address, "template": template, "vars": serde_json::from_str::<serde_json::Value>(vars_json).unwrap_or(serde_json::Value::Null)})
        .to_string();
    let result = do_http_with_auth(&cfg.host, cfg.port, &cfg.path, "POST", Some(&body), Some(&cfg.api_key));
    match result {
        Ok(parsed) if (200..300).contains(&parsed.status) => Ok(()),
        Ok(parsed) => Err(format!("provider returned status {}", parsed.status)),
        Err(e) => Err(e),
    }
}

/// `0`=ok, `1`=`ProviderNotConfigured`, `2`=`ProviderRequestFailed(msg)` —
/// `codegen::emit_send_notification`'s own doc comment maps these back
/// onto the real `WorkflowActionError` variant tags (3 and index-matched
/// respectively) at the call site, the same "kernel reports a small
/// status code, codegen builds the real enum value" split every other
/// `Result`-returning builtin in this file already uses.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_workflow_send(
    channel_ptr: *const u8,
    channel_len: i64,
    conn_handle: i64,
    to_ptr: *const u8,
    to_len: i64,
    template_ptr: *const u8,
    template_len: i64,
    vars_json_ptr: *const u8,
    vars_json_len: i64,
    out_err_msg: *mut NirStrOut,
) -> i32 {
    let (Some(channel), Some(to), Some(template), Some(vars_json)) = (
        unsafe { str_from_raw(channel_ptr, channel_len) },
        unsafe { str_from_raw(to_ptr, to_len) },
        unsafe { str_from_raw(template_ptr, template_len) },
        unsafe { str_from_raw(vars_json_ptr, vars_json_len) },
    ) else {
        unsafe { write_str_out(out_err_msg, "argument is not valid UTF-8".to_string()) };
        return 2;
    };
    let cfg = match load_active_provider_config(conn_handle, channel) {
        Ok(cfg) => cfg,
        Err(e) if e == "provider_not_configured" => return 1,
        Err(e) => {
            unsafe { write_str_out(out_err_msg, e) };
            return 2;
        }
    };
    match send_via_provider(&cfg, to, template, vars_json) {
        Ok(()) => 0,
        Err(e) => {
            unsafe { write_str_out(out_err_msg, e) };
            2
        }
    }
}

/// `notify`'s real implementation — always takes the offline
/// (`send_email`) path; see this section's own doc comment for why the
/// presence-bridge online path isn't reachable this round. Same status-
/// code convention as `nir_workflow_send`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_notify(
    conn_handle: i64,
    _mq_handle: i64,
    to_ptr: *const u8,
    to_len: i64,
    template_ptr: *const u8,
    template_len: i64,
    vars_json_ptr: *const u8,
    vars_json_len: i64,
    out_err_msg: *mut NirStrOut,
) -> i32 {
    unsafe { nir_workflow_send(b"email".as_ptr(), 5, conn_handle, to_ptr, to_len, template_ptr, template_len, vars_json_ptr, vars_json_len, out_err_msg) }
}

#[cfg(test)]
mod workflow_send_tests {
    use super::*;

    fn open_memory_db() -> i64 {
        let path = ":memory:";
        let mut handle = 0i64;
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        assert_eq!(unsafe { nir_db_connect(path.as_ptr(), path.len() as i64, &mut handle, &mut err) }, 1);
        handle
    }

    #[test]
    fn send_email_with_no_active_provider_row_is_not_configured() {
        let conn = open_memory_db();
        let sql = "CREATE TABLE email_provider_config (id INTEGER PRIMARY KEY, active INTEGER, host TEXT, port INTEGER, path TEXT, api_key TEXT, from_address TEXT)";
        let mut affected = 0i64;
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        unsafe { nir_db_execute(conn, sql.as_ptr(), sql.len() as i64, std::ptr::null(), 0, &mut affected, &mut err) };

        let channel = "email";
        let to = "alice@example.com";
        let template = "welcome";
        let vars = "{}";
        let mut err_msg = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let status = unsafe {
            nir_workflow_send(
                channel.as_ptr(),
                channel.len() as i64,
                conn,
                to.as_ptr(),
                to.len() as i64,
                template.as_ptr(),
                template.len() as i64,
                vars.as_ptr(),
                vars.len() as i64,
                &mut err_msg,
            )
        };
        assert_eq!(status, 1);
        unsafe { nir_db_stop(conn) };
    }

    #[test]
    fn send_email_posts_to_a_real_configured_provider() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let received = String::from_utf8_lossy(&buf[..n]).into_owned();
            assert!(received.contains("Authorization: Bearer secret-key-123"));
            assert!(received.contains("alice@example.com"));
            stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nsent").unwrap();
        });

        let conn = open_memory_db();
        let create_sql = "CREATE TABLE email_provider_config (id INTEGER PRIMARY KEY, active INTEGER, host TEXT, port INTEGER, path TEXT, api_key TEXT, from_address TEXT)";
        let mut affected = 0i64;
        let mut err = NirStrOut { ptr: std::ptr::null(), len: 0 };
        unsafe { nir_db_execute(conn, create_sql.as_ptr(), create_sql.len() as i64, std::ptr::null(), 0, &mut affected, &mut err) };
        let insert_sql = "INSERT INTO email_provider_config (active, host, port, path, api_key, from_address) VALUES (1, ?, ?, ?, ?, ?)";
        let host = "127.0.0.1";
        let path = "/send";
        let api_key = "secret-key-123";
        let from_address = "noreply@example.com";
        let binds = [
            NirBindValue { tag: 2, i: 0, f: 0.0, s_ptr: host.as_ptr(), s_len: host.len() as i64 },
            NirBindValue { tag: 0, i: port as i64, f: 0.0, s_ptr: std::ptr::null(), s_len: 0 },
            NirBindValue { tag: 2, i: 0, f: 0.0, s_ptr: path.as_ptr(), s_len: path.len() as i64 },
            NirBindValue { tag: 2, i: 0, f: 0.0, s_ptr: api_key.as_ptr(), s_len: api_key.len() as i64 },
            NirBindValue { tag: 2, i: 0, f: 0.0, s_ptr: from_address.as_ptr(), s_len: from_address.len() as i64 },
        ];
        unsafe { nir_db_execute(conn, insert_sql.as_ptr(), insert_sql.len() as i64, binds.as_ptr(), binds.len() as i64, &mut affected, &mut err) };

        let channel = "email";
        let to = "alice@example.com";
        let template = "welcome";
        let vars = "{}";
        let mut err_msg = NirStrOut { ptr: std::ptr::null(), len: 0 };
        let status = unsafe {
            nir_workflow_send(
                channel.as_ptr(),
                channel.len() as i64,
                conn,
                to.as_ptr(),
                to.len() as i64,
                template.as_ptr(),
                template.len() as i64,
                vars.as_ptr(),
                vars.len() as i64,
                &mut err_msg,
            )
        };
        server.join().unwrap();
        assert_eq!(status, 0);
        unsafe { nir_db_stop(conn) };
    }
}

#[cfg(test)]
mod workflow_kernel_tests {
    use super::*;

    #[test]
    fn create_then_get_state_round_trips() {
        let name = "TestWorkflow1";
        let state = "Pending";
        let mut id = 0i64;
        assert_eq!(unsafe { nir_workflow_create_instance(name.as_ptr(), name.len() as i64, state.as_ptr(), state.len() as i64, &mut id) }, 1);

        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        assert_eq!(unsafe { nir_workflow_get_state(name.as_ptr(), name.len() as i64, id, &mut out) }, 1);
        let seen = unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap() };
        assert_eq!(seen, "Pending");
    }

    #[test]
    fn set_state_then_get_state_sees_the_update() {
        let name = "TestWorkflow2";
        let state = "Start";
        let mut id = 0i64;
        unsafe { nir_workflow_create_instance(name.as_ptr(), name.len() as i64, state.as_ptr(), state.len() as i64, &mut id) };

        let next = "Done";
        assert_eq!(unsafe { nir_workflow_set_state(name.as_ptr(), name.len() as i64, id, next.as_ptr(), next.len() as i64) }, 1);

        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        unsafe { nir_workflow_get_state(name.as_ptr(), name.len() as i64, id, &mut out) };
        let seen = unsafe { std::str::from_utf8(std::slice::from_raw_parts(out.ptr, out.len as usize)).unwrap() };
        assert_eq!(seen, "Done");
    }

    #[test]
    fn unknown_instance_id_is_not_found() {
        let name = "TestWorkflow3";
        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        assert_eq!(unsafe { nir_workflow_get_state(name.as_ptr(), name.len() as i64, 999_999_999, &mut out) }, 0);
    }

    #[test]
    fn an_instance_id_belonging_to_a_different_workflow_is_not_found() {
        let name_a = "TestWorkflowA";
        let name_b = "TestWorkflowB";
        let state = "S";
        let mut id = 0i64;
        unsafe { nir_workflow_create_instance(name_a.as_ptr(), name_a.len() as i64, state.as_ptr(), state.len() as i64, &mut id) };

        let mut out = NirStrOut { ptr: std::ptr::null(), len: 0 };
        assert_eq!(unsafe { nir_workflow_get_state(name_b.as_ptr(), name_b.len() as i64, id, &mut out) }, 0);
    }
}

// ---- dec128 kernels ---------------------------------------------------
//
// The actual point of this crate's split into a real Cargo package
// (`Cargo.toml`'s own doc comment): `rust_decimal::Decimal` is a real
// dependency here, reachable for the first time. `Ty::Dec128` stays a
// plain two-word *value* (not an aggregate — `ast::Ty::is_aggregate()`
// deliberately excludes it, since `transact_log.rs`'s slot-eligibility
// check already depends on `dec128` being a plain scalar it can
// serialize directly), so every kernel here takes/returns `Dec128Bits`
// by value — a `#[repr(C)]` two-`u64` struct, the same "return in two
// registers" ABI shape `codegen.rs` already relies on for `str`'s own
// `{ptr, i64}` value (LLVM `{i64, i64}` on the caller side; see
// `codegen.rs`'s own `Ty::Dec128` doc comment for the exact LLVM type
// string this pairs with).
//
// `Decimal::serialize()`/`deserialize()` (stable, public API, not an
// internal-layout assumption -- `Cargo.toml`'s own doc comment) is the
// actual boundary every kernel crosses: `Dec128Bits`'s two `u64`s are
// exactly that 16-byte buffer, split at the midpoint, little-endian
// (matching `serialize()`'s own byte order).
use rust_decimal::Decimal;

#[repr(C)]
pub struct Dec128Bits {
    pub lo: u64,
    pub hi: u64,
}

fn bits_to_decimal(bits: Dec128Bits) -> Decimal {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&bits.lo.to_le_bytes());
    bytes[8..16].copy_from_slice(&bits.hi.to_le_bytes());
    Decimal::deserialize(bytes)
}

fn decimal_to_bits(d: Decimal) -> Dec128Bits {
    let bytes = d.serialize();
    let lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    Dec128Bits { lo, hi }
}

/// `dec_from_i64(v, scale)` — matches `interpreter.rs`'s own
/// `dec_from_i64` arm exactly, `Decimal::new`'s own call included:
/// `Decimal::new` panics past `scale > 28` (the representation's own
/// limit — `docs/LANGUAGE.md` §5), which is exactly the right behavior
/// here too, unchanged, since this crate's own `panic = "abort"`
/// profile (`Cargo.toml`) turns that panic into a clean process abort
/// at the FFI boundary rather than an unwind — the same "checker can't
/// see this coming, so trap at runtime" treatment every other Tier-2
/// guard in this codebase already gets.
#[unsafe(no_mangle)]
pub extern "C" fn nir_dec128_from_i64(value: i64, scale: u32) -> Dec128Bits {
    decimal_to_bits(Decimal::new(value, scale))
}

/// `dec_to_str(d)` — writes `d`'s canonical `Display` string into the
/// caller-provided `out_ptr`/`out_cap` buffer, same "fixed buffer,
/// return actual length" convention `nir_tcp_recv`/`nir_file_read`
/// already use for a variable-length result. A `dec128`'s longest
/// possible representation (a sign, up to 29 decimal digits for the
/// 96-bit mantissa, one decimal point) is well under 64 bytes —
/// `codegen.rs` allocates exactly that; `-1` here (never expected in
/// practice, kept as a real, checked failure mode rather than an
/// assumed-safe `unwrap`) means the caller's buffer was too small.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_dec128_to_str(value: Dec128Bits, out_ptr: *mut u8, out_cap: i64) -> i64 {
    let d = bits_to_decimal(value);
    let s = d.to_string();
    let bytes = s.as_bytes();
    if bytes.len() as i64 > out_cap {
        return -1;
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, bytes.len()) };
    bytes.len() as i64
}

/// `dec_from_str(s)` — matches `interpreter.rs`'s `Decimal::from_str`
/// call exactly. Returns `Dec128Bits` by value plus an `i32` success
/// flag (`1` ok, `0` malformed) via `ok_ptr`, the same "packed result,
/// no `Result` type at this ABI layer" shape `nir_inv`/`nir_solve`
/// already use for their own fallible linear-algebra kernels.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nir_dec128_from_str(s_ptr: *const u8, s_len: i64, ok_ptr: *mut i32) -> Dec128Bits {
    use std::str::FromStr;
    let bytes = unsafe { std::slice::from_raw_parts(s_ptr, s_len as usize) };
    let parsed = std::str::from_utf8(bytes).ok().and_then(|s| Decimal::from_str(s).ok());
    match parsed {
        Some(d) => {
            unsafe { *ok_ptr = 1 };
            decimal_to_bits(d)
        }
        None => {
            unsafe { *ok_ptr = 0 };
            decimal_to_bits(Decimal::ZERO)
        }
    }
}

/// `a + b` — matches `interpreter.rs`'s `scalar_binop` `Dec128` arm:
/// `x + y` via `rust_decimal`'s own `Add` impl, which panics on genuine
/// overflow (the 96-bit mantissa's own limit) -- same abort-at-the-FFI-
/// boundary reasoning as `nir_dec128_from_i64`.
#[unsafe(no_mangle)]
pub extern "C" fn nir_dec128_add(a: Dec128Bits, b: Dec128Bits) -> Dec128Bits {
    decimal_to_bits(bits_to_decimal(a) + bits_to_decimal(b))
}

#[unsafe(no_mangle)]
pub extern "C" fn nir_dec128_sub(a: Dec128Bits, b: Dec128Bits) -> Dec128Bits {
    decimal_to_bits(bits_to_decimal(a) - bits_to_decimal(b))
}

#[unsafe(no_mangle)]
pub extern "C" fn nir_dec128_mul(a: Dec128Bits, b: Dec128Bits) -> Dec128Bits {
    decimal_to_bits(bits_to_decimal(a) * bits_to_decimal(b))
}

/// `a <=> b` — a real total ordering (`Decimal: Ord`, no NaN-like case
/// to worry about, unlike `f64`), matching `interpreter.rs`'s own
/// `Eq`/`NotEq`/`Lt`/`Gt`/`LtEq`/`GtEq` arm exactly: every one of those
/// six operators is just this result compared against `0`.
#[unsafe(no_mangle)]
pub extern "C" fn nir_dec128_cmp(a: Dec128Bits, b: Dec128Bits) -> i32 {
    match bits_to_decimal(a).cmp(&bits_to_decimal(b)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// `dec_round(d, scale)` — matches `interpreter.rs`'s own `dec_round`
/// exactly: `round_dp_with_strategy`, `MidpointNearestEven` (banker's
/// rounding, `docs/LANGUAGE.md` §5's "the only rounding policy v1
/// ships," not `rust_decimal`'s own away-from-zero default).
#[unsafe(no_mangle)]
pub extern "C" fn nir_dec128_round(value: Dec128Bits, scale: u32) -> Dec128Bits {
    decimal_to_bits(bits_to_decimal(value).round_dp_with_strategy(scale, rust_decimal::RoundingStrategy::MidpointNearestEven))
}

/// `dec_scale(d)` — matches `interpreter.rs`'s own `dec_scale` exactly:
/// `Decimal::scale()`.
#[unsafe(no_mangle)]
pub extern "C" fn nir_dec128_scale(value: Dec128Bits) -> i64 {
    bits_to_decimal(value).scale() as i64
}

/// `a / b` — matches `interpreter.rs`'s own `Div`/`ElemDiv` arm: a zero
/// divisor is `ErrorKind::DivByZero` there (a real, catchable Nirdosha
/// `RuntimeError`, not a Rust panic) — the compiled path has no
/// equivalent catchable-error channel (same category as integer
/// division's own Tier-2 div-by-zero guard, which `codegen.rs` compiles
/// to an unconditional `abort()`, never a language-visible `Result`),
/// so this traps too, via a genuine Rust panic (`panic = "abort"` turns
/// it into exactly that `abort()`), for the same reason.
#[unsafe(no_mangle)]
pub extern "C" fn nir_dec128_div(a: Dec128Bits, b: Dec128Bits) -> Dec128Bits {
    let (a, b) = (bits_to_decimal(a), bits_to_decimal(b));
    if b.is_zero() {
        std::process::abort();
    }
    decimal_to_bits(a / b)
}
