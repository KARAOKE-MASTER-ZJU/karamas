import { onMounted, onScopeDispose, watch } from "vue";
import { castingApi, errorMessage, isTauri } from "@/services/casting";
import { useCastingStore } from "@/stores/casting";
import { useBilibiliStore } from "@/stores/bilibili";
import { useLogsStore } from "@/stores/logs";
import { usePlaybackOutputStore } from "@/stores/playbackOutput";
import { usePreferencesStore } from "@/stores/preferences";
import { useOperationNotice } from "@/composables/useOperationNotice";
import {
  isPlaybackController,
  setPlaybackOutput,
} from "@/services/playbackWindow";

// 协调投屏状态轮询、播放输出切换和界面恢复。
export function useCastingRuntime() {
  const casting = useCastingStore();
  const auth = useBilibiliStore();
  const logs = useLogsStore();
  const playbackOutput = usePlaybackOutputStore();
  const preferences = usePreferencesStore();
  const { notify } = useOperationNotice();
  function restoreLogin() {
    return auth.restore().catch((error: unknown) => {
      notify(`读取登录状态失败：${errorMessage(error)}`, "warning");
    });
  }
  if (!isPlaybackController) {
    watch(
      () =>
        [
          casting.activeDevice?.kind,
          casting.connecting,
          casting.enteringRoom,
        ] as const,
      ([kind, connecting, enteringRoom]) => {
        if (
          kind !== "local" &&
          !connecting &&
          !enteringRoom &&
          playbackOutput.state.target.kind !== "embedded"
        ) {
          void setPlaybackOutput("embedded").catch((error: unknown) => {
            notify(errorMessage(error), "error");
          });
        }
      },
    );
    watch(
      () => playbackOutput.videoOnly,
      (videoOnly) => {
        if (videoOnly) return;
        preferences.reload();
        void restoreLogin();
      },
    );
  }
  watch(
    () => casting.playback?.playlistSyncError,
    (message) => {
      if (message) notify(message, "error");
    },
  );
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
    await casting.connectionReady;
    if (disposed) return;
    try {
      await casting.initializeDlnaProxy();
    } catch (error) {
      notify(`恢复反向代理设置失败：${errorMessage(error)}`, "error");
    }
    if (disposed) return;
    try {
      await castingApi.setLogLevel(logs.recordingLevel);
    } catch (error) {
      notify(`设置日志记录级别失败：${errorMessage(error)}`, "error");
    }
    if (disposed) return;
    void restoreLogin();
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
