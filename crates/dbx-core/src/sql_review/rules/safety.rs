use sqlparser::ast::{Expr, FromTable, SetExpr, Statement, Value, BinaryOperator};

use super::{make_finding, find_span, ReviewRule, RuleContext};
use crate::sql_review::types::*;

// ─── S001: UPDATE/DELETE without WHERE ───────────────────────────────────────

pub struct RuleNoWhereClause;

impl ReviewRule for RuleNoWhereClause {
    fn id(&self) -> &str { "S001" }
    fn name(&self) -> &str { "UPDATE/DELETE without WHERE" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Error }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            match stmt {
                Statement::Update(update) => {
                    if update.selection.is_none() {
                        let fragment = extract_statement_fragment(ctx.sql, "UPDATE");
                        findings.push(make_finding(
                            self,
                            "UPDATE 缺少 WHERE 条件".to_string(),
                            "此 UPDATE 将修改表中所有行。请添加 WHERE 子句限定范围。".to_string(),
                            Some("添加: WHERE <条件>".to_string()),
                            fragment.as_ref().and_then(|f| find_span(ctx.sql, f)),
                            false,
                        ));
                    }
                }
                Statement::Delete(delete) => {
                    if delete.selection.is_none() {
                        let fragment = extract_statement_fragment(ctx.sql, "DELETE");
                        findings.push(make_finding(
                            self,
                            "DELETE 缺少 WHERE 条件".to_string(),
                            "此 DELETE 将删除表中所有行。请添加 WHERE 子句限定范围。".to_string(),
                            Some("添加: WHERE <条件>".to_string()),
                            fragment.as_ref().and_then(|f| find_span(ctx.sql, f)),
                            false,
                        ));
                    }
                }
                _ => {}
            }
        }
        findings
    }
}

// ─── S002: Tautological WHERE (WHERE 1=1, WHERE TRUE) ───────────────────────

pub struct RuleTautologicalWhere;

impl ReviewRule for RuleTautologicalWhere {
    fn id(&self) -> &str { "S002" }
    fn name(&self) -> &str { "Tautological WHERE condition" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Error }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            let selection = match stmt {
                Statement::Update(update) => update.selection.as_ref(),
                Statement::Delete(delete) => delete.selection.as_ref(),
                _ => None,
            };
            if let Some(expr) = selection {
                if is_tautology(expr) {
                    findings.push(make_finding(
                        self,
                        "WHERE 条件恒为真".to_string(),
                        "WHERE 子句对所有行恒为 TRUE（如 WHERE 1=1、WHERE TRUE），等同于无 WHERE 条件。".to_string(),
                        Some("替换为有意义的过滤条件".to_string()),
                        find_span(ctx.sql, "WHERE"),
                        false,
                    ));
                }
            }
        }
        findings
    }
}

// ─── S003: TRUNCATE TABLE ────────────────────────────────────────────────────

pub struct RuleTruncate;

impl ReviewRule for RuleTruncate {
    fn id(&self) -> &str { "S003" }
    fn name(&self) -> &str { "TRUNCATE TABLE" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Error }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            if let Statement::Truncate { .. } = stmt {
                findings.push(make_finding(
                    self,
                    "TRUNCATE TABLE".to_string(),
                    "TRUNCATE 将删除所有行且在多数数据库中不可回滚。建议使用带 WHERE 的 DELETE 进行可逆操作。".to_string(),
                    Some("改用: DELETE FROM <表> WHERE <条件>".to_string()),
                    find_span(ctx.sql, "TRUNCATE"),
                    false,
                ));
            }
        }
        findings
    }
}

// ─── S004: DROP TABLE/DATABASE ───────────────────────────────────────────────

pub struct RuleDrop;

impl ReviewRule for RuleDrop {
    fn id(&self) -> &str { "S004" }
    fn name(&self) -> &str { "DROP statement" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Error }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            if let Statement::Drop { object_type, names, if_exists, .. } = stmt {
                let obj_str = format!("{:?}", object_type);
                let name_str = names.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
                findings.push(make_finding(
                    self,
                    format!("DROP {}", obj_str),
                    format!(
                        "正在删除{} '{}'，此操作不可逆{}。",
                        obj_str.to_lowercase(),
                        name_str,
                        if *if_exists { "（IF EXISTS 仅在对象不存在时抑制报错）" } else { "" }
                    ),
                    Some("请确保已有备份后再执行".to_string()),
                    find_span(ctx.sql, "DROP"),
                    false,
                ));
            }
        }
        findings
    }
}

// ─── S005: Multi-table DELETE/UPDATE (JOIN) ──────────────────────────────────

pub struct RuleMultiTableMutation;

impl ReviewRule for RuleMultiTableMutation {
    fn id(&self) -> &str { "S005" }
    fn name(&self) -> &str { "Multi-table UPDATE/DELETE with JOIN" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            match stmt {
                Statement::Delete(delete) => {
                    let from = match &delete.from {
                        FromTable::WithFromKeyword(exprs) | FromTable::WithoutKeyword(exprs) => exprs,
                    };
                    if from.len() > 1 || !delete.tables.is_empty() || delete.using.is_some() {
                        findings.push(make_finding(
                            self,
                            "多表 DELETE".to_string(),
                            "此 DELETE 涉及多张表，多表删除逻辑复杂且容易出错。".to_string(),
                            None,
                            find_span(ctx.sql, "DELETE"),
                            false,
                        ));
                    }
                }
                Statement::Update(update) => {
                    if update.from.is_some() || !update.table.joins.is_empty() {
                        findings.push(make_finding(
                            self,
                            "UPDATE 含 FROM/JOIN".to_string(),
                            "此 UPDATE 使用 FROM/JOIN 子句，基于其他表的值更新行。".to_string(),
                            None,
                            find_span(ctx.sql, "UPDATE"),
                            false,
                        ));
                    }
                }
                _ => {}
            }
        }
        findings
    }
}

// ─── S006: INSERT ... SELECT without LIMIT ───────────────────────────────────

pub struct RuleInsertSelectNoLimit;

impl ReviewRule for RuleInsertSelectNoLimit {
    fn id(&self) -> &str { "S006" }
    fn name(&self) -> &str { "INSERT ... SELECT without LIMIT" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            if let Statement::Insert(insert) = stmt {
                if let Some(query) = &insert.source {
                    if !matches!(query.body.as_ref(), SetExpr::Values(_)) {
                        findings.push(make_finding(
                            self,
                            "INSERT ... SELECT 缺少 LIMIT".to_string(),
                            "此 INSERT 从 SELECT 复制数据，若源表数据量大，可能插入远超预期的行数。".to_string(),
                            Some("添加 LIMIT 或预先确认源表行数".to_string()),
                            find_span(ctx.sql, "INSERT"),
                            false,
                        ));
                    }
                }
            }
        }
        findings
    }
}

// ─── S008: GRANT ALL / broad privilege grants ────────────────────────────────

pub struct RuleBroadGrant;

impl ReviewRule for RuleBroadGrant {
    fn id(&self) -> &str { "S008" }
    fn name(&self) -> &str { "Broad GRANT statement" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let upper = ctx.sql.to_uppercase();
        if upper.contains("GRANT ALL") || upper.contains("GRANT SUPER") || upper.contains("WITH GRANT OPTION") {
            findings.push(make_finding(
                self,
                "宽泛权限 GRANT".to_string(),
                "授予 ALL/SUPER 权限或 WITH GRANT OPTION 可能带来安全风险。".to_string(),
                Some("授予具体权限: 对指定表授予 SELECT、INSERT、UPDATE".to_string()),
                find_span(ctx.sql, "GRANT"),
                false,
            ));
        }
        findings
    }
}

// ─── S010: LOAD DATA / COPY file operations ──────────────────────────────────

pub struct RuleFileOperation;

impl ReviewRule for RuleFileOperation {
    fn id(&self) -> &str { "S010" }
    fn name(&self) -> &str { "File I/O operation" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let upper = ctx.sql.to_uppercase();
        if upper.contains("LOAD DATA") || (upper.contains("COPY") && upper.contains("FROM")) {
            findings.push(make_finding(
                self,
                "文件 I/O 操作".to_string(),
                "此语句将从服务器文件读取数据或向文件写入数据。".to_string(),
                None,
                None,
                false,
            ));
        }
        findings
    }
}

// ─── S007: INSERT without explicit column list ──────────────────────────────

pub struct RuleInsertWithoutColumns;

impl ReviewRule for RuleInsertWithoutColumns {
    fn id(&self) -> &str { "S007" }
    fn name(&self) -> &str { "INSERT without explicit column list" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            if let Statement::Insert(insert) = stmt {
                if insert.columns.is_empty() {
                    findings.push(make_finding(
                        self,
                        "INSERT 未指定列清单".to_string(),
                        "省略列清单依赖表的列顺序，表结构变化（增删列）后极易插入错位数据。".to_string(),
                        Some("显式列出目标列: INSERT INTO t (col1, col2) VALUES (...)".to_string()),
                        find_span(ctx.sql, "INSERT"),
                        false,
                    ));
                    break;
                }
            }
        }
        findings
    }
}

// ─── S009: ALTER TABLE DROP COLUMN ──────────────────────────────────────────

pub struct RuleDropColumn;

impl ReviewRule for RuleDropColumn {
    fn id(&self) -> &str { "S009" }
    fn name(&self) -> &str { "ALTER TABLE DROP COLUMN" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Error }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let upper = ctx.sql.to_uppercase();
        if upper.contains("ALTER TABLE") && upper.contains("DROP COLUMN") {
            findings.push(make_finding(
                self,
                "删除表列".to_string(),
                "DROP COLUMN 会永久移除列及其数据，依赖该列的查询、视图和程序将失效。".to_string(),
                Some("确认无依赖后再执行，并提前备份数据".to_string()),
                find_span(ctx.sql, "DROP COLUMN"),
                false,
            ));
        }
        findings
    }
}

// ─── S011: UPDATE/DELETE LIMIT without ORDER BY ─────────────────────────────

pub struct RuleMutationLimitWithoutOrderBy;

impl ReviewRule for RuleMutationLimitWithoutOrderBy {
    fn id(&self) -> &str { "S011" }
    fn name(&self) -> &str { "UPDATE/DELETE LIMIT without ORDER BY" }
    fn category(&self) -> FindingCategory { FindingCategory::Safety }
    fn default_severity(&self) -> Severity { Severity::Warning }

    fn check(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        let Some(stmts) = ctx.ast else { return findings };

        for stmt in stmts {
            let (has_limit, ordered) = match stmt {
                Statement::Update(u) => (u.limit.is_some(), !u.order_by.is_empty()),
                Statement::Delete(d) => (d.limit.is_some(), !d.order_by.is_empty()),
                _ => (false, false),
            };
            if has_limit && !ordered {
                findings.push(make_finding(
                    self,
                    "LIMIT 修改缺少 ORDER BY".to_string(),
                    "带 LIMIT 的 UPDATE/DELETE 在没有 ORDER BY 时影响的行是不确定的，重复执行结果不可预期。".to_string(),
                    Some("添加 ORDER BY 以固定被修改的行".to_string()),
                    find_span(ctx.sql, "LIMIT"),
                    false,
                ));
                break;
            }
        }
        findings
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn extract_statement_fragment(sql: &str, keyword: &str) -> Option<String> {
    let upper = sql.to_uppercase();
    if let Some(pos) = upper.find(keyword) {
        let end = (pos + 40).min(sql.len());
        Some(sql[pos..end].trim().to_string())
    } else {
        None
    }
}

fn is_tautology(expr: &Expr) -> bool {
    match expr {
        Expr::Value(v) => matches!(v.value, Value::Boolean(true)),
        Expr::BinaryOp { left, op: BinaryOperator::Eq, right } => left == right,
        Expr::Nested(inner) => is_tautology(inner),
        _ => false,
    }
}

/// Registry of all built-in safety rules.
pub fn safety_rules() -> Vec<Box<dyn ReviewRule>> {
    vec![
        Box::new(RuleNoWhereClause),
        Box::new(RuleTautologicalWhere),
        Box::new(RuleTruncate),
        Box::new(RuleDrop),
        Box::new(RuleMultiTableMutation),
        Box::new(RuleInsertSelectNoLimit),
        Box::new(RuleBroadGrant),
        Box::new(RuleFileOperation),
        Box::new(RuleInsertWithoutColumns),
        Box::new(RuleDropColumn),
        Box::new(RuleMutationLimitWithoutOrderBy),
    ]
}
