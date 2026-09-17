//! Syntactic (Stage 1) enforcement of a domain pack's `mandatory_fns`/
//! `protected_structs` invariants — RFC 0016 Phase 3's own rules
//! ("For every plugin-declared protected type/field, any read, write,
//! or mutation outside certified primitive units fails the generate
//! gate"; "every fn a governing pack marks `mandatory_fns` must have
//! at least one real call site"), ported here (issue #76) from
//! `nirdosha-hi::hi_llm::check_primitive_exclusivity_coverage`/
//! `check_mandatory_primitive_coverage` — real, already-tested logic
//! that until now only ever fed an LLM-generation guidance loop, never
//! a build that actually refuses on violation.
//!
//! **Real minimal version, disclosed scope** (same discipline as
//! `nirdosha-contract-core::scan`'s own Stage 1/Stage 2 split): this is
//! a `syn`-based syntactic scan over source text, not a MIR-resolved
//! check over real name resolution. A macro-generated construction site
//! or an aliased import can evade it, the same honest limit
//! `docs/V2_GUARANTEES.md` already states for `effects(pure)`'s own
//! Stage 1. A MIR-level port (real `DefId` resolution, immune to macro
//! expansion and renaming) is real, disclosed follow-on work, the same
//! shape issue #74 already gave `effects(pure)`.

use std::collections::HashSet;

use syn::spanned::Spanned;

/// Every plain-name call target and struct-literal construction inside
/// `block` — struct construction and an ordinary fn call share one AST
/// node (`syn::ExprCall`) only when disambiguated by name resolution
/// (which this syntactic pass doesn't have), so `syn::ExprStruct`
/// (`Name { .. }`/`Name { ..Default::default() }`) is tracked
/// separately and unambiguously.
pub fn collect_calls_and_constructs(block: &syn::Block) -> (HashSet<String>, HashSet<String>) {
    struct Collector<'a> {
        called: &'a mut HashSet<String>,
        constructed: &'a mut HashSet<String>,
    }
    impl<'ast> syn::visit::Visit<'ast> for Collector<'_> {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(p) = node.func.as_ref() {
                if let Some(seg) = p.path.segments.last() {
                    self.called.insert(seg.ident.to_string());
                }
            }
            syn::visit::visit_expr_call(self, node);
        }
        fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
            if let Some(seg) = node.path.segments.last() {
                self.constructed.insert(seg.ident.to_string());
            }
            syn::visit::visit_expr_struct(self, node);
        }
    }
    let mut called = HashSet::new();
    let mut constructed = HashSet::new();
    syn::visit::Visit::visit_block(&mut Collector { called: &mut called, constructed: &mut constructed }, block);
    (called, constructed)
}

pub struct ExclusivityViolation {
    pub fn_name: String,
    pub struct_name: String,
    pub line: u32,
}

/// A pack-protected struct's literal construction outside one of
/// `mandatory_fns`'s own bodies — "the model constructed its own
/// protected value instead of calling the certified primitive," now
/// checked against real (not LLM-drafted) source. A no-op when no
/// active pack declares any `protected_structs` (the common case: no
/// pack installed).
pub fn check_primitive_exclusivity(
    file: &syn::File,
    protected_structs: &HashSet<String>,
    mandatory_fns: &HashSet<String>,
) -> Vec<ExclusivityViolation> {
    if protected_structs.is_empty() {
        return Vec::new();
    }
    let mut violations = Vec::new();
    for item in &file.items {
        let syn::Item::Fn(f) = item else { continue };
        let fn_name = f.sig.ident.to_string();
        if mandatory_fns.contains(&fn_name) {
            continue; // a certified primitive is exactly where this construction belongs.
        }
        let (_called, constructed) = collect_calls_and_constructs(&f.block);
        let mut offending: Vec<&String> = protected_structs.intersection(&constructed).collect();
        offending.sort();
        for struct_name in offending {
            violations.push(ExclusivityViolation {
                fn_name: fn_name.clone(),
                struct_name: struct_name.clone(),
                line: f.sig.fn_token.span().start().line as u32,
            });
        }
    }
    violations
}

/// Every `mandatory_fns` name with no call site anywhere across
/// `files` — "the model wires the ledger" is only a real guarantee if
/// failing to do so refuses the build, not just biases what an LLM is
/// nudged to draft next. A no-op when no active pack declares any
/// `mandatory_fns`.
pub fn missing_mandatory_call_sites(files: &[syn::File], mandatory_fns: &HashSet<String>) -> Vec<String> {
    if mandatory_fns.is_empty() {
        return Vec::new();
    }
    let mut called: HashSet<String> = HashSet::new();
    for file in files {
        for item in &file.items {
            if let syn::Item::Fn(f) = item {
                let (fn_called, _constructed) = collect_calls_and_constructs(&f.block);
                called.extend(fn_called);
            }
        }
    }
    let mut missing: Vec<&String> = mandatory_fns.iter().filter(|name| !called.contains(name.as_str())).collect();
    missing.sort();
    missing.into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> syn::File {
        syn::parse_file(src).unwrap()
    }

    #[test]
    fn exclusivity_is_a_no_op_with_nothing_protected() {
        let file = parse("fn main() { Account { balance_cents: 0 }; }");
        assert!(check_primitive_exclusivity(&file, &HashSet::new(), &HashSet::new()).is_empty());
    }

    #[test]
    fn exclusivity_flags_construction_outside_the_certified_primitive() {
        let file = parse(
            r#"
            fn debit_ledger(n: i64) -> Account { Account { balance_cents: n } }
            fn bad_charge(n: i64) -> Account { Account { balance_cents: n } }
            "#,
        );
        let protected: HashSet<String> = ["Account".to_string()].into();
        let mandatory: HashSet<String> = ["debit_ledger".to_string()].into();
        let violations = check_primitive_exclusivity(&file, &protected, &mandatory);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].fn_name, "bad_charge");
        assert_eq!(violations[0].struct_name, "Account");
    }

    #[test]
    fn missing_call_sites_is_empty_when_nothing_mandatory() {
        assert!(missing_mandatory_call_sites(&[parse("fn main() {}")], &HashSet::new()).is_empty());
    }

    #[test]
    fn missing_call_sites_flags_an_uncalled_mandatory_fn() {
        let files = vec![parse("fn debit_ledger() {} fn main() {}")];
        let mandatory: HashSet<String> = ["debit_ledger".to_string()].into();
        assert_eq!(missing_mandatory_call_sites(&files, &mandatory), vec!["debit_ledger".to_string()]);
    }

    #[test]
    fn missing_call_sites_passes_when_called_from_a_different_file() {
        let files = vec![parse("fn debit_ledger() {}"), parse("fn main() { debit_ledger(); }")];
        let mandatory: HashSet<String> = ["debit_ledger".to_string()].into();
        assert!(missing_mandatory_call_sites(&files, &mandatory).is_empty());
    }
}
