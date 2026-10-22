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
  let unlisten: (() => void) | undefined;
  let lastPollError = "";
  async function poll() {
    try {
      await casting.poll();
      lastPollError = "";
    } catch (error) {
      const message = errorMessage(error);
      if (message !== lastPollError)
        logs.add(`读取播放状态失败：${message}`, "error");
      lastPollError = message;
    }
    if (!disposed) timer = setTimeout(() => void poll(), 1000);
  }
  onMounted(async () => {
    if (!isTauri()) return;
    try {
      const stop = await castingApi.listenLogs(({ level, target, message }) =>
        logs.add(`[${target}] ${message}`, level),
      );
      if (disposed) {
        stop();
        return;
      }
      unlisten = stop;
    } catch (error) {
      logs.add(`订阅原生日志失败：${errorMessage(error)}`, "error");
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
    unlisten?.();
    if (auth.loggingIn) auth.closeLogin();
  }
  onScopeDispose(dispose);
  if (import.meta.hot) import.meta.hot.dispose(dispose);
}
