//! Request-level decision and parameterized filter caches.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::{Action, Cap, Decision, DecisionCacheKey, FieldMask, FilterExpr, FilterFragment, Value};

#[derive(Debug, Clone)]
pub struct CachedDecision {
    pub decision: Decision,
    pub obligations: Vec<crate::Obligation>,
    /// `residual_filter`/`caps`/`masks` were dropped on every cache hit
    /// until now (`get` only ever returned `(decision, obligations)`,
    /// forcing every caller to substitute `residual_filter: None` and no
    /// caps/masks) — silently unsound for a cacheable action whose plan
    /// actually depends on any of the three: a tenant-scoped write would
    /// be a strange case to cache (writes already aren't cacheable, see
    /// `insert`'s doc comment) but a *cacheable read* needing its
    /// `RowCap`/masks enforced on the second, cache-hit request would
    /// silently lose them. Found wiring Plan Phase 7 (the read path
    /// actually consumes caps/masks for the first time), fixed here
    /// rather than reproduced.
    pub residual_filter: Option<FilterExpr>,
    pub caps: Vec<Cap>,
    pub masks: Vec<FieldMask>,
    inserted_at: Instant,
}

#[derive(Debug)]
pub struct DecisionCache {
    map: HashMap<DecisionCacheKey, CachedDecision>,
    ttl: Duration,
}

impl DecisionCache {
    pub fn new(ttl: Duration) -> Self { Self { map: HashMap::new(), ttl } }

    pub fn get(&mut self, key: &DecisionCacheKey) -> Option<CachedDecision> {
        let cached = self.map.get(key)?;
        if cached.inserted_at.elapsed() > self.ttl {
            self.map.remove(key);
            return None;
        }
        Some(cached.clone())
    }

    /// Only cache decisions whose request shape is safe to reuse. Writes,
    /// exports, and restricted reads remain uncached by contract.
    /// `Delegate` joins that list — minting a delegation token is a
    /// stateful, non-idempotent action (RFC 0023 §9.4/I11), not a read;
    /// `Aggregate`/`LineageQuery`/`Simulate` stay cacheable — all three are
    /// read-only by the RFCs that define them (I9 aggregate leak control,
    /// RFC 0026 §9's lineage views, and `policy_simulation!`'s own "results
    /// are read-only").
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &mut self,
        key: DecisionCacheKey,
        decision: Decision,
        obligations: Vec<crate::Obligation>,
        residual_filter: Option<FilterExpr>,
        caps: Vec<Cap>,
        masks: Vec<FieldMask>,
        classification: crate::Classification,
    ) -> bool {
        if matches!(key.action, Action::Create | Action::Update | Action::Delete | Action::Migrate | Action::Export | Action::Delegate)
            || classification >= crate::Classification::Restricted
        {
            return false;
        }
        self.map.insert(key, CachedDecision { decision, obligations, residual_filter, caps, masks, inserted_at: Instant::now() });
        true
    }

    pub fn invalidate_policy(&mut self, version: &str) {
        self.map.retain(|key, _| key.policy_version != version);
    }

    pub fn clear(&mut self) { self.map.clear(); }
}

#[derive(Debug, Default)]
pub struct FragmentCache {
    map: HashMap<u64, FilterFragment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentError {
    UnknownFragment(u64),
    MissingParameter(String),
}

impl FragmentCache {
    pub fn insert(&mut self, hash: u64, fragment: FilterFragment) { self.map.insert(hash, fragment); }

    pub fn bind(&self, hash: u64, params: &HashMap<String, Value>) -> Result<FilterExpr, FragmentError> {
        let fragment = self.map.get(&hash).ok_or(FragmentError::UnknownFragment(hash))?;
        bind_expr(&fragment.expr, params)
    }
}

fn bind_expr(expr: &FilterExpr, params: &HashMap<String, Value>) -> Result<FilterExpr, FragmentError> {
    let bind_value = |value: &Value| -> Result<Value, FragmentError> {
        match value {
            Value::Str(name) if name.starts_with('$') => params.get(&name[1..]).cloned().ok_or_else(|| FragmentError::MissingParameter(name[1..].to_owned())),
            other => Ok(other.clone()),
        }
    };
    Ok(match expr {
        FilterExpr::Eq { field, value } => FilterExpr::Eq { field: field.clone(), value: bind_value(value)? },
        FilterExpr::In { field, values } => FilterExpr::In { field: field.clone(), values: values.iter().map(bind_value).collect::<Result<_, _>>()? },
        FilterExpr::Compare { field, op, value } => FilterExpr::Compare { field: field.clone(), op: op.clone(), value: bind_value(value)? },
        FilterExpr::And(items) => FilterExpr::And(items.iter().map(|item| bind_expr(item, params)).collect::<Result<_, _>>()?),
        FilterExpr::Or(items) => FilterExpr::Or(items.iter().map(|item| bind_expr(item, params)).collect::<Result<_, _>>()?),
        FilterExpr::Not(item) => FilterExpr::Not(Box::new(bind_expr(item, params)?)),
        FilterExpr::TimeRange { field, from, to } => FilterExpr::TimeRange { field: field.clone(), from: from.clone(), to: to.clone() },
        FilterExpr::Pattern { field, matcher } => FilterExpr::Pattern { field: field.clone(), matcher: matcher.clone() },
        FilterExpr::TenantEq { value } => FilterExpr::TenantEq { value: bind_value(value)? },
        FilterExpr::RelationIn { field, relation } => FilterExpr::RelationIn { field: field.clone(), relation: relation.clone() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Destination, Purpose, Tenant};

    fn key(action: Action) -> DecisionCacheKey {
        DecisionCacheKey { subject_id: "u".into(), roles_hash: 1, tenant: Tenant("t".into()), entity: "e".into(), dataset: "d".into(), action, environment_hash: 1, destination: Destination::Browser, time_bucket: "b".into(), query_shape_hash: 1, purpose: Purpose("p".into()), policy_version: "v".into() }
    }

    #[test]
    fn unsafe_decisions_are_not_cached() {
        let mut cache = DecisionCache::new(Duration::from_secs(60));
        assert!(!cache.insert(key(Action::Update), Decision::Allow, vec![], None, vec![], vec![], crate::Classification::Internal));
        assert!(!cache.insert(key(Action::Read), Decision::Allow, vec![], None, vec![], vec![], crate::Classification::Restricted));
    }

    #[test]
    fn cache_hit_preserves_filter_caps_and_masks() {
        let mut cache = DecisionCache::new(Duration::from_secs(60));
        let filter = FilterExpr::TenantEq { value: Value::Str("t1".into()) };
        let caps = vec![crate::Cap::RowCap(50)];
        let masks = vec![crate::FieldMask { field: vec!["salary".into()], transform: crate::MaskTransform::Full }];
        assert!(cache.insert(key(Action::Read), Decision::Allow, vec![], Some(filter.clone()), caps.clone(), masks.clone(), crate::Classification::Internal));
        let hit = cache.get(&key(Action::Read)).expect("cache hit");
        assert_eq!(hit.residual_filter, Some(filter));
        assert_eq!(hit.caps, caps);
        assert_eq!(hit.masks, masks);
    }

    #[test]
    fn fragments_bind_named_values_recursively() {
        let mut cache = FragmentCache::default();
        cache.insert(7, FilterFragment { expr: FilterExpr::Eq { field: vec!["tenant".into()], value: Value::Str("$tenant".into()) }, parameters: vec!["tenant".into()] });
        let mut params = HashMap::new();
        params.insert("tenant".into(), Value::Str("t1".into()));
        assert_eq!(cache.bind(7, &params).unwrap(), FilterExpr::Eq { field: vec!["tenant".into()], value: Value::Str("t1".into()) });
    }
}
