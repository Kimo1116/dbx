pub mod ai_reviewer;
pub mod engine;
pub mod rules;
pub mod types;

pub use types::*;

use std::time::Instant;

/// Run a complete SQL review (rule engine + optional AI).
///
/// This is the main entry point for the SQL review module.
/// Returns a complete `SqlReviewReport`.
pub fn run_review(
    sql: &str,
    dialect: &str,
    database_type: crate::models::connection::DatabaseType,
    connection_id: Option<&str>,
    database: Option<&str>,
    schema_info: Option<&rules::SchemaInfo>,
    settings: &ReviewSettings,
    ai_response: Option<&str>, // Pre-fetched AI response (if AI was called externally)
) -> SqlReviewReport {
    // 1. Run rule engine
    let (rule_findings, rule_elapsed) = engine::run_rule_engine(
        sql,
        dialect,
        database_type,
        schema_info,
        &settings.rule_engine,
    );

    // 2. Parse AI findings if response provided
    let (ai_findings, ai_elapsed, ai_parse_note) = if let Some(response) = ai_response {
        let ai_start = Instant::now();
        let (findings, note) = ai_reviewer::parse_ai_findings(
            response,
            settings.ai_review.confidence_threshold,
        );
        let elapsed = ai_start.elapsed().as_millis() as u64;
        (findings, Some(elapsed), note)
    } else {
        (Vec::new(), None, None)
    };

    // 3. Merge and deduplicate
    let findings = merge_findings(rule_findings, ai_findings);

    // 4. Determine verdict
    let verdict = determine_verdict(&findings);

    // 5. Build context snapshot
    let context = ReviewContext {
        schema_snapshot: schema_info.map(|s| format_schema_snapshot(s)),
        row_count_hints: schema_info
            .map(|s| {
                s.tables
                    .iter()
                    .map(|t| TableRowCountHint {
                        table_name: t.name.clone(),
                        estimated_rows: t.estimated_rows,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        is_production: false, // Caller should set this
        connection_env: "unknown".to_string(),
    };

    SqlReviewReport {
        id: uuid::Uuid::new_v4().to_string(),
        sql: sql.to_string(),
        dialect: dialect.to_string(),
        connection_id: connection_id.map(|s| s.to_string()),
        database: database.map(|s| s.to_string()),
        findings,
        verdict,
        rule_engine_elapsed_ms: rule_elapsed,
        ai_elapsed_ms: ai_elapsed,
        ai_parse_note,
        reviewed_at: chrono::Utc::now().timestamp(),
        context,
    }
}

/// Merge rule engine and AI findings, deduplicating similar items.
fn merge_findings(mut rule_findings: Vec<Finding>, ai_findings: Vec<Finding>) -> Vec<Finding> {
    // Simple dedup: skip AI findings whose title is very similar to a rule finding
    for ai_f in ai_findings {
        let is_duplicate = rule_findings.iter().any(|rf| {
            title_similarity(&rf.title, &ai_f.title) > 0.7
        });
        if !is_duplicate {
            rule_findings.push(ai_f);
        }
    }

    // Sort: Error > Warning > Info > Style, then by category
    rule_findings.sort_by(|a, b| {
        a.severity.cmp(&b.severity).then(a.category.cmp(&b.category))
    });

    rule_findings
}

/// Determine overall verdict from findings.
fn determine_verdict(findings: &[Finding]) -> ReviewVerdict {
    if findings.iter().any(|f| f.severity == Severity::Error) {
        ReviewVerdict::Block
    } else if findings.iter().any(|f| f.severity == Severity::Warning) {
        ReviewVerdict::Warn
    } else {
        ReviewVerdict::Pass
    }
}

/// Simple title similarity (Jaccard on word sets).
fn title_similarity(a: &str, b: &str) -> f32 {
    let words_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let words_b: std::collections::HashSet<&str> = b.split_whitespace().collect();
    if words_a.is_empty() || words_b.is_empty() {
        return 0.0;
    }
    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();
    intersection as f32 / union as f32
}

/// Format schema info as DDL text for AI context.
fn format_schema_snapshot(schema: &rules::SchemaInfo) -> String {
    schema
        .tables
        .iter()
        .map(|t| {
            let cols: Vec<String> = t
                .columns
                .iter()
                .map(|c| format!("  {} {}{}", c.name, c.data_type, if c.nullable { " NULL" } else { " NOT NULL" }))
                .collect();
            let indexes: Vec<String> = t
                .indexes
                .iter()
                .map(|i| format!("  INDEX {} ({})", i.name, i.columns.join(", ")))
                .collect();
            let mut ddl = format!("CREATE TABLE {} (\n{}\n)", t.name, cols.join(",\n"));
            if !indexes.is_empty() {
                ddl.push_str(&format!(";\n{}", indexes.join(";\n")));
            }
            ddl
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comprehensive_sql() -> String {
        let in_list: Vec<String> = (1..=101).map(|i| i.to_string()).collect();
        format!(
            r#"
UPDATE users SET status = 'inactive';
DELETE FROM logs;
DELETE FROM orders WHERE 1=1;
TRUNCATE TABLE temp_cache;
DROP TABLE obsolete_backup;
UPDATE orders o JOIN order_items oi ON o.id = oi.order_id SET o.status = 'archived' WHERE o.created_at < '2020-01-01';
INSERT INTO archive_orders (id, user_id) SELECT id, user_id FROM orders WHERE status = 'closed';
INSERT INTO audit_log VALUES (NULL, 'system');
GRANT ALL PRIVILEGES ON *.* TO 'app_user';
ALTER TABLE customers DROP COLUMN legacy_flag;
-- LOAD DATA INFILE '/tmp/data.csv' INTO TABLE staging;
SELECT 1;
UPDATE sessions SET expired = 1 WHERE last_active < '2024-01-01' LIMIT 1000;
DELETE FROM queue_jobs WHERE processed = 1 LIMIT 500;
SELECT * FROM products;
SELECT DISTINCT category FROM products;
SELECT id FROM events WHERE YEAR(created_at) = 2024;
SELECT id FROM banners ORDER BY RAND() LIMIT 5;
SELECT id FROM users WHERE id NOT IN (SELECT user_id FROM banned_users);
SELECT a.id, b.id FROM table_a a, table_b b;
SELECT id FROM articles WHERE title LIKE '%database%';
SELECT o.id FROM t1 JOIN t2 ON t1.id=t2.a JOIN t3 ON t2.id=t3.b JOIN t4 ON t3.id=t4.c JOIN t5 ON t4.id=t5.d JOIN t6 ON t5.id=t6.e JOIN t7 ON t6.id=t7.f;
SELECT id FROM users WHERE deleted_at = NULL;
SELECT id, CASE WHEN status = 1 THEN 'active' WHEN status = 2 THEN 'paused' END AS s FROM accounts;
SELECT id, total / quantity AS avg_price FROM sales;
SELECT id FROM big WHERE id IN ({ins});
"#,
            ins = in_list.join(", ")
        )
    }

    #[test]
    fn diagnose_parse() {
        let sql = comprehensive_sql();
        let dialect = sqlparser::dialect::MySqlDialect {};
        for (i, stmt) in sql.split(';').enumerate() {
            let trimmed = stmt.trim();
            if trimmed.is_empty() {
                continue;
            }
            let one = format!("{};", trimmed);
            match sqlparser::parser::Parser::parse_sql(&dialect, &one) {
                Ok(_) => println!("[OK]   #{}: {}", i, first_line(trimmed)),
                Err(e) => println!("[FAIL] #{}: {} -> {}", i, first_line(trimmed), e),
            }
        }
    }

    fn first_line(s: &str) -> String {
        s.lines().next().unwrap_or("").chars().take(70).collect()
    }

    #[test]
    fn comprehensive_rules_fire() {
        let sql = comprehensive_sql();
        let settings = ReviewSettings::default();
        let report = run_review(
            &sql,
            "mysql",
            crate::models::connection::DatabaseType::Mysql,
            None,
            None,
            None,
            &settings,
            None,
        );

        let mut ids: Vec<String> = report.findings.iter().map(|f| f.rule_id.clone()).collect();
        ids.sort();
        ids.dedup();
        println!("FIRED RULES: {:?}", ids);

        let expected = [
            "S001", "S002", "S003", "S004", "S005", "S006", "S007", "S008", "S009", "S010", "S011",
            "P001", "P003", "P004", "P005", "P006", "P007", "P008", "P009", "P010",
            "C001", "C003", "C004",
        ];
        let missing: Vec<&str> = expected.iter().copied().filter(|e| !ids.iter().any(|id| id == e)).collect();
        assert!(missing.is_empty(), "rules did not fire: {:?} (fired: {:?})", missing, ids);
    }
}
