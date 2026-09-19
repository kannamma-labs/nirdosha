//! Time-boxed, reviewable break-glass grants.

use serde::{Deserialize, Serialize};

use crate::FilterExpr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakGlassGrant {
    pub id: String,
    pub resource: String,
    pub scope: Option<FilterExpr>,
    pub reason: String,
    pub ticket: String,
    pub severity: u8,
    pub approvals: Vec<String>,
    pub required_approvals: u8,
    pub expires_at: u64,
    pub review_task_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakGlassError { MissingApproval, ExpiryTooLong, InvalidReason }

impl BreakGlassGrant {
    pub fn validate(&self, now: u64) -> Result<(), BreakGlassError> {
        if self.reason.trim().is_empty() || self.ticket.trim().is_empty() { return Err(BreakGlassError::InvalidReason); }
        if self.expires_at <= now || self.expires_at - now > 3600 { return Err(BreakGlassError::ExpiryTooLong); }
        if self.approvals.len() < self.required_approvals as usize { return Err(BreakGlassError::MissingApproval); }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakGlassReviewTask { pub grant_id: String, pub task_id: String, pub opened_at: u64, pub closed: bool }

#[derive(Debug, Default)]
pub struct BreakGlassLedger { grants: Vec<BreakGlassGrant>, reviews: Vec<BreakGlassReviewTask> }

impl BreakGlassLedger {
    pub fn issue(&mut self, grant: BreakGlassGrant, now: u64) -> Result<(), BreakGlassError> { grant.validate(now)?; self.reviews.push(BreakGlassReviewTask { grant_id: grant.id.clone(), task_id: grant.review_task_id.clone(), opened_at: now, closed: false }); self.grants.push(grant); Ok(()) }
    pub fn close_review(&mut self, task_id: &str) -> bool { self.reviews.iter_mut().find(|task| task.task_id == task_id).map(|task| { task.closed = true; true }).unwrap_or(false) }
    pub fn active(&self, resource: &str, now: u64) -> Vec<&BreakGlassGrant> { self.grants.iter().filter(|grant| grant.resource == resource && grant.expires_at > now).collect() }
    pub fn open_review_count(&self) -> usize { self.reviews.iter().filter(|task| !task.closed).count() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dual_approval_and_review_are_required() { let grant = BreakGlassGrant { id: "g".into(), resource: "payments".into(), scope: None, reason: "incident".into(), ticket: "INC-1".into(), severity: 1, approvals: vec!["one".into()], required_approvals: 2, expires_at: 100, review_task_id: "r".into() }; let mut ledger = BreakGlassLedger::default(); assert!(matches!(ledger.issue(grant.clone(), 1), Err(BreakGlassError::MissingApproval))); let mut approved = grant; approved.approvals.push("two".into()); ledger.issue(approved, 1).unwrap(); assert_eq!(ledger.open_review_count(), 1); assert!(ledger.close_review("r")); }
}
