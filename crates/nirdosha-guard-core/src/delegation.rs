//! Scoped, expiring delegation credentials.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{Action, Destination, FilterExpr, Purpose};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialScope {
    pub subject_id: String,
    pub agent_id: String,
    pub entity: String,
    pub action: Action,
    pub filter: Option<FilterExpr>,
    pub purpose: Purpose,
    pub destination: Destination,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegatedCredential { pub id: String, pub scope: CredentialScope, pub issued_at: u64, pub revoked: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegationError { ScopeMismatch, Expired, Revoked, TtlTooLong }

pub trait DelegationStore {
    fn put(&mut self, credential: DelegatedCredential);
    fn get(&self, id: &str) -> Option<&DelegatedCredential>;
    fn revoke(&mut self, id: &str) -> bool;
    fn remove_expired(&mut self, now: u64) -> usize;
}

#[derive(Debug, Default)]
pub struct InMemoryDelegationStore { credentials: HashMap<String, DelegatedCredential> }

impl DelegationStore for InMemoryDelegationStore {
    fn put(&mut self, credential: DelegatedCredential) { self.credentials.insert(credential.id.clone(), credential); }
    fn get(&self, id: &str) -> Option<&DelegatedCredential> { self.credentials.get(id) }
    fn revoke(&mut self, id: &str) -> bool { self.credentials.get_mut(id).map(|item| { item.revoked = true; true }).unwrap_or(false) }
    fn remove_expired(&mut self, now: u64) -> usize { let before = self.credentials.len(); self.credentials.retain(|_, item| item.scope.expires_at > now && !item.revoked); before - self.credentials.len() }
}

#[derive(Debug)]
pub struct DelegationRegistry<S = InMemoryDelegationStore> { pub store: S, next_id: u64, pub max_ttl_seconds: u64 }

impl<S: DelegationStore> DelegationRegistry<S> {
    pub fn new(store: S) -> Self { Self { store, next_id: 0, max_ttl_seconds: 3600 } }
    pub fn mint(&mut self, scope: CredentialScope, now: u64) -> Result<DelegatedCredential, DelegationError> {
        if scope.expires_at <= now { return Err(DelegationError::Expired); }
        if scope.expires_at - now > self.max_ttl_seconds { return Err(DelegationError::TtlTooLong); }
        let credential = DelegatedCredential { id: format!("delegation-{}", self.next_id), scope, issued_at: now, revoked: false };
        self.next_id += 1; self.store.put(credential.clone()); Ok(credential)
    }
    pub fn authorize(&self, id: &str, requested: &CredentialScope, now: u64) -> Result<(), DelegationError> {
        let credential = self.store.get(id).ok_or(DelegationError::Revoked)?;
        if credential.revoked { return Err(DelegationError::Revoked); }
        if credential.scope.expires_at <= now { return Err(DelegationError::Expired); }
        if &credential.scope != requested { return Err(DelegationError::ScopeMismatch); }
        Ok(())
    }
    pub fn revoke(&mut self, id: &str) -> bool { self.store.revoke(id) }
    pub fn sweep_expired(&mut self, now: u64) -> usize { self.store.remove_expired(now) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope(expires_at: u64) -> CredentialScope { CredentialScope { subject_id: "u".into(), agent_id: "a".into(), entity: "e".into(), action: Action::Read, filter: None, purpose: Purpose("p".into()), destination: Destination::LlmContext, expires_at } }
    #[test]
    fn scope_is_exact_and_expiry_is_enforced() { let mut registry = DelegationRegistry::new(InMemoryDelegationStore::default()); let wanted = scope(10); let token = registry.mint(wanted.clone(), 1).unwrap(); assert!(registry.authorize(&token.id, &wanted, 2).is_ok()); assert!(matches!(registry.authorize(&token.id, &scope(11), 2), Err(DelegationError::ScopeMismatch))); assert!(matches!(registry.mint(scope(4002), 1), Err(DelegationError::TtlTooLong))); }
}
