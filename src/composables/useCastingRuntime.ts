import { onMounted, onScopeDispose } from "vue";
import { castingApi, errorMessage, isTauri } from "@/services/casting";
import { useCastingStore } from "@/stores/casting";
import { useBilibiliStore } from "@/stores/bilibili";
import { useLogsStore } from "@/stores/logs";
import { useOperationNotice } from "@/composables/useOperationNotice";

// 定期同步播放状态，并驱动自动切歌
export function useCastingRuntime() {
  const casting = useCastingStore();
  const auth = useBilibiliStore();
  const logs = useLogsStore();
  const { notify } = useOperationNotice();
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let lastPollError = "";
  async function poll() {
    try {
      await casting.poll();
      lastPollError = "";
    } catch (error) {
      const message = errorMessage(error);
      if (message !== lastPollError) {
        const detail = `读取播放状态失败：${message}`;
        void logs.record(detail, "error").catch(() => notify(detail, "error"));
      }
      lastPollError = message;
    }
    if (!disposed) timer = setTimeout(() => void poll(), 1000);
  }
  onMounted(async () => {
    if (!isTauri()) return;
    if (disposed) return;
    try {
      await castingApi.setLogLevel(logs.recordingLevel);
    } catch (error) {
      notify(`设置日志记录级别失败：${errorMessage(error)}`, "error");
    }
    if (disposed) return;
    void auth
      .restore()
      .catch((error: unknown) =>
        notify(`读取登录状态失败：${errorMessage(error)}`, "warning"),
      );
    void poll();
  });
  function dispose() {
    disposed = true;
    clearTimeout(timer);
    if (auth.loggingIn) auth.closeLogin();
  }
  onScopeDispose(dispose);
  if (import.meta.hot) import.meta.hot.dispose(dispose);
}
