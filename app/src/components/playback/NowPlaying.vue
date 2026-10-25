<script setup lang="ts">
import { Music2, RotateCcw } from "@lucide/vue";
defineProps<{
  title: string | null;
  addedBy: string | null;
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
      <div class="track-meta">
        <span v-if="songId" class="user-badge">{{ addedBy || "SYSTEM" }}</span>
        <p v-if="queuedCount !== null && sungCount !== null">
          待唱 {{ queuedCount }} 首 · 已唱 {{ sungCount }} 首
        </p>
      </div>
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
.track-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px 8px;
  margin-top: 6px;
}
.user-badge {
  max-width: 100%;
  padding: 2px 6px;
  border-radius: 4px;
  background: var(--subtle);
  color: var(--muted);
  font-size: 10px;
  line-height: 1.4;
}
.track-meta p {
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
