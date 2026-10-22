<script setup lang="ts">
import { X } from "@lucide/vue";
import { onMounted, ref, useId } from "vue";

defineProps<{ title: string; closeLabel: string }>();
const emit = defineEmits<{ close: [] }>();
const dialog = ref<HTMLDialogElement | null>(null);
const titleId = useId();
onMounted(() => dialog.value?.showModal());
function close() {
  dialog.value?.close();
}
function closeOnBackdrop(event: MouseEvent) {
  if (event.target === dialog.value) close();
}
</script>

<template>
  <dialog
    ref="dialog"
    class="app-dialog"
    :aria-labelledby="titleId"
    @click="closeOnBackdrop"
    @close="emit('close')"
  >
    <div class="dialog-heading">
      <h2 :id="titleId" tabindex="-1" autofocus>{{ title }}</h2>
      <button class="icon-button" :aria-label="closeLabel" @click="close">
        <X />
      </button>
    </div>
    <slot :close="close" />
  </dialog>
</template>

<style scoped>
.app-dialog {
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface);
  color: var(--text);
  width: min(460px, calc(100% - 40px));
  padding: 24px;
  box-shadow: 0 16px 64px #0003;
}
.app-dialog::backdrop {
  background: #0006;
}
.dialog-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 20px;
  gap: 12px;
}
.dialog-heading h2:focus {
  outline: none;
}
</style>
