import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ConnectionConfig, RoomEntryMode } from "@/types";
import type {
  BilibiliDevice,
  BilibiliSession,
  DlnaDevice,
  NativeLog,
  PlaybackState,
  RecastResult,
  CurrentBilibiliParts,
} from "@/types/casting";

// 所有原生调用集中在此，页面不直接拼接 IPC 命令和参数。
function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri())
    return Promise.reject(new Error("请在 Tauri 应用中使用此功能。"));
  return invoke<T>(command, args);
}
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof error.message === "string"
  )
    return error.message;
  return "操作失败，请查看运行日志。";
}
export const castingApi = {
  enterRoom: (room: ConnectionConfig, mode: RoomEntryMode) =>
    call<void>("enter_room", { ...room, mode }),
  discoverDlnaDevices: () => call<DlnaDevice[]>("discover_dlna_devices"),
  discoverDlnaDevice: (url: string) =>
    call<DlnaDevice>("discover_dlna_device", { url }),
  listBilibiliDevices: (session: BilibiliSession) =>
    call<BilibiliDevice[]>("list_bilibili_devices", { session }),
  start: (
    room: ConnectionConfig,
    target:
      | { kind: "dlna"; location: string }
      | { kind: "bilibili"; session: BilibiliSession; deviceBuvid: string },
  ) => call<void>("start_casting", { ...room, target }),
  disconnect: () => call<void>("disconnect_casting"),
  poll: () => call<PlaybackState | null>("poll_playback"),
  togglePause: () => call<boolean>("toggle_pause"),
  nextSong: () => call<void>("next_song"),
  previousSong: () => call<void>("previous_song"),
  recast: () => call<RecastResult>("recast_current_song"),
  setQuality: (qn: number) => call<number>("set_quality", { qn }),
  setDanmaku: (enabled: boolean) => call<boolean>("set_danmaku", { enabled }),
  getBilibiliParts: () =>
    call<CurrentBilibiliParts>("get_current_bilibili_parts"),
  selectBilibiliPart: (songId: string, pageIndex: number) =>
    call<boolean>("select_current_bilibili_part", { songId, pageIndex }),
  seek: (seconds: number) => call<void>("seek", { seconds }),
  getVolume: () => call<number>("get_volume"),
  setVolume: (volume: number) => call<number>("set_volume", { volume }),
  changeVolume: (increase: boolean) =>
    call<void>("change_volume", { step: 5, increase }),
  loadSession: () => call<BilibiliSession | null>("load_bilibili_session"),
  saveSession: (session: BilibiliSession) =>
    call<void>("save_bilibili_session", { session }),
  clearSession: () => call<void>("clear_bilibili_session"),
  login: (onQr: (url: string) => void) => {
    if (!isTauri())
      return Promise.reject(new Error("请在 Tauri 应用中使用此功能。"));
    const channel = new Channel<string>();
    channel.onmessage = onQr;
    return call<BilibiliSession>("login_bilibili", { onQr: channel });
  },
  cancelLogin: () => call<void>("cancel_bilibili_login"),
  listenLogs: (callback: (log: NativeLog) => void) =>
    listen<NativeLog>("casting://log", ({ payload }) => callback(payload)),
};
export { isTauri };
