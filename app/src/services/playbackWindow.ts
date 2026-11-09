import { emitTo, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { supportsMultipleWindows } from "@tauri-apps/api/app";
import { castingApi, isTauri } from "@/services/casting";
import type {
  LocalCommand,
  LocalPlayerState,
  LocalReport,
} from "@/types/localPlayer";

import {
  listenPresentationCommand,
  sendPresentationMessage,
} from "@/services/presentationBridge";

export const isPlaybackController = new URLSearchParams(location.search).has(
  "playback-controller",
);

const COMMAND_EVENT = "playback-window-command";
const MESSAGE_EVENT = "playback-window-message";
const WINDOW_LABEL = "playback";

declare global {
  interface Window {
    __KARAMAS_PLAYBACK_WINDOW__?: string;
  }
}

type CommandMessage =
  | {
      token: string;
      type: "apply";
      sequence: number;
      command: LocalCommand | null;
    }
  | { token: string; type: "suspend"; requestId: number };
type WindowMessage = { token: string } & (
  | { type: "ready" | "closing" | "closed" | "retry" }
  | { type: "playback"; sequence: number; report: LocalReport }
  | { type: "state"; sequence: number; state: LocalPlayerState }
  | { type: "suspended"; requestId: number; command: LocalCommand | null }
);

export async function supportsPlaybackWindow() {
  return isTauri() && (await supportsMultipleWindows());
}

function sendCommand(message: CommandMessage) {
  return window.__KARAMAS_PRESENTATION_SUPPORTED__
    ? castingApi.sendPlaybackWindowMessage(message)
    : emitTo(WINDOW_LABEL, COMMAND_EVENT, message);
}

function pending<T>(message: string, timeout: number) {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  const timer = setTimeout(() => reject(new Error(message)), timeout);
  // 调用方可能仍在等待原生窗口创建。
  void promise.catch(() => {});
  return {
    promise,
    resolve(value: T) {
      clearTimeout(timer);
      resolve(value);
    },
    reject(error: Error) {
      clearTimeout(timer);
      reject(error);
    },
  };
}

export function createPlaybackWindow(callbacks: {
  onReady: () => void;
  onPlayback: (report: LocalReport) => void;
  onState: (state: LocalPlayerState) => void;
  onClosing: () => void;
  onClosed: () => void;
  onRetry: () => void;
}) {
  const token = crypto.randomUUID();
  let sequence = 0;
  let requestId = 0;
  let unlisten: UnlistenFn | undefined;
  let closing = false;
  let ended = false;
  let closeTask: Promise<void> | undefined;
  let ready: ReturnType<typeof pending<void>> | undefined;
  let suspended: ReturnType<typeof pending<LocalCommand | null>> | undefined;
  let destroyed: ReturnType<typeof pending<void>> | undefined;

  async function open(displayId: string | null = null) {
    ready = pending("播放窗口启动超时，请重试。", 15000);
    try {
      unlisten = await listen<WindowMessage>(MESSAGE_EVENT, ({ payload }) => {
        if (payload.token !== token) return;
        switch (payload.type) {
          case "ready":
            ready?.resolve();
            callbacks.onReady();
            break;
          case "playback":
            if (payload.sequence === sequence)
              callbacks.onPlayback(payload.report);
            break;
          case "state":
            if (payload.sequence === sequence) callbacks.onState(payload.state);
            break;
          case "suspended":
            if (payload.requestId === requestId)
              suspended?.resolve(payload.command);
            break;
          case "retry":
            callbacks.onRetry();
            break;
          case "closing":
            callbacks.onClosing();
            break;
          case "closed":
            ended = true;
            destroyed?.resolve();
            ready?.reject(new Error("播放窗口已关闭。"));
            suspended?.reject(new Error("播放窗口已关闭。"));
            unlisten?.();
            if (!closing) callbacks.onClosed();
            break;
        }
      });
      await castingApi.openPlaybackWindow(token, displayId);
      await ready.promise;
    } catch (error) {
      ready.reject(new Error("播放窗口启动失败。"));
      await close();
      throw error;
    }
  }
  async function apply(command: LocalCommand | null) {
    await sendCommand({
      token,
      type: "apply",
      sequence: ++sequence,
      command,
    } satisfies CommandMessage);
  }
  async function suspend() {
    suspended = pending("读取播放窗口状态超时。", 3000);
    try {
      await sendCommand({
        token,
        type: "suspend",
        requestId: ++requestId,
      } satisfies CommandMessage);
      return await suspended.promise;
    } finally {
      suspended?.reject(new Error("播放窗口交接已结束。"));
      suspended = undefined;
    }
  }
  function close(): Promise<void> {
    if (ended) return Promise.resolve();
    if (closeTask) return closeTask;
    closeTask = (async () => {
      closing = true;
      destroyed = pending("关闭播放窗口超时。", 3000);
      try {
        if (await castingApi.closePlaybackWindow(token))
          await destroyed.promise;
        ended = true;
        unlisten?.();
        unlisten = undefined;
        ready?.reject(new Error("播放窗口已关闭。"));
        suspended?.reject(new Error("播放窗口已关闭。"));
      } finally {
        destroyed.reject(new Error("播放窗口关闭流程已结束。"));
        closing = false;
        closeTask = undefined;
      }
    })();
    return closeTask;
  }
  return {
    open,
    apply,
    suspend,
    close,
    focus: () => castingApi.focusPlaybackWindow(token),
  };
}

export async function connectPlaybackWindow(
  token: string,
  player: {
    apply: (command: LocalCommand | null) => Promise<void>;
    suspend: () => Promise<LocalCommand | null>;
  },
) {
  let sequence = 0;
  const send = (message: WindowMessage) =>
    window.KaramasPlayback
      ? sendPresentationMessage(message)
      : emitTo("main", MESSAGE_EVENT, message);
  const onCommand = ({ payload }: { payload: CommandMessage }) => {
    if (payload.token !== token) return;
    if (payload.type === "apply") {
      if (payload.sequence <= sequence) return;
      sequence = payload.sequence;
      void player
        .apply(payload.command)
        .catch((cause) => console.warn("播放窗口处理命令失败：", cause));
    } else {
      void player
        .suspend()
        .then((command) =>
          send({
            token,
            type: "suspended",
            requestId: payload.requestId,
            command,
          }),
        )
        .catch((cause) => console.warn("播放窗口交接失败：", cause));
    }
  };
  const unlisten = window.KaramasPlayback
    ? listenPresentationCommand(onCommand)
    : await listen<CommandMessage>(COMMAND_EVENT, onCommand);
  return {
    ready: () => send({ token, type: "ready" }),
    playback: (report: LocalReport) =>
      send({ token, type: "playback", sequence, report }),
    state: (state: LocalPlayerState) =>
      send({ token, type: "state", sequence, state }),
    retry: () => send({ token, type: "retry" }),
    dispose: unlisten,
  };
}
