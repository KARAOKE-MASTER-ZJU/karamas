import { computed, onScopeDispose, reactive, ref, watch } from "vue";
import type { Ref } from "vue";
import { useBilibiliStore } from "@/stores/bilibili";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { castingApi, errorMessage } from "@/services/casting";
import type { CastMode } from "@/types";
import type { CastDevice } from "@/types/casting";

export function useDeviceDiscovery(selectedDevice: Ref<CastDevice | null>) {
  const auth = useBilibiliStore();
  const { notify } = useOperationNotice();
  const mode = ref<CastMode>("dlna");
  const dlnaDevices = ref<CastDevice[]>([]);
  const bilibiliDevices = ref<CastDevice[]>([]);
  const searching = reactive({ dlna: false, bilibili: false });
  const deviceUrl = ref("");
  const deviceError = ref("");
  const resolvingDevice = ref(false);
  let disposed = false;
  onScopeDispose(() => {
    disposed = true;
  });
  const devices = computed(() => {
    const found =
      mode.value === "dlna" ? dlnaDevices.value : bilibiliDevices.value;
    const active = selectedDevice.value;
    return active?.kind === mode.value &&
      !found.some((device) => deviceKey(device) === deviceKey(active))
      ? [active, ...found]
      : found;
  });

  async function searchDevices(kind: CastMode = mode.value) {
    if (disposed || searching[kind]) return;
    const session = auth.session;
    if (kind === "bilibili" && !session) {
      void auth.login();
      return;
    }
    searching[kind] = true;
    try {
      if (kind === "dlna") {
        const found = await castingApi.discoverDlnaDevices((device) => {
          if (disposed) return;
          const index = dlnaDevices.value.findIndex(
            (item) => item.kind === "dlna" && item.location === device.location,
          );
          const entry = { ...device, kind: "dlna" as const };
          if (index < 0) dlnaDevices.value.push(entry);
          else dlnaDevices.value[index] = entry;
        });
        // 搜索期间保留已有设备，结束后用完整列表移除本轮未发现的项。
        if (!disposed)
          dlnaDevices.value = found.map((device) => ({
            ...device,
            kind: "dlna",
          }));
      } else if (session) {
        const result = await castingApi.listBilibiliDevices(session);
        if (!disposed && session === auth.session)
          bilibiliDevices.value = result.map((device) => ({
            ...device,
            kind: "bilibili",
          }));
      }
    } catch (error) {
      if (!disposed) notify(errorMessage(error), "error");
    } finally {
      searching[kind] = false;
      if (
        kind === "bilibili" &&
        mode.value === kind &&
        auth.session &&
        session !== auth.session
      )
        void searchDevices(kind);
    }
  }
  watch(
    () => auth.session,
    (session) => {
      bilibiliDevices.value = [];
      if (session && mode.value === "bilibili") void searchDevices("bilibili");
    },
  );
  watch(mode, (kind) => {
    if (kind === "bilibili" && auth.session) void searchDevices(kind);
  });
  function deviceKey(device: CastDevice) {
    return device.kind === "dlna" ? device.location : device.buvid;
  }
  function isSelected(device: CastDevice) {
    return (
      selectedDevice.value?.kind === device.kind &&
      deviceKey(selectedDevice.value) === deviceKey(device)
    );
  }

  async function resolveDevice(
    onResolved: (device: CastDevice) => Promise<void>,
  ) {
    deviceError.value = "";
    try {
      const address = new URL(deviceUrl.value.trim());
      if (
        !["http:", "https:"].includes(address.protocol) ||
        address.username ||
        address.password
      )
        throw new Error();
    } catch {
      deviceError.value = "请输入设备描述文件的完整 HTTP 或 HTTPS 地址。";
      return;
    }
    resolvingDevice.value = true;
    try {
      const device: CastDevice = {
        ...(await castingApi.discoverDlnaDevice(deviceUrl.value.trim())),
        kind: "dlna",
      };
      if (
        !dlnaDevices.value.some((item) => deviceKey(item) === deviceKey(device))
      )
        dlnaDevices.value.push(device);
      await onResolved(device);
    } catch (error) {
      notify(errorMessage(error), "error");
    } finally {
      resolvingDevice.value = false;
    }
  }

  return {
    mode,
    devices,
    searching,
    deviceUrl,
    deviceError,
    resolvingDevice,
    searchDevices,
    resolveDevice,
    deviceKey,
    isSelected,
  };
}
