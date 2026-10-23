import type { LogLevel } from "@/types";

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
  (DlnaDevice & { kind: "dlna" }) | (BilibiliDevice & { kind: "bilibili" });
export interface PlaybackState {
  songId: string | null;
  title: string | null;
  currentSecs: number | null;
  totalSecs: number | null;
  isPlaying: boolean;
  queuedCount: number;
  sungCount: number;
  qualityQn: number | null;
  danmaku: boolean | null;
  capabilities: {
    absoluteVolume: boolean;
    seek: boolean;
    hardwareProgress: boolean;
  };
}
export interface NativeLog {
  level: LogLevel;
  target: string;
  message: string;
}

export type RecastResult = "played" | "noCurrentSong" | "busy";
export interface BilibiliPart {
  pageIndex: number;
  page: number;
  cid: number;
  part: string;
  durationSecs: number;
}
export interface CurrentBilibiliParts {
  songId: string;
  bvid: string;
  currentPageIndex: number;
  parts: BilibiliPart[];
}
