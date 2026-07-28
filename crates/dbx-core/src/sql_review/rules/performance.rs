use sqlparser::ast::{visit_expressions, Expr, Query, Select, SelectItem, SetExpr, Statement, TableFactor};
use std::ops::ControlFlow;

use super::{make_finding, find_span, ReviewRule, RuleContext};
use crate::sql_review::types::*;

// ─── P001: SELECT * ──────────────────────────────────────────────────────────

pub struct RuleSelectStar;

impl ReviewRule for RuleSelectStar {
    fn id(&self) -> &str { "P001" }
    fn name(&self) -> &str { "SELECT *" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Info }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            if let Statement::Query(query) = stmt {
                if select_has_wildcard(query) {
                    findings.push(make_finding(
                        self,
                        "使用了 SELECT *".to_string(),
                        "SELECT * 会检索所有列，可能包含无用数据并阻止覆盖索引扫描。".to_string(),
                        Some("将 * 替换为明确的列名列表".to_string()),
                        find_span(ctx.sql, "*"),
                        true,
                    ));
                    break;
                }
            }
        }
        findings
    }
}

// ─── P002: Large table query without LIMIT ───────────────────────────────────

pub struct RuleNoLimit;

impl ReviewRule for RuleNoLimit {
    fn id(&self) -> &str { "P002" }
    fn name(&self) -> &str { "Query without LIMIT on large table" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };
        let Some(schema) = ctx.schema_info else { return findings };

        for stmt in stmts {
            if let Statement::Query(query) = stmt {
                if !query_has_limit(query) {
                    let tables = extract_table_names_from_query(query);
                    for table_name in &tables {
                        if let Some(ts) = schema.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table_name)) {
                            if let Some(rows) = ts.estimated_rows {
                                if rows > ctx.settings.large_table_threshold {
                                    findings.push(make_finding(
                                        self,
                                        format!("大表 '{}' 缺少 LIMIT", ts.name),
                                        format!("表 '{}' 约有 {} 行，缺少 LIMIT 可能返回超大结果集。", ts.name, rows),
                                        Some("添加: LIMIT 1000".to_string()),
                                        find_span(ctx.sql, &ts.name),
                                        true,
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        findings
    }
}

// ─── P004: Function wrapping indexed column ──────────────────────────────────

pub struct RuleFunctionOnColumn;

impl ReviewRule for RuleFunctionOnColumn {
    fn id(&self) -> &str { "P004" }
    fn name(&self) -> &str { "Function applied to column in WHERE" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let upper = ctx.sql.to_uppercase();

        let patterns = ["YEAR(", "MONTH(", "DATE(", "LOWER(", "UPPER(", "SUBSTRING(", "CAST("];
        for pat in &patterns {
            if upper.contains(pat) && upper.contains("WHERE") {
                findings.push(make_finding(
                    self,
                    format!("WHERE 中对列使用了函数 {}", pat.trim_end_matches('(')),
                    "在 WHERE 中对列施加函数会导致索引失效。".to_string(),
                    Some("改写: WHERE col >= '2024-01-01' AND col < '2024-02-01'".to_string()),
                    find_span(ctx.sql, pat),
                    false,
                ));
                break;
            }
        }
        findings
    }
}

// ─── P007: NOT IN subquery ───────────────────────────────────────────────────

pub struct RuleNotInSubquery;

impl ReviewRule for RuleNotInSubquery {
    fn id(&self) -> &str { "P007" }
    fn name(&self) -> &str { "NOT IN with subquery" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let upper = ctx.sql.to_uppercase();
        if upper.contains("NOT IN") && upper.contains("SELECT") {
            findings.push(make_finding(
                self,
                "NOT IN 子查询".to_string(),
                "NOT IN 搭配子查询存在 NULL 安全问题且性能通常较差，建议使用 NOT EXISTS。".to_string(),
                Some("改写: WHERE NOT EXISTS (SELECT 1 FROM ... WHERE ...)".to_string()),
                find_span(ctx.sql, "NOT IN"),
                false,
            ));
        }
        findings
    }
}

// ─── P008: Cartesian product ─────────────────────────────────────────────────

pub struct RuleCartesianProduct;

impl ReviewRule for RuleCartesianProduct {
    fn id(&self) -> &str { "P008" }
    fn name(&self) -> &str { "Possible Cartesian product" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Error }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            if let Statement::Query(query) = stmt {
                if let SetExpr::Select(select) = query.body.as_ref() {
                    let from = &select.from;
                    if from.len() > 1 {
                        let has_where = select.selection.is_some();
                        let has_join_cond = from.iter().any(|t| !t.joins.is_empty());
                        if !has_where && !has_join_cond {
                            findings.push(make_finding(
                                self,
                                "可能的笛卡尔积".to_string(),
                                format!("查询引用了 {} 张表但缺少 WHERE 或 JOIN 条件。", from.len()),
                                Some("添加 JOIN ... ON 或 WHERE 条件".to_string()),
                                find_span(ctx.sql, "FROM"),
                                false,
                            ));
                        }
                    }
                }
            }
        }
        findings
    }
}

// ─── P009: LIKE '%xxx' leading wildcard ──────────────────────────────────────

pub struct RuleLeadingWildcardLike;

impl ReviewRule for RuleLeadingWildcardLike {
    fn id(&self) -> &str { "P009" }
    fn name(&self) -> &str { "LIKE with leading wildcard" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Info }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let upper = ctx.sql.to_uppercase();
        if upper.contains("LIKE '%") || upper.contains("LIKE N'%") {
            findings.push(make_finding(
                self,
                "LIKE 前置通配符".to_string(),
                "以 '%' 开头的 LIKE 模式无法使用 B-tree 索引，将导致全表扫描。".to_string(),
                None,
                find_span(ctx.sql, "LIKE"),
                false,
            ));
        }
        findings
    }
}

// ─── P010: Too many JOINs ────────────────────────────────────────────────────

pub struct RuleTooManyJoins;

impl ReviewRule for RuleTooManyJoins {
    fn id(&self) -> &str { "P010" }
    fn name(&self) -> &str { "Excessive JOINs" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Info }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            if let Statement::Query(query) = stmt {
                if let SetExpr::Select(select) = query.body.as_ref() {
                    let join_count = count_joins(select);
                    if join_count > ctx.settings.max_join_tables {
                        findings.push(make_finding(
                            self,
                            format!("查询包含 {} 个 JOIN", join_count),
                            format!("此查询关联了 {} 张表（阈值: {}）。", join_count, ctx.settings.max_join_tables),
                            None,
                            None,
                            false,
                        ));
                    }
                }
            }
        }
        findings
    }
}

// ─── P003: SELECT DISTINCT ──────────────────────────────────────────────────

pub struct RuleSelectDistinct;

impl ReviewRule for RuleSelectDistinct {
    fn id(&self) -> &str { "P003" }
    fn name(&self) -> &str { "SELECT DISTINCT" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Info }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            if let Statement::Query(query) = stmt {
                if let SetExpr::Select(select) = query.body.as_ref() {
                    if select.distinct.is_some() {
                        findings.push(make_finding(
                            self,
                            "使用了 SELECT DISTINCT".to_string(),
                            "DISTINCT 需要额外的排序/哈希去重开销，常被用来掩盖 JOIN 产生的重复行。".to_string(),
                            Some("优先检查 JOIN 条件是否正确，必要时再使用 DISTINCT".to_string()),
                            find_span(ctx.sql, "DISTINCT"),
                            false,
                        ));
                        break;
                    }
                }
            }
        }
        findings
    }
}

// ─── P005: ORDER BY RAND()/NEWID() ──────────────────────────────────────────

pub struct RuleRandomOrderBy;

impl ReviewRule for RuleRandomOrderBy {
    fn id(&self) -> &str { "P005" }
    fn name(&self) -> &str { "ORDER BY RAND()/NEWID()" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let upper = ctx.sql.to_uppercase();
        if upper.contains("ORDER BY") && (upper.contains("RAND()") || upper.contains("NEWID()")) {
            findings.push(make_finding(
                self,
                "随机排序取样".to_string(),
                "ORDER BY RAND()/NEWID() 需要对全表计算随机数并排序，大表上开销极大。".to_string(),
                Some("改用 TABLESAMPLE 或按主键随机偏移取样".to_string()),
                find_span(ctx.sql, "ORDER BY"),
                false,
            ));
        }
        findings
    }
}

// ─── P006: Very long IN list ────────────────────────────────────────────────

pub struct RuleLongInList;

impl ReviewRule for RuleLongInList {
    fn id(&self) -> &str { "P006" }
    fn name(&self) -> &str { "Very long IN list" }
    fn category(&self) -> FindingCategory { FindingCategory::Performance }
    fn default_severity(&self) -> Severity { Severity::Info }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        const THRESHOLD: usize = 100;
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            let result = visit_expressions(stmt, |expr| {
                if let Expr::InList { list, .. } = expr {
                    if list.len() > THRESHOLD {
                        return ControlFlow::Break(list.len());
                    }
                }
                ControlFlow::Continue(())
            });
            if let ControlFlow::Break(n) = result {
                findings.push(make_finding(
                    self,
                    format!("IN 列表包含 {} 个字面量", n),
                    format!("超长 IN 列表（阈值 {}）会增大解析与执行开销，且可能超出数据库限制。", THRESHOLD),
                    Some("改用临时表 + JOIN 或分批处理".to_string()),
                    find_span(ctx.sql, "IN"),
                    false,
                ));
                break;
            }
        }
        findings
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn select_has_wildcard(query: &Query) -> bool {
    if let SetExpr::Select(select) = query.body.as_ref() {
        select.projection.iter().any(|item| matches!(item, SelectItem::Wildcard(_)))
    } else {
        false
    }
}

fn query_has_limit(query: &Query) -> bool {
    query.limit_clause.is_some()
}

fn extract_table_names_from_query(query: &Query) -> Vec<String> {
    let mut names = Vec::new();
    if let SetExpr::Select(select) = query.body.as_ref() {
        for table_with_joins in &select.from {
            if let TableFactor::Table { name, .. } = &table_with_joins.relation {
                names.push(name.to_string());
            }
            for join in &table_with_joins.joins {
                if let TableFactor::Table { name, .. } = &join.relation {
                    names.push(name.to_string());
                }
            }
        }
    }
    names
}

fn count_joins(select: &Select) -> usize {
    select.from.iter().map(|t| t.joins.len()).sum()
}

/// Registry of all built-in performance rules.
pub fn performance_rules() -> Vec<Box<dyn ReviewRule>> {
    vec![
        Box::new(RuleSelectStar),
        Box::new(RuleNoLimit),
        Box::new(RuleFunctionOnColumn),
        Box::new(RuleNotInSubquery),
        Box::new(RuleCartesianProduct),
        Box::new(RuleLeadingWildcardLike),
        Box::new(RuleTooManyJoins),
        Box::new(RuleSelectDistinct),
        Box::new(RuleRandomOrderBy),
        Box::new(RuleLongInList),
    ]
}
