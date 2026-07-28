use std::time::Instant;

use sqlparser::ast::Statement;
use sqlparser::dialect::{
    ClickHouseDialect, DuckDbDialect, GenericDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
};
use sqlparser::parser::Parser;

use super::rules::{self, ReviewRule, RuleContext};
use super::types::*;

/// Run the rule engine against the given SQL, returning findings.
pub fn run_rule_engine(
    sql: &str,
    dialect: &str,
    database_type: crate::models::connection::DatabaseType,
    schema_info: Option<&rules::SchemaInfo>,
    settings: &RuleEngineSettings,
) -> (Vec<Finding>, u64) {
    let start = Instant::now();

    // Parse SQL into AST
    let normalized = normalize_dialect(dialect);
    let dialect_obj = resolve_dialect(normalized);
    let ast: Option<Vec<Statement>> = Parser::parse_sql(dialect_obj.as_ref(), sql).ok();

    let ctx = RuleContext {
        sql,
        dialect: normalized,
        database_type,
        ast: ast.as_ref(),
        schema_info,
        settings,
    };

    let mut findings = Vec::new();

    // Run all built-in rules
    let all_rules: Vec<Box<dyn ReviewRule>> = {
        let mut v: Vec<Box<dyn ReviewRule>> = Vec::new();
        v.extend(rules::safety::safety_rules());
        v.extend(rules::performance::performance_rules());
        v.extend(rules::correctness::correctness_rules());
        v
    };

    for rule in &all_rules {
        // Check if rule is disabled via overrides
        let enabled = settings
            .rule_overrides
            .get(rule.id())
            .copied()
            .unwrap_or_else(|| rule.default_enabled());
        if !enabled {
            continue;
        }

        // Check dialect compatibility
        let supported = rule.supported_dialects();
        if !supported.is_empty() && !supported.contains(&normalized) {
            continue;
        }

        let mut rule_findings = rule.check(&ctx);

        // Apply severity override
        if let Some(sev) = settings.severity_overrides.get(rule.id()) {
            for f in &mut rule_findings {
                f.severity = *sev;
            }
        }

        findings.extend(rule_findings);
    }

    // Run custom rules
    for custom in &settings.custom_rules {
        if !custom.enabled {
            continue;
        }
        if !custom.dialects.is_empty() && !custom.dialects.iter().any(|d| d == normalized) {
            continue;
        }
        if let Some(finding) = eval_custom_rule(custom, sql) {
            findings.push(finding);
        }
    }

    // Sort: Error > Warning > Info > Style
    findings.sort_by(|a, b| a.severity.cmp(&b.severity));

    let elapsed = start.elapsed().as_millis() as u64;
    (findings, elapsed)
}

/// Evaluate a custom rule against SQL text.
fn eval_custom_rule(rule: &CustomRule, sql: &str) -> Option<Finding> {
    let matched = match &rule.match_type {
        RuleMatchType::Regex { pattern } => {
            regex::Regex::new(pattern)
                .map(|re| re.is_match(sql))
                .unwrap_or(false)
        }
        RuleMatchType::Keyword { keywords, mode } => {
            let upper = sql.to_uppercase();
            match mode.as_str() {
                "all" => keywords.iter().all(|k| upper.contains(&k.to_uppercase())),
                _ => keywords.iter().any(|k| upper.contains(&k.to_uppercase())),
            }
        }
    };

    if matched {
        Some(Finding {
            id: format!("{}-{}", rule.id, uuid::Uuid::new_v4().as_simple()),
            rule_id: rule.id.clone(),
            source: FindingSource::RuleEngine,
            severity: rule.severity,
            category: rule.category,
            title: rule.name.clone(),
            detail: format!("Matched custom rule '{}'", rule.name),
            suggestion: None,
            span: None,
            auto_fixable: false,
            confidence: 1.0,
        })
    } else {
        None
    }
}

/// Normalize dialect string (mirrors sql_risk.rs logic).
fn normalize_dialect(dialect: &str) -> &'static str {
    match dialect.to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" | "redshift" | "opengauss" | "gaussdb" | "kingbase" | "highgo" | "vastbase" | "kwdb" => "postgres",
        "mysql" | "mariadb" | "doris" | "starrocks" | "manticoresearch" | "oceanbase" => "mysql",
        "sqlite" => "sqlite",
        "sqlserver" | "mssql" => "sqlserver",
        "clickhouse" => "clickhouse",
        "duckdb" => "duckdb",
        _ => "generic",
    }
}

/// Resolve dialect to sqlparser Dialect trait object.
fn resolve_dialect(dialect: &str) -> Box<dyn sqlparser::dialect::Dialect> {
    match dialect {
        "postgres" => Box::new(PostgreSqlDialect {}),
        "mysql" => Box::new(MySqlDialect {}),
        "sqlite" => Box::new(SQLiteDialect {}),
        "sqlserver" => Box::new(MsSqlDialect {}),
        "clickhouse" => Box::new(ClickHouseDialect {}),
        "duckdb" => Box::new(DuckDbDialect {}),
        _ => Box::new(GenericDialect {}),
    }
}
