<script setup lang="ts">
import { ChevronRight, Trash2 } from "@lucide/vue";
import { computed } from "vue";
import type { DiagnosticEntry } from "@/types/diagnostics";

const props = defineProps<{ entries: DiagnosticEntry[] }>();
const sortedEntries = computed(() =>
  [...props.entries].sort((a, b) => b.result.time - a.result.time),
);
const emit = defineEmits<{ open: [entry: DiagnosticEntry]; clear: [] }>();
const clock = new Intl.DateTimeFormat("zh-CN", {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  fractionalSecondDigits: 3,
  hour12: false,
});
</script>

<template>
  <div class="result-toolbar">
    <h2>
      请求记录 <span class="result-count">{{ entries.length }}</span>
    </h2>
    <button
      type="button"
      class="icon-button"
      :disabled="!entries.length"
      aria-label="清空记录"
      title="清空记录"
      @click="emit('clear')"
    >
      <Trash2 :size="18" aria-hidden="true" />
    </button>
  </div>
  <ul class="result-list" aria-label="测试记录" aria-live="polite">
    <li v-for="(entry, index) in sortedEntries" :key="index">
      <button type="button" class="result-row" @click="emit('open', entry)">
        <span class="result-main">
          <span class="result-title">{{ entry.title }}</span>
          <span class="result-caption">
            <time :datetime="new Date(entry.result.time).toISOString()">{{
              clock.format(entry.result.time)
            }}</time>
            <span class="result-url">{{ entry.result.url }}</span>
          </span>
        </span>
        <span class="result-meta">
          <span
            :class="{
              error:
                entry.result.error ||
                (entry.result.status !== null && entry.result.status >= 400),
            }"
            >{{ entry.result.error ? "错误" : entry.result.status }}</span
          >
          <span class="result-duration">{{ entry.result.elapsedMs }} ms</span>
        </span>
        <ChevronRight :size="16" aria-hidden="true" />
      </button>
    </li>
  </ul>
</template>

<style scoped>
.result-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--line);
}
.result-toolbar h2 {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 14px;
  font-weight: 500;
}
.result-count {
  color: var(--muted);
  font-size: 12px;
  font-weight: 400;
  font-variant-numeric: tabular-nums;
}
.result-list {
  margin: 0;
  padding: 0;
  list-style: none;
}
.result-list li {
  border-bottom: 1px solid var(--line);
}
.result-row {
  display: flex;
  align-items: center;
  gap: 14px;
  width: 100%;
  min-height: 64px;
  padding: 12px 0;
  border: 0;
  background: none;
  text-align: left;
}
.result-row:hover {
  background: var(--subtle);
}
.result-main {
  flex: 1;
  min-width: 0;
}
.result-title,
.result-url {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.result-title {
  font-size: 14px;
  font-weight: 500;
  line-height: 20px;
}
.result-caption {
  display: flex;
  gap: 10px;
  margin-top: 4px;
  color: var(--muted);
  font-size: 12px;
  line-height: 18px;
}
.result-caption time {
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}
.result-meta {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 4px;
  flex-shrink: 0;
  font-size: 13px;
  line-height: 18px;
  font-variant-numeric: tabular-nums;
}
.result-duration {
  color: var(--muted);
  font-size: 12px;
}
.result-row > svg {
  color: var(--muted);
}
.error {
  color: var(--error);
}
</style>
