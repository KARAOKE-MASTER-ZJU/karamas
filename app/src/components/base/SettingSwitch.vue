<script setup lang="ts">
import { Info } from "@lucide/vue";
import { onDeactivated, onMounted, onScopeDispose, ref, useId } from "vue";

defineProps<{
  label: string;
  description?: string;
  checked: boolean;
  busy?: boolean;
  disabled?: boolean;
}>();
const emit = defineEmits<{ change: [checked: boolean] }>();
const id = useId();
const showInfo = ref(false);
const info = ref<HTMLElement | null>(null);
const infoButton = ref<HTMLButtonElement | null>(null);

function closeInfoOutside(event: PointerEvent) {
  if (event.target instanceof Node && !info.value?.contains(event.target))
    showInfo.value = false;
}
function closeInfoOnEscape(event: KeyboardEvent) {
  if (event.key === "Escape" && showInfo.value) {
    showInfo.value = false;
    infoButton.value?.focus();
  }
}
onMounted(() => {
  document.addEventListener("pointerdown", closeInfoOutside);
  document.addEventListener("keydown", closeInfoOnEscape);
});
onDeactivated(() => {
  showInfo.value = false;
});
onScopeDispose(() => {
  document.removeEventListener("pointerdown", closeInfoOutside);
  document.removeEventListener("keydown", closeInfoOnEscape);
});
</script>

<template>
  <div class="setting-row">
    <div ref="info" class="setting-label">
      <span :id="`${id}-label`">{{ label }}</span>
      <button
        v-if="description"
        ref="infoButton"
        type="button"
        class="icon-button info-button"
        :aria-label="`${label}说明`"
        :aria-controls="`${id}-info`"
        :aria-expanded="showInfo"
        @click="showInfo = !showInfo"
      >
        <Info :size="15" aria-hidden="true" />
      </button>
      <div
        v-if="description && showInfo"
        :id="`${id}-info`"
        class="info-popover"
        role="note"
        :aria-label="`${label}说明`"
      >
        <p>{{ description }}</p>
      </div>
    </div>
    <button
      type="button"
      class="setting-switch"
      role="switch"
      :aria-labelledby="`${id}-label`"
      :aria-checked="checked"
      :aria-busy="busy ?? false"
      :disabled="disabled"
      @click="emit('change', !checked)"
    >
      <span />
    </button>
  </div>
</template>

<style scoped>
.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  min-height: 62px;
  padding: 12px 0;
  font-size: 14px;
}
.setting-label {
  position: relative;
  display: flex;
  align-items: center;
  gap: 4px;
}
.info-button {
  width: 28px;
  height: 28px;
  color: var(--muted);
}
.info-popover {
  position: absolute;
  top: calc(100% + 8px);
  left: 0;
  z-index: 10;
  width: min(300px, calc(100vw - 40px));
  padding: 14px 16px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--surface);
  box-shadow: 0 6px 24px #0002;
  color: var(--muted);
  font-size: 13px;
  line-height: 1.7;
}
.setting-switch {
  flex-shrink: 0;
  width: 38px;
  height: 22px;
  border: 0;
  border-radius: 11px;
  padding: 3px;
  background: var(--input-line);
}
.setting-switch[aria-checked="true"] {
  background: var(--accent);
}
.setting-switch span {
  display: block;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #fff;
}
.setting-switch[aria-checked="true"] span {
  transform: translateX(16px);
}
</style>
