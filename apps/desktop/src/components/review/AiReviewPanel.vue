<script setup lang="ts">
import { ref, computed, watch, nextTick, onBeforeUnmount } from "vue";
import { Sparkles, X, Loader2, AlertCircle, AlertTriangle, Info, Diamond, CheckCircle2, Square, Trash2 } from "@lucide/vue";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useSettingsStore, normalizeAiConfig } from "@/stores/settingsStore";
import { useToast } from "@/composables/useToast";
import { uuid } from "@/lib/common/utils";
import * as api from "@/lib/backend/api";
import { runSqlReview, renderAiReviewPrompt, type SqlReviewReport, type Finding } from "@/lib/review/sqlReview";
import { useReviewSettingsStore } from "@/stores/reviewSettingsStore";

const props = defineProps<{
  sql: string;
  dialect: string;
  connectionId?: string;
  database?: string;
}>();

const emit = defineEmits<{
  (e: "close"): void;
}>();

const settings = useSettingsStore();
const reviewSettingsStore = useReviewSettingsStore();
const { toast } = useToast();
const report = ref<SqlReviewReport | null>(null);
const streaming = ref(false);
const streamingText = ref("");
const error = ref<string | null>(null);
const reviewedAt = ref<Date | null>(null);
const elapsedMs = ref(0);
const streamBox = ref<HTMLElement | null>(null);
let currentSessionId: string | null = null;

const aiFindings = computed(() => report.value?.findings.filter((f) => f.source === "ai_reviewer") ?? []);
const errorCount = computed(() => aiFindings.value.filter((f) => f.severity === "error").length);
const warnCount = computed(() => aiFindings.value.filter((f) => f.severity === "warning").length);
const infoCount = computed(() => aiFindings.value.filter((f) => f.severity === "info" || f.severity === "style").length);

const verdict = computed(() => {
  if (aiFindings.value.some((f) => f.severity === "error")) return "block";
  if (aiFindings.value.some((f) => f.severity === "warning")) return "warn";
  return "pass";
});

const verdictLabel = computed(() => {
  if (!report.value) return "";
  switch (verdict.value) {
    case "pass": return "通过";
    case "warn": return "警告";
    case "block": return "阻止";
    default: return "";
  }
});

const verdictClass = computed(() => {
  if (!report.value) return "";
  switch (verdict.value) {
    case "pass": return "text-emerald-500";
    case "warn": return "text-amber-500";
    case "block": return "text-red-500";
    default: return "";
  }
});

const activeFullConfig = computed(() => {
  if (!settings.activeModel) return null;
  const item = settings.aiConfigs.find((c) => c.id === settings.activeModel!.configId);
  if (!item) return null;
  return normalizeAiConfig({ ...item, model: settings.activeModel.modelId });
});

const aiReady = computed(() => settings.isConfigured && !!activeFullConfig.value);

const reviewedAtLabel = computed(() => {
  if (!reviewedAt.value) return "";
  const d = reviewedAt.value;
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
});

// Keep the streaming box pinned to the bottom while tokens arrive.
watch(streamingText, () => {
  void nextTick(() => {
    const el = streamBox.value;
    if (el) el.scrollTop = el.scrollHeight;
  });
});

async function runAiReview() {
  if (!props.sql.trim()) return;
  if (!aiReady.value) {
    toast("请先在设置 → AI 中配置模型");
    settings.requestSettingsNavigation("ai");
    return;
  }
  streaming.value = true;
  streamingText.value = "";
  error.value = null;
  report.value = null;
  const config = activeFullConfig.value!;
  const sessionId = uuid();
  currentSessionId = sessionId;
  const startedAt = performance.now();
  try {
    await reviewSettingsStore.ensureLoaded();
    const ruleReport = await runSqlReview({
      sql: props.sql,
      dialect: props.dialect,
      connectionId: props.connectionId,
      database: props.database,
      settings: reviewSettingsStore.settings,
    });

    const ruleSummary = ruleReport.findings.length
      ? ruleReport.findings.map((f) => `- [${f.severity}] ${f.rule_id}: ${f.title}`).join("\n")
      : "None";

    const systemPrompt = renderAiReviewPrompt(reviewSettingsStore.effectivePromptTemplate, {
      dialect: props.dialect,
      database: props.database || "default",
      sql: props.sql,
      ruleSummary,
    });

    await api.aiStream(
      sessionId,
      {
        config,
        systemPrompt,
        messages: [{ role: "user", content: "请审查以上 SQL 语句，以 JSON 数组返回发现的问题。" }],
        maxTokens: 4096,
      },
      (chunk) => {
        if (!chunk.done && chunk.delta) {
          streamingText.value += chunk.delta;
        }
      },
    );

    report.value = await runSqlReview({
      sql: props.sql,
      dialect: props.dialect,
      connectionId: props.connectionId,
      database: props.database,
      settings: reviewSettingsStore.settings,
      aiResponse: streamingText.value,
    });
    elapsedMs.value = Math.round(performance.now() - startedAt);
    reviewedAt.value = new Date();
  } catch (e) {
    error.value = String(e);
  } finally {
    streaming.value = false;
    currentSessionId = null;
  }
}

async function stopStream() {
  if (currentSessionId) {
    await api.aiCancelStream(currentSessionId).catch(() => {});
  }
}

function clearReview() {
  report.value = null;
  streamingText.value = "";
  error.value = null;
  reviewedAt.value = null;
  elapsedMs.value = 0;
}

onBeforeUnmount(() => {
  void stopStream();
});

function severityIcon(severity: string) {
  switch (severity) {
    case "error": return AlertCircle;
    case "warning": return AlertTriangle;
    case "info": return Info;
    case "style": return Diamond;
    default: return Info;
  }
}

function severityClass(severity: string) {
  switch (severity) {
    case "error": return "text-red-500";
    case "warning": return "text-amber-500";
    case "info": return "text-blue-500";
    case "style": return "text-muted-foreground";
    default: return "text-muted-foreground";
  }
}

function borderClass(severity: string) {
  switch (severity) {
    case "error": return "border-l-red-500";
    case "warning": return "border-l-amber-500";
    case "info": return "border-l-blue-500";
    case "style": return "border-l-muted-foreground/40";
    default: return "border-l-border";
  }
}

function sourceLabel(f: Finding) {
  return f.source === "ai_reviewer" ? `AI ${Math.round(f.confidence * 100)}%` : f.rule_id;
}

defineExpose({ runAiReview });
</script>

<template>
  <div class="flex h-full flex-col bg-background text-foreground">
    <!-- Header -->
    <div class="flex h-9 shrink-0 items-center gap-2 border-b border-border px-3">
      <template v-if="report">
        <span class="flex items-center gap-2 text-xs text-muted-foreground">
          <span v-if="errorCount" class="text-red-500">{{ errorCount }} 错误</span>
          <span v-if="warnCount" class="text-amber-500">{{ warnCount }} 警告</span>
          <span v-if="infoCount" class="text-blue-500">{{ infoCount }} 提示</span>
        </span>
        <span :class="verdictClass" class="text-xs font-medium">
          {{ verdictLabel }} · {{ elapsedMs }}ms
        </span>
      </template>
      <span v-if="reviewedAtLabel" class="text-[10px] text-muted-foreground">{{ reviewedAtLabel }}</span>

      <div class="ml-auto flex items-center gap-1">
        <Button v-if="streaming" variant="ghost" size="sm" class="h-6 gap-1 px-2 text-xs text-destructive hover:text-destructive" @click="stopStream">
          <Square class="h-3 w-3" />
          停止
        </Button>
        <Button v-else variant="ghost" size="sm" class="h-6 gap-1 px-2 text-xs text-primary hover:text-primary" :class="{ 'opacity-60': !aiReady }" :disabled="!sql.trim()" :title="aiReady ? 'AI 深度审查' : '未配置 AI，点击前往设置'" @click="runAiReview">
          <Sparkles class="h-3 w-3" />
          AI 审查
        </Button>
        <Button v-if="(report || error || streamingText) && !streaming" variant="ghost" size="icon" class="h-6 w-6 text-muted-foreground" title="清空结果" @click="clearReview">
          <Trash2 class="h-3 w-3" />
        </Button>
        <Button variant="ghost" size="icon" class="h-6 w-6" @click="emit('close')">
          <X class="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>

    <!-- Body -->
    <ScrollArea class="min-h-0 flex-1">
      <div class="space-y-3 px-4 py-3">
        <!-- Empty state -->
        <div v-if="!report && !error && !streaming && !streamingText" class="flex flex-col items-center justify-center gap-2 py-16 text-muted-foreground">
          <Sparkles class="h-8 w-8 opacity-20" />
          <p class="text-xs">点击「AI 审查」对当前 SQL 进行深度审查</p>
        </div>

        <!-- Error -->
        <div v-if="error" class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {{ error }}
        </div>

        <!-- AI live response -->
        <div v-if="streamingText || streaming" class="rounded-md border border-border bg-muted/30">
          <div class="flex items-center gap-1.5 border-b border-border px-3 py-1.5 text-[10px] text-muted-foreground">
            <Loader2 v-if="streaming" class="h-3 w-3 animate-spin" />
            <Sparkles v-else class="h-3 w-3" />
            AI 实时回复
          </div>
          <div ref="streamBox" class="max-h-56 overflow-y-auto px-3 py-2 font-mono text-xs leading-relaxed text-foreground/80">
            <span class="whitespace-pre-wrap">{{ streamingText }}</span><span v-if="streaming" class="ml-0.5 inline-block h-3.5 w-1.5 animate-pulse bg-primary align-middle" />
          </div>
        </div>

        <!-- AI parse diagnostic -->
        <div v-if="report?.ai_parse_note" class="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs leading-relaxed text-amber-500">
          {{ report.ai_parse_note }}
        </div>

        <!-- Findings -->
        <div
          v-for="f in aiFindings"
          :key="f.id"
          class="rounded-r-md border-l-2 bg-muted/50 px-3 py-2"
          :class="borderClass(f.severity)"
        >
          <div class="flex items-center gap-2">
            <component :is="severityIcon(f.severity)" class="h-3.5 w-3.5 shrink-0" :class="severityClass(f.severity)" />
            <span class="text-xs font-medium">{{ f.title }}</span>
            <span class="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground">{{ sourceLabel(f) }}</span>
          </div>
          <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ f.detail }}</p>
          <div v-if="f.suggestion" class="mt-1.5 rounded bg-muted px-2 py-1 font-mono text-xs text-primary/80">
            {{ f.suggestion }}
          </div>
        </div>

        <!-- Pass state -->
        <div v-if="report && aiFindings.length === 0 && !streaming" class="flex flex-col items-center justify-center gap-2 py-10 text-emerald-500">
          <CheckCircle2 class="h-8 w-8 opacity-40" />
          <p class="text-xs">未发现问题</p>
        </div>
      </div>
    </ScrollArea>
  </div>
</template>
