//! Stage-1 scanners: impure-call detection for `effects(pure)` claims,
//! and the dialect's global restrictions (`unsafe`, raw threads).
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
    fn visit_expr_macro(&mut self, mac: &'ast syn::ExprMacro) {
        let name = mac
            .mac
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if let Some((_, why)) = MACRO_DENIES.iter().find(|(m, _)| *m == name) {
            self.hits.push(hit(format!("{name}!(..)"), why, mac.span()));
        }
        syn::visit::visit_expr_macro(self, mac);
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
}