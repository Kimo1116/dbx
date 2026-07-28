use sqlparser::ast::{visit_expressions, Expr, BinaryOperator, FunctionArg, FunctionArgExpr, FunctionArguments, Statement, Value};
use std::ops::ControlFlow;

use super::{make_finding, find_span, ReviewRule, RuleContext};
use crate::sql_review::types::*;

// ─── C001: NULL comparison with = instead of IS NULL ─────────────────────────

pub struct RuleNullEquality;

impl ReviewRule for RuleNullEquality {
    fn id(&self) -> &str { "C001" }
    fn name(&self) -> &str { "NULL comparison with =" }
    fn category(&self) -> FindingCategory { FindingCategory::Correctness }
    fn default_severity(&self) -> Severity { Severity::Error }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            if has_null_equality(stmt) {
                findings.push(make_finding(
                    self,
                    "使用 = 与 NULL 比较".to_string(),
                    "使用 = 或 != 与 NULL 比较结果永远为 UNKNOWN。应使用 IS NULL 或 IS NOT NULL。".to_string(),
                    Some("替换: col = NULL -> col IS NULL".to_string()),
                    find_span(ctx.sql, "NULL"),
                    true,
                ));
                break;
            }
        }
        findings
    }
}

// ─── C004: Division without NULLIF protection ────────────────────────────────

pub struct RuleDivisionByZero;

impl ReviewRule for RuleDivisionByZero {
    fn id(&self) -> &str { "C004" }
    fn name(&self) -> &str { "Division without zero protection" }
    fn category(&self) -> FindingCategory { FindingCategory::Correctness }
    fn default_severity(&self) -> Severity { Severity::Info }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let upper = ctx.sql.to_uppercase();

        if upper.contains('/') && upper.contains("SELECT") && !upper.contains("NULLIF") {
            if upper.contains("/ 0") || has_division_pattern(ctx.sql) {
                findings.push(make_finding(
                    self,
                    "潜在的除零错误".to_string(),
                    "除法未使用 NULLIF 保护，当分母为零时会报错。".to_string(),
                    Some("使用: 分子 / NULLIF(分母, 0)".to_string()),
                    None,
                    false,
                ));
            }
        }
        findings
    }
}

// ─── C002: COUNT(nullable column) ignores NULLs ─────────────────────────────

pub struct RuleCountNullableColumn;

impl ReviewRule for RuleCountNullableColumn {
    fn id(&self) -> &str { "C002" }
    fn name(&self) -> &str { "COUNT on nullable column" }
    fn category(&self) -> FindingCategory { FindingCategory::Correctness }
    fn default_severity(&self) -> Severity { Severity::Info }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };
        let Some(schema) = ctx.schema_info else { return findings };

        for stmt in stmts {
            let result = visit_expressions(stmt, |expr| {
                if let Expr::Function(func) = expr {
                    if func.name.to_string().eq_ignore_ascii_case("COUNT") {
                        if let FunctionArguments::List(arg_list) = &func.args {
                            if let Some(FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Identifier(ident)))) = arg_list.args.first() {
                                let col = ident.value.clone();
                                let nullable = schema.tables.iter().any(|t| {
                                    t.columns.iter().any(|c| c.name.eq_ignore_ascii_case(&col) && c.nullable)
                                });
                                if nullable {
                                    return ControlFlow::Break(col);
                                }
                            }
                        }
                    }
                }
                ControlFlow::Continue(())
            });
            if let ControlFlow::Break(col) = result {
                findings.push(make_finding(
                    self,
                    format!("COUNT({}) 会忽略 NULL", col),
                    format!("列 '{}' 允许为空，COUNT({}) 不统计 NULL 行，结果可能与预期行数不一致。", col, col),
                    Some("如需统计所有行请使用 COUNT(*)".to_string()),
                    find_span(ctx.sql, "COUNT"),
                    false,
                ));
                break;
            }
        }
        findings
    }
}

// ─── C003: CASE without ELSE ────────────────────────────────────────────────

pub struct RuleCaseWithoutElse;

impl ReviewRule for RuleCaseWithoutElse {
    fn id(&self) -> &str { "C003" }
    fn name(&self) -> &str { "CASE without ELSE" }
    fn category(&self) -> FindingCategory { FindingCategory::Correctness }
    fn default_severity(&self) -> Severity { Severity::Info }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            let result = visit_expressions(stmt, |expr| {
                if let Expr::Case { else_result, .. } = expr {
                    if else_result.is_none() {
                        return ControlFlow::Break(());
                    }
                }
                ControlFlow::Continue(())
            });
            if result.is_break() {
                findings.push(make_finding(
                    self,
                    "CASE 缺少 ELSE 分支".to_string(),
                    "未匹配任何 WHEN 条件时 CASE 返回 NULL，可能引入意外的空值。".to_string(),
                    Some("添加 ELSE 分支明确默认值".to_string()),
                    find_span(ctx.sql, "CASE"),
                    false,
                ));
                break;
            }
        }
        findings
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn has_null_equality(stmt: &Statement) -> bool {
    let result = visit_expressions(stmt, |expr| {
        if let Expr::BinaryOp { left, op, right } = expr {
            let is_eq_or_neq = matches!(op, BinaryOperator::Eq | BinaryOperator::NotEq);
            if is_eq_or_neq {
                let left_is_null = matches!(left.as_ref(), Expr::Value(v) if matches!(v.value, Value::Null));
                let right_is_null = matches!(right.as_ref(), Expr::Value(v) if matches!(v.value, Value::Null));
                if left_is_null || right_is_null {
                    return ControlFlow::Break(());
                }
            }
        }
        ControlFlow::Continue(())
    });
    result.is_break()
}

fn has_division_pattern(sql: &str) -> bool {
    let chars: Vec<char> = sql.chars().collect();
    for i in 1..chars.len().saturating_sub(1) {
        if chars[i] == '/' && chars[i - 1] != '*' && chars[i + 1] != '*' {
            let before: String = chars[..i].iter().collect();
            let before_trimmed = before.trim_end();
            if before_trimmed.ends_with(|c: char| c.is_alphanumeric() || c == ')' || c == '_') {
                return true;
            }
        }
    }
    false
}

/// Registry of all built-in correctness rules.
pub fn correctness_rules() -> Vec<Box<dyn ReviewRule>> {
    vec![
        Box::new(RuleNullEquality),
        Box::new(RuleDivisionByZero),
        Box::new(RuleCountNullableColumn),
        Box::new(RuleCaseWithoutElse),
    ]
}
