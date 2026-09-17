//! Issue #76's disclosed follow-on: a MIR-level (Stage 2) port of
//! `nirdosha-contract-core::pack_check`'s syntactic `mandatory_fns`/
//! `protected_structs` enforcement — the same properties, checked
//! against real, resolved MIR instead of `syn` source text, immune to
//! macro expansion and aliased imports, the same Stage 1/Stage 2 split
//! `effects(pure)` already has (issue #74) and `resource(kind=..)`/
//! `sequence(before=..,after=..)` were designed into from the start.
//!
//! Active packs are loaded from `nirdosha_contract_core::pack` — the
//! same on-disk, install-time-signature-verified state
//! `cargo-nirdosha`'s own Stage 1 wiring reads — rooted at
//! `NIRDOSHA_PACKAGE_ROOT` (set by `cargo-nirdosha`'s `delegate_with`
//! when it wires this driver in via `RUSTC_WORKSPACE_WRAPPER`).
//! Falling back to the current directory when that env var is absent
//! (e.g. a differential test invoking this driver binary directly,
//! with no `cargo-nirdosha` in front of it) is safe by construction:
//! `pack::load_active_packs` itself is a no-op when no `.nir/hi.db`
//! exists at the given root, the overwhelmingly common case.

use std::collections::HashSet;
use std::path::PathBuf;

use rustc_hir::def::DefKind;
use rustc_middle::mir::{AggregateKind, Rvalue, StatementKind, TerminatorKind};
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_span::Span;

fn package_root() -> PathBuf {
    std::env::var_os("NIRDOSHA_PACKAGE_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
}

pub struct ActivePacks {
    pub mandatory_fns: HashSet<String>,
    pub protected_structs: HashSet<String>,
}

impl ActivePacks {
    pub fn is_empty(&self) -> bool {
        self.mandatory_fns.is_empty() && self.protected_structs.is_empty()
    }
}

pub fn load_active_packs() -> Result<ActivePacks, String> {
    let root = package_root();
    Ok(ActivePacks {
        mandatory_fns: nirdosha_contract_core::pack::active_mandatory_fns(&root)?,
        protected_structs: nirdosha_contract_core::pack::active_protected_structs(&root)?,
    })
}

pub struct ExclusivityViolation {
    pub fn_name: String,
    pub struct_name: String,
    pub span: Span,
}

/// A pack-protected struct's construction (MIR `Rvalue::Aggregate` over
/// `AggregateKind::Adt`) outside one of `mandatory_fns`'s own bodies —
/// the MIR-resolved twin of `nirdosha_contract_core::pack_check::
/// check_primitive_exclusivity`'s syntactic `syn::ExprStruct` check.
/// Matches the real, resolved struct `DefId`'s own name, so a macro
/// that expands into the same construction, or a `use Account as Acc`
/// alias, is still caught — the exact class of evasion the syntactic
/// Stage 1 version can't see through.
pub fn check_protected_struct_exclusivity(tcx: TyCtxt<'_>, packs: &ActivePacks) -> Vec<ExclusivityViolation> {
    if packs.protected_structs.is_empty() {
        return Vec::new();
    }
    let mut violations = Vec::new();
    for did in tcx.mir_keys(()).iter() {
        if !matches!(tcx.def_kind(*did), DefKind::Fn | DefKind::AssocFn) {
            continue;
        }
        if tcx.is_coroutine(did.to_def_id()) {
            continue; // no MIR shape this check understands; not this check's job to gate the build on.
        }
        let fn_name = tcx.item_name(did.to_def_id()).to_string();
        if packs.mandatory_fns.contains(&fn_name) {
            continue; // a certified primitive is exactly where this construction belongs.
        }
        let mir = tcx.mir_drops_elaborated_and_const_checked(*did);
        if mir.is_stolen() {
            continue;
        }
        let body = mir.borrow();
        for block in body.basic_blocks.iter() {
            for statement in &block.statements {
                let StatementKind::Assign(assign) = &statement.kind else {
                    continue;
                };
                let Rvalue::Aggregate(kind, _) = &assign.1 else {
                    continue;
                };
                let AggregateKind::Adt(adt_did, ..) = kind.as_ref() else {
                    continue;
                };
                let struct_name = tcx.item_name(*adt_did).to_string();
                if packs.protected_structs.contains(&struct_name) {
                    violations.push(ExclusivityViolation {
                        fn_name: fn_name.clone(),
                        struct_name,
                        span: statement.source_info.span,
                    });
                }
            }
        }
    }
    violations
}

/// Every `mandatory_fns` name with no call site anywhere in the
/// crate's real, resolved call graph — the MIR-resolved twin of
/// `pack_check::missing_mandatory_call_sites`. Matched by the
/// resolved callee's own simple name (pack manifests name bare
/// identifiers, not full paths — the same convention `sequence_
/// dataflow.rs`'s `callee_matches` and `cargo-nirdosha`'s own
/// syntactic checker use).
pub fn check_mandatory_call_sites(tcx: TyCtxt<'_>, packs: &ActivePacks) -> Vec<String> {
    if packs.mandatory_fns.is_empty() {
        return Vec::new();
    }
    let mut called: HashSet<String> = HashSet::new();
    for did in tcx.mir_keys(()).iter() {
        if !matches!(tcx.def_kind(*did), DefKind::Fn | DefKind::AssocFn | DefKind::Closure) {
            continue;
        }
        if tcx.is_coroutine(did.to_def_id()) {
            continue;
        }
        let mir = tcx.mir_drops_elaborated_and_const_checked(*did);
        if mir.is_stolen() {
            continue;
        }
        let body = mir.borrow();
        for block in body.basic_blocks.iter() {
            let Some(terminator) = &block.terminator else {
                continue;
            };
            let func = match &terminator.kind {
                TerminatorKind::Call { func, .. } | TerminatorKind::TailCall { func, .. } => func,
                _ => continue,
            };
            if let Some(name) = callee_simple_name(tcx, func) {
                called.insert(name);
            }
        }
    }
    let mut missing: Vec<&String> = packs.mandatory_fns.iter().filter(|name| !called.contains(name.as_str())).collect();
    missing.sort();
    missing.into_iter().cloned().collect()
}

fn callee_simple_name<'tcx>(tcx: TyCtxt<'tcx>, func: &rustc_middle::mir::Operand<'tcx>) -> Option<String> {
    let rustc_middle::mir::Operand::Constant(c) = func else {
        return None;
    };
    let TyKind::FnDef(did, _) = c.const_.ty().kind() else {
        return None;
    };
    Some(tcx.item_name(*did).to_string())
}
