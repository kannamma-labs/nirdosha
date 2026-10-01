//! Relation resolution — Plan Phase 11 (RFC 0023 §4).
//!
//! `FilterExpr::RelationIn` must never reach a `StoreDriver::query`:
//! `RdbmsEmitter::emit_where` panics on it (`"RelationIn must be erased
//! before reaching a driver"`) and `MemStoreDriver::match_filter` silently
//! excludes every row on it — both are honest reactions to an unresolved
//! relation reaching a place that has no way to evaluate it, not bugs to
//! route around. Nothing in the workspace erased `RelationIn` before this:
//! `relation_lower::lower_relation`/`reject_negative_relation`
//! (`nirdosha-guard-core`) are real and tested, but were never called from
//! anywhere. `resolve_filter_relations` is what actually calls them, on the
//! real path a read takes.

use nirdosha_guard_core::relation_lower::{lower_relation, reject_negative_relation, LoweredRelation};
use nirdosha_guard_core::{
	Cap, FilterExpr, Freshness, PaginationMode, PatternMatcher, RelationError, RelationExpr, RelationResolver,
	ResolutionTier, ResolvedRelation, Subject, Value,
};

use crate::{ReadPlanIr, StoreDriver};

/// The outcome of resolving every `RelationIn` node in a filter: either a
/// filter with none left (safe to hand to a driver), or `Escalate` — a
/// Tier-3 relation was hit, which the caller must turn into a real
/// escalation (mirroring `evaluator::evaluate`'s own `Decision::Escalate`),
/// never silently downgraded to an empty/unfiltered read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedFilter {
	Filter(FilterExpr),
	Escalate,
}

/// Resolves every `RelationIn` node in `filter` against `resolver`,
/// erasing it into the concrete `FilterExpr` `relation_lower::lower_relation`
/// produces for whatever tier the resolver reports. Rejects `Not(RelationIn)`
/// up front (RFC 0023 §4: relations are positive-only) via the existing,
/// tested `reject_negative_relation` — reused, not reimplemented.
pub fn resolve_filter_relations<R: RelationResolver>(
	filter: &FilterExpr,
	resolver: &R,
	subject: &Subject,
) -> Result<ResolvedFilter, RelationError> {
	reject_negative_relation(filter)?;
	resolve(filter, resolver, subject)
}

fn resolve<R: RelationResolver>(filter: &FilterExpr, resolver: &R, subject: &Subject) -> Result<ResolvedFilter, RelationError> {
	match filter {
		FilterExpr::RelationIn { field, relation } => {
			let resolved = resolver.resolve(subject, relation)?;
			Ok(match lower_relation(field.clone(), &resolved)? {
				LoweredRelation::Native(expr) | LoweredRelation::InList(expr) | LoweredRelation::Materialized(expr) => ResolvedFilter::Filter(expr),
				LoweredRelation::Escalate => ResolvedFilter::Escalate,
			})
		}
		FilterExpr::And(children) => resolve_combinator(children, resolver, subject, true),
		FilterExpr::Or(children) => resolve_combinator(children, resolver, subject, false),
		FilterExpr::Not(inner) => match resolve(inner, resolver, subject)? {
			ResolvedFilter::Filter(expr) => Ok(ResolvedFilter::Filter(FilterExpr::Not(Box::new(expr)))),
			ResolvedFilter::Escalate => Ok(ResolvedFilter::Escalate),
		},
		other => Ok(ResolvedFilter::Filter(other.clone())),
	}
}

fn resolve_combinator<R: RelationResolver>(
	children: &[FilterExpr],
	resolver: &R,
	subject: &Subject,
	is_and: bool,
) -> Result<ResolvedFilter, RelationError> {
	let mut resolved_children = Vec::with_capacity(children.len());
	for child in children {
		match resolve(child, resolver, subject)? {
			ResolvedFilter::Filter(expr) => resolved_children.push(expr),
			ResolvedFilter::Escalate => return Ok(ResolvedFilter::Escalate),
		}
	}
	Ok(ResolvedFilter::Filter(if is_and { FilterExpr::And(resolved_children) } else { FilterExpr::Or(resolved_children) }))
}

/// An in-process `RelationResolver` backed by the same `StoreDriver` a
/// read is already executing against — RFC 0023 §4's Tier 0/1 path. No
/// OpenFGA/SpiceDB instance exists in this workspace, so this resolves
/// bounded relations the same honest way canary attestation already
/// queries a driver: a real, capped query, not a mock. The
/// `RelationResolver` trait slot stays open for a real OpenFGA/SpiceDB
/// implementor elsewhere — this is one honest implementation of it, the
/// same "trait exists, one honest implementation ships" pattern the
/// workspace already uses for `StoreDriver`/`PostgresStoreDriver`.
///
/// **Storage contract.** Neither real driver's schema
/// (`resource`/`tenant`/opaque `payload`) has room for an arbitrary join
/// column, so a relation's backing rows are modeled the only structured
/// way that schema honestly allows: each related value is one row whose
/// `resource` key starts with `relation:<relation.name>:<subject.id>:`
/// (unique per row; the exact suffix doesn't matter, `resolve` never reads
/// it) and whose `payload` *is* the related value's UTF-8 bytes. Both real
/// drivers' manifests already attest genuine `Pattern` pushdown, so the
/// prefix scan below is real filter pushdown, not a post-read scan.
///
/// **Tier honesty.** `resolve` always reports `Tier1InList` — neither
/// driver has a native join (`Tier0Native`) or a write-through
/// materialized column (`Tier2Materialized`) for this resolver to honestly
/// claim; it is exactly what its own doc says: a bounded `IN`-list lookup.
pub struct InProcessRelationResolver<'d, D: StoreDriver> {
	driver: &'d D,
	tenant: String,
}

impl<'d, D: StoreDriver> InProcessRelationResolver<'d, D> {
	pub fn new(driver: &'d D, tenant: impl Into<String>) -> Self {
		Self { driver, tenant: tenant.into() }
	}

	fn key_prefix(relation: &RelationExpr, subject_id: &str) -> String {
		format!("relation:{}:{}:", relation.name, subject_id)
	}
}

impl<'d, D: StoreDriver> RelationResolver for InProcessRelationResolver<'d, D> {
	fn resolve(&self, subject: &Subject, relation: &RelationExpr) -> Result<ResolvedRelation, RelationError> {
		let prefix = Self::key_prefix(relation, &subject.id);
		// One row past the declared ceiling so an over-large relation is
		// caught as CardinalityExceeded rather than silently truncated to
		// a smaller-looking, seemingly-valid bounded set.
		let probe_limit = relation.max_cardinality.saturating_add(1);
		let plan = ReadPlanIr {
			resource: relation.source.clone(),
			dataset: relation.source.clone(),
			filter: Some(FilterExpr::And(vec![
				FilterExpr::TenantEq { value: Value::Str(self.tenant.clone()) },
				FilterExpr::Pattern { field: vec!["resource".into()], matcher: PatternMatcher::Prefix(prefix) },
			])),
			caps: vec![Cap::RowCap(probe_limit)],
			pagination: PaginationMode::LimitOnly { limit: probe_limit },
			policy_version: "relation-resolve".into(),
		};
		let result = self.driver.query(&plan).map_err(|_| RelationError::SourceUnavailable)?;
		if result.rows.len() as u64 > relation.max_cardinality {
			return Err(RelationError::CardinalityExceeded { max: relation.max_cardinality, actual: result.rows.len() as u64 });
		}
		let values: Vec<Value> = result.rows.iter().filter_map(|row| String::from_utf8(row.0.clone()).ok()).map(Value::Str).collect();
		Ok(ResolvedRelation {
			values,
			freshness: Freshness { source: relation.source.clone(), source_epoch: "live".into(), resolved_at: "resolved-at-query-time".into(), ttl_seconds: relation.ttl_seconds },
			tier: ResolutionTier::Tier1InList,
		})
	}

	fn resolve_many(&self, subjects: &[Subject], relation: &RelationExpr) -> Result<Vec<ResolvedRelation>, RelationError> {
		subjects.iter().map(|subject| self.resolve(subject, relation)).collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{EntityBytes, PlanIr};
	use nirdosha_guard_core::Classification;

	fn subject(id: &str) -> Subject {
		Subject { id: id.into(), roles: vec!["analyst".into()], claims: vec![], clearance: Classification::Internal }
	}

	fn seed_relation_row(driver: &crate::MemStoreDriver, resource: &str, tenant: &str, value: &str) {
		let plan = PlanIr {
			resource: resource.into(),
			dataset: "accounts_of".into(),
			filter: Some(FilterExpr::TenantEq { value: Value::Str(tenant.into()) }),
			row_scope: None,
			action: WriteActionAlias::Create,
			affected_row_cap: None,
			policy_version: "seed".into(),
		};
		let prepared = driver.prepare(&plan).expect("seed prepare");
		driver.commit(prepared, EntityBytes(value.as_bytes().to_vec())).expect("seed commit");
	}

	use nirdosha_guard_core::WriteAction as WriteActionAlias;

	#[test]
	fn resolves_a_bounded_relation_into_a_real_in_list_filter() {
		let driver = crate::MemStoreDriver::new();
		seed_relation_row(&driver, "relation:accounts_of:cust-1:acct-1", "tenant-a", "acct-1");
		seed_relation_row(&driver, "relation:accounts_of:cust-1:acct-2", "tenant-a", "acct-2");
		// A different subject's rows must not leak into cust-1's resolution.
		seed_relation_row(&driver, "relation:accounts_of:cust-2:acct-9", "tenant-a", "acct-9");

		let resolver = InProcessRelationResolver::new(&driver, "tenant-a");
		let relation = RelationExpr { name: "accounts_of".into(), source: "accounts_of".into(), max_cardinality: 10, ttl_seconds: 60 };
		let resolved = resolver.resolve(&subject("cust-1"), &relation).expect("resolve must succeed");
		assert_eq!(resolved.tier, ResolutionTier::Tier1InList);
		let mut values: Vec<String> = resolved.values.into_iter().map(|v| match v { Value::Str(s) => s, _ => panic!("expected Str") }).collect();
		values.sort();
		assert_eq!(values, vec!["acct-1".to_string(), "acct-2".to_string()]);
	}

	#[test]
	fn refuses_a_relation_that_exceeds_its_declared_cardinality() {
		let driver = crate::MemStoreDriver::new();
		for i in 0..5 {
			seed_relation_row(&driver, &format!("relation:accounts_of:cust-1:acct-{i}"), "tenant-a", &format!("acct-{i}"));
		}
		let resolver = InProcessRelationResolver::new(&driver, "tenant-a");
		let relation = RelationExpr { name: "accounts_of".into(), source: "accounts_of".into(), max_cardinality: 3, ttl_seconds: 60 };
		let result = resolver.resolve(&subject("cust-1"), &relation);
		assert!(matches!(result, Err(RelationError::CardinalityExceeded { max: 3, actual: 4 })), "expected CardinalityExceeded, got {result:?}");
	}

	#[test]
	fn resolve_many_resolves_a_bounded_relation_for_every_subject() {
		let driver = crate::MemStoreDriver::new();
		seed_relation_row(&driver, "relation:accounts_of:cust-1:acct-1", "tenant-a", "acct-1");
		seed_relation_row(&driver, "relation:accounts_of:cust-2:acct-2", "tenant-a", "acct-2");
		seed_relation_row(&driver, "relation:accounts_of:cust-2:acct-3", "tenant-a", "acct-3");
		let resolver = InProcessRelationResolver::new(&driver, "tenant-a");
		let relation = RelationExpr { name: "accounts_of".into(), source: "accounts_of".into(), max_cardinality: 10, ttl_seconds: 60 };
		let subjects = [subject("cust-1"), subject("cust-2")];
		let resolved = resolver.resolve_many(&subjects, &relation).expect("resolve_many must succeed");
		assert_eq!(resolved.len(), 2);
		assert_eq!(resolved[0].values, vec![Value::Str("acct-1".into())]);
		let mut cust2_values: Vec<String> = resolved[1].values.iter().map(|v| match v { Value::Str(s) => s.clone(), _ => panic!("expected Str") }).collect();
		cust2_values.sort();
		assert_eq!(cust2_values, vec!["acct-2".to_string(), "acct-3".to_string()]);
	}

	#[test]
	fn resolve_filter_relations_erases_relation_in_from_a_composite_filter() {
		let driver = crate::MemStoreDriver::new();
		seed_relation_row(&driver, "relation:accounts_of:cust-1:acct-1", "tenant-a", "acct-1");
		let resolver = InProcessRelationResolver::new(&driver, "tenant-a");
		let relation = RelationExpr { name: "accounts_of".into(), source: "accounts_of".into(), max_cardinality: 10, ttl_seconds: 60 };
		let filter = FilterExpr::And(vec![
			FilterExpr::TenantEq { value: Value::Str("tenant-a".into()) },
			FilterExpr::RelationIn { field: vec!["account_id".into()], relation },
		]);
		let resolved = resolve_filter_relations(&filter, &resolver, &subject("cust-1")).expect("resolution must succeed");
		match resolved {
			ResolvedFilter::Filter(FilterExpr::And(children)) => {
				assert!(children.iter().any(|c| matches!(c, FilterExpr::In { field, values } if field == &vec!["account_id".to_string()] && values == &vec![Value::Str("acct-1".into())])), "expected the RelationIn node erased into a concrete In filter: {children:?}");
			}
			other => panic!("expected a resolved And filter, got {other:?}"),
		}
	}

	#[test]
	fn resolve_filter_relations_rejects_a_negated_relation() {
		let driver = crate::MemStoreDriver::new();
		let resolver = InProcessRelationResolver::new(&driver, "tenant-a");
		let relation = RelationExpr { name: "accounts_of".into(), source: "accounts_of".into(), max_cardinality: 10, ttl_seconds: 60 };
		let filter = FilterExpr::Not(Box::new(FilterExpr::RelationIn { field: vec!["account_id".into()], relation }));
		let result = resolve_filter_relations(&filter, &resolver, &subject("cust-1"));
		assert!(matches!(result, Err(RelationError::Unresolvable)), "a negated relation must be rejected, not silently resolved: {result:?}");
	}
}
