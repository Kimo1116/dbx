pub mod correctness;
pub mod performance;
pub mod safety;

use sqlparser::ast::Statement;

use crate::models::connection::DatabaseType;

use super::types::*;

/// Context passed to each rule during evaluation.
pub struct RuleContext<'a> {
    pub sql: &'a str,
    pub dialect: &'a str,
    pub database_type: DatabaseType,
    /// Parsed AST statements (None if parsing failed)
    pub ast: Option<&'a Vec<Statement>>,
    /// Schema information (table structures, indexes) if available
    pub schema_info: Option<&'a SchemaInfo>,
    /// Engine settings
    pub settings: &'a RuleEngineSettings,
}

/// Schema information for context-aware rules.
#[derive(Debug, Clone, Default)]
pub struct SchemaInfo {
    pub tables: Vec<TableSchema>,
}

#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnSchema>,
    pub indexes: Vec<IndexSchema>,
    pub estimated_rows: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ColumnSchema {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone)]
pub struct IndexSchema {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

/// Trait for a review rule.
pub trait ReviewRule: Send + Sync {
    /// Unique rule ID, e.g. "S001"
    fn id(&self) -> &str;
    /// Human-readable name
    fn name(&self) -> &str;
    /// Category
    fn category(&self) -> FindingCategory;
    /// Default severity
    fn default_severity(&self) -> Severity;
    /// Whether enabled by default
    fn default_enabled(&self) -> bool {
        true
    }
    /// Supported dialects (empty = all)
    fn supported_dialects(&self) -> &[&str] {
        &[]
    }
    /// Run the check, returning any findings.
    fn check(&self, ctx: &RuleContext) -> Vec<Finding>;
}

/// Serializable metadata describing a built-in rule, for rendering a settings catalog.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuleMeta {
    pub id: String,
    pub name: String,
    pub category: FindingCategory,
    pub severity: Severity,
    pub default_enabled: bool,
    /// Supported dialects (empty = all)
    pub supported_dialects: Vec<String>,
}

/// Enumerate metadata for all built-in rules.
pub fn list_builtin_rules() -> Vec<RuleMeta> {
    let mut v: Vec<Box<dyn ReviewRule>> = Vec::new();
    v.extend(safety::safety_rules());
    v.extend(performance::performance_rules());
    v.extend(correctness::correctness_rules());
    v.iter()
        .map(|r| RuleMeta {
            id: r.id().to_string(),
            name: r.name().to_string(),
            category: r.category(),
            severity: r.default_severity(),
            default_enabled: r.default_enabled(),
            supported_dialects: r.supported_dialects().iter().map(|s| s.to_string()).collect(),
        })
        .collect()
}

/// Helper to create a Finding from a rule.
pub fn make_finding(
    rule: &dyn ReviewRule,
    title: String,
    detail: String,
    suggestion: Option<String>,
    span: Option<SqlSpan>,
    auto_fixable: bool,
) -> Finding {
    Finding {
        id: format!("{}-{}", rule.id(), uuid::Uuid::new_v4().as_simple()),
        rule_id: rule.id().to_string(),
        source: FindingSource::RuleEngine,
        severity: rule.default_severity(),
        category: rule.category(),
        title,
        detail,
        suggestion,
        span,
        auto_fixable,
        confidence: 1.0,
    }
}

/// Find the line/col position of a fragment in the SQL text.
pub fn find_span(sql: &str, fragment: &str) -> Option<SqlSpan> {
    if let Some(pos) = sql.find(fragment) {
        let before = &sql[..pos];
        let line = before.matches('\n').count() + 1;
        let col = pos - before.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;
        Some(SqlSpan {
            line,
            col,
            length: fragment.len(),
            fragment: fragment.to_string(),
        })
    } else {
        None
    }
}
