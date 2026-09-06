//! `kv_open`/`kv_set`/`kv_get`/`kv_close` — the reference example for
//! rfcs/0008-native-plugin-abi-widening.md Phase 1's `handle(Kind)`
//! crossing, combined with `str` crossing (`plugin-example-native-shout`
//! covers `str` alone). This is the *shape* every real I/O plugin the
//! deleted interpreter-era gallery had (`mysql_connect`/`cassandra_
//! connect`/`activemq_connect`, all `connect -> handle`, `... -> ...`,
//! `close(handle)`) actually needs — an in-memory store instead of a
//! real network service only so this example needs no Docker container
//! to try, not because the pattern is different for a real one.
//!
//! ## The one thing a plugin author has to hand-roll here
//!
//! The deleted `nirdosha-plugin-support::HandleRegistry<T>` (interpreter
//! path) gave every stateful plugin a shared, generic `Mutex<HashMap<
//! u64, T>>` for free. No compiled-path equivalent exists yet — a real
//! follow-up (rfcs/0008 Phase 2's authoring-convention crate would be
//! the natural home for one). Until then, the ~15 lines below
//! (`STORES`, `next_handle`) are what a plugin author writes by hand;
//! `ownership.rs`'s affine tracking (`Ty::Handle`'s own safety story,
//! rfcs/0005 §1) is what actually prevents a `.nir` program from
//! double-closing or leaking one of these, entirely on the *compiler*
//! side — this crate does not, and does not need to, defend against
//! that itself.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

fn stores() -> &'static Mutex<HashMap<i64, HashMap<String, String>>> {
    static STORES: OnceLock<Mutex<HashMap<i64, HashMap<String, String>>>> = OnceLock::new();
    STORES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> i64 {
    static NEXT: AtomicI64 = AtomicI64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Must match `plugin-example-native-shout::NirStr` field-for-field —
/// each plugin crate declares its own copy rather than sharing one
/// (rfcs/0008 Phase 2's future authoring-convention crate is the real
/// fix for that duplication; see this crate's own module doc).
#[repr(C)]
pub struct NirStr {
    pub ptr: *const u8,
    pub len: i64,
}

unsafe fn nir_str_to_string(s: &NirStr) -> String {
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn string_to_nir_str(s: String) -> NirStr {
    let leaked: &'static mut [u8] = Box::leak(s.into_bytes().into_boxed_slice());
    NirStr { ptr: leaked.as_ptr(), len: leaked.len() as i64 }
}

/// Mints a fresh, empty store and returns its `handle(KvStore)` id.
/// `ownership.rs` requires every `.nir` binding of this type to be
/// consumed exactly once (moved into exactly one later call) — this
/// function's own Rust code has no matching enforcement of its own, by
/// design; that's the whole point of `Ty::Handle` doing this at the
/// type level instead of every plugin reimplementing it at runtime.
#[unsafe(no_mangle)]
pub extern "C" fn kv_open() -> i64 {
    let h = next_handle();
    stores().lock().unwrap().insert(h, HashMap::new());
    h
}

/// Takes `&handle(KvStore)`, not a bare `handle(KvStore)` — a *read*
/// (well, a write-through-a-reference, but not a resource-closing
/// operation either) must **borrow** the handle, not consume it, or
/// `ownership.rs`'s affine tracking would only ever allow one `kv_set`/
/// `kv_get` before `kv_close` per store (rfcs/0005 §1's own guarantee).
/// A `Ty::Ref(_)` crosses as a plain one-word `ptr` — the address of the
/// caller's `i64` handle slot — so this dereferences it once to read
/// the id back out; `codegen.rs` never inspects a `Ty::Handle` value
/// itself, only ever passes the word through, so a plain, unsynchronized
/// read here is exactly as safe as reading any other `&i64` would be.
///
/// Returns `1` on success, `0` if `handle` names no open store (e.g.
/// already closed) — a plain sentinel return, not a panic, matching
/// this narrow ABI's "no `Result`/aggregate crossing yet" limit
/// (rfcs/0008 Phase 1's own disclosed scope).
#[unsafe(no_mangle)]
pub extern "C" fn kv_set(handle: *const i64, key: NirStr, val: NirStr) -> i64 {
    let handle = unsafe { *handle };
    let key = unsafe { nir_str_to_string(&key) };
    let val = unsafe { nir_str_to_string(&val) };
    match stores().lock().unwrap().get_mut(&handle) {
        Some(store) => {
            store.insert(key, val);
            1
        }
        None => 0,
    }
}

/// Same `&handle(KvStore)`-borrows story as `kv_set` above.
///
/// Returns the stored value, or an empty `str` if `key`/`handle` isn't
/// found — not a sentinel error code, since the return type here is
/// `str`, not `i64`; a real plugin with a richer error story is exactly
/// the "harder, still-open question" rfcs/0005 §3 and rfcs/0008's
/// Rejected Alternatives section both name (no `Result`/`Option`
/// crossing yet).
#[unsafe(no_mangle)]
pub extern "C" fn kv_get(handle: *const i64, key: NirStr) -> NirStr {
    let handle = unsafe { *handle };
    let key = unsafe { nir_str_to_string(&key) };
    let found = stores().lock().unwrap().get(&handle).and_then(|store| store.get(&key).cloned());
    string_to_nir_str(found.unwrap_or_default())
}

/// Returns `1` on success, `0` if `handle` was already closed (or never
/// opened) — the double-close case `ownership.rs` already rejects at
/// **compile time** for any well-typed `.nir` program (see
/// `crates/compiler/tests/native_plugin_codegen.rs`'s
/// `a_native_plugin_handle_used_twice_is_a_compile_time_ownership_error`),
/// so this runtime check only ever fires for a handle value that
/// reached this function some other way (a future FFI caller not going
/// through Nirdosha's own ownership checker) — defense in depth, not
/// something a real `.nir` program can trigger.
#[unsafe(no_mangle)]
pub extern "C" fn kv_close(handle: i64) -> i64 {
    match stores().lock().unwrap().remove(&handle) {
        Some(_) => 1,
        None => 0,
    }
}
