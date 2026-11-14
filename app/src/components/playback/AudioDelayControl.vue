<script setup lang="ts">
import { computed, ref, useId, watch } from "vue";
import { Minus, Plus, RotateCcw } from "@lucide/vue";

const props = defineProps<{
  value: number | null;
  disabled: boolean;
  pending: boolean;
}>();
const emit = defineEmits<{ setDelay: [milliseconds: number] }>();
const inputId = useId();
const input = ref<HTMLInputElement | null>(null);
const draft = ref("0");
const error = ref("");
const composing = ref(false);
const editableInteger = /^[+-]?\d*$/;
const milliseconds = computed(() =>
  /^[+-]?\d+$/.test(draft.value.trim()) ? Number(draft.value) : NaN,
);
const valid = computed(
  () =>
    Number.isInteger(milliseconds.value) &&
    Math.abs(milliseconds.value) <= 5000,
);
function setDraft(value: number) {
  draft.value = `${value > 0 ? "+" : ""}${value}`;
  error.value = "";
  input.value?.setCustomValidity("");
}
watch(
  () => props.value,
  (value) => setDraft(value ?? 0),
  { immediate: true },
);
function beforeEdit(event: InputEvent) {
  if (event.isComposing || !event.cancelable) return;
  const text = event.data ?? event.dataTransfer?.getData("text/plain");
  if (text == null) return;
  const target = event.target as HTMLInputElement;
  const start = target.selectionStart ?? target.value.length;
  const end = target.selectionEnd ?? start;
  const next = target.value.slice(0, start) + text + target.value.slice(end);
  if (!editableInteger.test(next)) event.preventDefault();
}
function edit(event: Event) {
  if (composing.value) return;
  const target = event.target as HTMLInputElement;
  if (!editableInteger.test(target.value)) {
    target.value = draft.value;
    return;
  }
  draft.value = target.value;
  error.value = "";
  target.setCustomValidity("");
}
function validate() {
  error.value = valid.value ? "" : "请输入 -5000 到 5000 之间的整数";
  input.value?.setCustomValidity(error.value);
  return valid.value;
}
function finishEditing() {
  if (composing.value) return;
  if (validate()) setDraft(milliseconds.value);
}
function apply() {
  if (
    props.disabled ||
    props.pending ||
    composing.value ||
    props.value === null ||
    milliseconds.value === props.value
  )
    return;
  if (!validate()) {
    input.value?.reportValidity();
    return;
  }
  emit("setDelay", milliseconds.value);
}
</script>

<template>
  <form class="audio-delay" aria-label="声音偏移设置" @submit.prevent="apply">
    <label :for="inputId" class="delay-label">声音偏移</label>
    <fieldset :disabled="disabled || pending || value === null">
      <legend class="sr-only">声音偏移</legend>
      <div class="delay-controls">
        <div class="delay-stepper">
          <button
            type="button"
            class="icon-button"
            aria-label="声音提前 50 毫秒"
            title="声音提前 50 ms"
            :disabled="!valid || milliseconds - 50 < -5000"
            @click="setDraft(milliseconds - 50)"
          >
            <Minus :size="18" aria-hidden="true" />
          </button>
          <span class="delay-value">
            <template v-if="value !== null">
              <input
                :id="inputId"
                ref="input"
                class="delay-input"
                type="text"
                name="audioDelay"
                :value="draft"
                :aria-invalid="error ? true : undefined"
                :aria-describedby="
                  error ? `${inputId}-error` : `${inputId}-unit`
                "
                title="-5000 至 5000 ms"
                autocomplete="off"
                :spellcheck="false"
                @beforeinput="beforeEdit"
                @input="edit"
                @compositionstart="composing = true"
                @compositionend="
                  composing = false;
                  edit($event);
                "
                @blur="finishEditing"
              />
              <span :id="`${inputId}-unit`">ms</span>
            </template>
            <template v-else>暂不支持</template>
          </span>
          <button
            type="button"
            class="icon-button"
            aria-label="声音延后 50 毫秒"
            title="声音延后 50 ms"
            :disabled="!valid || milliseconds + 50 > 5000"
            @click="setDraft(milliseconds + 50)"
          >
            <Plus :size="18" aria-hidden="true" />
          </button>
        </div>
        <button
          type="button"
          class="icon-button"
          aria-label="重置声音偏移"
          title="重置声音偏移"
          :disabled="milliseconds === 0"
          @click="setDraft(0)"
        >
          <RotateCcw :size="18" aria-hidden="true" />
        </button>
        <button
          type="submit"
          class="button secondary"
          :disabled="milliseconds === value"
          :aria-busy="pending"
        >
          应用
        </button>
      </div>
      <p
        v-if="error"
        :id="`${inputId}-error`"
        class="field-error delay-error"
        role="alert"
      >
        {{ error }}
      </p>
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
  gap: 4px;
}
.delay-value {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 2px;
  width: 8ch;
  flex-shrink: 0;
  text-align: center;
  white-space: nowrap;
  font-size: 14px;
  font-variant-numeric: tabular-nums;
}
.delay-input[type="text"] {
  flex: 1;
  width: 0;
  min-width: 0;
  height: 32px;
  padding: 0;
  border: 0;
  border-bottom: 1px solid var(--input-line);
  border-radius: 0;
  background: transparent;
  font: inherit;
  text-align: center;
}
.delay-input[type="text"]:focus-visible {
  outline: none;
  border-bottom-color: var(--accent);
  box-shadow: 0 1px 0 var(--accent);
}
.delay-error {
  margin: 8px 0 0;
  text-align: end;
}
@media (pointer: coarse) {
  .delay-value {
    font-size: 16px;
  }
  .delay-input[type="text"] {
    height: 48px;
  }
  .icon-button {
    width: 48px;
    height: 48px;
  }
  .button {
    min-height: 48px;
  }
}
</style>
