//! Stage-1 scanners: impure-call detection for `effects(pure)` claims,
//! and the dialect's global restrictions (`unsafe`, raw threads, raw
//! locks -- issue #77).
//!
//! These are deliberately *best-effort, path-based* analyses that run
//! without a type-checked IR — they fire inside the attribute macro (so
//! obvious lies are `compile_error!` even under plain cargo) and again,
//! file-wide, inside `cargo nirdosha` (so hand-written doc contracts are
//! checked too). Stage 2 (the rustc driver) replaces the guessing with
//! real name resolution over HIR/MIR; until then, the honest contract is
//! "over-approximate, document, iterate."

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::visit::Visit;

#[derive(Debug, Clone)]
pub struct ImpureCall {
    pub what: String,
    pub why: &'static str,
    pub line: u32,
    pub col: u32,
}

/// Macros that cannot appear in a `pure` fn body.
const MACRO_DENIES: &[(&str, &str)] = &[
    ("println", "terminal I/O"),
    ("eprintln", "terminal I/O"),
    ("print", "terminal I/O"),
    ("eprint", "terminal I/O"),
    ("panic", "diverging panic"),
    ("todo", "diverging panic"),
    ("unimplemented", "diverging panic"),
    ("unreachable", "diverging panic"),
    ("dbg", "terminal I/O"),
];

/// Method calls that cannot appear in a `pure` fn body.
const METHOD_DENIES: &[(&str, &str)] = &[
    ("unwrap", "can panic (unwrap)"),
    ("expect", "can panic (expect)"),
    ("spawn", "raw thread/process spawn"),
    ("sleep", "clock-dependent"),
    ("now", "clock read"),
    ("connect", "network access"),
    ("read_to_string", "file I/O"),
    ("read_dir", "file system access"),
    ("remove_file", "file system access"),
    ("write_all", "file I/O"),
];

/// Path needles — matched against `::a::b::c::` normalized paths, so they
/// catch both fully-qualified (`std::fs::read_to_string`) and imported
/// (`fs::read_to_string`) spellings. Chosen to avoid the common false
/// positives (e.g. `Duration::from_secs` is *not* impure, so bare
/// `::time::` is not on the list).
const PATH_NEEDLES: &[(&str, &str)] = &[
    ("::fs::", "file system access"),
    ("::net::", "network access"),
    ("::process::", "process spawn"),
    ("::thread::", "raw threads (not nirdosha-rt managed)"),
    ("::env::", "environment access"),
    ("::io::", "I/O"),
    ("::libc::", "libc FFI"),
    ("::rand::", "OS randomness"),
    ("::SystemTime::", "wall-clock access"),
    ("::Instant::", "clock access"),
];

/// Bare function names (called unqualified after a `use`) that cannot
/// appear in a `pure` fn body.
const BARE_FN_DENIES: &[(&str, &str)] = &[
    ("read_to_string", "file I/O"),
    ("read_dir", "file system access"),
    ("remove_file", "file system access"),
];

fn path_spine(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn hit(what: String, why: &'static str, span: Span) -> ImpureCall {
    let start = span.start();
    ImpureCall {
        what,
        why,
        line: start.line as u32,
        col: start.column as u32,
    }
}

#[derive(Default)]
struct ImpureScanner {
    hits: Vec<ImpureCall>,
}

impl<'ast> Visit<'ast> for ImpureScanner {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let name = mac
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if let Some((_, why)) = MACRO_DENIES.iter().find(|(m, _)| *m == name) {
            self.hits.push(hit(format!("{name}!(..)"), why, mac.span()));
        }
        syn::visit::visit_macro(self, mac);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        if let Some((_, why)) = METHOD_DENIES.iter().find(|(m, _)| *m == name) {
            self.hits
                .push(hit(format!(".{name}(..)"), why, call.method.span()));
        }
        // The receiver may itself be an impure path, e.g.
        // `std::process::Command::new("ls")` — `new` is innocent, the
        // receiver is not.
        if let syn::Expr::Path(path_expr) = &*call.receiver {
            self.check_path(&path_expr.path, call.receiver.span());
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path_expr) = &*call.func {
            self.check_path(&path_expr.path, call.func.span());
        }
        syn::visit::visit_expr_call(self, call);
    }
}

impl ImpureScanner {
    fn check_path(&mut self, path: &syn::Path, span: Span) {
        let spine = path_spine(path);
        if spine.is_empty() {
            return;
        }
        let normalized = format!("::{spine}::");
        if let Some((_, why)) = PATH_NEEDLES
            .iter()
            .find(|(needle, _)| normalized.contains(needle))
        {
            self.hits
                .push(hit(spine.clone(), why, span));
            return;
        }
        if let Some((_, why)) = BARE_FN_DENIES.iter().find(|(b, _)| *b == spine) {
            self.hits.push(hit(spine.clone(), why, span));
        }
    }
}

/// All impure operations reachable inside one expression (recursively,
/// including closures and nested blocks).
pub fn impure_calls(expr: &syn::Expr) -> Vec<ImpureCall> {
    let mut scanner = ImpureScanner::default();
    scanner.visit_expr(expr);
    scanner.hits
}

/// All impure operations inside a function body.
pub fn impure_calls_in_block(block: &syn::Block) -> Vec<ImpureCall> {
    let mut scanner = ImpureScanner::default();
    scanner.visit_block(block);
    scanner.hits
}

// ---------------------------------------------------------------------------
// Dialect-wide restrictions (apply with or without contracts).
// ---------------------------------------------------------------------------

/// Lock/monitor types. The dialect has no hand-written locking (the
/// `.nir` language's own position, `crates/compiler/src/ast.rs`: "there
/// is no `mutex`/`lock`") -- hand-written locks are exactly the
/// data-race/deadlock construct the design excludes by construction.
/// Shared state goes through nirdosha-rt's managed primitives instead:
/// `SharedTable<K, V>` / `SharedCell<T>` (keyed/singleton tables whose
/// critical sections are complete methods, so no guard can ever
/// escape) or the copyable `Chan`/`Frozen` handles. `OnceLock` is
/// deliberately NOT on this list -- init-once is not a contended lock.
/// Matched on exact path segments, so fully-qualified (`std::sync::
/// Mutex`), imported (`Mutex`), and constructor (`Mutex::new`)
/// spellings are all caught. Over-approximate by design (Stage 1's
/// documented honesty contract): a user type named exactly `Mutex`
/// would also trip, and that is acceptable -- the name itself is the
/// lie. `use std::sync::Mutex as M` is caught at the import (aliased
/// *use sites* are Stage 2's DefId-resolution job, driver follow-on).
const LOCK_DENIES: &[&str] = &[
    "Mutex", "RwLock", "Condvar",
];

const LOCK_WHY: &str = "raw locks are not supported -- the dialect is data-race-free by construction; use nirdosha-rt's managed primitives (SharedTable/SharedCell) or Chan/Frozen";

fn lock_segment_hit(path: &syn::Path) -> bool {
    path.segments
        .iter()
        .any(|s| LOCK_DENIES.contains(&s.ident.to_string().as_str()))
}

fn lock_use_leaves(tree: &syn::UseTree, out: &mut Vec<String>) {
    match tree {
        syn::UseTree::Name(n) => {
            if LOCK_DENIES.contains(&n.ident.to_string().as_str()) {
                out.push(n.ident.to_string());
            }
        }
        syn::UseTree::Path(p) => {
            if LOCK_DENIES.contains(&p.ident.to_string().as_str()) {
                out.push(p.ident.to_string());
            }
            lock_use_leaves(&p.tree, out);
        }
        syn::UseTree::Rename(r) => {
            // `use std::sync::Mutex as M`: the import itself is the
            // violation, whatever it is renamed to at the call site.
            if LOCK_DENIES.contains(&r.ident.to_string().as_str()) {
                out.push(r.ident.to_string());
            }
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                lock_use_leaves(item, out);
            }
        }
        syn::UseTree::Glob(_) => {
            // `use std::sync::*` cannot be resolved at Stage 1; the
            // Stage 2 driver (DefId-based) closes that gap.
        }
    }
}

#[derive(Debug, Clone)]
pub struct DenyHit {
    pub what: String,
    pub why: &'static str,
    pub line: u32,
}

#[derive(Default)]
struct DenyScanner {
    hits: Vec<DenyHit>,
}

impl<'ast> Visit<'ast> for DenyScanner {
    fn visit_expr_unsafe(&mut self, u: &'ast syn::ExprUnsafe) {
        self.hits.push(DenyHit {
            what: "unsafe { .. } block".into(),
            why: "the dialect forbids unsafe code",
            line: u.span().start().line as u32,
        });
        syn::visit::visit_expr_unsafe(self, u);
    }

    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        let mut leaves = Vec::new();
        lock_use_leaves(&u.tree, &mut leaves);
        let line = u.span().start().line as u32;
        for name in leaves {
            self.hits.push(DenyHit {
                what: format!("import of {name}"),
                why: LOCK_WHY,
                line,
            });
        }
        syn::visit::visit_item_use(self, u);
    }

    fn visit_expr_path(&mut self, e: &'ast syn::ExprPath) {
        if lock_segment_hit(&e.path) {
            self.hits.push(DenyHit {
                what: format!("{} (raw lock)", path_spine(&e.path)),
                why: LOCK_WHY,
                line: e.span().start().line as u32,
            });
        }
        syn::visit::visit_expr_path(self, e);
    }

    fn visit_type_path(&mut self, t: &'ast syn::TypePath) {
        if lock_segment_hit(&t.path) {
            self.hits.push(DenyHit {
                what: format!("{} in type position", path_spine(&t.path)),
                why: LOCK_WHY,
                line: t.span().start().line as u32,
            });
        }
        syn::visit::visit_type_path(self, t);
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if f.sig.unsafety.is_some() {
            self.hits.push(DenyHit {
                what: format!("unsafe fn {}", f.sig.ident),
                why: "the dialect forbids unsafe code",
                line: f.sig.fn_token.span.start().line as u32,
            });
        }
        syn::visit::visit_item_fn(self, f);
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if f.sig.unsafety.is_some() {
            self.hits.push(DenyHit {
                what: format!("unsafe method {}", f.sig.ident),
                why: "the dialect forbids unsafe code",
                line: f.sig.fn_token.span.start().line as u32,
            });
        }
        syn::visit::visit_impl_item_fn(self, f);
    }

    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        if f.sig.unsafety.is_some() {
            self.hits.push(DenyHit {
                what: format!("unsafe trait method {}", f.sig.ident),
                why: "the dialect forbids unsafe code",
                line: f.sig.fn_token.span.start().line as u32,
            });
        }
        syn::visit::visit_trait_item_fn(self, f);
    }

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        if matches!(s.mutability, syn::StaticMutability::Mut(_)) {
            self.hits.push(DenyHit {
                what: format!("static mut {}", s.ident),
                why: "the dialect forbids static mut (no interior mutability outside nirdosha-rt)",
                line: s.static_token.span.start().line as u32,
            });
        }
        syn::visit::visit_item_static(self, s);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "spawn" {
            self.hits.push(DenyHit {
                what: format!(".spawn() at {}", call.method),
                why: "raw threads/processes are not nirdosha-rt managed — use the runtime's concurrency primitives",
                line: call.method.span().start().line as u32,
            });
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path_expr) = &*call.func {
            if path_spine(&path_expr.path).contains("thread::spawn") {
                self.hits.push(DenyHit {
                    what: format!("{}(..)", path_spine(&path_expr.path)),
                    why: "raw threads are not nirdosha-rt managed",
                    line: call.func.span().start().line as u32,
                });
            }
        }
        // Recurse: without this, a hit inside a call argument -- e.g.
        // `STORE.get_or_init(|| Mutex::new(HashMap::new()))`, the
        // corpus's own table pattern -- was never visited. The same
        // blind spot applied to the existing `.spawn()`/`unsafe` deny
        // arms; recursing closes it for all of them.
        syn::visit::visit_expr_call(self, call);
    }
}

/// Dialect-wide restrictions violated anywhere in a file.
pub fn dialect_denies(file: &syn::File) -> Vec<DenyHit> {
    let mut scanner = DenyScanner::default();
    scanner.visit_file(file);
    scanner.hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impure_in(src: &str) -> Vec<ImpureCall> {
        let expr: syn::Expr = syn::parse_str(src).unwrap();
        impure_calls(&expr)
    }

    #[test]
    fn catches_file_io() {
        assert!(!impure_in(r#"std::fs::read_to_string("x")"#).is_empty());
        assert!(!impure_in(r#"fs::read_to_string("x")"#).is_empty());
        assert!(!impure_in(r#"read_to_string("x")"#).is_empty());
    }

    #[test]
    fn catches_clock_and_process() {
        assert!(!impure_in("std::time::Instant::now()").is_empty());
        assert!(!impure_in("Instant::now()").is_empty());
        assert!(!impure_in("std::time::SystemTime::now()").is_empty());
        assert!(!impure_in("std::process::Command::new(\"ls\")").is_empty());
        assert!(!impure_in("std::thread::spawn(|| 1)").is_empty());
    }

    #[test]
    fn catches_panic_family_and_prints() {
        assert!(!impure_in("println!(\"hi\")").is_empty());
        assert!(!impure_in("panic!(\"boom\")").is_empty());
        assert!(!impure_in("Some(1).unwrap()").is_empty());
        assert!(!impure_in("Some(1).expect(\"msg\")").is_empty());
    }

    #[test]
    fn allows_pure_std() {
        // Duration::from_secs is a pure constructor — must NOT fire.
        assert!(impure_in("std::time::Duration::from_secs(5)").is_empty());
        assert!(impure_in("format!(\"{}\", 1)").is_empty());
        assert!(impure_in("vec![1, 2, 3].len()").is_empty());
    }

    #[test]
    fn catches_unsafe_and_static_mut() {
        let file = syn::parse_file("static mut X: u8 = 0;\nunsafe fn bad() {}\nfn f() { unsafe { let _ = X; } }").unwrap();
        let denies = dialect_denies(&file);
        assert_eq!(denies.len(), 3, "{denies:?}");
    }

    #[test]
    fn catches_raw_lock_imports_grouped_aliased_and_qualified() {
        let file = syn::parse_file(
            "use std::sync::{Arc, Condvar, Mutex};\nuse std::sync::RwLock as RW;",
        )
        .unwrap();
        let denies = dialect_denies(&file);
        // Condvar + Mutex from the group, RwLock under its original
        // name (the import is the violation, whatever the alias) --
        // but never `Arc`, and never the `RW` rename target.
        assert_eq!(denies.len(), 3, "{denies:?}");
        for (hit, name) in denies.iter().zip(["Condvar", "Mutex", "RwLock"]) {
            assert!(hit.what.contains(name), "{} must name {name}", hit.what);
        }
    }

    #[test]
    fn catches_raw_lock_in_type_and_constructor_positions() {
        // The corpus's own table pattern: a `OnceLock<Mutex<HashMap>>`
        // singleton initialized through a closure argument -- the type
        // mention AND the constructor inside the `get_or_init` arg
        // must both be caught (the recursion fix).
        let file = syn::parse_file(
            "fn instances() -> &'static std::sync::Mutex<u64> { \
                 static STORE: std::sync::OnceLock<std::sync::Mutex<u64>> = std::sync::OnceLock::new(); \
                 STORE.get_or_init(|| std::sync::Mutex::new(0)) \
             }",
        )
        .unwrap();
        let denies = dialect_denies(&file);
        assert_eq!(denies.len(), 3, "{denies:?}");
    }

    #[test]
    fn allows_once_lock_and_managed_primitives() {
        let file = syn::parse_file(
            "use std::sync::OnceLock;\nstatic X: OnceLock<u64> = OnceLock::new();\nfn f() -> i64 { let t = nirdosha_rt::prelude::SharedTable::<i64, u64>::new(); t.len() }",
        )
        .unwrap();
        let denies = dialect_denies(&file);
        assert!(denies.is_empty(), "{denies:?}");
    }
}
