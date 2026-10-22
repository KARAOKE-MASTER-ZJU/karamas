<script setup lang="ts">
import { Volume2, Minus, Plus } from "@lucide/vue";
import { useId } from "vue";

defineProps<{ volume: number | null; disabled: boolean; absolute: boolean }>();
const emit = defineEmits<{
  change: [increase: boolean];
  setVolume: [value: number];
}>();
const inputId = useId();
function setVolume(event: Event) {
  emit("setVolume", Number((event.target as HTMLInputElement).value));
}
</script>

<template>
  <div class="volume-heading">
    <label :for="inputId"><Volume2 :size="16" />音量</label>
  </div>
  <fieldset class="volume-control" :disabled="disabled">
    <legend class="sr-only">音量控制，连接设备后可用</legend>
    <button
      class="icon-button"
      aria-label="降低音量"
      @click="emit('change', false)"
    >
      <Minus :size="18" />
    </button>
    <input
      :id="inputId"
      type="range"
      min="0"
      max="100"
      :value="volume ?? 0"
      :disabled="!absolute || volume === null"
      aria-label="音量"
      @change="setVolume"
    />
    <button
      class="icon-button"
      aria-label="提高音量"
      @click="emit('change', true)"
    >
      <Plus :size="18" />
    </button>
  </fieldset>
</template>

<style scoped>
.volume-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  max-width: 320px;
  margin: 0 auto 4px;
  color: var(--muted);
  font-size: 12px;
  line-height: 20px;
}
.volume-heading label {
  display: flex;
  align-items: center;
  gap: 6px;
}
.volume-control {
  display: flex;
  align-items: center;
  gap: 12px;
  margin: 0 auto;
  max-width: 336px;
}
.volume-control input {
  flex: 1;
  width: 0;
  min-width: 0;
  margin: 0;
  accent-color: var(--accent);
}
</style>
