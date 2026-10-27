export interface LocalCommand {
  sessionId: string;
  commandId: number;
  action:
    | { kind: "loading" | "play" | "pause" | "stop" }
    | {
        kind: "load";
        url: string;
        position: number;
        playing: boolean;
        volume: number;
        muted: boolean;
      }
    | { kind: "seek"; seconds: number }
    | { kind: "volume"; value: number }
    | { kind: "error"; message: string };
}
export interface LocalReport {
  sessionId: string;
  commandId: number;
  currentSecs: number;
  totalSecs: number;
  paused: boolean;
  /** 媒体正在定位，或用户尚未松开播放器中的拖动操作。 */
  seeking: boolean;
  muted: boolean;
  ended: boolean;
  volume: number;
  acknowledged: boolean;
  error: string | null;
}

export interface LocalPlayerState {
  hasMedia: boolean;
  loading: boolean;
  error: string;
  needsGesture: boolean;
}

export interface LocalPlayerCallbacks {
  onState: (state: LocalPlayerState) => void;
  onPlayback: (report: LocalReport) => void;
  onReport: (report: LocalReport) => Promise<unknown>;
}
