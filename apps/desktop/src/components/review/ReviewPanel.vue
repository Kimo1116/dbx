<script setup lang="ts">
import { ref, computed } from "vue";
import { ShieldCheck, X, Loader2, AlertCircle, AlertTriangle, Info, Diamond, CheckCircle2, Trash2 } from "@lucide/vue";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { runSqlReview, type SqlReviewReport, type Finding } from "@/lib/review/sqlReview";
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

const report = ref<SqlReviewReport | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);
const reviewSettingsStore = useReviewSettingsStore();

const errorCount = computed(() => report.value?.findings.filter((f) => f.severity === "error").length ?? 0);
const warnCount = computed(() => report.value?.findings.filter((f) => f.severity === "warning").length ?? 0);
const infoCount = computed(() => report.value?.findings.filter((f) => f.severity === "info" || f.severity === "style").length ?? 0);

const verdictLabel = computed(() => {
  if (!report.value) return "";
  switch (report.value.verdict) {
    case "pass": return "通过";
    case "warn": return "警告";
    case "block": return "阻止";
    default: return "";
  }
});

const verdictClass = computed(() => {
  if (!report.value) return "";
  switch (report.value.verdict) {
    case "pass": return "text-emerald-500";
    case "warn": return "text-amber-500";
    case "block": return "text-red-500";
    default: return "";
  }
});

const reviewedAtLabel = computed(() => {
  if (!report.value) return "";
  const d = new Date(report.value.reviewed_at * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
});

async function runReview() {
  if (!props.sql.trim()) return;
  loading.value = true;
  error.value = null;
  try {
    await reviewSettingsStore.ensureLoaded();
    report.value = await runSqlReview({
      sql: props.sql,
      dialect: props.dialect,
      connectionId: props.connectionId,
      database: props.database,
      settings: reviewSettingsStore.settings,
    });
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

function clearReview() {
  report.value = null;
  error.value = null;
  loading.value = false;
}

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

defineExpose({ runReview });
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
          {{ verdictLabel }} · {{ report.rule_engine_elapsed_ms }}ms
        </span>
      </template>
      <span v-if="reviewedAtLabel" class="text-[10px] text-muted-foreground">{{ reviewedAtLabel }}</span>

      <div class="ml-auto flex items-center gap-1">
        <Button variant="ghost" size="sm" class="h-6 gap-1 px-2 text-xs" :disabled="loading || !sql.trim()" @click="runReview">
          <Loader2 v-if="loading" class="h-3 w-3 animate-spin" />
          <ShieldCheck v-else class="h-3 w-3" />
          审查
        </Button>
        <Button v-if="report || error" variant="ghost" size="icon" class="h-6 w-6 text-muted-foreground" title="清空结果" @click="clearReview">
          <Trash2 class="h-3 w-3" />
        </Button>
        <Button variant="ghost" size="icon" class="h-6 w-6" @click="emit('close')">
          <X class="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>

    <!-- Body -->
    <ScrollArea class="min-h-0 flex-1">
      <div class="space-y-2 px-4 py-3">
        <!-- Empty state -->
        <div v-if="!report && !error && !loading" class="flex flex-col items-center justify-center gap-2 py-16 text-muted-foreground">
          <ShieldCheck class="h-8 w-8 opacity-20" />
          <p class="text-xs">点击「审查」检查当前 SQL</p>
        </div>

        <!-- Loading -->
        <div v-if="loading && !report" class="flex flex-col items-center justify-center gap-2 py-16 text-muted-foreground">
          <Loader2 class="h-5 w-5 animate-spin opacity-40" />
          <p class="text-xs">审查中...</p>
        </div>

        <!-- Error -->
        <div v-if="error" class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {{ error }}
        </div>

        <!-- Findings -->
        <div
          v-for="f in report?.findings"
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
        <div v-if="report && report.findings.length === 0 && !loading" class="flex flex-col items-center justify-center gap-2 py-16 text-emerald-500">
          <CheckCircle2 class="h-8 w-8 opacity-40" />
          <p class="text-xs">未发现问题</p>
        </div>
      </div>
    </ScrollArea>
  </div>
</template>
