<script setup lang="ts">
import { computed, toRef } from "vue";
import { ChevronDown, RefreshCw, Check } from "@lucide/vue";
import AppDialog from "@/components/base/AppDialog.vue";
import { useBilibiliParts } from "@/composables/useBilibiliParts";
import { formatTime } from "@/utils/time";

const props = defineProps<{ songId: string; disabled: boolean }>();
const { parts, loading, error, showDialog, refresh, select } = useBilibiliParts(
  toRef(props, "songId"),
);
const currentPart = computed(() =>
  parts.value?.parts.find(
    (part) => part.pageIndex === parts.value?.currentPageIndex,
  ),
);
</script>

<template>
  <div class="parts-control">
    <button
      class="button text-button parts-button"
      :disabled="disabled"
      @click="showDialog = true"
    >
      <span>{{
        currentPart
          ? `P${currentPart.page} · ${currentPart.part}`
          : loading
            ? "正在加载分 P…"
            : "选择分 P"
      }}</span>
      <ChevronDown :size="16" />
    </button>
    <AppDialog
      v-if="showDialog"
      title="选择分 P"
      close-label="关闭分 P 选择"
      @close="showDialog = false"
    >
      <template #actions>
        <button
          type="button"
          class="icon-button"
          aria-label="刷新分 P"
          :disabled="loading || disabled"
          @click="refresh"
        >
          <RefreshCw :size="18" aria-hidden="true" />
        </button>
      </template>
      <p v-if="loading" class="parts-message" role="status">正在加载分 P…</p>
      <p v-else-if="error" class="field-error" role="alert">{{ error }}</p>
      <ul v-if="!loading && parts" class="parts-list" aria-label="分 P 列表">
        <li v-for="part in parts.parts" :key="part.pageIndex">
          <button
            class="part-option"
            :class="{ selected: part.pageIndex === parts.currentPageIndex }"
            :disabled="disabled"
            @click="select(part.pageIndex)"
          >
            <span class="part-title">P{{ part.page }} · {{ part.part }}</span>
            <span class="part-duration">{{
              formatTime(part.durationSecs)
            }}</span>
            <Check
              v-if="part.pageIndex === parts.currentPageIndex"
              :size="16"
              aria-label="当前分 P"
            />
          </button>
        </li>
      </ul>
    </AppDialog>
  </div>
</template>

<style scoped>
.parts-control {
  margin: 0 0 8px;
}
.parts-button {
  max-width: 100%;
}
.parts-button > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.parts-message {
  font-size: 13px;
  line-height: 1.6;
  color: var(--muted);
  margin-bottom: 12px;
}
.parts-list {
  list-style: none;
  padding: 0;
  margin: 0;
  max-height: min(360px, 50dvh);
  overflow-y: auto;
}
.part-option {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 12px 4px;
  border: 0;
  border-bottom: 1px solid var(--line);
  background: transparent;
  text-align: left;
  font-size: 14px;
}
.part-option:hover:not(:disabled) {
  background: var(--subtle);
}
.part-option.selected {
  color: var(--accent);
}
.part-title {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
}
.part-duration {
  font-size: 12px;
  color: var(--muted);
}
</style>
