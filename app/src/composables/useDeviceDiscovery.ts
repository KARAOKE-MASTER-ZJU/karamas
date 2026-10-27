import { computed, onScopeDispose, reactive, ref, watch } from "vue";
import type { Ref } from "vue";
import { useBilibiliStore } from "@/stores/bilibili";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { castingApi, errorMessage } from "@/services/casting";
import type { BilibiliDevice, DlnaDevice } from "@/types/casting";
import type { CastMode } from "@/types";

export function useDeviceDiscovery(mode: Ref<CastMode>) {
  const auth = useBilibiliStore();
  const { notify } = useOperationNotice();
  const dlnaDevices = ref<DlnaDevice[]>([]);
  const bilibiliDevices = ref<BilibiliDevice[]>([]);
  const localDevices = [{ kind: "local" as const, name: "本地" }];
  const searching = reactive({ dlna: false, bilibili: false, local: false });
  const deviceUrl = ref("");
  const deviceError = ref("");
  const resolvingDevice = ref(false);
  let disposed = false;
  onScopeDispose(() => {
    disposed = true;
  });
  const devices = computed(() => {
    if (mode.value === "local") return localDevices;
    if (mode.value === "dlna")
      return dlnaDevices.value.map((device) => ({
        ...device,
        kind: "dlna" as const,
      }));
    if (mode.value === "bilibili")
      return bilibiliDevices.value.map((device) => ({
        ...device,
        kind: "bilibili" as const,
      }));
    return [];
  });

  async function searchDevices(kind = mode.value) {
    if (kind === "local" || disposed || searching[kind]) return;
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
            (item) => item.location === device.location,
          );
          if (index < 0) dlnaDevices.value.push(device);
          else dlnaDevices.value[index] = device;
        });
        // 搜索期间保留已有设备，结束后用完整列表移除本轮未发现的项。
        if (!disposed) dlnaDevices.value = found;
      } else if (session) {
        const result = await castingApi.listBilibiliDevices(session);
        if (!disposed && session === auth.session)
          bilibiliDevices.value = result;
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
  async function resolveDevice(
    onResolved: (device: DlnaDevice & { kind: "dlna" }) => Promise<void>,
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
      const device = await castingApi.discoverDlnaDevice(
        deviceUrl.value.trim(),
      );
      if (!dlnaDevices.value.some((item) => item.location === device.location))
        dlnaDevices.value.push(device);
      await onResolved({ ...device, kind: "dlna" });
    } catch (error) {
      notify(errorMessage(error), "error");
    } finally {
      resolvingDevice.value = false;
    }
  }

  return {
    devices,
    searching,
    deviceUrl,
    deviceError,
    resolvingDevice,
    searchDevices,
    resolveDevice,
  };
}
