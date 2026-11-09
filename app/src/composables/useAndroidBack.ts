import { onBackButtonPress } from "@/services/platform";
import { playbackOutput } from "@/services/playbackWindow";
import { onMounted, onScopeDispose, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { castingApi, errorMessage, isTauri } from "@/services/casting";
import { useNotificationsStore } from "@/stores/notifications";

const BACK_HINT = "再返回一次退到桌面";
const BACK_INTERVAL = 2000;

export function useAndroidBack() {
  const route = useRoute();
  const router = useRouter();
  const notifications = useNotificationsStore();
  let lastBackAt = 0;
  let exitingFullscreen = false;
  let listener: { unregister: () => Promise<void> } | undefined;
  let disposed = false;

  function reset() {
    lastBackAt = 0;
    if (notifications.message === BACK_HINT) notifications.dismiss();
  }

  async function back() {
    // 退出全屏的返回操作不参与“双击返回桌面”的判断。
    if (exitingFullscreen) return;
    if (document.fullscreenElement) {
      reset();
      exitingFullscreen = true;
      try {
        await document.exitFullscreen();
      } finally {
        exitingFullscreen = false;
      }
      return;
    }
    const dialogs =
      document.querySelectorAll<HTMLDialogElement>("dialog[open]");
    const dialog = dialogs[dialogs.length - 1];
    if (dialog) {
      reset();
      if (dialog.dispatchEvent(new Event("cancel", { cancelable: true })))
        dialog.close();
    } else if (route.meta.back) {
      reset();
      await router.replace(route.meta.back.to);
    } else if (lastBackAt && performance.now() - lastBackAt < BACK_INTERVAL) {
      reset();
      await castingApi.moveToBackground();
    } else {
      lastBackAt = performance.now();
      notifications.show(BACK_HINT, "info", BACK_INTERVAL);
    }
  }

  watch(() => route.fullPath, reset);
  async function registerBack() {
    if (
      !isTauri() ||
      !/Android/i.test(navigator.userAgent) ||
      playbackOutput.videoOnly
    )
      return;
    document.addEventListener("visibilitychange", reset);
    document.addEventListener("fullscreenchange", reset);
    try {
      const registered = await onBackButtonPress(() => {
        void back().catch((error: unknown) =>
          notifications.show(errorMessage(error), "error"),
        );
      });
      if (disposed || playbackOutput.videoOnly) await registered.unregister();
      else listener = registered;
    } catch (error) {
      notifications.show(`无法接管返回手势：${errorMessage(error)}`, "error");
    }
  }
  onMounted(registerBack);
  watch(
    () => playbackOutput.videoOnly,
    async (videoOnly) => {
      await listener?.unregister();
      listener = undefined;
      if (!videoOnly) await registerBack();
    },
  );
  onScopeDispose(() => {
    disposed = true;
    document.removeEventListener("visibilitychange", reset);
    document.removeEventListener("fullscreenchange", reset);
    void listener?.unregister();
  });
}
