<!-- 日志页面 -->

<script setup lang="ts">
import { Copy, Download, Trash2, Logs, ChevronDown } from "@lucide/vue";
import { computed, ref } from "vue";
import { storeToRefs } from "pinia";
import { useLogsStore } from "@/stores/logs";
import { useOperationNotice } from "@/composables/useOperationNotice";
import type { LogLevel } from "@/types";

const logs = useLogsStore();
const { entries } = storeToRefs(logs);
const { notify } = useOperationNotice();
const filter = ref<"all" | LogLevel>("all");
const query = ref("");
const levelLabels = {
  debug: "调试",
  info: "信息",
  warning: "提醒",
  error: "错误",
} satisfies Record<LogLevel, string>;
// 计算过滤后的日志条目，根据选定的日志级别和搜索查询进行筛选
const filtered = computed(() =>
  entries.value.filter(
    (entry) =>
      (filter.value === "all" || entry.level === filter.value) &&
      entry.message
        .toLocaleLowerCase()
        .includes(query.value.trim().toLocaleLowerCase()),
  ),
);
// 将过滤后的日志条目格式化为文本字符串，便于复制或导出
const formatted = computed(() =>
  filtered.value
    .map(
      (entry) => `${entry.time} [${levelLabels[entry.level]}] ${entry.message}`,
    )
    .join("\n"),
);

// 复制过滤后的日志条目到剪贴板
async function copyLogs() {
  try {
    await navigator.clipboard.writeText(formatted.value);
    notify("日志已复制");
  } catch {
    notify("无法访问剪贴板，请使用导出功能。", "warning");
  }
}

// 导出过滤后的日志条目到文本文件
function exportLogs() {
  const url = URL.createObjectURL(
    new Blob([formatted.value], { type: "text/plain;charset=utf-8" }),
  );
  const link = document.createElement("a");
  link.href = url;
  link.download = `ktv-casting-${new Date().toISOString().slice(0, 10)}.log`;
  link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
</script>

<template>
  <section class="logs-panel">
    <!-- 日志工具栏，包括日志级别筛选、搜索输入框和操作按钮（复制、导出、清空） -->
    <div class="logs-toolbar">
      <div class="log-filters">
        <div class="select-control log-level-select">
          <label class="sr-only" for="log-filter">日志级别</label>
          <select id="log-filter" v-model="filter">
            <option value="all">全部级别</option>
            <option
              v-for="(label, level) in levelLabels"
              :key="level"
              :value="level"
            >
              {{ label }}
            </option>
          </select>
          <ChevronDown :size="16" aria-hidden="true" />
        </div>
        <label class="sr-only" for="log-search">搜索日志</label>
        <input
          id="log-search"
          v-model="query"
          type="search"
          placeholder="搜索日志内容"
        />
      </div>
      <div class="log-actions">
        <button
          class="icon-button toolbar-icon"
          aria-label="复制日志"
          :disabled="!filtered.length"
          @click="copyLogs"
        >
          <Copy :size="18" aria-hidden="true" /></button
        ><button
          class="icon-button toolbar-icon"
          aria-label="导出日志"
          :disabled="!filtered.length"
          @click="exportLogs"
        >
          <Download :size="18" aria-hidden="true" /></button
        ><button
          class="icon-button toolbar-icon"
          :disabled="!entries.length"
          aria-label="清空日志"
          @click="logs.clear"
        >
          <Trash2 :size="18" />
        </button>
      </div>
    </div>

    <!-- 日志列表部分，显示过滤后的日志条目 -->
    <p class="log-caption">{{ filtered.length }} 条</p>
    <div class="log-list" role="log" aria-label="运行日志">
      <div
        v-for="entry in filtered"
        :key="entry.id"
        class="log-entry"
        :class="entry.level"
      >
        <time>{{ entry.time }}</time
        ><span class="log-level" :class="entry.level">{{
          levelLabels[entry.level]
        }}</span
        ><span>{{ entry.message }}</span>
      </div>
      <div v-if="!filtered.length" class="small-empty">
        <Logs :size="32" />
        <p>{{ entries.length ? "没有匹配的日志" : "暂无日志" }}</p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.logs-toolbar,
.log-filters,
.log-actions {
  display: flex;
  align-items: center;
  gap: 10px;
}
.logs-toolbar {
  justify-content: space-between;
  flex-wrap: wrap;
  padding-top: 8px;
}
.log-filters {
  flex: 1 1 300px;
  min-width: 0;
}
.log-filters input {
  flex: 1;
  width: 0;
  min-width: 0;
}
.log-level-select {
  width: 132px;
}
.log-actions {
  gap: 8px;
}
.toolbar-icon {
  height: var(--control-height);
  width: var(--control-height);
}
.log-caption {
  font-size: 12px;
  color: var(--muted);
  margin: 20px 0 12px;
  line-height: 1.6;
}
.log-list {
  border-top: 1px solid var(--line);
}
.log-entry {
  display: grid;
  grid-template-columns: 64px 36px 1fr;
  gap: 12px;
  padding: 14px 0;
  border-bottom: 1px solid var(--line);
  font-size: 12px;
  line-height: 1.7;
  overflow-wrap: anywhere;
}
.log-entry.debug {
  color: var(--muted);
}
.log-entry time {
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}
.log-level {
  color: var(--muted);
}
.log-level.warning {
  color: var(--warning);
}
.log-level.error {
  color: var(--error);
}
.small-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  min-height: 220px;
  color: var(--muted);
  font-size: 14px;
}
@media (max-width: 640px) {
  .logs-toolbar {
    gap: 12px;
  }
  .log-filters {
    flex-basis: 100%;
  }
  .log-actions {
    margin-left: auto;
  }
  .log-entry {
    grid-template-columns: 58px 30px 1fr;
    gap: 8px;
  }
}
</style>
