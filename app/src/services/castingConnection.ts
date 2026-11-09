import { emit, listen } from "@/services/platform";
import type { ConnectionConfig } from "@/types";
import type { CastDevice } from "@/types/casting";
import type { LocalCommand, LocalReport } from "@/types/localPlayer";
import { isPlaybackController } from "@/services/playbackWindow";

// 只同步连接选择；播放命令、进度和媒体状态仍分别来自核心和唯一播放器。
export interface CastingConnection {
  room: ConnectionConfig | null;
  selectedDevice: CastDevice | null;
  activeDevice: CastDevice | null;
  connecting: boolean;
  enteringRoom: boolean;
  localPlayback: Pick<LocalCommand, "sessionId" | "commandId"> | null;
}

const CONNECTION = "casting-connection-changed";
export async function connectCastingView(callbacks: {
  read: () => CastingConnection;
  apply: (connection: CastingConnection) => void;
  localCommand: (command: LocalCommand | null) => void;
  localReport: (report: LocalReport) => void;
}) {
  const source = crypto.randomUUID();
  const unlisten = await Promise.all([
    listen<{ source: string; connection: CastingConnection }>(
      CONNECTION,
      ({ payload }) => {
        if (payload.source !== source) callbacks.apply(payload.connection);
      },
    ),
    listen("casting-connection-request", () => {
      if (!isPlaybackController) void publish();
    }),
    listen<LocalCommand | null>("local-playback-command", ({ payload }) => {
      if (!isPlaybackController) callbacks.localCommand(payload);
    }),
    listen<LocalReport>("local-playback-report", ({ payload }) => {
      if (isPlaybackController) callbacks.localReport(payload);
    }),
  ]);
  function publish() {
    return emit(CONNECTION, { source, connection: callbacks.read() });
  }
  if (isPlaybackController) await emit("casting-connection-request");
  return { publish, dispose: () => unlisten.forEach((stop) => stop()) };
}

export function publishLocalReport(report: LocalReport) {
  return emit("local-playback-report", report);
}
