//! Gate-3: cross-crate registry totality — Plan Phase 17.
//!
//! Today's registry totality (V1, `nirdosha-guard-verify`) is per-crate:
//! `linkme` slices only see what got linked into one compiled binary.
//! This module extends `nirdosha-driver`'s existing rustc-driver harness
//! (already used for `#[contract(effects(pure))]` call-graph checking) to
//! give the *compiler* — not the linker — a per-crate view of the same
//! registry shape, written to a fragment file every time a crate compiles
//! through this driver. `cargo nirdosha verify --guard --workspace`
//! (`cargo-nirdosha`) merges every crate's fragment after a full
//! `--deep` workspace build, giving true cross-crate totality: a crate A
//! whose policy names a resource only ever declared by a `#[dataset]` in
//! sibling crate B is caught even though nothing ever links A and B into
//! one binary together.
//!
//! **HIR-syntactic, not const-eval.** The macro-generated statics this
//! module looks for (`PolicyRegistration`/`CatalogRegistration` — see
//! `nirdosha-guard-macros`'s `policy_impl`/`attribute_impl`) are always,
//! by construction, simple struct literals with string-literal fields —
//! never a computed expression. Matching that shape directly in HIR
//! (`ExprKind::Struct` with `ExprKind::Lit` field values) is far more
//! tractable than decoding a `mir::interpret::ConstAllocation`'s raw
//! bytes/relocations to recover a `&'static str`'s pointer+length, and is
//! exactly what "the expanded static items are discoverable in HIR"
//! (this plan's own scoping note) means: syntactic discovery, not
//! const-evaluation. A static whose initializer isn't this exact shape
//! (a future macro version, or hand-written code of the same type) is
//! silently skipped, not guessed at — Gate-3 stays honest about what it
//! actually inspects.

use rustc_ast::LitKind;
use rustc_hir::{Expr, ExprKind};
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::LocalDefId;
use serde::{Deserialize, Serialize};

const POLICY_REGISTRATION_PATH: &str = "nirdosha_guard_registry::PolicyRegistration";
const CATALOG_REGISTRATION_PATH: &str = "nirdosha_guard_registry::CatalogRegistration";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Gate3Fragment {
	pub crate_name: String,
	/// `resource` field literal from every real `PolicyRegistration`
	/// static this crate declares.
	pub policy_resources: Vec<String>,
	/// `entity = "..."` parsed out of every `CatalogRegistration` static
	/// whose `kind` field literal is `"dataset"` — the `#[dataset(entity
	/// = "...", store = "...")]` attribute's own declared entity name.
	pub dataset_entities: Vec<String>,
}

/// Walks every `static` item in the local crate whose type is exactly
/// `PolicyRegistration` or `CatalogRegistration`, extracting the literal
/// field values Gate-3's cross-crate check needs.
pub fn collect(tcx: TyCtxt<'_>) -> Gate3Fragment {
	let mut fragment = Gate3Fragment { crate_name: tcx.crate_name(rustc_span::def_id::LOCAL_CRATE).to_string(), ..Default::default() };

	for did in tcx.mir_keys(()).iter().copied() {
		if !matches!(tcx.def_kind(did), rustc_hir::def::DefKind::Static { .. }) {
			continue;
		}
		let Some(adt_def) = tcx.type_of(did).skip_binder().ty_adt_def() else { continue };
		let ty_path = tcx.def_path_str(adt_def.did());
		if ty_path == POLICY_REGISTRATION_PATH {
			if let Some(fields) = struct_literal_fields(tcx, did) {
				if let Some(resource) = string_field(&fields, "resource") {
					fragment.policy_resources.push(resource);
				}
			}
		} else if ty_path == CATALOG_REGISTRATION_PATH {
			if let Some(fields) = struct_literal_fields(tcx, did) {
				if string_field(&fields, "kind").as_deref() == Some("dataset") {
					if let Some(source) = string_field(&fields, "source") {
						if let Some(entity) = parse_dataset_entity(&source) {
							fragment.dataset_entities.push(entity);
						}
					}
				}
			}
		}
	}
	fragment.policy_resources.sort();
	fragment.policy_resources.dedup();
	fragment.dataset_entities.sort();
	fragment.dataset_entities.dedup();
	fragment
}

/// If `did`'s initializer body is a bare struct-literal expression
/// (`Type { field: expr, ... }`, optionally behind blocks), returns its
/// named fields. Anything else (a function call, a `match`, ...) — not
/// the shape this driver's own macros ever generate — returns `None`
/// rather than guessing.
fn struct_literal_fields<'tcx>(tcx: TyCtxt<'tcx>, did: LocalDefId) -> Option<&'tcx [rustc_hir::ExprField<'tcx>]> {
	let body_id = tcx.hir_body_owned_by(did);
	let mut expr = body_id.value;
	loop {
		match &expr.kind {
			ExprKind::Struct(_, fields, _) => return Some(fields),
			ExprKind::Block(block, _) => {
				expr = block.expr?;
			}
			ExprKind::DropTemps(inner) => {
				expr = inner;
			}
			_ => return None,
		}
	}
}

fn string_field(fields: &[rustc_hir::ExprField<'_>], name: &str) -> Option<String> {
	fields.iter().find(|field| field.ident.as_str() == name).and_then(|field| string_literal(field.expr))
}

/// `stringify!(#name)` (used by `attribute_impl`'s `name` field) expands,
/// by macro-expansion time, into a plain string literal already — no
/// special case needed here. A `Path` reference to a `const` string would
/// need a further resolution step this function doesn't attempt; none of
/// the fields Gate-3 reads (`resource`/`kind`/`source`) are ever
/// generated that way by this workspace's own macros.
fn string_literal(expr: &Expr<'_>) -> Option<String> {
	match &expr.kind {
		ExprKind::Lit(lit) => match &lit.node {
			LitKind::Str(symbol, _) => Some(symbol.as_str().to_string()),
			_ => None,
		},
		_ => None,
	}
}

/// Parses `entity = "value"` out of a `#[dataset(...)]` attribute's raw
/// argument text (`CatalogRegistration.source`) — the same
/// whitespace-tolerant, "verify the exact shape, don't guess" string
/// extraction `nirdosha-guard-macros`'s `parse_quorum_clause` (Plan Phase
/// 15) uses, applied to a different clause shape.
fn parse_dataset_entity(source: &str) -> Option<String> {
	let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
	let after = compact.find("entity=\"")? + "entity=\"".len();
	let rest = &compact[after..];
	let end = rest.find('"')?;
	Some(rest[..end].to_string())
}

/// Writes `fragment` to `<dir>/<crate_name>.json`, creating `dir` if
/// needed. Called only when `NIRDOSHA_GATE3_DIR` is set (`cargo nirdosha
/// verify --guard --workspace`'s own `--deep` build sets it) — every
/// other build of every crate through this driver is unaffected, same as
/// this driver's existing pass-through posture for crates with no
/// Nirdosha contracts.
pub fn write_fragment(dir: &std::path::Path, fragment: &Gate3Fragment) -> std::io::Result<()> {
	std::fs::create_dir_all(dir)?;
	let path = dir.join(format!("{}.json", fragment.crate_name));
	let json = serde_json::to_string_pretty(fragment).expect("Gate3Fragment always serializes");
	std::fs::write(path, json)
}

// Merging fragments and reporting the cross-crate gap
// (`resource`-with-no-matching-`#[dataset]`) is `cargo-nirdosha`'s job,
// not this driver binary's — `cargo nirdosha verify --guard --workspace`
// (crates/cargo-nirdosha/src/main.rs's `gate3_check`) reads the same
// `Gate3Fragment` JSON shape this module writes. Kept as two independent
// readers/writer of one JSON contract rather than sharing Rust code
// across a rustc-driver binary and a plain CLI binary, which would need
// `nirdosha-driver` to also ship as a library crate for no other reason.
