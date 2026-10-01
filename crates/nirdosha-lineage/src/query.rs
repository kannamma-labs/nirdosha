//! Query vocabulary for `lineage_query!` (RFC 0026 §9).
//!
//! Phase 1 provides the *types only*: the row value space, the parameter
//! space, the query shape, and the runner trait the query-builder plumbing
//! will target. Guard-evaluated execution (`lineage_query_plan`) lands in
//! Phase 2 with the MIC; no query actually runs here yet.

use std::collections::BTreeMap;

use nirdosha_guard_core::Value;

use crate::{LineageEdge, NodeId, TraceId};

/// A field value in a view row. Closed set — no runtime string building.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineageValue {
    Node(NodeId),
    Text(String),
    Edge(LineageEdge),
}

/// A projected view row: field name → value.
pub type LineageQueryRow = BTreeMap<String, LineageValue>;

/// Value of a view constructor parameter (`start`, `depth`, `trace_id`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineageParamValue {
    Node(NodeId),
    Depth(u32),
    Trace(TraceId),
}

/// Types a view parameter accepts. Local trait over foreign types keeps the
/// closed set total at macro level (a param of an unregistered type is a
/// compile error in the impl resolution).
pub trait IntoLineageParam {
    fn into_param(self) -> LineageParamValue;
}

impl IntoLineageParam for NodeId {
    fn into_param(self) -> LineageParamValue {
        LineageParamValue::Node(self)
    }
}

impl IntoLineageParam for u32 {
    fn into_param(self) -> LineageParamValue {
        LineageParamValue::Depth(self)
    }
}

impl IntoLineageParam for TraceId {
    fn into_param(self) -> LineageParamValue {
        LineageParamValue::Trace(self)
    }
}

/// Types a view `[filter]` setter accepts: lowered to guard-core `Value`.
pub trait IntoLineageFilter {
    fn into_value(self) -> Value;
}

impl IntoLineageFilter for String {
    fn into_value(self) -> Value {
        Value::Str(self)
    }
}

impl IntoLineageFilter for &str {
    fn into_value(self) -> Value {
        Value::Str(self.to_owned())
    }
}

/// Build one typed `FilterExpr` leaf for a view field. The macro layer emits
/// these so query filters travel the same IR every other predicate does
/// (RFC 0023 §5 — no string-level query rewriting).
pub fn filter_eq(field: &'static str, value: impl IntoLineageFilter) -> crate::FilterExpr {
    crate::FilterExpr::Eq { field: vec![field.to_owned()], value: value.into_value() }
}

/// Normalize a view parameter at construction time.
pub fn param(value: impl IntoLineageParam) -> LineageParamValue {
    value.into_param()
}

/// Everything a view's `execute` hands to the guard: view name, bound
/// parameters, accumulated filters. Transparent data, fully inspectable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageQueryShape {
    pub view: &'static str,
    pub params: Vec<LineageParamValue>,
    pub filters: Vec<crate::FilterExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineageQueryError {
    /// Guard denied the lineage_query action for this subject/purpose.
    GuardDenied { reason: String },
    /// The view's start node is not a catalog entity (should be a compile
    /// error via macro-time catalog check; this is the runtime backstop).
    UnknownNode { catalog_id: String },
    /// The boundary depth/cap was hit; the result is a stated watermark.
    CapHit { remaining_depth: u32 },
}

impl std::fmt::Display for LineageQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LineageQueryError::GuardDenied { reason } => {
                write!(f, "lineage query denied by guard: {reason}")
            }
            LineageQueryError::UnknownNode { catalog_id } => {
                write!(f, "lineage query references unknown catalog node: {catalog_id}")
            }
            LineageQueryError::CapHit { remaining_depth } => {
                write!(f, "lineage query hit a cap at depth {remaining_depth}")
            }
        }
    }
}

impl std::error::Error for LineageQueryError {}

/// The Phase-2 runner: executes a query shape against the projected graph
/// through the guard. Implemented by the guard client's query-plan surface
/// when it lands (`ctx.guard.lineage_query_plan(...)`); Phase 1 defines the
/// contract so generated code type-checks today.
pub trait LineageQueryRunner {
    fn run(&self, shape: &LineageQueryShape) -> Result<Vec<LineageQueryRow>, LineageQueryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_normalizes_by_type() {
        let node = NodeId::data("txn_events".to_owned(), None);
        assert_eq!(param(node.clone()), LineageParamValue::Node(node));
        assert_eq!(param(6u32), LineageParamValue::Depth(6));
        assert_eq!(
            param("01JD8WQ7".to_owned()),
            LineageParamValue::Trace("01JD8WQ7".to_owned())
        );
    }

    #[test]
    fn filter_eq_lowers_to_typed_ir() {
        let f = filter_eq("policy_version", "2025.11.4");
        match f {
            crate::FilterExpr::Eq { field, value } => {
                assert_eq!(field, vec!["policy_version".to_owned()]);
                assert_eq!(value, Value::Str("2025.11.4".to_owned()));
            }
            other => panic!("expected Eq leaf, got {other:?}"),
        }
    }
}
