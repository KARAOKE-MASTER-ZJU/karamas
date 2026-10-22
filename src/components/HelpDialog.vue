<!-- HelpDialog.vue: 使用帮助对话框组件 -->

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { X } from "@lucide/vue";
const emit = defineEmits<{ close: [] }>();
const dialog = ref<HTMLDialogElement | null>(null);
onMounted(() => dialog.value?.showModal());
function closeOnBackdrop(event: MouseEvent) {
  if (event.target === dialog.value) dialog.value?.close();
}
</script>
<template>
  <dialog
    ref="dialog"
    class="help-dialog"
    aria-labelledby="help-title"
    @click="closeOnBackdrop"
    @close="emit('close')"
  >
    <div class="dialog-heading">
      <h2 id="help-title" tabindex="-1" autofocus>使用帮助</h2>
      <button
        class="icon-button"
        aria-label="关闭帮助"
        @click="dialog?.close()"
      >
        <X />
      </button>
    </div>
    <h3>投屏设备</h3>
    <p>DLNA 需要本机与电视连接同一网络。小电视云投屏需要登录哔哩哔哩账号。</p>
    <div class="dialog-actions">
      <button class="button primary" @click="dialog?.close()">知道了</button>
    </div>
  </dialog>
</template>

<style scoped>
/* 使用帮助对话框的样式，设置边框、背景色、字体和阴影效果 */
.help-dialog {
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface);
  color: var(--text);
  width: min(460px, calc(100% - 40px));
  padding: 24px;
  box-shadow: 0 16px 64px #0003;
}
.help-dialog::backdrop {
  background: #0006;
}
.dialog-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 20px;
}
.dialog-heading h2:focus {
  outline: none;
}
.help-dialog h3 {
  margin: 20px 0 8px;
}
.help-dialog p {
  font-size: 14px;
  line-height: 1.8;
  margin-bottom: 16px;
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 24px;
}
</style>
