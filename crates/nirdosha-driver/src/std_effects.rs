//! A curated effect-classification table for std/core/alloc call
//! targets (issue #71). Without it, Stage 2's purity checker rejects
//! *every* external call — including `Vec::push`, `.iter().map(..)`,
//! and the dialect's own injected NFR guard — because nothing is
//! trusted by name (`docs/V2_GUARANTEES.md`'s documented gap).
//! `Effects::visit_callee`/`Effects::foreign_reason` (`main.rs`)
//! consult this before falling back to blanket rejection.
//!
//! Entries are keyed by the *normalized* path (see [`normalize`]):
//! `rustc`'s own `def_path_str` embeds a generic type's resolved
//! arguments as a literal `<T, A>`-shaped path segment for its own
//! inherent methods — confirmed empirically, not guessed, by
//! compiling real corpus-shaped fixtures through this driver with a
//! debug print on the resolved path and reading the actual output:
//! `v.push(x)` on a `Vec<i64>` resolves to `std::vec::Vec::<T,
//! A>::push`, not `std::vec::Vec::push`. Stripping that segment means
//! one table entry covers every instantiation, the same "resolved
//! DefId, not string-matched source" principle `FOREIGN_NEEDLES`
//! already uses in `main.rs`, applied to a table that grants trust
//! instead of denying it.
//!
//! Grown incrementally, per this issue's own stated process: a new
//! entry is a one-line data addition plus a test
//! (`nirdosha-driver/tests/std_effects.rs`), not a design change.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Trusted unconditionally — a data operation (push, len, a map
    /// lookup, an autoderef coercion, …) with no closure or fn-item
    /// argument whose own body could hide an effect.
    Pure,
    /// Trusted for the call itself, but the caller must still
    /// recursively check any closure/fn-item argument's own body.
    /// This is exactly what keeps a real side effect hidden inside
    /// `.map(|x| { println!(...); x })` rejected even once
    /// `Iterator::map` itself is trusted — see
    /// `tests/effects.rs::std_callback_is_not_trusted_by_crate_name`.
    HigherOrderPure,
}

/// Strip a `::<...>::` generic-argument segment `def_path_str` embeds
/// for a generic type's own inherent/trait methods (`Vec::<T,
/// A>::push` → `Vec::push`, `Option::<&T>::copied` → `Option::copied`).
/// A real Rust path segment can never start with `<`, so this can't
/// misfire on an ordinarily-named item.
pub fn normalize(path: &str) -> String {
    path.split("::")
        .filter(|seg| !seg.starts_with('<'))
        .collect::<Vec<_>>()
        .join("::")
}

pub fn classify(path: &str) -> Option<Effect> {
    let normalized = normalize(path);
    if PURE.contains(&normalized.as_str()) {
        return Some(Effect::Pure);
    }
    if HIGHER_ORDER_PURE.contains(&normalized.as_str()) {
        return Some(Effect::HigherOrderPure);
    }
    None
}

// Deliberately NOT trusted, even though real corpus-shaped code hits
// them constantly: `std::ops::{Deref,DerefMut,Mul,Add,Sub,...}`,
// `std::clone::Clone::clone`, `std::cmp::{Ord,PartialOrd,PartialEq}`.
// `def_path_str` resolves a trait method call to the *trait's own*
// declared path, not the concrete impl's — confirmed empirically with
// a throwaway fixture: a hand-written `impl Mul<i64> for Loud { fn
// mul(..) { println!(..); .. } }` resolves to the exact same
// `std::ops::Mul::mul` string a primitive `i64 * i64` would (if it
// even needed a call at all — see below). Unlike an *inherent* method
// on a concrete std type (`Vec::push` can only ever mean one thing),
// these are traits every domain type routinely re-implements with
// type-specific — and potentially side-effecting — logic, so trusting
// the path name would silently accept a lying `effects(pure)` claim
// whenever the actual operand is a user type, not a std one. A sound
// fix needs the call's *resolved Self type* checked against Rust's own
// orphan-rule guarantee (a foreign trait + foreign Self type can only
// ever be implemented inside std/core/alloc, never downstream) before
// trusting it — real, and worth doing, but its own follow-on rather
// than bundled in here as a shortcut. Note plain `i64 * i64` (value
// operands, not references) doesn't hit this at all: MIR lowers it to
// a `BinOp::Mul` rvalue directly, no call and nothing to classify —
// only a *reference* operand (`&i64 * i64`) goes through the trait.
const PURE: &[&str] = &[
    "std::cmp::min",
    "std::cmp::max",
    "std::mem::swap",
    "std::mem::replace",
    "std::mem::take",
    "std::mem::drop",
    // alloc::vec::Vec
    "std::vec::Vec::new",
    "std::vec::Vec::with_capacity",
    "std::vec::Vec::push",
    "std::vec::Vec::pop",
    "std::vec::Vec::len",
    "std::vec::Vec::is_empty",
    "std::vec::Vec::get",
    "std::vec::Vec::get_mut",
    "std::vec::Vec::first",
    "std::vec::Vec::last",
    "std::vec::Vec::clear",
    "std::vec::Vec::contains",
    "std::vec::Vec::insert",
    "std::vec::Vec::remove",
    "std::vec::Vec::truncate",
    "std::vec::Vec::capacity",
    "std::vec::Vec::reverse",
    "std::vec::Vec::extend_from_slice",
    "std::vec::Vec::sort",
    "std::vec::Vec::dedup",
    // core::slice / std::slice/alloc::slice -- the same `[T]` inherent
    // methods print under different crate prefixes depending on
    // whether the method needs an allocator (confirmed empirically:
    // `.iter()` printed as `core::slice::..`, `.sort_by(..)` as
    // `std::slice::..`) -- both listed rather than guessing which.
    "core::slice::iter",
    "core::slice::first",
    "core::slice::last",
    "core::slice::len",
    "core::slice::is_empty",
    "core::slice::get",
    "core::slice::contains",
    "core::slice::to_vec",
    "core::slice::concat",
    "core::slice::sort",
    "std::slice::iter",
    "std::slice::first",
    "std::slice::last",
    "std::slice::len",
    "std::slice::is_empty",
    "std::slice::get",
    "std::slice::contains",
    "std::slice::to_vec",
    "std::slice::concat",
    "std::slice::sort",
    // core::str / std::str -- same dual-prefix reasoning as slice above.
    "core::str::len",
    "core::str::is_empty",
    "core::str::trim",
    "core::str::contains",
    "core::str::starts_with",
    "core::str::ends_with",
    "core::str::chars",
    "core::str::as_bytes",
    "core::str::split",
    "std::str::len",
    "std::str::is_empty",
    "std::str::trim",
    "std::str::contains",
    "std::str::starts_with",
    "std::str::ends_with",
    "std::str::chars",
    "std::str::as_bytes",
    "std::str::split",
    "std::str::to_uppercase",
    "std::str::to_lowercase",
    "std::str::replace",
    // `str::parse::<T>()`/`ToString::to_string` deliberately excluded:
    // both are generic over a caller-chosen type (`T: FromStr`, `T:
    // Display`) whose own impl is exactly the same "routinely
    // downstream-provided, resolves to the same path regardless"
    // hazard as the trait methods excluded above.
    // alloc::string::String
    "std::string::String::new",
    "std::string::String::len",
    "std::string::String::is_empty",
    "std::string::String::push",
    "std::string::String::push_str",
    "std::string::String::as_str",
    "std::string::String::clear",
    // core::option::Option -- data-only variants (no closure argument)
    "std::option::Option::is_some",
    "std::option::Option::is_none",
    "std::option::Option::unwrap",
    "std::option::Option::unwrap_or",
    "std::option::Option::unwrap_or_default",
    "std::option::Option::as_ref",
    "std::option::Option::as_mut",
    "std::option::Option::take",
    "std::option::Option::replace",
    "std::option::Option::copied",
    "std::option::Option::cloned",
    // core::result::Result -- data-only variants
    "std::result::Result::is_ok",
    "std::result::Result::is_err",
    "std::result::Result::unwrap",
    "std::result::Result::unwrap_or",
    "std::result::Result::unwrap_or_default",
    "std::result::Result::ok",
    "std::result::Result::err",
    "std::result::Result::as_ref",
    // core::iter::Iterator -- terminal/adaptor methods that take no
    // closure/fn-item argument, so nothing to hide an effect in.
    "std::iter::Iterator::next",
    "std::iter::Iterator::count",
    "std::iter::Iterator::sum",
    "std::iter::Iterator::product",
    "std::iter::Iterator::min",
    "std::iter::Iterator::max",
    "std::iter::Iterator::collect",
    "std::iter::Iterator::rev",
    "std::iter::Iterator::enumerate",
    "std::iter::Iterator::zip",
    "std::iter::Iterator::chain",
    "std::iter::Iterator::take",
    "std::iter::Iterator::skip",
    "std::iter::Iterator::cloned",
    "std::iter::Iterator::copied",
    "std::iter::Iterator::peekable",
    "std::iter::Iterator::flatten",
    // std::collections::HashMap / BTreeMap -- data-only variants
    "std::collections::HashMap::new",
    "std::collections::HashMap::insert",
    "std::collections::HashMap::get",
    "std::collections::HashMap::get_mut",
    "std::collections::HashMap::contains_key",
    "std::collections::HashMap::remove",
    "std::collections::HashMap::len",
    "std::collections::HashMap::is_empty",
    "std::collections::BTreeMap::new",
    "std::collections::BTreeMap::insert",
    "std::collections::BTreeMap::get",
    "std::collections::BTreeMap::contains_key",
    "std::collections::BTreeMap::remove",
    "std::collections::BTreeMap::len",
    // The dialect's own injected instrumentation (issue #71's other
    // headline example): `nfr(..)`'s Drop-based guard records timing
    // (`nirdosha-rt/src/nfr.rs`) but doesn't feed back into or alter
    // the wrapped body's own result -- a sanctioned, first-class
    // contract clause, not smuggled impurity. Without this, no fn
    // could ever claim `effects(pure)` and `nfr(..)` together.
    //
    // `def_path_str` resolves a foreign call to the *shortest visible*
    // path, not necessarily the item's defining module: `nirdosha-rt`
    // re-exports `nfr::enter` at its crate root (`pub use nfr::enter`,
    // `nirdosha-rt/src/lib.rs`), and a call site through that root path
    // resolves the same `DefId` to `nirdosha_rt::enter`, not `nirdosha_rt
    // ::nfr::enter` — confirmed empirically compiling `rt-payroll`
    // through this driver. Both spellings are listed, same dual-path
    // reasoning `core::slice`/`std::slice` above already uses for a
    // path that prints differently depending on context.
    "nirdosha_rt::nfr::enter",
    "nirdosha_rt::enter",
];

/// Foreign types (issue #78) whose *own* explicit `impl Drop` is vetted
/// safe to admit under a pure claim — keyed by the ADT's own path
/// (`AdtDef::destructor(tcx)`'s returned `DefId` belongs to the impl
/// method, e.g. `<std::vec::Vec<T, A> as std::ops::Drop>::drop`, which
/// isn't a stable thing to string-match; the type's own path is what a
/// maintainer actually recognizes and vouches for). A local type's
/// explicit `Drop::drop` gets real recursive analysis instead
/// (`Effects::effects_of` on its `DefId`, the same treatment a direct
/// call already gets) — this table is only consulted for *foreign*
/// destructors.
///
/// Confirmed empirically the same way `PURE`/`HIGHER_ORDER_PURE` were:
/// std's owned containers (`Vec`, `String`, `HashMap`, `BTreeMap`) all
/// carry an *explicit* `impl Drop` (buffer deallocation) — "no custom
/// `Drop` impl" is not actually why they were rejected; an unconditional
/// "any `Drop` terminator is an impurity" check was.
const PURE_DESTRUCTOR_TYPES: &[&str] = &[
    "std::vec::Vec",
    "std::string::String",
    "std::collections::HashMap",
    "std::collections::BTreeMap",
    // `nirdosha_rt::nfr::Guard`'s `Drop` records timing (mutex + clock
    // read) -- real effects, but a sanctioned, first-class part of the
    // dialect's own `nfr(..)` instrumentation (see the `nirdosha_rt::
    // enter` `PURE` entry above), not a smuggled impurity.
    "nirdosha_rt::Guard",
];

pub fn classify_destructor(type_path: &str) -> bool {
    PURE_DESTRUCTOR_TYPES.contains(&normalize(type_path).as_str())
}

const HIGHER_ORDER_PURE: &[&str] = &[
    "std::iter::Iterator::map",
    "std::iter::Iterator::filter",
    "std::iter::Iterator::filter_map",
    "std::iter::Iterator::for_each",
    "std::iter::Iterator::find",
    "std::iter::Iterator::find_map",
    "std::iter::Iterator::all",
    "std::iter::Iterator::any",
    "std::iter::Iterator::fold",
    "std::iter::Iterator::flat_map",
    "std::iter::Iterator::inspect",
    "std::iter::Iterator::min_by",
    "std::iter::Iterator::min_by_key",
    "std::iter::Iterator::max_by",
    "std::iter::Iterator::max_by_key",
    "std::option::Option::map",
    "std::option::Option::and_then",
    "std::option::Option::unwrap_or_else",
    "std::option::Option::filter",
    "std::option::Option::map_or",
    "std::option::Option::map_or_else",
    "std::option::Option::get_or_insert_with",
    "std::result::Result::map",
    "std::result::Result::map_err",
    "std::result::Result::and_then",
    "std::result::Result::unwrap_or_else",
    "std::vec::Vec::retain",
    "std::vec::Vec::sort_by",
    "std::vec::Vec::sort_by_key",
    "std::vec::Vec::dedup_by",
    "std::vec::Vec::dedup_by_key",
    "core::slice::sort_by",
    "core::slice::sort_by_key",
    "std::slice::sort_by",
    "std::slice::sort_by_key",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_generic_argument_segment() {
        assert_eq!(normalize("std::vec::Vec::<T, A>::push"), "std::vec::Vec::push");
        assert_eq!(normalize("std::option::Option::<&T>::copied"), "std::option::Option::copied");
        assert_eq!(normalize("std::clone::Clone::clone"), "std::clone::Clone::clone");
    }

    #[test]
    fn classifies_pure_and_higher_order_and_unknown_paths() {
        assert_eq!(classify("std::vec::Vec::<T, A>::push"), Some(Effect::Pure));
        assert_eq!(classify("std::iter::Iterator::map"), Some(Effect::HigherOrderPure));
        assert_eq!(classify("std::fs::read_to_string"), None);
    }

    /// The dialect's own injected NFR guard (`nirdosha-macros/src/
    /// lib.rs`'s `nfr(..)` expansion) must not block an `effects(pure)`
    /// claim on the same fn — see `main.rs`'s own doc-referenced gap in
    /// `docs/V2_GUARANTEES.md`.
    #[test]
    fn classifies_nirdosha_rts_own_injected_nfr_guard() {
        assert_eq!(classify("nirdosha_rt::nfr::enter"), Some(Effect::Pure));
        // The crate-root re-export path `def_path_str` actually resolves
        // to at a real call site (issue #74's rt-payroll regression).
        assert_eq!(classify("nirdosha_rt::enter"), Some(Effect::Pure));
    }

    #[test]
    fn classifies_trusted_foreign_destructors() {
        assert!(classify_destructor("std::vec::Vec"));
        assert!(classify_destructor("std::string::String"));
        assert!(classify_destructor("nirdosha_rt::Guard"));
        assert!(!classify_destructor("std::sync::MutexGuard"));
    }
}
