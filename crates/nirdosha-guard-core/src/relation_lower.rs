//! Relation lowering before a plan reaches a store driver.

use crate::{FilterExpr, RelationError, RelationExpr, ResolutionTier, ResolvedRelation};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweredRelation { Native(FilterExpr), InList(FilterExpr), Materialized(FilterExpr), Escalate }

pub fn lower_relation(field: Vec<String>, relation: &ResolvedRelation) -> Result<LoweredRelation, RelationError> {
    if matches!(relation.tier, ResolutionTier::Tier3DenyOrEscalate) { return Ok(LoweredRelation::Escalate); }
    let expression = match relation.tier {
        ResolutionTier::Tier0Native => FilterExpr::RelationIn { field, relation: RelationExpr { name: "native".into(), source: relation.freshness.source.clone(), max_cardinality: relation.values.len() as u64, ttl_seconds: relation.freshness.ttl_seconds } },
        ResolutionTier::Tier1InList => FilterExpr::In { field, values: relation.values.clone() },
        ResolutionTier::Tier2Materialized => FilterExpr::Eq { field, value: relation.values.first().cloned().unwrap_or(crate::Value::Null) },
        ResolutionTier::Tier3DenyOrEscalate => unreachable!(),
    };
    Ok(match relation.tier { ResolutionTier::Tier0Native => LoweredRelation::Native(expression), ResolutionTier::Tier1InList => LoweredRelation::InList(expression), ResolutionTier::Tier2Materialized => LoweredRelation::Materialized(expression), ResolutionTier::Tier3DenyOrEscalate => LoweredRelation::Escalate })
}

pub fn reject_negative_relation(expr: &FilterExpr) -> Result<(), RelationError> {
    if let FilterExpr::Not(inner) = expr { if matches!(inner.as_ref(), FilterExpr::RelationIn { .. }) { return Err(RelationError::Unresolvable); } }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Freshness, Value};
    fn relation(tier: ResolutionTier) -> ResolvedRelation { ResolvedRelation { values: vec![Value::Str("u".into())], freshness: Freshness { source: "rel".into(), source_epoch: "1".into(), resolved_at: "now".into(), ttl_seconds: 1 }, tier } }
    #[test]
    fn selects_resolution_tiers() { assert!(matches!(lower_relation(vec!["owner".into()], &relation(ResolutionTier::Tier1InList)).unwrap(), LoweredRelation::InList(_))); assert!(matches!(lower_relation(vec!["owner".into()], &relation(ResolutionTier::Tier3DenyOrEscalate)).unwrap(), LoweredRelation::Escalate)); }
    #[test]
    fn negative_relation_is_rejected() { let expr = FilterExpr::Not(Box::new(FilterExpr::RelationIn { field: vec!["owner".into()], relation: RelationExpr { name: "r".into(), source: "s".into(), max_cardinality: 1, ttl_seconds: 1 } })); assert!(reject_negative_relation(&expr).is_err()); }
}
