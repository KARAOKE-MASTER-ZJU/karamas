import { acceptHMRUpdate, defineStore } from "pinia";
import { computed, ref } from "vue";
import { castingApi } from "@/services/casting";
import { useBilibiliStore } from "@/stores/bilibili";
import type { ConnectionConfig, RoomEntryMode } from "@/types";
import type { CastDevice, PlaybackState } from "@/types/casting";

export const useCastingStore = defineStore("casting", () => {
  const room = ref<ConnectionConfig | null>(null);
  const activeDevice = ref<CastDevice | null>(null);
  const playback = ref<PlaybackState | null>(null);
  const volume = ref<number | null>(null);
  const connecting = ref(false);
  const enteringRoom = ref(false);
  const controlling = ref(false);
  const connected = computed(
    () => activeDevice.value !== null || playback.value !== null,
  );
  const busy = computed(
    () => connecting.value || enteringRoom.value || controlling.value,
  );
  const auth = useBilibiliStore();
  // 会话变化后丢弃尚未返回的旧状态，避免重连时界面回退。
  let revision = 0;
  let polling = false;

  async function connect(device: CastDevice) {
    if (busy.value) return;
    if (!room.value) throw new Error("请先创建或加入点歌房间。");
    const session = auth.session;
    if (
      device.kind === "bilibili" &&
      (!session ||
        (session.expires_at !== null &&
          session.expires_at <= Date.now() / 1000))
    )
      throw new Error("请先登录哔哩哔哩。");
    connecting.value = true;
    ++revision;
    playback.value = null;
    activeDevice.value = null;
    volume.value = null;
    try {
      await castingApi.start(
        room.value,
        device.kind === "dlna"
          ? { kind: "dlna", location: device.location }
          : { kind: "bilibili", deviceBuvid: device.buvid, session: session! },
      );
      activeDevice.value = device;
    } finally {
      connecting.value = false;
    }
  }
  async function enterRoom(config: ConnectionConfig, mode: RoomEntryMode) {
    if (busy.value) return;
    enteringRoom.value = true;
    try {
      await castingApi.enterRoom(config, mode);
      room.value = { ...config };
    } finally {
      enteringRoom.value = false;
    }
    // 切换房间时让实际会话跟随已确认的房间。
    if (activeDevice.value) await connect(activeDevice.value);
  }
  async function disconnect() {
    if (busy.value) return;
    connecting.value = true;
    ++revision;
    try {
      await castingApi.disconnect();
      playback.value = null;
      activeDevice.value = null;
      volume.value = null;
    } finally {
      connecting.value = false;
    }
  }
  async function refreshVolume() {
    const currentRevision = revision;
    if (!playback.value?.capabilities.absoluteVolume) return;
    const value = await castingApi.getVolume();
    if (currentRevision === revision) volume.value = value;
  }
  async function poll() {
    if (polling || connecting.value || controlling.value) return;
    polling = true;
    const currentRevision = revision;
    try {
      const value = await castingApi.poll();
      if (currentRevision !== revision) return;
      playback.value = value;
      if (!value) {
        activeDevice.value = null;
        volume.value = null;
      } else if (volume.value === null && value.capabilities.absoluteVolume) {
        await refreshVolume();
      }
    } finally {
      polling = false;
    }
  }
  async function control(action: () => Promise<unknown>) {
    if (!connected.value || busy.value) return;
    controlling.value = true;
    // 先前开始的轮询不能覆盖刚完成的控制结果。
    ++revision;
    try {
      await action();
    } finally {
      controlling.value = false;
    }
  }
  async function togglePause() {
    await control(async () => {
      const playing = await castingApi.togglePause();
      if (playback.value) playback.value.isPlaying = playing;
    });
  }
  async function changeVolume(increase: boolean) {
    await control(async () => {
      await castingApi.changeVolume(increase);
      await refreshVolume();
    });
  }
  async function setVolume(value: number) {
    await control(async () => {
      volume.value = await castingApi.setVolume(value);
    });
  }
  return {
    room,
    activeDevice,
    playback,
    volume,
    connecting,
    enteringRoom,
    controlling,
    connected,
    busy,
    connect,
    enterRoom,
    disconnect,
    poll,
    control,
    togglePause,
    changeVolume,
    setVolume,
  };
});
if (import.meta.hot)
  import.meta.hot.accept(acceptHMRUpdate(useCastingStore, import.meta.hot));
