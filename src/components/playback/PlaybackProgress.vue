<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{
  currentSecs: number | null;
  totalSecs: number | null;
  disabled: boolean;
}>();
const emit = defineEmits<{ seek: [seconds: number] }>();
const progress = computed(() => {
  if (props.currentSecs === null || !props.totalSecs) return 0;
  return Math.min(100, (100 * props.currentSecs) / props.totalSecs);
});
function time(seconds: number | null) {
  if (seconds === null) return "--:--";
  return `${Math.floor(seconds / 60)
    .toString()
    .padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
}
function seek(event: Event) {
  emit("seek", Number((event.target as HTMLInputElement).value));
}
</script>

<template>
  <div class="progress-track">
    <div class="progress-fill" :style="{ width: `${progress}%` }" />
    <input
      class="progress-input"
      type="range"
      min="0"
      :max="totalSecs || 1"
      :value="currentSecs ?? 0"
      :disabled="disabled || !totalSecs || currentSecs === null"
      aria-label="播放进度"
      :aria-valuetext="time(currentSecs)"
      @change="seek"
    />
  </div>
  <div class="time-labels">
    <span>{{ time(currentSecs) }}</span
    ><span>{{ time(totalSecs) }}</span>
  </div>
</template>

<style scoped>
.progress-track {
  position: relative;
  height: 4px;
  background: var(--line);
  border-radius: 2px;
}
.progress-fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent);
}
.progress-input {
  position: absolute;
  inset: -8px 0;
  width: 100%;
  height: 20px;
  margin: 0;
  opacity: 0;
  cursor: pointer;
}
.progress-track:has(input:focus-visible) {
  outline: 2px solid var(--accent);
  outline-offset: 3px;
}
.time-labels {
  display: flex;
  justify-content: space-between;
  color: var(--muted);
  font-size: 12px;
  margin-top: 8px;
  font-variant-numeric: tabular-nums;
}
</style>
