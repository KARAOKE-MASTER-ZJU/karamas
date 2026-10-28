<script setup lang="ts">
import { computed } from "vue";
import { useUpdaterStore } from "@/stores/updater";

const updater = useUpdaterStore();
const updateLabel = computed(() => {
  if (updater.status === "downloading") return "下载中";
  if (updater.status === "ready") return "下载完成";
  if (updater.status === "installing") return "安装中";
  return "发现新版本";
});
const showUpdateProgress = computed(() =>
  ["downloading", "ready", "installing"].includes(updater.status),
);
const updateProgress = computed(() =>
  updater.status === "downloading" ? updater.progress : 100,
);
const updateAccessibleLabel = computed(() =>
  updater.status === "downloading" && updateProgress.value !== undefined
    ? `下载中 ${updateProgress.value}%，查看更新`
    : `${updateLabel.value}，查看更新`,
);
</script>

<template>
  <button
    v-if="updater.update"
    class="button text-button"
    :aria-label="updateAccessibleLabel"
    title="查看更新"
    @click="updater.showDialog = true"
  >
    <svg
      v-if="showUpdateProgress"
      class="update-progress"
      :class="{ indeterminate: updateProgress === undefined }"
      viewBox="0 0 20 20"
      aria-hidden="true"
    >
      <circle class="update-progress-track" cx="10" cy="10" r="8" />
      <circle
        class="update-progress-value"
        cx="10"
        cy="10"
        r="8"
        pathLength="100"
        :stroke-dasharray="updateProgress === undefined ? '25 75' : '100 100'"
        :stroke-dashoffset="
          updateProgress === undefined ? 0 : 100 - updateProgress
        "
      />
    </svg>
    <span>{{ updateLabel }}</span>
  </button>
</template>

<style scoped>
.update-progress {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  fill: none;
  stroke: currentColor;
  stroke-width: 2;
  transform: rotate(-90deg);
}
.update-progress-track {
  opacity: 0.18;
}
.update-progress-value {
  stroke-linecap: round;
  transition: stroke-dashoffset 0.2s ease;
}
.update-progress.indeterminate {
  animation: update-progress-spin 1s linear infinite;
}
@keyframes update-progress-spin {
  to {
    transform: rotate(270deg);
  }
}
@media (prefers-reduced-motion: reduce) {
  .update-progress.indeterminate {
    animation: none;
  }
  .update-progress-value {
    transition: none;
  }
}
</style>
