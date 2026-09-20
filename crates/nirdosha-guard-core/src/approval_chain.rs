//! Real quorum-based `approval_chain!` runtime — Plan Phase 15.
//!
//! `Decision::Escalate { to: EscalateTarget::Approval { chain } }` has
//! been handled by `evaluator::evaluate` since Phase 2, and
//! `break_glass.rs`/`delegation.rs` (dual-approval grants, scoped
//! credentials) are genuinely `[DONE]` per the RFC 0026 checklist — but
//! neither is what `escalate to approval(chain sar_release)` names.
//! Nothing in the workspace resolved a *named chain* (`quorum(2, of =
//! [ComplianceLead]); timeout(deny)`) to a real quorum runtime before
//! this: `approval_chain!` (`nirdosha-guard-macros`) only ever registered
//! the block's raw source text into the `CATALOG` slice, never the
//! structured `ApprovalChainRecord` (`nirdosha-guard-registry`) the
//! `APPROVAL_CHAINS` slice exists to hold — so `RegistryDump.approval_chains`
//! was silently empty regardless of how many chains a crate declared, and
//! an escalated decision had nowhere real to go.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A registered chain's real requirements: how many distinct approvals,
/// from which roles. Built from a real `ApprovalChainRecord` (now
/// actually populated by the fixed `approval_chain!` macro), not
/// hand-constructed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalChainDefinition {
	pub name: String,
	pub quorum: u8,
	/// `of = [Role, Role, ...]` — an approval from *any* of these roles
	/// counts toward quorum (RFC's `policy_release`/`model_release`
	/// chains name two roles for exactly this reason: either a
	/// `PolicyEngineer` or a `ComplianceLead` approval counts, quorum is
	/// on distinct *approvers*, not on covering every listed role).
	pub approver_roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
	pub approver_id: String,
	pub role: String,
	pub approved_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EscalationOutcome {
	Approved,
	/// I3/`timeout(deny)`: the chain's own non-negotiable default — a
	/// quorum not reached by the deadline resolves to denied, never left
	/// pending indefinitely and never defaults to allowed.
	DeniedTimeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEscalation {
	pub id: String,
	pub chain: String,
	pub resource: String,
	pub subject_id: String,
	pub opened_at: u64,
	pub deadline: u64,
	pub approvals: Vec<Approval>,
	pub outcome: Option<EscalationOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationStatus {
	Pending { approvals_so_far: u8, quorum: u8 },
	Approved,
	DeniedTimeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalChainError {
	UnknownChain(String),
	/// A chain requires at least one real approver role — an empty
	/// `of = [...]` would make quorum unreachable by construction,
	/// exactly the kind of "declared but can never actually resolve"
	/// trap this runtime exists to prevent, not silently accept.
	ChainHasNoEligibleRoles(String),
	AlreadyOpen(String),
	NotFound(String),
	AlreadyResolved(String),
	RoleNotEligible { role: String, chain: String },
	DuplicateApprover { approver_id: String },
	Expired,
}

/// Real quorum-based escalation runtime: registers chain definitions,
/// opens/tracks/resolves pending escalations. Every state transition
/// (`open`/`approve`/`check_timeout`) is explicit and total — there is no
/// path from `Pending` to `Approved` except a real quorum of distinct,
/// role-eligible approvals, and no path out of `Pending` past `deadline`
/// except `DeniedTimeout` (I3: fail closed, never silently allow on
/// timeout).
#[derive(Debug, Default)]
pub struct ApprovalChainRuntime {
	definitions: HashMap<String, ApprovalChainDefinition>,
	pending: HashMap<String, PendingEscalation>,
}

impl ApprovalChainRuntime {
	pub fn new(definitions: Vec<ApprovalChainDefinition>) -> Self {
		Self { definitions: definitions.into_iter().map(|definition| (definition.name.clone(), definition)).collect(), pending: HashMap::new() }
	}

	pub fn definition(&self, chain: &str) -> Option<&ApprovalChainDefinition> {
		self.definitions.get(chain)
	}

	/// Opens a pending escalation for `chain`, idempotent per `id`
	/// (matching `IdempotencyStore`'s own "replaying the same trace_id is
	/// harmless" convention elsewhere in this workspace) — a caller
	/// re-opening with the same `id` gets `AlreadyOpen`, not a second,
	/// conflicting pending record.
	pub fn open(&mut self, id: impl Into<String>, chain: &str, resource: impl Into<String>, subject_id: impl Into<String>, now: u64, deadline: u64) -> Result<(), ApprovalChainError> {
		let id = id.into();
		if self.pending.contains_key(&id) {
			return Err(ApprovalChainError::AlreadyOpen(id));
		}
		let definition = self.definitions.get(chain).ok_or_else(|| ApprovalChainError::UnknownChain(chain.to_string()))?;
		if definition.approver_roles.is_empty() {
			return Err(ApprovalChainError::ChainHasNoEligibleRoles(chain.to_string()));
		}
		self.pending.insert(id.clone(), PendingEscalation { id, chain: chain.to_string(), resource: resource.into(), subject_id: subject_id.into(), opened_at: now, deadline, approvals: vec![], outcome: None });
		Ok(())
	}

	/// Records one approval. Fails closed on: unknown escalation, an
	/// already-resolved escalation, a role not in the chain's `of = [...]`
	/// list, the same approver approving twice (quorum counts *distinct*
	/// approvers, not approval count), or a call past the deadline (a
	/// late approval doesn't resurrect a timed-out escalation).
	pub fn approve(&mut self, id: &str, approver_id: impl Into<String>, role: &str, now: u64) -> Result<EscalationStatus, ApprovalChainError> {
		let approver_id = approver_id.into();
		let definition = {
			let pending = self.pending.get(id).ok_or_else(|| ApprovalChainError::NotFound(id.to_string()))?;
			self.definitions.get(&pending.chain).expect("a pending escalation always references a chain that existed at open() time").clone()
		};
		let pending = self.pending.get_mut(id).ok_or_else(|| ApprovalChainError::NotFound(id.to_string()))?;
		if pending.outcome.is_some() {
			return Err(ApprovalChainError::AlreadyResolved(id.to_string()));
		}
		if now >= pending.deadline {
			pending.outcome = Some(EscalationOutcome::DeniedTimeout);
			return Err(ApprovalChainError::Expired);
		}
		if !definition.approver_roles.iter().any(|eligible| eligible == role) {
			return Err(ApprovalChainError::RoleNotEligible { role: role.to_string(), chain: pending.chain.clone() });
		}
		if pending.approvals.iter().any(|approval| approval.approver_id == approver_id) {
			return Err(ApprovalChainError::DuplicateApprover { approver_id });
		}
		pending.approvals.push(Approval { approver_id, role: role.to_string(), approved_at: now });
		if pending.approvals.len() as u8 >= definition.quorum {
			pending.outcome = Some(EscalationOutcome::Approved);
			Ok(EscalationStatus::Approved)
		} else {
			Ok(EscalationStatus::Pending { approvals_so_far: pending.approvals.len() as u8, quorum: definition.quorum })
		}
	}

	/// I3/`timeout(deny)`: resolves a still-pending escalation past its
	/// deadline to `DeniedTimeout`. A caller (or a periodic sweep) drives
	/// this explicitly rather than the runtime polling a clock itself —
	/// the same "caller supplies `now`" determinism every other
	/// time-sensitive type in this workspace uses.
	pub fn check_timeout(&mut self, id: &str, now: u64) -> Result<EscalationStatus, ApprovalChainError> {
		let pending = self.pending.get_mut(id).ok_or_else(|| ApprovalChainError::NotFound(id.to_string()))?;
		if let Some(outcome) = &pending.outcome {
			return Ok(match outcome {
				EscalationOutcome::Approved => EscalationStatus::Approved,
				EscalationOutcome::DeniedTimeout => EscalationStatus::DeniedTimeout,
			});
		}
		if now >= pending.deadline {
			pending.outcome = Some(EscalationOutcome::DeniedTimeout);
			return Ok(EscalationStatus::DeniedTimeout);
		}
		let quorum = self.definitions.get(&pending.chain).map(|definition| definition.quorum).unwrap_or(0);
		Ok(EscalationStatus::Pending { approvals_so_far: pending.approvals.len() as u8, quorum })
	}

	pub fn status(&self, id: &str) -> Option<EscalationStatus> {
		let pending = self.pending.get(id)?;
		Some(match &pending.outcome {
			Some(EscalationOutcome::Approved) => EscalationStatus::Approved,
			Some(EscalationOutcome::DeniedTimeout) => EscalationStatus::DeniedTimeout,
			None => {
				let quorum = self.definitions.get(&pending.chain).map(|definition| definition.quorum).unwrap_or(0);
				EscalationStatus::Pending { approvals_so_far: pending.approvals.len() as u8, quorum }
			}
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sar_release() -> ApprovalChainDefinition {
		ApprovalChainDefinition { name: "sar_release".into(), quorum: 2, approver_roles: vec!["ComplianceLead".into()] }
	}

	#[test]
	fn open_rejects_an_unknown_chain() {
		let mut runtime = ApprovalChainRuntime::new(vec![]);
		let result = runtime.open("esc-1", "sar_release", "sar-42", "analyst-1", 0, 1000);
		assert_eq!(result, Err(ApprovalChainError::UnknownChain("sar_release".into())));
	}

	#[test]
	fn quorum_of_two_distinct_compliance_leads_resolves_to_approved() {
		let mut runtime = ApprovalChainRuntime::new(vec![sar_release()]);
		runtime.open("esc-1", "sar_release", "sar-42", "analyst-1", 0, 1000).unwrap();
		let after_first = runtime.approve("esc-1", "lead-a", "ComplianceLead", 10).unwrap();
		assert_eq!(after_first, EscalationStatus::Pending { approvals_so_far: 1, quorum: 2 });
		let after_second = runtime.approve("esc-1", "lead-b", "ComplianceLead", 20).unwrap();
		assert_eq!(after_second, EscalationStatus::Approved);
		assert_eq!(runtime.status("esc-1"), Some(EscalationStatus::Approved));
	}

	#[test]
	fn the_same_approver_twice_does_not_satisfy_quorum() {
		let mut runtime = ApprovalChainRuntime::new(vec![sar_release()]);
		runtime.open("esc-1", "sar_release", "sar-42", "analyst-1", 0, 1000).unwrap();
		runtime.approve("esc-1", "lead-a", "ComplianceLead", 10).unwrap();
		let result = runtime.approve("esc-1", "lead-a", "ComplianceLead", 20);
		assert_eq!(result, Err(ApprovalChainError::DuplicateApprover { approver_id: "lead-a".into() }));
		assert_eq!(runtime.status("esc-1"), Some(EscalationStatus::Pending { approvals_so_far: 1, quorum: 2 }), "a rejected duplicate must not silently count toward quorum");
	}

	#[test]
	fn a_role_outside_of_the_chains_eligible_list_is_rejected() {
		let mut runtime = ApprovalChainRuntime::new(vec![sar_release()]);
		runtime.open("esc-1", "sar_release", "sar-42", "analyst-1", 0, 1000).unwrap();
		let result = runtime.approve("esc-1", "someone", "Analyst", 10);
		assert_eq!(result, Err(ApprovalChainError::RoleNotEligible { role: "Analyst".into(), chain: "sar_release".into() }));
	}

	#[test]
	fn an_approval_from_either_eligible_role_counts_toward_a_two_role_chain() {
		let policy_release = ApprovalChainDefinition { name: "policy_release".into(), quorum: 2, approver_roles: vec!["PolicyEngineer".into(), "ComplianceLead".into()] };
		let mut runtime = ApprovalChainRuntime::new(vec![policy_release]);
		runtime.open("esc-1", "policy_release", "policy-9", "engineer-1", 0, 1000).unwrap();
		runtime.approve("esc-1", "eng-1", "PolicyEngineer", 10).unwrap();
		let status = runtime.approve("esc-1", "lead-1", "ComplianceLead", 20).unwrap();
		assert_eq!(status, EscalationStatus::Approved, "quorum(2, of=[PolicyEngineer, ComplianceLead]) must accept one of each, not require both from the same role");
	}

	#[test]
	fn timeout_denies_a_quorum_never_reached_and_a_late_approval_does_not_resurrect_it() {
		let mut runtime = ApprovalChainRuntime::new(vec![sar_release()]);
		runtime.open("esc-1", "sar_release", "sar-42", "analyst-1", 0, 100).unwrap();
		runtime.approve("esc-1", "lead-a", "ComplianceLead", 10).unwrap();
		let status = runtime.check_timeout("esc-1", 200).unwrap();
		assert_eq!(status, EscalationStatus::DeniedTimeout, "I3: a quorum not reached by the deadline must deny, never default to allow");

		let late = runtime.approve("esc-1", "lead-b", "ComplianceLead", 250);
		assert_eq!(late, Err(ApprovalChainError::AlreadyResolved("esc-1".into())), "a late approval must not resurrect a timed-out escalation");
	}

	#[test]
	fn approve_called_past_the_deadline_denies_even_without_a_prior_check_timeout_call() {
		let mut runtime = ApprovalChainRuntime::new(vec![sar_release()]);
		runtime.open("esc-1", "sar_release", "sar-42", "analyst-1", 0, 100).unwrap();
		let result = runtime.approve("esc-1", "lead-a", "ComplianceLead", 150);
		assert_eq!(result, Err(ApprovalChainError::Expired));
		assert_eq!(runtime.status("esc-1"), Some(EscalationStatus::DeniedTimeout), "an expired approve() call must itself resolve the escalation to denied, not leave it silently pending");
	}

	#[test]
	fn open_is_idempotent_per_id_and_rejects_a_chain_with_no_eligible_roles() {
		let mut runtime = ApprovalChainRuntime::new(vec![sar_release(), ApprovalChainDefinition { name: "broken".into(), quorum: 1, approver_roles: vec![] }]);
		runtime.open("esc-1", "sar_release", "sar-42", "analyst-1", 0, 1000).unwrap();
		assert_eq!(runtime.open("esc-1", "sar_release", "sar-42", "analyst-1", 0, 1000), Err(ApprovalChainError::AlreadyOpen("esc-1".into())));
		assert_eq!(runtime.open("esc-2", "broken", "r", "s", 0, 1000), Err(ApprovalChainError::ChainHasNoEligibleRoles("broken".into())));
	}
}
