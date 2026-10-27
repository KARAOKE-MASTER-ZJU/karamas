import type { LogEntry } from "@/types";

// 嵌套会话沿用 Rust 的字段名，仅通过原生接口保存。
export interface BilibiliSession {
  access_token: string;
  mid: number;
  expires_at: number | null;
}
export interface DlnaDevice {
  name: string;
  location: string;
}
export interface BilibiliDevice {
  name: string;
  brand: string;
  model: string;
  buvid: string;
}
export type CastDevice =
  | (DlnaDevice & { kind: "dlna" })
  | (BilibiliDevice & { kind: "bilibili" })
  | { kind: "local"; name: string };
export type MediaFormat = "auto" | "mp4" | "dash";
export interface PlaybackQuality {
  qn: number;
  format: MediaFormat;
}
export interface PlaybackState {
  songId: string | null;
  songUrl: string | null;
  title: string | null;
  addedBy: string | null;
  playlistSyncError: string | null;
  currentSecs: number | null;
  totalSecs: number | null;
  isPlaying: boolean;
  queuedCount: number;
  sungCount: number;
  qualityQn: number | null;
  qualityFormat: MediaFormat;
  danmaku: boolean | null;
  capabilities: {
    absoluteVolume: boolean;
    seek: boolean;
    hardwareProgress: boolean;
  };
}
export interface LogSnapshot {
  entries: LogEntry[];
  total: number;
  retained: number;
}

export type RecastResult = "played" | "noCurrentSong" | "busy";
export interface BilibiliPart {
  pageIndex: number;
  page: number;
  cid: number;
  part: string;
  durationSecs: number;
}
export interface BilibiliQuality {
  qn: number;
  label: string;
}
// 媒体资源能力，供 DLNA 和本机播放查询；与设备的编码支持情况分开。
export interface BilibiliMediaQuality extends BilibiliQuality {
  // null 表示该档位的 MP4 查询失败，尚未确认。
  directMp4: boolean | null;
  dash: boolean;
}
export interface CurrentBilibiliParts {
  songId: string;
  songUrl: string;
  bvid: string;
  currentPageIndex: number;
  parts: BilibiliPart[];
}
