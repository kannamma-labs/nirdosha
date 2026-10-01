//! RDBMS SQL Emitter, Postgres RLS session binder, and DDL AST generator for RFC 0023 §9.3 & §9.5.

use crate::{CompareOp, FilterExpr, PatternMatcher, Tenant, Value};
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
                // Column is `tenant`, not `tenant_id` — matching
                // `guard_entities(resource, tenant, policy_version,
                // payload)`, the actual schema
                // `nirdosha-guard-store-postgres` creates. This was wrong
                // from `"tenant_id"` until a real `SELECT` actually ran
                // it (Plan Phase 7's `PostgresStoreDriver::query`) —
                // `prepare()` only ever validated this method's output
                // was well-formed, never executed it, and no unit test
                // here exercised the `TenantEq` branch at all.
                format!("\"tenant\" = {placeholder}")
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
            FilterExpr::Compare { field, op, value } => {
                params.push(value.clone());
                let idx = params.len();
                let placeholder = Self::placeholder(dialect, idx);
                let op_sql = match op {
                    CompareOp::Lt => "<",
                    CompareOp::Le => "<=",
                    CompareOp::Gt => ">",
                    CompareOp::Ge => ">=",
                };
                format!("\"{}\" {op_sql} {placeholder}", field.join("."))
            }
            FilterExpr::TimeRange { field, from, to } => {
                params.push(Value::Str(from.clone()));
                let from_placeholder = Self::placeholder(dialect, params.len());
                params.push(Value::Str(to.clone()));
                let to_placeholder = Self::placeholder(dialect, params.len());
                let column = format!("\"{}\"", field.join("."));
                format!("({column} >= {from_placeholder} AND {column} <= {to_placeholder})")
            }
            FilterExpr::Pattern { field, matcher } => {
                let (like_value, needs_like) = match matcher {
                    PatternMatcher::Exact(value) => (value.clone(), false),
                    PatternMatcher::Prefix(value) => (format!("{}%", escape_like_literal(value)), true),
                    PatternMatcher::Glob(value) => (glob_to_like(value), true),
                };
                params.push(Value::Str(like_value));
                let placeholder = Self::placeholder(dialect, params.len());
                let column = format!("\"{}\"", field.join("."));
                if needs_like {
                    format!("{column} LIKE {placeholder} ESCAPE '\\'")
                } else {
                    format!("{column} = {placeholder}")
                }
            }
            // Relations are erased at plan-compile time (RFC 0023 §4) — one must
            // never reach a driver. Failing loudly here catches a bug upstream
            // instead of silently under-filtering with an unfiltered scan.
            FilterExpr::RelationIn { .. } => {
                unreachable!("RelationIn must be erased before reaching a driver, RFC 0023 §4")
            }
        }
    }

    fn placeholder(dialect: SqlDialect, idx: usize) -> String {
        match dialect {
            SqlDialect::Postgres => format!("${idx}"),
            SqlDialect::Sqlite | SqlDialect::GenericSql => "?".into(),
        }
    }
}

/// Escapes literal `%`, `_`, and `\` in a value that will be embedded in a
/// `LIKE ... ESCAPE '\'` clause, so a stored literal never acts as a wildcard.
fn escape_like_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            other => out.push(other),
        }
    }
    out
}

/// Translates a `*`/`?` glob pattern into a `LIKE`-safe string, escaping any
/// literal `%`/`_`/`\` in the source first so they can't be mistaken for the
/// wildcards they're being translated into.
fn glob_to_like(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    for ch in pattern.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            '*' => out.push('%'),
            '?' => out.push('_'),
            other => out.push(other),
        }
    }
    out
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
    fn emits_tenant_eq_against_the_real_column_name() {
        // Regression test: this emitted "tenant_id" (no such column)
        // instead of "tenant" (the real one in
        // nirdosha-guard-store-postgres's `guard_entities` table) until a
        // real SELECT actually ran it — nothing here exercised this
        // branch before.
        let expr = FilterExpr::TenantEq { value: Value::Str("tenant-alpha".into()) };
        let plan = RdbmsEmitter::compile_plan(&expr, SqlDialect::Postgres, &Tenant("t1".into()));
        assert_eq!(plan.where_clause, "\"tenant\" = $1");
        assert_eq!(plan.parameters, vec![Value::Str("tenant-alpha".into())]);
    }

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
    fn emits_compare_operator() {
        let expr = FilterExpr::Compare {
            field: vec!["amount".into()],
            op: CompareOp::Ge,
            value: Value::Int(10_000),
        };
        let plan = RdbmsEmitter::compile_plan(&expr, SqlDialect::Postgres, &Tenant("t1".into()));
        assert_eq!(plan.where_clause, "\"amount\" >= $1");
        assert_eq!(plan.parameters, vec![Value::Int(10_000)]);
    }

    #[test]
    fn emits_time_range_with_both_bounds_parameterized() {
        let expr = FilterExpr::TimeRange {
            field: vec!["occurred_at".into()],
            from: "2026-01-01T00:00:00Z".into(),
            to: "2026-02-01T00:00:00Z".into(),
        };
        let plan = RdbmsEmitter::compile_plan(&expr, SqlDialect::Postgres, &Tenant("t1".into()));
        assert_eq!(plan.where_clause, "(\"occurred_at\" >= $1 AND \"occurred_at\" <= $2)");
        assert_eq!(
            plan.parameters,
            vec![
                Value::Str("2026-01-01T00:00:00Z".into()),
                Value::Str("2026-02-01T00:00:00Z".into()),
            ]
        );
    }

    #[test]
    fn emits_exact_pattern_as_equality() {
        let expr = FilterExpr::Pattern {
            field: vec!["narrative".into()],
            matcher: PatternMatcher::Exact("wire transfer".into()),
        };
        let plan = RdbmsEmitter::compile_plan(&expr, SqlDialect::Postgres, &Tenant("t1".into()));
        assert_eq!(plan.where_clause, "\"narrative\" = $1");
        assert_eq!(plan.parameters, vec![Value::Str("wire transfer".into())]);
    }

    #[test]
    fn emits_prefix_pattern_as_escaped_like() {
        let expr = FilterExpr::Pattern {
            field: vec!["narrative".into()],
            matcher: PatternMatcher::Prefix("50% off_deal".into()),
        };
        let plan = RdbmsEmitter::compile_plan(&expr, SqlDialect::Postgres, &Tenant("t1".into()));
        assert_eq!(plan.where_clause, "\"narrative\" LIKE $1 ESCAPE '\\'");
        // The literal `%` and `_` in the source value must not act as wildcards.
        assert_eq!(plan.parameters, vec![Value::Str("50\\% off\\_deal%".into())]);
    }

    #[test]
    fn emits_glob_pattern_translated_to_like() {
        let expr = FilterExpr::Pattern {
            field: vec!["narrative".into()],
            matcher: PatternMatcher::Glob("cash*deposit?".into()),
        };
        let plan = RdbmsEmitter::compile_plan(&expr, SqlDialect::Postgres, &Tenant("t1".into()));
        assert_eq!(plan.where_clause, "\"narrative\" LIKE $1 ESCAPE '\\'");
        assert_eq!(plan.parameters, vec![Value::Str("cash%deposit_".into())]);
    }

    #[test]
    fn glob_pattern_escapes_literal_wildcard_lookalikes() {
        let expr = FilterExpr::Pattern {
            field: vec!["narrative".into()],
            matcher: PatternMatcher::Glob("100%_match".into()),
        };
        let plan = RdbmsEmitter::compile_plan(&expr, SqlDialect::Postgres, &Tenant("t1".into()));
        assert_eq!(plan.parameters, vec![Value::Str("100\\%\\_match".into())]);
    }

    #[test]
    #[should_panic(expected = "RelationIn must be erased")]
    fn relation_in_reaching_a_driver_panics_instead_of_under_filtering() {
        let expr = FilterExpr::RelationIn {
            field: vec!["counterparty_id".into()],
            relation: crate::RelationExpr {
                name: "related_parties".into(),
                source: "core_kyc".into(),
                max_cardinality: 100,
                ttl_seconds: 300,
            },
        };
        let _ = RdbmsEmitter::compile_plan(&expr, SqlDialect::Postgres, &Tenant("t1".into()));
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
