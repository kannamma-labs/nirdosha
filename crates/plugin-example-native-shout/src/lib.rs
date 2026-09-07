//! `plugin_shout(s: str) -> str` — the reference example for
//! rfcs/0008-native-plugin-abi-widening.md Phase 1's `str` crossing: a
//! plugin author's own `#[repr(C)]` two-word struct, `(ptr, len)`,
//! passed and returned *by value*, matching the exact LLVM shape
//! `codegen.rs::llvm_ty` already emits for every `.nir` `str` value
//! (`Ty::Str => "{ptr, i64}"`) — the same convention
//! `runtime-kernels/src/lib.rs`'s own `Dec128Bits` already establishes
//! for a different two-word value. Nothing here is Nirdosha-specific
//! beyond that struct's shape: this crate has zero dependency on the
//! `nirdosha` crate itself.
//!
//! See this crate's `README.md` for how to build it and wire it into a
//! real `nirdosha build`, and `crates/compiler/tests/
//! native_plugin_examples.rs` for the automated, run-the-real-binary
//! proof that this exact source does what it claims.

/// Must match `Ty::Str`'s own `{ptr, i64}` field order and widths
/// exactly — this is the ABI, not an internal convenience type. Not
/// NUL-terminated: `len` is the only thing saying how many bytes are
/// valid, the same as every other `str` value in this compiler.
#[repr(C)]
pub struct NirStr {
    pub ptr: *const u8,
    pub len: i64,
}

/// Upper-cases the input and appends `!`. Allocates its own buffer and
/// leaks it (`Box::leak`) — nothing on the Nirdosha side ever frees a
/// plugin-returned string, the same permanent-leak posture
/// `codegen.rs`'s own `sha256_hex` builtin already has for the
/// identical reason: `Ty::Str` isn't affine, so there is no
/// scope-closing point to hook a free onto. A plugin returning a *lot*
/// of short-lived strings in a loop should keep this in mind — it's a
/// real, disclosed limit of the current `str`-return convention, not an
/// oversight in this one example.
#[unsafe(no_mangle)]
pub extern "C" fn plugin_shout(s: NirStr) -> NirStr {
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len as usize) };
    let mut v: Vec<u8> = bytes.iter().map(|b| b.to_ascii_uppercase()).collect();
    v.push(b'!');
    let leaked: &'static mut [u8] = Box::leak(v.into_boxed_slice());
    NirStr { ptr: leaked.as_ptr(), len: leaked.len() as i64 }
}
