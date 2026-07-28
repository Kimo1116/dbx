use serde::Deserialize;

use super::types::*;

/// Build the system prompt for AI SQL review.
pub fn build_review_prompt(
    sql: &str,
    dialect: &str,
    database: &str,
    connection_env: &str,
    schema_ddl: &str,
    row_counts: &str,
    rule_findings_summary: &str,
) -> String {
    format!(
        r#"You are a senior database DBA reviewing SQL statements.

## Environment
- Dialect: {dialect}
- Database: {database} ({connection_env})
- Relevant table structures:
{schema_ddl}
- Estimated row counts: {row_counts}

## SQL to review
```sql
{sql}
```

## Issues already found by rule engine (do NOT repeat these)
{rule_findings_summary}

## Your task
Review from these angles, reporting ONLY issues NOT already covered by the rule engine:
1. Semantic correctness: Does the SQL accurately express a reasonable intent? Any logical contradictions?
2. Performance risks: Any degradation risks at the current data scale?
3. Data integrity: Missing related table operations? Breaking referential integrity?
4. Dialect pitfalls: Any version/dialect-specific traps?

## Output format (strict JSON array)
```json
[
  {{
    "severity": "error|warning|info",
    "category": "safety|performance|correctness|style",
    "title": "short title",
    "detail": "detailed explanation",
    "suggestion": "fix suggestion or rewritten SQL",
    "span_fragment": "problematic SQL fragment",
    "confidence": 0.0-1.0
  }}
]
```

If no additional issues, return empty array [].
Do NOT repeat issues already reported by the rule engine.
Do NOT invent tables or columns that don't exist in the schema."#
    )
}

/// Parse AI response into findings.
///
/// Returns the parsed findings plus an optional diagnostic note explaining why
/// a non-empty response produced no findings (malformed JSON, missing fields,
/// low confidence, ...). The note is meant to be shown to the user so silent
/// "nothing happened" results become debuggable.
pub fn parse_ai_findings(response: &str, confidence_threshold: f32) -> (Vec<Finding>, Option<String>) {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return (Vec::new(), Some("AI 返回内容为空".to_string()));
    }

    // Extract JSON from response (handle ```json wrapping)
    let Some(json_str) = extract_json_array(trimmed) else {
        return (
            Vec::new(),
            Some(format!(
                "AI 返回内容中未找到 JSON 数组。返回内容：{}",
                excerpt(trimmed)
            )),
        );
    };

    let items: Vec<serde_json::Value> = match serde_json::from_str(&json_str) {
        Ok(items) => items,
        Err(err) => {
            return (
                Vec::new(),
                Some(format!(
                    "AI 返回的 JSON 无法解析：{err}。返回内容：{}",
                    excerpt(&json_str)
                )),
            );
        }
    };

    let mut findings = Vec::new();
    let mut malformed = 0usize;
    let mut low_confidence = 0usize;
    for item in items {
        // Parse each item individually so a single malformed entry does not
        // discard the whole response.
        let Ok(raw) = serde_json::from_value::<AiFindingRaw>(item) else {
            malformed += 1;
            continue;
        };
        if raw.title.trim().is_empty() {
            malformed += 1;
            continue;
        }
        if raw.confidence < confidence_threshold {
            low_confidence += 1;
            continue;
        }
        findings.push(convert_ai_finding(raw));
    }

    let note = if findings.is_empty() && (malformed > 0 || low_confidence > 0) {
        let mut parts = Vec::new();
        if malformed > 0 {
            parts.push(format!("{malformed} 条格式不符（需包含 title 等字段）"));
        }
        if low_confidence > 0 {
            parts.push(format!("{low_confidence} 条置信度低于阈值 {confidence_threshold}"));
        }
        Some(format!("AI 返回内容未能解析为有效问题：{}", parts.join("，")))
    } else {
        None
    };

    (findings, note)
}

fn convert_ai_finding(item: AiFindingRaw) -> Finding {
    let severity = match normalize_severity(&item.severity) {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => Severity::Info,
    };
    let category = match normalize_category(&item.category) {
        "safety" => FindingCategory::Safety,
        "performance" => FindingCategory::Performance,
        "style" => FindingCategory::Style,
        _ => FindingCategory::Correctness,
    };
    Finding {
        id: format!("AI-{}", uuid::Uuid::new_v4().as_simple()),
        rule_id: format!("AI-{}", category_str(&category).to_uppercase()),
        source: FindingSource::AiReviewer,
        severity,
        category,
        title: item.title,
        detail: item.detail,
        suggestion: if item.suggestion.is_empty() {
            None
        } else {
            Some(item.suggestion)
        },
        span: None,
        auto_fixable: false,
        confidence: item.confidence,
    }
}

/// Accept case-variant and Chinese severity values from the model.
fn normalize_severity(value: &str) -> &'static str {
    match value.trim().to_lowercase().as_str() {
        "error" | "错误" | "严重" => "error",
        "warning" | "warn" | "警告" => "warning",
        _ => "info",
    }
}

/// Accept case-variant and Chinese category values from the model.
fn normalize_category(value: &str) -> &'static str {
    match value.trim().to_lowercase().as_str() {
        "safety" | "安全" => "safety",
        "performance" | "性能" => "performance",
        "style" | "风格" | "规范" => "style",
        _ => "correctness",
    }
}

fn category_str(category: &FindingCategory) -> &'static str {
    match category {
        FindingCategory::Safety => "safety",
        FindingCategory::Performance => "performance",
        FindingCategory::Correctness => "correctness",
        FindingCategory::Style => "style",
        FindingCategory::Convention => "convention",
    }
}

/// Truncate long response text for inclusion in a user-facing note.
fn excerpt(text: &str) -> String {
    let flat: String = text.chars().take(300).collect();
    if text.chars().count() > 300 {
        format!("{flat}…")
    } else {
        flat
    }
}

/// Summarize rule findings for inclusion in AI prompt.
pub fn summarize_rule_findings(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "None".to_string();
    }
    findings
        .iter()
        .map(|f| format!("- [{}] {}: {}", f.severity, f.rule_id, f.title))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Determine whether AI review should be triggered.
pub fn should_trigger_ai(
    settings: &AiReviewSettings,
    rule_findings: &[Finding],
    manual_trigger: bool,
) -> bool {
    if !settings.enabled {
        return false;
    }
    if manual_trigger {
        return true;
    }
    match settings.trigger {
        AiReviewTrigger::Always => true,
        AiReviewTrigger::OnWarnOrAbove => {
            rule_findings.iter().any(|f| f.severity <= Severity::Warning)
        }
        AiReviewTrigger::Manual => false,
    }
}

// ─── Internal ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AiFindingRaw {
    #[serde(default)]
    severity: String,
    #[serde(default)]
    category: String,
    title: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    suggestion: String,
    #[serde(default)]
    #[allow(dead_code)]
    span_fragment: String,
    #[serde(default = "default_confidence")]
    confidence: f32,
}

fn default_confidence() -> f32 {
    0.8
}

/// Extract a JSON array from AI response text.
fn extract_json_array(text: &str) -> Option<String> {
    // Try to find ```json ... ``` block
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            let json = after[..end].trim();
            if json.starts_with('[') {
                return Some(json.to_string());
            }
        }
    }
    // Try to find raw [ ... ]
    if let Some(start) = text.find('[') {
        if let Some(end) = text.rfind(']') {
            if end > start {
                return Some(text[start..=end].to_string());
            }
        }
    }
    None
}
