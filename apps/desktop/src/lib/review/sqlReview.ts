import { invoke } from "@tauri-apps/api/core";

export interface SqlSpan {
  line: number;
  col: number;
  length: number;
  fragment: string;
}

export interface Finding {
  id: string;
  rule_id: string;
  source: "rule_engine" | "ai_reviewer";
  severity: "error" | "warning" | "info" | "style";
  category: "safety" | "performance" | "correctness" | "style" | "convention";
  title: string;
  detail: string;
  suggestion?: string;
  span?: SqlSpan;
  auto_fixable: boolean;
  confidence: number;
}

export interface ReviewVerdict {
  verdict: "pass" | "warn" | "block";
}

export interface SqlReviewReport {
  id: string;
  sql: string;
  dialect: string;
  connection_id?: string;
  database?: string;
  findings: Finding[];
  verdict: "pass" | "warn" | "block";
  rule_engine_elapsed_ms: number;
  ai_elapsed_ms?: number;
  ai_parse_note?: string;
  reviewed_at: number;
}

export interface ReviewSettings {
  enabled: boolean;
  intercept_mode: "off" | "warn" | "block";
  rule_engine: {
    rule_overrides: Record<string, boolean>;
    severity_overrides: Record<string, string>;
    custom_rules: unknown[];
    max_join_tables: number;
    large_table_threshold: number;
  };
  ai_review: {
    enabled: boolean;
    trigger: "always" | "on_warn_or_above" | "manual";
    provider_config_id?: string;
    model_override?: string;
    timeout_ms: number;
    confidence_threshold: number;
    max_schema_tables: number;
    system_prompt_override?: string;
  };
  scope: {
    apply_to_manual_queries: boolean;
    apply_to_ai_agent: boolean;
    apply_to_mcp: boolean;
    exclude_read_only: boolean;
  };
}

export interface RuleMeta {
  id: string;
  name: string;
  category: "safety" | "performance" | "correctness" | "style" | "convention";
  severity: "error" | "warning" | "info" | "style";
  default_enabled: boolean;
  supported_dialects: string[];
}

export async function listReviewRules(): Promise<RuleMeta[]> {
  return invoke("sql_review_list_rules");
}

/** Built-in AI review system prompt template. Placeholders: {{dialect}} {{database}} {{sql}} {{rule_summary}} */
export const DEFAULT_AI_REVIEW_PROMPT = [
  "你是一位资深数据库 DBA，负责审查 SQL 语句。",
  "",
  "## 环境",
  "- 方言: {{dialect}}",
  "- 数据库: {{database}}",
  "",
  "## 待审查 SQL",
  "```sql",
  "{{sql}}",
  "```",
  "",
  "## 规则引擎已发现的问题（不要重复）",
  "{{rule_summary}}",
  "",
  "## 你的任务",
  "从以下角度审查，仅报告规则引擎未覆盖的问题：",
  "1. 语义正确性：SQL 是否准确表达了合理意图？",
  "2. 性能风险：在当前数据规模下是否存在性能退化风险？",
  "3. 数据完整性：是否遗漏了关联表操作？是否破坏引用完整性？",
  "4. 方言陷阱：是否存在版本/方言特有的坑？",
  "",
  "## 输出格式（严格 JSON 数组，title/detail/suggestion 必须使用中文）",
  "```json",
  "[",
  "  {",
  '    "severity": "error|warning|info",',
  '    "category": "safety|performance|correctness|style",',
  '    "title": "简短中文标题",',
  '    "detail": "详细中文说明",',
  '    "suggestion": "中文修复建议或重写 SQL",',
  '    "span_fragment": "问题 SQL 片段",',
  '    "confidence": 0.85',
  "  }",
  "]",
  "```",
  "",
  "如果没有额外问题，返回空数组 []。",
  "不要重复规则引擎已报告的问题。",
].join("\n");

/** Substitute {{dialect}} {{database}} {{sql}} {{rule_summary}} placeholders in a prompt template. */
export function renderAiReviewPrompt(
  template: string,
  vars: { dialect: string; database: string; sql: string; ruleSummary: string },
): string {
  return template
    .replaceAll("{{dialect}}", vars.dialect)
    .replaceAll("{{database}}", vars.database)
    .replaceAll("{{sql}}", vars.sql)
    .replaceAll("{{rule_summary}}", vars.ruleSummary);
}

export async function runSqlReview(params: {
  sql: string;
  dialect: string;
  connectionId?: string;
  database?: string;
  settings?: ReviewSettings;
  aiResponse?: string;
}): Promise<SqlReviewReport> {
  return invoke("sql_review_run", {
    sql: params.sql,
    dialect: params.dialect,
    connectionId: params.connectionId,
    database: params.database,
    settings: params.settings,
    aiResponse: params.aiResponse,
  });
}

export async function loadReviewSettings(): Promise<ReviewSettings> {
  return invoke("sql_review_load_settings");
}

export async function saveReviewSettings(settings: ReviewSettings): Promise<void> {
  return invoke("sql_review_save_settings", { settings });
}
