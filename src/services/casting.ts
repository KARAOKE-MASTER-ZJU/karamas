import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ConnectionConfig, LogLevel, RoomEntryMode } from "@/types";
import type {
  BilibiliDevice,
  BilibiliSession,
  DlnaDevice,
  LogSnapshot,
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
  moveToBackground: () => call<void>("move_to_background"),
  setDlnaDirect: (enabled: boolean) =>
    call<void>("set_dlna_direct", { enabled }),
  setLogLevel: (level: LogLevel) => call<void>("set_log_level", { level }),
  writeLog: (message: string, level: LogLevel) =>
    call<void>("write_log", { message, level }),
  getLogs: (level: LogLevel, query: string, limit?: number) =>
    call<LogSnapshot>("get_logs", { level, query, limit: limit ?? null }),
  clearLogs: () => call<void>("clear_logs"),
  canShareLogs: () => call<boolean>("can_share_logs"),
  exportLogs: (action: "save" | "share", position?: { x: number; y: number }) =>
    call<boolean>("export_logs", { action, position: position ?? null }),
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
      | {
          kind: "bilibili";
          session: BilibiliSession;
          deviceBuvid: string;
          senderBuvid: string;
        },
  ) => call<void>("start_casting", { ...room, target }),
  disconnect: () => call<void>("disconnect_casting"),
  poll: () => call<PlaybackState | null>("poll_playback"),
  play: () => call<boolean>("play"),
  pause: () => call<boolean>("pause"),
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
  login: (senderBuvid: string, onQr: (url: string) => void) => {
    if (!isTauri())
      return Promise.reject(new Error("请在 Tauri 应用中使用此功能。"));
    const channel = new Channel<string>();
    channel.onmessage = onQr;
    return call<BilibiliSession>("login_bilibili", {
      onQr: channel,
      senderBuvid,
    });
  },
  cancelLogin: () => call<void>("cancel_bilibili_login"),
  listenLogs: (callback: () => void) =>
    listen<void>("casting://logs-changed", callback),
};
export { isTauri };
