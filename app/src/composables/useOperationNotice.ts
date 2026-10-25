// 显示临时操作提示。
import { useNotificationsStore } from "@/stores/notifications";
import type { NoticeLevel } from "@/types";

export function useOperationNotice() {
  const notifications = useNotificationsStore();

  function notify(message: string, level: NoticeLevel = "info") {
    notifications.show(message, level);
  }

  return { notify };
}
