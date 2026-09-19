//! RDBMS SQL Emitter, Postgres RLS session binder, and DDL AST generator for RFC 0023 §9.3 & §9.5.

use crate::{FilterExpr, Tenant, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqlDialect {
    Postgres,
    Sqlite,
    GenericSql,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlPlan {
    pub where_clause: String,
    pub parameters: Vec<Value>,
    pub rls_session_settings: Vec<(String, String)>,
}

pub struct RdbmsEmitter;

impl RdbmsEmitter {
    /// Compiles `FilterExpr` into a parameterized SQL `WHERE` clause and Postgres RLS settings.
    pub fn compile_plan(
        expr: &FilterExpr,
        dialect: SqlDialect,
        tenant: &Tenant,
    ) -> SqlPlan {
        let mut params = Vec::new();
        let where_clause = Self::emit_where(expr, dialect, &mut params);

        let rls_session_settings = vec![
            ("app.tenant".into(), tenant.0.clone()),
            ("app.current_setting_mode".into(), "strict".into()),
        ];

        SqlPlan {
            where_clause,
            parameters: params,
            rls_session_settings,
        }
    }

    fn emit_where(
        expr: &FilterExpr,
        dialect: SqlDialect,
        params: &mut Vec<Value>,
    ) -> String {
        match expr {
            FilterExpr::Eq { field, value } => {
                params.push(value.clone());
                let idx = params.len();
                let placeholder = match dialect {
                    SqlDialect::Postgres => format!("${idx}"),
                    SqlDialect::Sqlite | SqlDialect::GenericSql => "?".into(),
                };
                format!("\"{}\" = {placeholder}", field.join("."))
            }
            FilterExpr::In { field, values } => {
                let mut placeholders = Vec::new();
                for val in values {
                    params.push(val.clone());
                    let idx = params.len();
                    placeholders.push(match dialect {
                        SqlDialect::Postgres => format!("${idx}"),
                        SqlDialect::Sqlite | SqlDialect::GenericSql => "?".into(),
                    });
                }
                format!(
                    "\"{}\" IN ({})",
                    field.join("."),
                    placeholders.join(", ")
                )
            }
            FilterExpr::TenantEq { value } => {
                params.push(value.clone());
                let idx = params.len();
                let placeholder = match dialect {
                    SqlDialect::Postgres => format!("${idx}"),
                    SqlDialect::Sqlite | SqlDialect::GenericSql => "?".into(),
                };
                format!("\"tenant_id\" = {placeholder}")
            }
            FilterExpr::And(children) => {
                let parts: Vec<String> = children
                    .iter()
                    .map(|child| Self::emit_where(child, dialect, params))
                    .collect();
                format!("({})", parts.join(" AND "))
            }
            FilterExpr::Or(children) => {
                let parts: Vec<String> = children
                    .iter()
                    .map(|child| Self::emit_where(child, dialect, params))
                    .collect();
                format!("({})", parts.join(" OR "))
            }
            FilterExpr::Not(inner) => {
                let inner_sql = Self::emit_where(inner, dialect, params);
                format!("NOT ({inner_sql})")
            }
            _ => "1=1".into(),
        }
    }
}

/// DDL AST with strict identifier quoting for delegated engines (§9.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DdlAst {
    CreateView {
        view_name: String,
        target_table: String,
        where_clause: String,
    },
    CreateColumnMask {
        mask_name: String,
        target_table: String,
        target_column: String,
        expression: String,
    },
    DropView {
        view_name: String,
    },
}

impl DdlAst {
    /// Safely renders DDL statement with strict identifier sanitization.
    pub fn render_ddl(&self) -> String {
        match self {
            DdlAst::CreateView {
                view_name,
                target_table,
                where_clause,
            } => {
                let clean_view = sanitize_ident(view_name);
                let clean_table = sanitize_ident(target_table);
                format!(
                    "CREATE OR REPLACE VIEW \"{clean_view}\" AS SELECT * FROM \"{clean_table}\" WHERE {where_clause};"
                )
            }
            DdlAst::CreateColumnMask {
                mask_name,
                target_table,
                target_column,
                expression,
            } => {
                let clean_mask = sanitize_ident(mask_name);
                let clean_table = sanitize_ident(target_table);
                let clean_col = sanitize_ident(target_column);
                format!(
                    "CREATE MASK \"{clean_mask}\" ON \"{clean_table}\"(\"{clean_col}\") AS {expression};"
                )
            }
            DdlAst::DropView { view_name } => {
                let clean_view = sanitize_ident(view_name);
                format!("DROP VIEW IF EXISTS \"{clean_view}\";")
            }
        }
    }
}

fn sanitize_ident(ident: &str) -> String {
    ident
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    #[test]
    fn emits_postgres_bound_parameters() {
        let expr = FilterExpr::Eq {
            field: vec!["status".into()],
            value: Value::Str("active".into()),
        };
        let plan = RdbmsEmitter::compile_plan(
            &expr,
            SqlDialect::Postgres,
            &Tenant("t1".into()),
        );
        assert_eq!(plan.where_clause, "\"status\" = $1");
        assert_eq!(plan.parameters, vec![Value::Str("active".into())]);
        assert_eq!(plan.rls_session_settings[0].1, "t1");
    }

    #[test]
    fn renders_sanitized_ddl_ast() {
        let ddl = DdlAst::CreateView {
            view_name: "ng_view_42; DROP TABLE customers;".into(),
            target_table: "customers".into(),
            where_clause: "\"tenant_id\" = 't1'".into(),
        };
        let sql = ddl.render_ddl();
        assert!(sql.contains("CREATE OR REPLACE VIEW \"ng_view_42DROPTABLEcustomers\""));
        assert!(!sql.contains("; DROP TABLE"));
    }
}
