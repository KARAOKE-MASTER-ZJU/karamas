<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { formatTime } from "@/utils/time";

const props = defineProps<{
  currentSecs: number | null;
  totalSecs: number | null;
  live?: boolean;
  disabled: boolean;
}>();
const emit = defineEmits<{ seek: [seconds: number] }>();
// 拖动时固定预览位置和滑杆范围，避免轮询更新改变拖动目标。
const draft = ref<{ seconds: number; max: number } | null>(null);
const displaySecs = computed(() => draft.value?.seconds ?? props.currentSecs);
const maxSecs = computed(
  () =>
    draft.value?.max ??
    (props.totalSecs || Math.max(600, (props.currentSecs ?? 0) * 2)),
);
const progress = computed(() => {
  if (displaySecs.value === null) return 0;
  return Math.max(0, Math.min(100, (100 * displaySecs.value) / maxSecs.value));
});
const unavailable = computed(
  () => props.disabled || props.currentSecs === null,
);
function preview(event: Event) {
  const input = event.target as HTMLInputElement;
  draft.value = { seconds: Number(input.value), max: Number(input.max) };
}
function seek(event: Event) {
  if (!unavailable.value)
    emit("seek", Number((event.target as HTMLInputElement).value));
  draft.value = null;
}
watch([() => props.totalSecs, unavailable], () => {
  draft.value = null;
});
</script>

<template>
  <input
    class="progress-input"
    type="range"
    min="0"
    step="1"
    :max="maxSecs"
    :value="displaySecs ?? 0"
    :disabled="unavailable"
    :style="{ '--progress': `${progress}%` }"
    aria-label="播放进度"
    :aria-valuetext="formatTime(displaySecs)"
    @input="preview"
    @change="seek"
    @pointercancel="draft = null"
    @blur="draft = null"
  />
  <div class="time-labels">
    <span>{{ formatTime(displaySecs) }}</span
    ><span>{{ live ? "直播中" : formatTime(totalSecs) }}</span>
  </div>
</template>

<style scoped>
.progress-input {
  appearance: none;
  -webkit-appearance: none;
  display: block;
  width: 100%;
  height: 24px;
  margin: 0;
  padding: 0;
  background: transparent;
  cursor: pointer;
}
.progress-input::-webkit-slider-runnable-track {
  height: 4px;
  border-radius: 2px;
  background: linear-gradient(
    to right,
    var(--accent) var(--progress),
    var(--line) var(--progress)
  );
}
.progress-input::-webkit-slider-thumb {
  appearance: none;
  -webkit-appearance: none;
  width: 12px;
  height: 12px;
  margin-top: -4px;
  border: 0;
  border-radius: 50%;
  background: var(--accent);
}
.progress-input::-moz-range-track {
  height: 4px;
  border-radius: 2px;
  background: var(--line);
}
.progress-input::-moz-range-progress {
  height: 4px;
  border-radius: 2px;
  background: var(--accent);
}
.progress-input::-moz-range-thumb {
  width: 12px;
  height: 12px;
  border: 0;
  border-radius: 50%;
  background: var(--accent);
}
.progress-input:disabled {
  cursor: default;
  opacity: 0.38;
}
.time-labels {
  display: flex;
  justify-content: space-between;
  color: var(--muted);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
</style>
