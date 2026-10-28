import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { castingApi, errorMessage, isTauri } from "@/services/casting";
import { updaterApi, type UpdateInfo } from "@/services/updater";
import { useCastingStore } from "@/stores/casting";
import { useNotificationsStore } from "@/stores/notifications";
import { version } from "../../package.json";

type UpdateStatus =
  "idle" | "checking" | "available" | "downloading" | "ready" | "installing";

export const useUpdaterStore = defineStore("updater", () => {
  const supported = ref(false);
  const currentVersion = ref(version);
  const canCancel = ref(false);
  const cancelling = ref(false);
  const status = ref<UpdateStatus>("idle");
  const update = ref<UpdateInfo | null>(null);
  const showDialog = ref(false);
  const error = ref("");
  const downloaded = ref(0);
  const total = ref<number | null>(null);
  const busy = computed(() =>
    ["checking", "downloading", "installing"].includes(status.value),
  );
  const progress = computed(() =>
    total.value
      ? Math.min(100, Math.floor((downloaded.value / total.value) * 100))
      : undefined,
  );

  async function initialize() {
    if (!isTauri()) return;
    try {
      currentVersion.value = await updaterApi.version();
      supported.value = await updaterApi.supported();
      if (supported.value && !import.meta.env.DEV) await check(false);
    } catch (cause) {
      await recordFailure(cause);
    }
  }

  async function recordFailure(cause: unknown) {
    await castingApi
      .writeLog(`应用更新失败：${errorMessage(cause)}`, "warning")
      .catch(() => {});
  }

  async function check(manual = true) {
    if (!supported.value || busy.value) return;
    // 下载好的安装包保留到本次应用退出，重复点击不重新检查或下载。
    if (status.value === "ready") {
      if (manual) showDialog.value = true;
      return;
    }
    status.value = "checking";
    error.value = "";
    try {
      update.value = await updaterApi.check();
      status.value = update.value ? "available" : "idle";
      if (!update.value) {
        showDialog.value = false;
        if (manual) useNotificationsStore().show("当前已是最新版本");
      } else if (manual) {
        showDialog.value = true;
      } else {
        useNotificationsStore().show(
          `发现新版本 ${update.value.version}，可在设置中更新。`,
        );
      }
    } catch (cause) {
      status.value = update.value ? "available" : "idle";
      if (manual) useNotificationsStore().show(errorMessage(cause), "error");
      await recordFailure(cause);
    }
  }

  async function download() {
    if (status.value !== "available") return;
    status.value = "downloading";
    error.value = "";
    downloaded.value = 0;
    total.value = null;
    canCancel.value = false;
    cancelling.value = false;
    try {
      await updaterApi.download((value) => {
        canCancel.value = true;
        downloaded.value = value.downloaded;
        total.value = value.total;
      });
      status.value = "ready";
    } catch (cause) {
      status.value = "available";
      if (!(
        cause &&
        typeof cause === "object" &&
        "code" in cause &&
        cause.code === "UPDATE_CANCELLED"
      )) {
        error.value = errorMessage(cause);
        await recordFailure(cause);
      }
    } finally {
      canCancel.value = false;
      cancelling.value = false;
    }
  }

  async function cancelDownload() {
    if (!canCancel.value || cancelling.value) return;
    cancelling.value = true;
    try {
      await updaterApi.cancelDownload();
    } catch (cause) {
      cancelling.value = false;
      error.value = errorMessage(cause);
    }
  }

  async function install() {
    const casting = useCastingStore();
    if (status.value !== "ready" || casting.busy) return;
    status.value = "installing";
    error.value = "";
    try {
      if (casting.connected) await casting.disconnect();
      await updaterApi.install();
      // Android 交给系统安装器；用户取消后仍可再次安装。
      status.value = "ready";
    } catch (cause) {
      status.value = "ready";
      error.value = errorMessage(cause);
      await recordFailure(cause);
    }
  }

  return {
    supported,
    currentVersion,
    canCancel,
    cancelling,
    cancelDownload,
    status,
    update,
    showDialog,
    error,
    downloaded,
    progress,
    busy,
    initialize,
    check,
    download,
    install,
  };
});
