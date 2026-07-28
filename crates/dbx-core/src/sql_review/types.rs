use serde::{Deserialize, Serialize};

/// Source of a finding: rule engine (deterministic) or AI reviewer (probabilistic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSource {
    RuleEngine,
    AiReviewer,
}

/// Severity level of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
    Style,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
            Severity::Info => write!(f, "info"),
            Severity::Style => write!(f, "style"),
        }
    }
}

/// Category of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    Safety,
    Performance,
    Correctness,
    Style,
    Convention,
}

impl std::fmt::Display for FindingCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindingCategory::Safety => write!(f, "safety"),
            FindingCategory::Performance => write!(f, "performance"),
            FindingCategory::Correctness => write!(f, "correctness"),
            FindingCategory::Style => write!(f, "style"),
            FindingCategory::Convention => write!(f, "convention"),
        }
    }
}

/// Location span within the SQL text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlSpan {
    /// 1-based line number
    pub line: usize,
    /// 1-based column offset
    pub col: usize,
    /// Character length of the fragment
    pub length: usize,
    /// The problematic SQL fragment
    pub fragment: String,
}

/// A single review finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Unique ID for dedup and frontend key
    pub id: String,
    /// Rule identifier, e.g. "S001", "AI-PERF-01"
    pub rule_id: String,
    /// Where this finding came from
    pub source: FindingSource,
    /// Severity level
    pub severity: Severity,
    /// Category
    pub category: FindingCategory,
    /// Short title
    pub title: String,
    /// Detailed explanation
    pub detail: String,
    /// Fix suggestion (SQL snippet or guidance)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// Location in the SQL text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<SqlSpan>,
    /// Whether the suggestion can be auto-applied
    pub auto_fixable: bool,
    /// Confidence score (1.0 for rule engine, 0.0-1.0 for AI)
    pub confidence: f32,
}

/// Overall verdict of a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    /// No Error-level findings
    Pass,
    /// Has Warning but no Error
    Warn,
    /// Has Error-level findings
    Block,
}

/// Context snapshot at review time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewContext {
    /// DDL of relevant tables (for AI reference)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_snapshot: Option<String>,
    /// Table row count estimates
    pub row_count_hints: Vec<TableRowCountHint>,
    /// Whether the target is a production database
    pub is_production: bool,
    /// Connection environment label
    pub connection_env: String,
}

/// Row count hint for a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRowCountHint {
    pub table_name: String,
    pub estimated_rows: Option<i64>,
}

/// Complete review report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlReviewReport {
    pub id: String,
    pub sql: String,
    pub dialect: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub findings: Vec<Finding>,
    pub verdict: ReviewVerdict,
    pub rule_engine_elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_elapsed_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_parse_note: Option<String>,
    pub reviewed_at: i64,
    pub context: ReviewContext,
}

/// Intercept mode for execution pipeline integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewInterceptMode {
    /// No interception, manual review only
    Off,
    /// Show confirmation dialog on Error findings, user can force execute
    Warn,
    /// Block execution on Error findings, must fix first
    Block,
}

impl Default for ReviewInterceptMode {
    fn default() -> Self {
        Self::Off
    }
}

/// AI review trigger strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiReviewTrigger {
    /// Always invoke AI review
    Always,
    /// Only when rule engine finds Warning or above
    OnWarnOrAbove,
    /// Only when user manually triggers
    Manual,
}

impl Default for AiReviewTrigger {
    fn default() -> Self {
        Self::OnWarnOrAbove
    }
}

/// Settings for the SQL review module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSettings {
    /// Master switch
    pub enabled: bool,
    /// Intercept mode
    pub intercept_mode: ReviewInterceptMode,
    /// Rule engine settings
    pub rule_engine: RuleEngineSettings,
    /// AI review settings
    pub ai_review: AiReviewSettings,
    /// Scope settings
    pub scope: ReviewScope,
}

impl Default for ReviewSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            intercept_mode: ReviewInterceptMode::Warn,
            rule_engine: RuleEngineSettings::default(),
            ai_review: AiReviewSettings::default(),
            scope: ReviewScope::default(),
        }
    }
}

/// Rule engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleEngineSettings {
    /// Per-rule enable/disable overrides (rule_id -> enabled)
    pub rule_overrides: std::collections::HashMap<String, bool>,
    /// Per-rule severity overrides (rule_id -> severity)
    pub severity_overrides: std::collections::HashMap<String, Severity>,
    /// Custom user-defined rules
    pub custom_rules: Vec<CustomRule>,
    /// Max JOIN tables threshold for P010
    pub max_join_tables: usize,
    /// Row count threshold for "large table" warnings (P002)
    pub large_table_threshold: i64,
}

impl Default for RuleEngineSettings {
    fn default() -> Self {
        Self {
            rule_overrides: std::collections::HashMap::new(),
            severity_overrides: std::collections::HashMap::new(),
            custom_rules: Vec::new(),
            max_join_tables: 5,
            large_table_threshold: 100_000,
        }
    }
}

/// AI reviewer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiReviewSettings {
    pub enabled: bool,
    pub trigger: AiReviewTrigger,
    /// Which AI config to use (by config id)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_config_id: Option<String>,
    /// Optional model override (use a lighter model for cost savings)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// Timeout in milliseconds
    pub timeout_ms: u64,
    /// Minimum confidence to display AI findings
    pub confidence_threshold: f32,
    /// Max tables to include in schema context
    pub max_schema_tables: usize,
    /// Optional custom system prompt template. When None, the built-in default is used.
    /// Supports placeholders: {{dialect}} {{database}} {{sql}} {{rule_summary}}
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_override: Option<String>,
}

impl Default for AiReviewSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            trigger: AiReviewTrigger::OnWarnOrAbove,
            provider_config_id: None,
            model_override: None,
            timeout_ms: 15_000,
            confidence_threshold: 0.6,
            max_schema_tables: 10,
            system_prompt_override: None,
        }
    }
}

/// Scope settings: which execution paths trigger review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewScope {
    /// Apply to manually executed queries
    pub apply_to_manual_queries: bool,
    /// Apply to AI agent generated SQL
    pub apply_to_ai_agent: bool,
    /// Apply to MCP requests
    pub apply_to_mcp: bool,
    /// Skip pure SELECT statements
    pub exclude_read_only: bool,
}

impl Default for ReviewScope {
    fn default() -> Self {
        Self {
            apply_to_manual_queries: true,
            apply_to_ai_agent: true,
            apply_to_mcp: false,
            exclude_read_only: true,
        }
    }
}

/// A user-defined custom rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub severity: Severity,
    pub category: FindingCategory,
    /// Applicable dialects (empty = all)
    pub dialects: Vec<String>,
    pub match_type: RuleMatchType,
}

/// How a custom rule matches SQL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleMatchType {
    /// Regex pattern match on SQL text
    Regex { pattern: String },
    /// Keyword presence/absence
    Keyword {
        keywords: Vec<String>,
        /// "any" = match if any keyword present, "all" = all must be present
        mode: String,
    },
}
