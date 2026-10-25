<!-- 日志页面 -->

<script setup lang="ts">
import { Copy, Download, Share2, Trash2, Logs, ChevronDown } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useLogViewer } from "@/composables/useLogViewer";
import { useOperationNotice } from "@/composables/useOperationNotice";
import type { LogEntry } from "@/types";
import {
  logLevelLabels as levelLabels,
  formatLogTime,
  formatLogMessage,
} from "@/utils/logs";
import { castingApi, errorMessage, isTauri } from "@/services/casting";

const {
  entries,
  filter,
  query,
  total,
  retained,
  loading,
  clearing,
  hasMore,
  clear,
  loadMore,
} = useLogViewer();
const { notify } = useOperationNotice();
const exporting = ref(false);
const shareSupported = ref(false);
onMounted(async () => {
  if (!isTauri()) return;
  try {
    shareSupported.value = await castingApi.canShareLogs();
  } catch (error) {
    notify(`读取分享支持状态失败：${errorMessage(error)}`, "error");
  }
});

function formatEntries(items: LogEntry[]) {
  return items
    .map(
      (entry) =>
        `${formatLogTime(entry.time)} [${levelLabels[entry.level]}] ${formatLogMessage(entry)}`,
    )
    .join("\n");
}
const displayEntries = computed(() => [...entries.value].reverse());
const formatted = computed(() => formatEntries(displayEntries.value));

async function copyLogs() {
  try {
    await navigator.clipboard.writeText(formatted.value);
    notify("日志已复制");
  } catch {
    notify("无法访问剪贴板，请使用导出功能。", "warning");
  }
}

async function exportLogs(action: "save" | "share", event: MouseEvent) {
  if (exporting.value) return;
  exporting.value = true;
  try {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const completed = await castingApi.exportLogs(action, {
      x: rect.left + rect.width / 2,
      y: rect.bottom,
    });
    if (completed && action === "save") notify("日志已保存");
  } catch (error) {
    notify(
      `${action === "save" ? "保存" : "分享"}日志失败：${errorMessage(error)}`,
      "error",
    );
  } finally {
    exporting.value = false;
  }
}
</script>

<template>
  <section class="logs-panel">
    <!-- 日志工具栏，包括日志级别筛选、搜索输入框和操作按钮（复制、导出、清空） -->
    <div class="logs-toolbar">
      <div class="log-filters">
        <div class="select-control log-level-select">
          <label class="sr-only" for="log-filter">显示级别</label>
          <select id="log-filter" v-model="filter">
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
          :disabled="loading || !entries.length"
          @click="copyLogs"
        >
          <Copy :size="18" aria-hidden="true" /></button
        ><button
          class="icon-button toolbar-icon"
          aria-label="保存日志"
          :disabled="loading || exporting || !retained"
          @click="exportLogs('save', $event)"
        >
          <Download :size="18" aria-hidden="true" /></button
        ><button
          v-if="shareSupported"
          class="icon-button toolbar-icon"
          aria-label="分享日志"
          :disabled="loading || exporting || !retained"
          @click="exportLogs('share', $event)"
        >
          <Share2 :size="18" aria-hidden="true" /></button
        ><button
          class="icon-button toolbar-icon"
          :disabled="!retained || clearing || exporting"
          aria-label="清空日志"
          @click="clear"
        >
          <Trash2 :size="18" />
        </button>
      </div>
    </div>

    <!-- 日志列表部分，显示过滤后的日志条目 -->
    <p class="log-caption">
      {{ entries.length }}<template v-if="hasMore"> / {{ total }}</template> 条
    </p>
    <div class="log-list" role="log" aria-label="运行日志">
      <div
        v-for="entry in displayEntries"
        :key="entry.id"
        class="log-entry"
        :class="entry.level"
      >
        <time :datetime="new Date(entry.time).toISOString()">{{
          formatLogTime(entry.time)
        }}</time
        ><span class="log-level" :class="entry.level">{{
          levelLabels[entry.level]
        }}</span
        ><span>{{ formatLogMessage(entry) }}</span>
      </div>
      <div v-if="!entries.length" class="small-empty">
        <Logs :size="32" />
        <p>
          {{
            loading ? "正在读取日志…" : retained ? "没有匹配的日志" : "暂无日志"
          }}
        </p>
      </div>
    </div>
    <button
      v-if="hasMore"
      class="button text-button"
      :disabled="loading"
      @click="loadMore"
    >
      加载更早日志
    </button>
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
  /* 限制 WebKit 跨条目选区的绘制范围。 */
  position: relative;
  border-top: 1px solid var(--line);
}
.log-entry {
  display: grid;
  grid-template-columns: 64px 56px 1fr;
  gap: 12px;
  padding: 14px 0;
  border-bottom: 1px solid var(--line);
  font-size: 12px;
  line-height: 1.7;
  overflow-wrap: anywhere;
}
.log-entry.trace,
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
    grid-template-columns: 58px 56px 1fr;
    gap: 8px;
  }
}
</style>
