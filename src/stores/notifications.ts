import { acceptHMRUpdate, defineStore } from "pinia";
import { onScopeDispose, ref } from "vue";
import type { NoticeLevel } from "@/types";

// 通知存储
export const useNotificationsStore = defineStore("notifications", () => {
  // 初始化通知消息和级别
  const message = ref("");
  const level = ref<NoticeLevel>("info");
  let timer: ReturnType<typeof setTimeout> | undefined;

  // 销毁通知
  function dismiss() {
    clearTimeout(timer);
    timer = undefined;
    message.value = "";
  }

  // 显示通知
  function show(text: string, severity: NoticeLevel = "info") {
    dismiss();
    message.value = text;
    level.value = severity;
    timer = setTimeout(dismiss, 4500);
  }

  // 在组件卸载及热模块替换时销毁通知
  onScopeDispose(dismiss);
  if (import.meta.hot) import.meta.hot.dispose(dismiss);

  return { message, level, show, dismiss };
});

// 热更新
if (import.meta.hot) {
  import.meta.hot.accept(
    acceptHMRUpdate(useNotificationsStore, import.meta.hot),
  );
}
