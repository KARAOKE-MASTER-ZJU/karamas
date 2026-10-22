<!-- AppToast.vue: 应用提示消息组件 -->

<!--
  应用内临时提示条， 用于显示信息、提醒或错误消息。它接收两个 props：
  - message: 要显示的提示文本。
  - level: 提示的级别，可能的值为 "info"、"warning" 或 "error"，用于决定提示条的样式和图标。

  当用户点击关闭按钮时，会触发一个 "dismiss" 事件，通知父组件提示条已被关闭。
  组件使用了 Vue 的 Transition 组件来实现提示条的淡入和淡出动画效果。
-->

<script setup lang="ts">
import { Check, Info, X } from "@lucide/vue";
import type { NoticeLevel } from "@/types";
defineProps<{ message: string; level: NoticeLevel }>();
const emit = defineEmits<{ dismiss: [] }>();
</script>

<template>
  <Transition name="toast">
    <div v-if="message" class="toast-message" :class="level" role="status">
      <component :is="level === 'info' ? Check : Info" :size="18" />
      <span>{{ message }}</span>
      <button
        class="icon-button"
        aria-label="关闭提示"
        @click="emit('dismiss')"
      >
        <X :size="16" />
      </button>
    </div>
  </Transition>
</template>

<style scoped>
/* 提示消息条的样式，设置位置、布局、背景色、字体和阴影效果 */
.toast-message {
  position: fixed;
  bottom: 24px;
  left: calc(50% + var(--navigation-rail-width) / 2);
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 10px;
  background: var(--text);
  color: var(--surface);
  border-radius: 8px;
  padding: 10px 10px 10px 16px;
  max-width: min(580px, calc(100vw - var(--navigation-rail-width) - 32px));
  font-size: 13px;
  line-height: 1.6;
  z-index: 20;
  box-shadow: 0 4px 20px #0002;
}
.toast-message .icon-button {
  color: inherit;
}
.toast-enter-active,
.toast-leave-active {
  transition: opacity 0.15s;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
}
@media (max-width: 640px) {
  .toast-message {
    bottom: calc(var(--navigation-bar-height) + 16px);
    max-width: calc(100vw - 32px);
    width: max-content;
  }
}
</style>
