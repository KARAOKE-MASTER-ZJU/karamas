import { computed, reactive, ref, watch } from "vue";
import type { Ref } from "vue";
import { useBilibiliStore } from "@/stores/bilibili";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { castingApi, errorMessage } from "@/services/casting";
import type { CastMode } from "@/types";
import type { CastDevice } from "@/types/casting";

export function useDeviceDiscovery(activeDevice: Ref<CastDevice | null>) {
  const auth = useBilibiliStore();
  const { notify } = useOperationNotice();
  const mode = ref<CastMode>("dlna");
  const dlnaDevices = ref<CastDevice[]>([]);
  const bilibiliDevices = ref<CastDevice[]>([]);
  const searching = reactive({ dlna: false, bilibili: false });
  const deviceUrl = ref("");
  const deviceError = ref("");
  const resolvingDevice = ref(false);
  const devices = computed(() => {
    const found =
      mode.value === "dlna" ? dlnaDevices.value : bilibiliDevices.value;
    const active = activeDevice.value;
    return active?.kind === mode.value &&
      !found.some((device) => deviceKey(device) === deviceKey(active))
      ? [active, ...found]
      : found;
  });

  async function searchDevices(kind: CastMode = mode.value) {
    if (searching[kind]) return;
    const session = auth.session;
    if (kind === "bilibili" && !session) {
      void auth.login();
      return;
    }
    searching[kind] = true;
    try {
      if (kind === "dlna")
        dlnaDevices.value = (await castingApi.discoverDlnaDevices()).map(
          (device) => ({ ...device, kind: "dlna" }),
        );
      else if (session) {
        const result = await castingApi.listBilibiliDevices(session);
        if (session === auth.session)
          bilibiliDevices.value = result.map((device) => ({
            ...device,
            kind: "bilibili",
          }));
      }
    } catch (error) {
      notify(errorMessage(error), "error");
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
  function isActive(device: CastDevice) {
    return (
      activeDevice.value?.kind === device.kind &&
      deviceKey(activeDevice.value) === deviceKey(device)
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
    isActive,
  };
}
