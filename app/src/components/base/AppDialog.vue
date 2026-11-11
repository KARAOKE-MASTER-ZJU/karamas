<script setup lang="ts">
import { X } from "@lucide/vue";
import { onMounted, ref, useId } from "vue";

defineProps<{ title: string; closeLabel: string }>();
const emit = defineEmits<{ close: [] }>();
const dialog = ref<HTMLDialogElement | null>(null);
const titleId = useId();
let backdropPointerId: number | null = null;
let backdropClick = false;
onMounted(() => dialog.value?.showModal());
function close() {
  dialog.value?.close();
}
function resetBackdropClick() {
  backdropPointerId = null;
  backdropClick = false;
}
function startBackdropClick(event: PointerEvent) {
  resetBackdropClick();
  if (
    !dialog.value ||
    !event.isPrimary ||
    event.button !== 0 ||
    event.target !== dialog.value
  )
    return;
  backdropPointerId = event.pointerId;
  // 释放触屏的隐式捕获，让松开事件命中手指实际所在的元素。
  if (dialog.value.hasPointerCapture(event.pointerId))
    dialog.value.releasePointerCapture(event.pointerId);
}
function finishBackdropClick(event: PointerEvent) {
  backdropClick =
    event.pointerId === backdropPointerId && event.target === dialog.value;
  backdropPointerId = null;
}
function closeOnBackdrop(event: MouseEvent) {
  if (backdropClick && event.target === dialog.value) close();
  resetBackdropClick();
}
</script>

<template>
  <dialog
    ref="dialog"
    class="app-dialog"
    :aria-labelledby="titleId"
    @pointerdown="startBackdropClick"
    @pointerup="finishBackdropClick"
    @pointercancel="resetBackdropClick"
    @click="closeOnBackdrop"
    @close="emit('close')"
  >
    <div class="dialog-panel">
      <div class="dialog-heading">
        <h2 :id="titleId" tabindex="-1" autofocus>{{ title }}</h2>
        <div class="dialog-heading-actions">
          <slot name="actions" />
          <button
            type="button"
            class="icon-button"
            :aria-label="closeLabel"
            @click="close"
          >
            <X />
          </button>
        </div>
      </div>
      <slot :close="close" />
    </div>
  </dialog>
</template>

<style scoped>
.app-dialog {
  border: 0;
  border-radius: 12px;
  background: transparent;
  color: var(--text);
  width: min(460px, calc(100% - 40px));
  padding: 0;
  box-shadow: 0 16px 64px #0003;
}
.app-dialog::backdrop {
  background: #0006;
}
.dialog-panel {
  border: 1px solid var(--line);
  border-radius: inherit;
  background: var(--surface);
  padding: 24px;
}
.dialog-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 20px;
  gap: 12px;
}
.dialog-heading-actions {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  gap: 4px;
}
.dialog-heading h2:focus {
  outline: none;
}
</style>
