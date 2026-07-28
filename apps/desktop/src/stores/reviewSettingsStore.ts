import { defineStore } from "pinia";
import { computed, ref } from "vue";
import {
  DEFAULT_AI_REVIEW_PROMPT,
  listReviewRules,
  loadReviewSettings,
  saveReviewSettings,
  type ReviewSettings,
  type RuleMeta,
} from "@/lib/review/sqlReview";

/** Frontend mirror of the Rust `ReviewSettings::default()`. */
export function createDefaultReviewSettings(): ReviewSettings {
  return {
    enabled: true,
    intercept_mode: "warn",
    rule_engine: {
      rule_overrides: {},
      severity_overrides: {},
      custom_rules: [],
      max_join_tables: 5,
      large_table_threshold: 100000,
    },
    ai_review: {
      enabled: false,
      trigger: "on_warn_or_above",
      timeout_ms: 15000,
      confidence_threshold: 0.6,
      max_schema_tables: 10,
    },
    scope: {
      apply_to_manual_queries: true,
      apply_to_ai_agent: true,
      apply_to_mcp: false,
      exclude_read_only: true,
    },
  };
}

export const useReviewSettingsStore = defineStore("reviewSettings", () => {
  const settings = ref<ReviewSettings>(createDefaultReviewSettings());
  const rules = ref<RuleMeta[]>([]);
  const isLoaded = ref(false);
  const isLoading = ref(false);
  let loadPromise: Promise<boolean> | null = null;

  async function init(): Promise<boolean> {
    if (isLoaded.value) return true;
    if (loadPromise) return loadPromise;
    isLoading.value = true;
    loadPromise = (async () => {
      try {
        const [loaded, ruleList] = await Promise.all([loadReviewSettings(), listReviewRules()]);
        settings.value = loaded;
        rules.value = ruleList;
        isLoaded.value = true;
        return true;
      } catch {
        // Keep failed initialization retryable.
        return false;
      } finally {
        isLoading.value = false;
        loadPromise = null;
      }
    })();
    return loadPromise;
  }

  async function ensureLoaded(): Promise<boolean> {
    return init();
  }

  async function persist(): Promise<void> {
    await saveReviewSettings(settings.value);
  }

  /** Effective enabled state for a rule, honoring overrides then default_enabled. */
  function isRuleEnabled(rule: RuleMeta): boolean {
    const override = settings.value.rule_engine.rule_overrides[rule.id];
    return override ?? rule.default_enabled;
  }

  async function setRuleEnabled(ruleId: string, enabled: boolean): Promise<void> {
    settings.value.rule_engine.rule_overrides[ruleId] = enabled;
    await persist();
  }

  async function resetRuleOverrides(): Promise<void> {
    settings.value.rule_engine.rule_overrides = {};
    await persist();
  }

  /** The prompt template currently in effect (custom override or built-in default). */
  const effectivePromptTemplate = computed(() => {
    const custom = settings.value.ai_review.system_prompt_override;
    return custom && custom.trim() ? custom : DEFAULT_AI_REVIEW_PROMPT;
  });

  const hasCustomPrompt = computed(() => {
    const custom = settings.value.ai_review.system_prompt_override;
    return !!custom && !!custom.trim();
  });

  async function setSystemPrompt(template: string): Promise<void> {
    const trimmed = template.trim();
    if (trimmed) {
      settings.value.ai_review.system_prompt_override = template;
    } else {
      delete settings.value.ai_review.system_prompt_override;
    }
    await persist();
  }

  async function resetSystemPrompt(): Promise<void> {
    delete settings.value.ai_review.system_prompt_override;
    await persist();
  }

  return {
    settings,
    rules,
    isLoaded,
    isLoading,
    init,
    ensureLoaded,
    isRuleEnabled,
    setRuleEnabled,
    resetRuleOverrides,
    effectivePromptTemplate,
    hasCustomPrompt,
    setSystemPrompt,
    resetSystemPrompt,
  };
});
