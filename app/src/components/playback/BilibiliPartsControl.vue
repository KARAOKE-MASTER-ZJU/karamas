<script setup lang="ts">
import { computed, toRef } from "vue";
import { ChevronDown } from "@lucide/vue";
import SelectionDialog from "@/components/base/SelectionDialog.vue";
import { useBilibiliParts } from "@/composables/useBilibiliParts";
import { formatTime } from "@/utils/time";

const props = defineProps<{
  songId: string;
  songUrl: string | null;
  disabled: boolean;
}>();
const { parts, loading, error, showDialog, refresh, select } = useBilibiliParts(
  toRef(props, "songId"),
  toRef(props, "songUrl"),
);
const currentPart = computed(() =>
  parts.value?.parts.find(
    (part) => part.pageIndex === parts.value?.currentPageIndex,
  ),
);
const options = computed(() =>
  (parts.value?.parts ?? []).map((part) => ({
    id: part.pageIndex,
    label: `P${part.page} · ${part.part}`,
    detail: formatTime(part.durationSecs),
  })),
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
    <SelectionDialog
      v-if="showDialog"
      title="选择分 P"
      close-label="关闭分 P 选择"
      refresh-label="刷新分 P"
      list-label="分 P 列表"
      loading-message="正在加载分 P…"
      empty-message="暂无可用分 P，请刷新重试"
      :options="options"
      :selected-id="parts?.currentPageIndex ?? null"
      :loading="loading"
      :disabled="disabled"
      :error="error"
      @close="showDialog = false"
      @refresh="refresh"
      @select="select"
    />
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
</style>
