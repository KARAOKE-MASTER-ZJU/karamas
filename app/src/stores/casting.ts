import { acceptHMRUpdate, defineStore } from "pinia";
import { computed, ref, shallowRef } from "vue";
import { castingApi, errorMessage, isTauri } from "@/services/casting";
import { useBilibiliMediaQualities } from "@/composables/useBilibiliMediaQualities";
import { useBilibiliStore } from "@/stores/bilibili";
import { usePreferencesStore } from "@/stores/preferences";
import type { ConnectionConfig, RoomEntryMode } from "@/types";
import type {
  CastDevice,
  PlaybackQuality,
  PlaybackState,
} from "@/types/casting";
import type { LocalCommand, LocalReport } from "@/types/localPlayer";

export const useCastingStore = defineStore("casting", () => {
  const preferences = usePreferencesStore();
  const localCommand = shallowRef<LocalCommand | null>(null);
  let localReport: LocalReport | null = null;
  let localReportReceivedAt = 0;

  function syncLocalState() {
    const command = localCommand.value;
    if (
      activeDevice.value?.kind !== "local" ||
      !command ||
      !localReport ||
      Date.now() - localReportReceivedAt >= 5000 ||
      localReport.sessionId !== command.sessionId ||
      localReport.commandId !== command.commandId
    )
      return;
    volume.value = localReport.muted ? 0 : localReport.volume;
    if (playback.value) {
      playback.value.isPlaying =
        !localReport.paused && !localReport.ended && !localReport.error;
      playback.value.currentSecs =
        localReport.totalSecs > 0 ? Math.floor(localReport.currentSecs) : null;
      playback.value.totalSecs =
        localReport.totalSecs > 0 ? Math.floor(localReport.totalSecs) : null;
    }
  }

  // 原生控件和页面控件共用播放器回传，更新界面时不再次发送控制命令。
  function applyLocalReport(report: LocalReport) {
    const command = localCommand.value;
    if (
      !command ||
      report.sessionId !== command.sessionId ||
      report.commandId !== command.commandId
    )
      return;
    localReport = report;
    localReportReceivedAt = Date.now();
    syncLocalState();
  }
  const dlnaProxyEnabled = ref(preferences.preferences.dlnaProxyEnabled);
  const changingDlnaProxy = ref(false);
  const room = ref<ConnectionConfig | null>(null);
  const selectedDevice = ref<CastDevice | null>(null);
  const activeDevice = ref<CastDevice | null>(null);
  const playback = ref<PlaybackState | null>(null);
  const {
    qualities: mediaQualities,
    loading: mediaQualitiesLoading,
    error: mediaQualitiesError,
    refresh: refreshMediaQualities,
  } = useBilibiliMediaQualities(
    computed(() => {
      const kind = activeDevice.value?.kind;
      return kind === "local" || kind === "dlna"
        ? (playback.value?.songUrl ?? null)
        : null;
    }),
  );
  const volume = ref<number | null>(null);
  const connecting = ref(false);
  const enteringRoom = ref(false);
  const controlling = ref(false);
  const recasting = ref(false);
  const connected = computed(
    () => activeDevice.value !== null || playback.value !== null,
  );
  const sessionBusy = computed(
    () => connecting.value || enteringRoom.value || changingDlnaProxy.value,
  );
  const busy = computed(() => sessionBusy.value || controlling.value);
  const auth = useBilibiliStore();
  // 会话变化后丢弃尚未返回的旧状态，避免重连时界面回退。
  let revision = 0;
  let polling = false;
  let proxyInitialization: Promise<void> | undefined;

  // 启动恢复和首次连接共用同一个任务，确保加载媒体前已应用配置。
  function initializeDlnaProxy(): Promise<void> {
    if (!isTauri()) return Promise.resolve();
    proxyInitialization ??= castingApi
      .setDlnaDirect(!dlnaProxyEnabled.value)
      .catch((error: unknown) => {
        proxyInitialization = undefined;
        throw error;
      });
    return proxyInitialization;
  }

  async function setDlnaProxyEnabled(enabled: boolean) {
    if (busy.value) return;
    changingDlnaProxy.value = true;
    ++revision;
    try {
      await initializeDlnaProxy();
      if (isTauri()) await castingApi.setDlnaDirect(!enabled);
      dlnaProxyEnabled.value = enabled;
      return preferences.saveDlnaProxyEnabled(enabled);
    } finally {
      changingDlnaProxy.value = false;
    }
  }

  function requireSession(device: CastDevice) {
    const session = auth.session;
    if (
      device.kind === "bilibili" &&
      (!session ||
        (session.expires_at !== null &&
          session.expires_at <= Date.now() / 1000))
    )
      throw new Error("请先登录哔哩哔哩。");
    return session;
  }

  // 房间和设备分别确认，两项齐全后才启动投屏会话。
  async function startIfReady() {
    const config = room.value;
    const device = selectedDevice.value;
    if (!config || !device) return;
    connecting.value = true;
    localCommand.value = null;
    localReport = null;
    ++revision;
    playback.value = null;
    activeDevice.value = null;
    volume.value = null;
    try {
      const session = requireSession(device);
      if (device.kind === "dlna") await initializeDlnaProxy();
      await castingApi.start(
        config,
        device.kind === "dlna"
          ? { kind: "dlna", location: device.location }
          : device.kind === "local"
            ? { kind: "local" }
            : {
                kind: "bilibili",
                deviceBuvid: device.buvid,
                senderBuvid: preferences.ensureBilibiliBuvid(),
                session: session!,
              },
        device.kind === "local"
          ? (command) => {
              ++revision;
              localCommand.value = command;
            }
          : undefined,
      );
      activeDevice.value = device;
      syncLocalState();
    } catch (error) {
      localCommand.value = null;
      localReport = null;
      // 输入校验等前置失败也需要停止旧会话，避免仍播放旧房间。
      try {
        await castingApi.disconnect();
      } catch (cleanupError) {
        throw Object.assign(
          new Error(
            `启动投屏失败：${errorMessage(error)}；清理会话失败：${errorMessage(cleanupError)}`,
          ),
          { cause: cleanupError },
        );
      }
      throw Object.assign(new Error(`启动投屏失败：${errorMessage(error)}`), {
        cause: error,
      });
    } finally {
      connecting.value = false;
    }
  }
  async function connect(device: CastDevice) {
    if (busy.value) return;
    requireSession(device);
    selectedDevice.value = device;
    await startIfReady();
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
    await startIfReady();
  }
  async function disconnect() {
    if (busy.value) return;
    connecting.value = true;
    ++revision;
    try {
      if (connected.value) await castingApi.disconnect();
      localCommand.value = null;
      localReport = null;
      playback.value = null;
      activeDevice.value = null;
      selectedDevice.value = null;
      volume.value = null;
    } finally {
      connecting.value = false;
    }
  }
  async function refreshVolume() {
    const currentRevision = revision;
    if (
      activeDevice.value?.kind === "local" ||
      !playback.value?.capabilities.absoluteVolume
    )
      return;
    const value = await castingApi.getVolume();
    if (currentRevision === revision) volume.value = value;
  }
  async function poll() {
    if (
      polling ||
      connecting.value ||
      controlling.value ||
      changingDlnaProxy.value
    )
      return;
    polling = true;
    const currentRevision = revision;
    try {
      const value = await castingApi.poll();
      if (currentRevision !== revision) return;
      playback.value = value;
      syncLocalState();
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
  async function control<T>(action: () => Promise<T>): Promise<T | undefined> {
    if (!connected.value || busy.value) return;
    controlling.value = true;
    // 先前开始的轮询不能覆盖刚完成的控制结果。
    ++revision;
    try {
      return await action();
    } finally {
      controlling.value = false;
    }
  }
  async function play() {
    await control(async () => {
      const playing = await castingApi.play();
      if (playback.value && activeDevice.value?.kind !== "local")
        playback.value.isPlaying = playing;
    });
  }
  async function pause() {
    await control(async () => {
      const playing = await castingApi.pause();
      if (playback.value && activeDevice.value?.kind !== "local")
        playback.value.isPlaying = playing;
    });
  }
  async function togglePause() {
    await control(async () => {
      const playing = await castingApi.togglePause();
      if (playback.value && activeDevice.value?.kind !== "local")
        playback.value.isPlaying = playing;
    });
  }
  async function changeVolume(increase: boolean) {
    if (!connected.value || sessionBusy.value) return;
    if (playback.value?.capabilities.absoluteVolume) {
      if (volume.value !== null)
        await setVolume(volume.value + (increase ? 5 : -5));
    } else {
      await castingApi.changeVolume(increase);
    }
  }
  async function setVolume(value: number) {
    if (!connected.value || sessionBusy.value) return;
    if (!playback.value?.capabilities.absoluteVolume) return;
    // 立即显示目标值，连续操作无需等待响应；旧的音量读取不能覆盖它。
    ++revision;
    const target = Math.max(0, Math.min(100, value));
    if (activeDevice.value?.kind !== "local") volume.value = target;
    await castingApi.setVolume(target);
  }

  async function recast() {
    return control(async () => {
      recasting.value = true;
      try {
        const result = await castingApi.recast();
        if (
          result === "played" &&
          playback.value &&
          activeDevice.value?.kind !== "local"
        )
          playback.value.isPlaying = true;
        return result;
      } finally {
        recasting.value = false;
      }
    });
  }
  async function seek(seconds: number) {
    await control(async () => {
      const current = playback.value;
      const previous = current?.currentSecs ?? null;
      // 提交后立即接替拖动预览，避免等待设备响应时跳回旧位置。
      if (current) current.currentSecs = seconds;
      try {
        await castingApi.seek(seconds);
      } catch (error) {
        if (current && playback.value === current)
          current.currentSecs = previous;
        throw error;
      }
    });
  }
  async function setQuality(selection: PlaybackQuality) {
    await control(async () => {
      const quality = await castingApi.setQuality(selection);
      if (playback.value) {
        playback.value.qualityQn = quality.qn;
        playback.value.qualityFormat = quality.format;
      }
    });
  }
  async function setDanmaku(enabled: boolean) {
    await control(async () => {
      const danmaku = await castingApi.setDanmaku(enabled);
      if (playback.value) playback.value.danmaku = danmaku;
    });
  }
  return {
    applyLocalReport,
    localCommand,
    dlnaProxyEnabled,
    changingDlnaProxy,
    initializeDlnaProxy,
    setDlnaProxyEnabled,
    room,
    selectedDevice,
    activeDevice,
    playback,
    mediaQualities,
    mediaQualitiesLoading,
    mediaQualitiesError,
    refreshMediaQualities,
    volume,
    connecting,
    enteringRoom,
    controlling,
    recasting,
    connected,
    sessionBusy,
    busy,
    connect,
    enterRoom,
    disconnect,
    poll,
    control,
    play,
    pause,
    togglePause,
    changeVolume,
    setVolume,
    recast,
    seek,
    setQuality,
    setDanmaku,
  };
});
if (import.meta.hot)
  import.meta.hot.accept(acceptHMRUpdate(useCastingStore, import.meta.hot));
