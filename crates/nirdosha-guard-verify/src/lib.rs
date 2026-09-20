pub mod conformance;

use nirdosha_guard_core::{Cap, Condition, Destination, EscalateTarget, FieldMask, FieldPolicy, FilterExpr};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity { Error, Warning }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding { pub pass: String, pub severity: Severity, pub message: String, pub item: Option<String> }

impl Finding { fn error(pass: &str, message: impl Into<String>, item: Option<String>) -> Self { Self { pass: pass.into(), severity: Severity::Error, message: message.into(), item } } }

/// Mirrors `nirdosha_guard_registry::PolicyRecord`'s shape (deserialized
/// from the registry's JSON dump, which now serializes `records()` — the
/// fully lowered policies, not the opaque `PolicyRegistration`). `#[serde(default)]`
/// on every clause-derived field keeps this readable from an older dump
/// and from hand-built test fixtures that only set the fields a given test
/// cares about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PolicyView {
	pub id: String,
	pub action: String,
	pub resource: String,
	pub purpose: Option<String>,
	pub effect: String,
	#[serde(default)]
	pub subjects: Vec<String>,
	#[serde(default)]
	pub caps: Vec<Cap>,
	#[serde(default)]
	pub field_policy: Vec<FieldPolicy>,
	#[serde(default)]
	pub conditions: Vec<Condition>,
	#[serde(default)]
	pub filter: Option<FilterExpr>,
	#[serde(default)]
	pub masks: Vec<FieldMask>,
	#[serde(default)]
	pub reason: Option<String>,
	#[serde(default)]
	pub destination: Option<Destination>,
	#[serde(default)]
	pub escalation: Option<EscalateTarget>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PortView { pub name: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DriverView { pub port: String, pub vendor: String, pub version: String, pub capabilities: Vec<String>, pub lineage_support: String }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RegistryView {
	pub policies: Vec<PolicyView>,
	pub ports: Vec<PortView>,
	// `nirdosha_guard_registry::RegistryDump` (what `dump_json()` actually
	// produces) calls this field `driver_manifests`, not `drivers` — a
	// mismatch nothing caught before now because nothing had fed a real
	// dump through `RegistryView::from_json` until the RTM policy corpus
	// test's end-to-end check did.
	#[serde(rename = "driver_manifests")]
	pub drivers: Vec<DriverView>,
}

impl RegistryView { pub fn from_json(value: &str) -> Result<Self, serde_json::Error> { serde_json::from_str(value) } }

pub trait VerifyPass { fn name(&self) -> &'static str; fn run(&self, registry: &RegistryView) -> Vec<Finding>; }

/// The closed wire-strings `nirdosha_guard_core::Action::parse_wire`
/// accepts. Kept as its own list rather than importing `Action` — this
/// crate checks the *wire string* a policy declared, independent of
/// whether the guard-core enum a given dump was produced against still
/// has exactly these variants.
const CANONICAL_ACTIONS: &[&str] = &[
	"read", "create", "update", "delete", "migrate", "export", "enumerate",
	"aggregate", "lineage_query", "simulate", "delegate",
];

struct V1; struct V2; struct V3; struct V4; struct V5; struct V6; struct V7; struct V8;

impl VerifyPass for V1 { fn name(&self) -> &'static str { "V1" } fn run(&self, registry: &RegistryView) -> Vec<Finding> { registry.policies.iter().filter(|p| p.id.is_empty() || p.action.is_empty() || p.resource.is_empty()).map(|p| Finding::error(self.name(), "policy record is missing an identity, action, or resource", Some(p.id.clone()))).collect() } }
impl VerifyPass for V2 { fn name(&self) -> &'static str { "V2" } fn run(&self, registry: &RegistryView) -> Vec<Finding> { registry.policies.iter().filter(|p| p.effect != "allow" && p.effect != "deny").map(|p| Finding::error(self.name(), "policy effect is outside the closed set", Some(p.id.clone()))).collect() } }

/// Coverage: a policy's `action` must be one of the seven wire strings
/// `Action::parse_wire` accepts. A typo here (`"raed"`, `"Read"`) doesn't
/// fail to compile or fail V1/V2 (id/action/resource are non-empty, effect
/// is valid) — it silently produces a `PolicyCandidate` that never matches
/// any real request, i.e. a policy that *looks* live in every listing but
/// never actually evaluates. That's exactly the kind of gap "coverage"
/// verification exists to catch.
impl VerifyPass for V3 {
	fn name(&self) -> &'static str { "V3" }
	fn run(&self, registry: &RegistryView) -> Vec<Finding> {
		registry
			.policies
			.iter()
			.filter(|p| !CANONICAL_ACTIONS.contains(&p.action.as_str()))
			.map(|p| {
				Finding::error(
					self.name(),
					format!(
						"action {:?} is not one of the canonical wire actions {:?} — this policy will never match a real request",
						p.action, CANONICAL_ACTIONS
					),
					Some(p.id.clone()),
				)
			})
			.collect()
	}
}

/// Relation positivity (RFC 0023 §4): no policy's `filter`/`conditions`
/// contain a negated relation. Reuses
/// `nirdosha_guard_core::relation_lower::reject_negative_relation` rather
/// than re-implementing the check.
impl VerifyPass for V4 {
	fn name(&self) -> &'static str { "V4" }
	fn run(&self, registry: &RegistryView) -> Vec<Finding> {
		use nirdosha_guard_core::relation_lower::reject_negative_relation;
		let mut findings = Vec::new();
		for policy in &registry.policies {
			let mut exprs: Vec<&FilterExpr> = policy.filter.iter().collect();
			exprs.extend(policy.conditions.iter().filter_map(|c| match c {
				Condition::Expr(expr) => Some(expr),
				Condition::Custom(_) => None,
			}));
			for expr in exprs {
				if reject_negative_relation(expr).is_err() {
					findings.push(Finding::error(
						self.name(),
						"negated relation (`Not(RelationIn)`) — relations are positive-only unless Tier 2 materialized (RFC 0023 §4)",
						Some(policy.id.clone()),
					));
				}
			}
		}
		findings
	}
}

impl VerifyPass for V5 { fn name(&self) -> &'static str { "V5" } fn run(&self, registry: &RegistryView) -> Vec<Finding> { registry.drivers.iter().filter(|d| d.lineage_support.is_empty()).map(|d| Finding::error(self.name(), "driver manifest omits lineage support", Some(d.vendor.clone()))).collect() } }

/// MCP tool list <-> catalog sync: intentionally still a no-op. Nothing to
/// check yet — `mcp_tools!` isn't a real callable macro anywhere in the
/// workspace (confirmed while building the RTM policy corpus test; see
/// its header), so no MCP tool registration exists for this pass to read.
/// Filling this in is Plan Phase 14's job, once that registration exists.
impl VerifyPass for V6 { fn name(&self) -> &'static str { "V6" } fn run(&self, _registry: &RegistryView) -> Vec<Finding> { Vec::new() } }

/// Separation of duties: the same subject must not be both `allow`ed and
/// `deny`ed for the same `(action, resource)` pair. An empty `subjects`
/// list means "applies to any subject," so it's treated as intersecting
/// with anything. Different roles having different permissions on the
/// same `(action, resource)` — the normal RBAC case, and how most of the
/// corpus's own named SoD records actually work (`ingest-no-readback`
/// denies `SvcIngest` reading `transaction`; `analyst-search-transaction`
/// allows `Analyst` reading it — different subjects, no conflict) — is not
/// flagged; only a literal same-subject self-contradiction is.
impl VerifyPass for V7 {
	fn name(&self) -> &'static str { "V7" }
	fn run(&self, registry: &RegistryView) -> Vec<Finding> {
		let mut by_action_resource: HashMap<(&str, &str), Vec<&PolicyView>> = HashMap::new();
		for policy in &registry.policies {
			by_action_resource
				.entry((policy.action.as_str(), policy.resource.as_str()))
				.or_default()
				.push(policy);
		}
		let mut findings = Vec::new();
		for policies in by_action_resource.values() {
			let allows: Vec<&&PolicyView> = policies.iter().filter(|p| p.effect == "allow").collect();
			let denies: Vec<&&PolicyView> = policies.iter().filter(|p| p.effect == "deny").collect();
			for allow in &allows {
				for deny in &denies {
					let allow_subjects: HashSet<&str> = allow.subjects.iter().map(String::as_str).collect();
					let deny_subjects: HashSet<&str> = deny.subjects.iter().map(String::as_str).collect();
					let conflicts = allow_subjects.is_empty()
						|| deny_subjects.is_empty()
						|| !allow_subjects.is_disjoint(&deny_subjects);
					if conflicts {
						findings.push(Finding::error(
							self.name(),
							format!(
								"`{}` (allow) and `{}` (deny) share a subject on action={:?} resource={:?} — the same principal is both allowed and denied",
								allow.id, deny.id, allow.action, allow.resource
							),
							Some(allow.id.clone()),
						));
					}
				}
			}
		}
		findings
	}
}

impl VerifyPass for V8 { fn name(&self) -> &'static str { "V8" } fn run(&self, registry: &RegistryView) -> Vec<Finding> { registry.ports.iter().filter_map(|port| { let count = registry.drivers.iter().filter(|driver| driver.port == port.name).count(); (count < 2).then(|| Finding::error(self.name(), format!("port requires two driver manifests, found {count}"), Some(port.name.clone()))) }).collect() } }

pub fn standard_passes() -> Vec<Box<dyn VerifyPass>> { vec![Box::new(V1), Box::new(V2), Box::new(V3), Box::new(V4), Box::new(V5), Box::new(V6), Box::new(V7), Box::new(V8)] }
pub fn verify(registry: &RegistryView) -> Vec<Finding> { standard_passes().into_iter().flat_map(|pass| pass.run(registry)).collect() }

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn v8_requires_two_drivers_per_port() { let registry = RegistryView { ports: vec![PortView { name: "store".into() }], drivers: vec![DriverView { port: "store".into(), vendor: "memory".into(), version: "1".into(), lineage_support: "datasets".into(), ..Default::default() }], ..Default::default() }; assert_eq!(verify(&registry).iter().filter(|f| f.pass == "V8").count(), 1); }
	#[test]
	fn v5_requires_lineage_support() { let registry = RegistryView { drivers: vec![DriverView { port: "store".into(), vendor: "memory".into(), version: "1".into(), ..Default::default() }], ..Default::default() }; assert!(verify(&registry).iter().any(|f| f.pass == "V5")); }
	#[test]
	fn valid_catalog_has_no_findings() { let registry = RegistryView { policies: vec![PolicyView { id: "read".into(), action: "read".into(), resource: "orders".into(), effect: "allow".into(), purpose: None, ..Default::default() }], ports: vec![PortView { name: "store".into() }], drivers: vec![DriverView { port: "store".into(), vendor: "memory".into(), version: "1".into(), lineage_support: "datasets".into(), ..Default::default() }, DriverView { port: "store".into(), vendor: "remote".into(), version: "1".into(), lineage_support: "datasets".into(), ..Default::default() }] }; assert!(verify(&registry).is_empty()); }

	fn policy(id: &str, effect: &str, action: &str, resource: &str, subjects: &[&str]) -> PolicyView {
		PolicyView {
			id: id.into(),
			effect: effect.into(),
			action: action.into(),
			resource: resource.into(),
			subjects: subjects.iter().map(|s| s.to_string()).collect(),
			..Default::default()
		}
	}

	#[test]
	fn v3_flags_a_non_canonical_action() {
		let registry = RegistryView { policies: vec![policy("p1", "allow", "raed", "orders", &["Analyst"])], ..Default::default() };
		let findings = verify(&registry);
		assert!(findings.iter().any(|f| f.pass == "V3"), "expected a V3 finding, got: {findings:?}");
	}

	#[test]
	fn v3_accepts_every_canonical_action() {
		for action in CANONICAL_ACTIONS {
			let registry = RegistryView { policies: vec![policy("p1", "allow", action, "orders", &["Analyst"])], ..Default::default() };
			assert!(verify(&registry).iter().all(|f| f.pass != "V3"), "action {action:?} should not be a V3 finding");
		}
	}

	#[test]
	fn v4_flags_a_negated_relation_in_a_filter() {
		use nirdosha_guard_core::RelationExpr;
		let negated = FilterExpr::Not(Box::new(FilterExpr::RelationIn {
			field: vec!["owner".into()],
			relation: RelationExpr { name: "r".into(), source: "s".into(), max_cardinality: 1, ttl_seconds: 1 },
		}));
		let mut p = policy("p1", "allow", "read", "orders", &["Analyst"]);
		p.filter = Some(negated);
		let registry = RegistryView { policies: vec![p], ..Default::default() };
		assert!(verify(&registry).iter().any(|f| f.pass == "V4"));
	}

	#[test]
	fn v4_accepts_a_positive_relation_filter() {
		use nirdosha_guard_core::{RelationExpr};
		let positive = FilterExpr::RelationIn {
			field: vec!["owner".into()],
			relation: RelationExpr { name: "r".into(), source: "s".into(), max_cardinality: 1, ttl_seconds: 1 },
		};
		let mut p = policy("p1", "allow", "read", "orders", &["Analyst"]);
		p.filter = Some(positive);
		let registry = RegistryView { policies: vec![p], ..Default::default() };
		assert!(verify(&registry).iter().all(|f| f.pass != "V4"));
	}

	#[test]
	fn v7_flags_the_same_subject_allowed_and_denied_for_one_action_resource() {
		let registry = RegistryView {
			policies: vec![
				policy("allow-it", "allow", "update", "alert", &["Analyst"]),
				policy("deny-it", "deny", "update", "alert", &["Analyst"]),
			],
			..Default::default()
		};
		let findings = verify(&registry);
		assert!(findings.iter().any(|f| f.pass == "V7"), "expected a V7 finding, got: {findings:?}");
	}

	#[test]
	fn v7_does_not_flag_different_subjects_on_the_same_action_resource() {
		// The corpus's own real shape: svc:ingest is write-only,
		// Analyst reads the same resource — different subjects, no SoD
		// violation.
		let registry = RegistryView {
			policies: vec![
				policy("ingest-no-readback", "deny", "read", "transaction", &["SvcIngest"]),
				policy("analyst-search-transaction", "allow", "read", "transaction", &["Analyst"]),
			],
			..Default::default()
		};
		assert!(verify(&registry).iter().all(|f| f.pass != "V7"));
	}
}
