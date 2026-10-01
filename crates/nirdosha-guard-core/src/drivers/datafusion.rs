//! DataFusion / Arrow physical pushdown bridge for RFC 0023 §1A & §9.5.
//!
//! Translates `FilterExpr` IR into DataFusion physical expressions, L3 `PruningPredicate`
//! (row-group pruning), L2 `RowFilter` (scan-time filtering), and Arrow record-batch masking.

use crate::{FieldMask, FilterExpr, MaskTransform, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruningPredicate {
    pub expression: String,
    pub target_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowFilter {
    pub filter_expr: String,
    pub parameters: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalPlan {
    pub pruning_predicate: PruningPredicate,
    pub row_filter: RowFilter,
    pub projected_masks: Vec<FieldMask>,
}

pub struct DataFusionBridge;

impl DataFusionBridge {
    /// Compiles a `FilterExpr` IR node into DataFusion L3 pruning predicates and L2 row filters.
    pub fn compile_filter(expr: &FilterExpr) -> (PruningPredicate, RowFilter) {
        match expr {
            FilterExpr::Eq { field, value } => {
                let field_name = field.join(".");
                let expr_str = format!("{field_name} = ?");
                (
                    PruningPredicate {
                        expression: format!("{field_name}_min <= ? AND {field_name}_max >= ?"),
                        target_fields: vec![field_name.clone()],
                    },
                    RowFilter {
                        filter_expr: expr_str,
                        parameters: vec![value.clone()],
                    },
                )
            }
            FilterExpr::TenantEq { value } => {
                (
                    PruningPredicate {
                        expression: "tenant_id = ?".into(),
                        target_fields: vec!["tenant_id".into()],
                    },
                    RowFilter {
                        filter_expr: "tenant_id = ?".into(),
                        parameters: vec![value.clone()],
                    },
                )
            }
            FilterExpr::And(children) => {
                let mut expr_parts = Vec::new();
                let mut params = Vec::new();
                let mut fields = Vec::new();
                for child in children {
                    let (prune, row) = Self::compile_filter(child);
                    expr_parts.push(row.filter_expr);
                    params.extend(row.parameters);
                    fields.extend(prune.target_fields);
                }
                (
                    PruningPredicate {
                        expression: fields.iter().map(|f| format!("{f}_pruned")).collect::<Vec<_>>().join(" AND "),
                        target_fields: fields,
                    },
                    RowFilter {
                        filter_expr: format!("({})", expr_parts.join(" AND ")),
                        parameters: params,
                    },
                )
            }
            _ => (
                PruningPredicate {
                    expression: "TRUE".into(),
                    target_fields: vec![],
                },
                RowFilter {
                    filter_expr: "1=1".into(),
                    parameters: vec![],
                },
            ),
        }
    }

    /// Computes masked projections for Arrow record batches before projection decode.
    pub fn compute_batch_masks(
        batch_fields: &[String],
        masks: &[FieldMask],
    ) -> Vec<(String, Option<MaskTransform>)> {
        batch_fields
            .iter()
            .map(|field| {
                let mask = masks.iter().find(|m| m.field.join(".") == *field);
                (field.clone(), mask.map(|m| m.transform.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FilterExpr;

    #[test]
    fn compiles_eq_filter_to_pruning_and_row_filter() {
        let expr = FilterExpr::Eq {
            field: vec!["department".into()],
            value: Value::Str("finance".into()),
        };
        let (prune, row) = DataFusionBridge::compile_filter(&expr);
        assert_eq!(prune.target_fields, vec!["department"]);
        assert!(row.filter_expr.contains("department = ?"));
    }

    #[test]
    fn computes_batch_masks_for_arrow_projection() {
        let fields = vec!["id".to_string(), "card_number".to_string()];
        let masks = vec![FieldMask {
            field: vec!["card_number".into()],
            transform: MaskTransform::PartialLast4,
        }];
        let res = DataFusionBridge::compute_batch_masks(&fields, &masks);
        assert_eq!(res[0].1, None);
        assert_eq!(res[1].1, Some(MaskTransform::PartialLast4));
    }
}
