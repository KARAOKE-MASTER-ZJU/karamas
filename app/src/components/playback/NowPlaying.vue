<script setup lang="ts">
import { Music2, RotateCcw } from "@lucide/vue";
defineProps<{
  title: string | null;
  songId: string | null;
  disabled: boolean;
  recasting: boolean;
  queuedCount: number | null;
  sungCount: number | null;
}>();
const emit = defineEmits<{ recast: [] }>();
</script>

<template>
  <div class="track-summary">
    <Music2 :size="24" />
    <div class="track-info">
      <h3>{{ title || "暂无播放内容" }}</h3>
      <p v-if="queuedCount !== null && sungCount !== null">
        待唱 {{ queuedCount }} 首 · 已唱 {{ sungCount }} 首
      </p>
    </div>
    <button
      class="icon-button recast-button"
      :aria-label="recasting ? '重新投屏中' : '重新投屏'"
      :title="recasting ? '重新投屏中' : '重新投屏'"
      :aria-busy="recasting"
      :disabled="disabled || !songId"
      @click="emit('recast')"
    >
      <RotateCcw :size="19" :class="{ spinning: recasting }" />
    </button>
  </div>
</template>

<style scoped>
.track-summary {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 4px 0 12px;
}
.track-summary > svg {
  box-sizing: content-box;
  padding: 16px;
  border-radius: 6px;
  background: var(--subtle);
  color: var(--muted);
}
.track-info {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
}
.track-info p {
  margin-top: 6px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--muted);
}
.recast-button,
.recast-button:hover:not(:disabled) {
  color: var(--accent);
}
.spinning {
  animation: spin 1s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@media (prefers-reduced-motion: reduce) {
  .spinning {
    animation: none;
  }
}
</style>
