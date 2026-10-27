import type {
  LocalCommand,
  LocalReport,
  LocalPlayerState,
  LocalPlayerCallbacks,
} from "@/types/localPlayer";

const MEDIA_EVENTS = [
  "play",
  "playing",
  "pause",
  "timeupdate",
  "durationchange",
  "volumechange",
  "waiting",
  "seeking",
  "seeked",
  "canplay",
  "ended",
  "error",
] as const;

function waitFor(
  element: HTMLVideoElement,
  event: string,
  signal: AbortSignal,
) {
  return new Promise<void>((resolve, reject) => {
    const finish = (failure?: Error) => {
      clearTimeout(timer);
      element.removeEventListener(event, ready);
      element.removeEventListener("error", failed);
      signal.removeEventListener("abort", cancelled);
      if (failure) reject(failure);
      else resolve();
    };
    const ready = () => finish();
    const failed = () =>
      finish(new Error("视频加载失败，可能是网络或格式不受支持，请重新播放。"));
    const cancelled = () => finish(new Error("播放操作已取消"));
    const timer = setTimeout(
      () => finish(new Error("视频加载超时，请重新播放。")),
      25000,
    );
    element.addEventListener(event, ready, { once: true });
    element.addEventListener("error", failed, { once: true });
    signal.addEventListener("abort", cancelled, { once: true });
    if (signal.aborted) cancelled();
  });
}

async function seekTo(
  element: HTMLVideoElement,
  seconds: number,
  signal: AbortSignal,
) {
  const target = Number.isFinite(element.duration)
    ? Math.min(seconds, element.duration)
    : seconds;
  if (Math.abs(element.currentTime - target) < 0.05) return;
  const settled = waitFor(element, "seeked", signal);
  element.currentTime = target;
  await settled;
}

// 每个视频元素独立持有播放状态；宿主通过回调接入界面和消息通道。
export function createLocalPlayer(
  video: HTMLVideoElement,
  callbacks: LocalPlayerCallbacks,
) {
  const state: LocalPlayerState = {
    hasMedia: false,
    loading: false,
    error: "",
    needsGesture: false,
  };
  function notifyState() {
    callbacks.onState({ ...state });
  }
  let current: LocalCommand | null = null;
  let processing = false;
  let buffering = false;
  let abort = new AbortController();
  let disposed = false;
  let reporting = false;
  let statusQueued = false;

  let clearingMedia: Promise<void> | undefined;
  function clearMedia(): Promise<void> {
    video.pause();
    // Stop 和会话清空可能连续到达，共用一次画中画退出操作。
    clearingMedia ??= (async () => {
      const document = video.ownerDocument;
      if (document.pictureInPictureElement === video)
        await document.exitPictureInPicture();
      video.removeAttribute("src");
      video.load();
      state.hasMedia = false;
      buffering = false;
    })().finally(() => {
      clearingMedia = undefined;
    });
    return clearingMedia;
  }

  function readReport(acknowledged = false) {
    const element = video;
    const command = current;
    if (!command || disposed || (!acknowledged && processing)) return;
    const payload: LocalReport = {
      sessionId: command.sessionId,
      commandId: command.commandId,
      currentSecs: Number.isFinite(element.currentTime)
        ? element.currentTime
        : 0,
      totalSecs: Number.isFinite(element.duration) ? element.duration : 0,
      paused: element.paused,
      buffering: buffering || element.seeking || element.readyState < 3,
      seeking: element.seeking,
      muted: element.muted,
      ended: element.ended,
      volume: Math.round(element.volume * 100),
      acknowledged,
      error: state.error || null,
    };
    return payload;
  }

  async function report(acknowledged = false) {
    const payload = readReport(acknowledged);
    if (!payload) return;
    callbacks.onPlayback(payload);
    await callbacks.onReport(payload);
  }

  async function apply(command: LocalCommand | null) {
    if (disposed) return;
    abort.abort();
    abort = new AbortController();
    const signal = abort.signal;
    current = command;
    processing = true;
    state.needsGesture = false;
    state.error = "";
    const element = video;
    try {
      if (
        !command ||
        ["loading", "load", "stop", "error"].includes(command.action.kind)
      ) {
        await clearMedia();
        if (signal.aborted) return;
      }
      if (!command) {
        state.loading = false;
        processing = false;
        notifyState();
        return;
      }
      const action = command.action;
      switch (action.kind) {
        case "loading":
          state.loading = true;
          break;
        case "load": {
          state.loading = true;
          element.volume = action.volume / 100;
          element.muted = action.muted;
          const ready = waitFor(element, "loadedmetadata", signal);
          element.src = action.url;
          element.load();
          state.hasMedia = true;
          notifyState();
          await ready;
          await seekTo(element, action.position, signal);
          if (action.playing) await element.play();
          if (signal.aborted) return;
          state.loading = false;
          break;
        }
        case "play":
          if (!state.hasMedia)
            throw new Error("视频尚未就绪，请点击重新播放。");
          await element.play();
          break;
        case "pause":
          element.pause();
          break;
        case "seek":
          if (!state.hasMedia) throw new Error("视频尚未就绪，无法调整进度。");
          await seekTo(element, action.seconds, signal);
          break;
        case "volume":
          element.muted = false;
          element.volume = action.value / 100;
          break;
        case "stop":
          state.loading = false;
          break;
        case "error":
          state.loading = false;
          state.error = action.message;
          break;
      }
    } catch (cause) {
      if (signal.aborted) return;
      state.loading = false;
      state.needsGesture =
        cause instanceof DOMException && cause.name === "NotAllowedError";
      state.error = state.needsGesture
        ? "点击开始播放，允许本机播放声音。"
        : cause instanceof Error
          ? cause.message
          : String(cause);
    }
    if (signal.aborted) return;
    notifyState();
    try {
      await report(true);
    } finally {
      if (!signal.aborted) processing = false;
    }
  }

  async function sendStatus() {
    if (processing || !current || disposed) return;
    if (reporting) {
      const payload = readReport();
      if (payload) callbacks.onPlayback(payload);
      statusQueued = true;
      return;
    }
    reporting = true;
    try {
      do {
        statusQueued = false;
        await report();
      } while (statusQueued && !processing && current && !disposed);
    } catch {
      /* 引擎切换期间丢弃旧状态，后续事件或心跳会重新报告。 */
    } finally {
      reporting = false;
    }
  }
  function mediaEvent(event: Event) {
    if (event.type === "waiting" || event.type === "seeking") buffering = true;
    if (["playing", "seeked", "canplay", "ended", "pause"].includes(event.type))
      buffering = false;
    if (event.type === "playing" && state.needsGesture) {
      state.needsGesture = false;
      state.error = "";
      notifyState();
    }
    if (event.type === "error" && state.hasMedia && !processing) {
      state.error = "视频播放失败，请重新播放或切换清晰度。";
      notifyState();
    }
    void sendStatus();
  }
  async function startWithGesture() {
    await video.play();
    state.needsGesture = false;
    state.error = "";
    notifyState();
  }

  for (const event of MEDIA_EVENTS) video.addEventListener(event, mediaEvent);
  const timer = setInterval(() => void sendStatus(), 1000);
  function dispose() {
    if (disposed) return;
    disposed = true;
    abort.abort();
    clearInterval(timer);
    for (const event of MEDIA_EVENTS)
      video.removeEventListener(event, mediaEvent);
    return clearMedia();
  }

  return { apply, startWithGesture, dispose };
}
