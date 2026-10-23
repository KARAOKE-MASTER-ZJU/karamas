<script setup lang="ts">
import { computed, useId } from "vue";
import { ChevronDown } from "@lucide/vue";
import type { CastMode } from "@/types";
const props = defineProps<{
  mode: CastMode;
  qualityQn: number | null;
  danmaku: boolean | null;
  disabled: boolean;
}>();
const emit = defineEmits<{
  quality: [qn: number];
  danmaku: [enabled: boolean];
}>();
const qualityId = useId();
const danmakuId = useId();
const qualities = [
  { qn: 16, label: "360P" },
  { qn: 32, label: "480P" },
  { qn: 64, label: "720P" },
  { qn: 80, label: "1080P" },
  { qn: 112, label: "1080PH" },
  { qn: 116, label: "1080P60" },
  { qn: 120, label: "4K" },
];
const options = computed(() =>
  props.mode === "dlna"
    ? qualities
        .filter(({ qn }) => qn === 64 || qn === 80)
        .map((option) => ({
          ...option,
          label: option.qn === 80 ? "1080P (Beta)" : option.label,
        }))
    : qualities,
);
function setQuality(event: Event) {
  emit("quality", Number((event.target as HTMLSelectElement).value));
}
</script>

<template>
  <div class="playback-options">
    <div class="option-row">
      <label :for="qualityId">清晰度</label>
      <div class="select-control quality-select">
        <select
          :id="qualityId"
          :value="qualityQn ?? ''"
          :disabled="disabled || qualityQn === null"
          @change="setQuality"
        >
          <option v-if="qualityQn === null" value="" disabled>—</option>
          <option v-for="option in options" :key="option.qn" :value="option.qn">
            {{ option.label }}
          </option>
        </select>
        <ChevronDown :size="16" aria-hidden="true" />
      </div>
    </div>
    <div v-if="mode === 'bilibili'" class="option-row">
      <span :id="danmakuId">弹幕</span>
      <button
        type="button"
        class="danmaku-switch"
        role="switch"
        :aria-labelledby="danmakuId"
        :aria-checked="danmaku ?? false"
        :disabled="disabled || danmaku === null"
        @click="emit('danmaku', !danmaku)"
      >
        <span />
      </button>
    </div>
  </div>
</template>

<style scoped>
.playback-options {
  margin-top: 24px;
  border-top: 1px solid var(--line);
}
.option-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-top: 16px;
  font-size: 14px;
}
.quality-select {
  width: 168px;
}
.danmaku-switch {
  width: 38px;
  height: 22px;
  border: 0;
  border-radius: 11px;
  padding: 3px;
  background: var(--input-line);
}
.danmaku-switch[aria-checked="true"] {
  background: var(--accent);
}
.danmaku-switch span {
  display: block;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #fff;
}
.danmaku-switch[aria-checked="true"] span {
  transform: translateX(16px);
}
</style>
