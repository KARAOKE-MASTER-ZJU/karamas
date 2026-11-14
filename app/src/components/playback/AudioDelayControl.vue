<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Minus, Plus, RotateCcw } from "@lucide/vue";

const props = defineProps<{
  value: number | null;
  disabled: boolean;
  pending: boolean;
}>();
const emit = defineEmits<{ setDelay: [milliseconds: number] }>();
const draft = ref(0);
watch(
  () => props.value,
  (value) => {
    draft.value = value ?? 0;
  },
  { immediate: true },
);
const offset = computed(() => {
  if (props.value === null) return "暂不支持";
  return `${draft.value > 0 ? "+" : ""}${draft.value} ms`;
});
function apply() {
  if (
    props.disabled ||
    props.pending ||
    props.value === null ||
    draft.value === props.value
  )
    return;
  emit("setDelay", draft.value);
}
</script>

<template>
  <form class="audio-delay" aria-label="声音偏移设置" @submit.prevent="apply">
    <span class="delay-label">声音偏移</span>
    <fieldset :disabled="disabled || pending || value === null">
      <legend class="sr-only">声音偏移</legend>
      <div class="delay-controls">
        <div class="delay-stepper">
          <button
            type="button"
            class="icon-button"
            aria-label="声音提前 50 毫秒"
            title="声音提前 50 ms"
            :disabled="draft - 50 < -5000"
            @click="draft -= 50"
          >
            <Minus :size="18" aria-hidden="true" />
          </button>
          <output class="delay-value" aria-live="polite" aria-atomic="true">{{
            offset
          }}</output>
          <button
            type="button"
            class="icon-button"
            aria-label="声音延后 50 毫秒"
            title="声音延后 50 ms"
            :disabled="draft + 50 > 5000"
            @click="draft += 50"
          >
            <Plus :size="18" aria-hidden="true" />
          </button>
        </div>
        <button
          type="button"
          class="icon-button"
          aria-label="重置声音偏移"
          title="重置声音偏移"
          :disabled="draft === 0"
          @click="draft = 0"
        >
          <RotateCcw :size="18" aria-hidden="true" />
        </button>
        <button
          type="submit"
          class="button secondary"
          :disabled="draft === value"
          :aria-busy="pending"
        >
          应用
        </button>
      </div>
    </fieldset>
  </form>
</template>

<style scoped>
.audio-delay {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 8px 16px;
  padding-top: 16px;
}
.delay-label {
  flex-shrink: 0;
  font-size: 14px;
}
fieldset {
  max-width: 100%;
  margin-inline-start: auto;
}
.delay-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-end;
  gap: 4px;
}
.delay-stepper {
  display: flex;
  align-items: center;
}
.delay-value {
  width: 8ch;
  flex-shrink: 0;
  text-align: center;
  white-space: nowrap;
  font-size: 14px;
  font-variant-numeric: tabular-nums;
}
@media (pointer: coarse) {
  .icon-button {
    width: 48px;
    height: 48px;
  }
  .button {
    min-height: 48px;
  }
}
</style>
