import type {
  LocalCommand,
  LocalReport,
  LocalPlayerState,
  LocalPlayerCallbacks,
} from "@/types/localPlayer";
import type Hls from "hls.js";

const PLAYER_ERROR = "local-player-error";

const MEDIA_EVENTS = [
  "play",
  "playing",
  "pause",
  "timeupdate",
  "durationchange",
  "volumechange",
  "seeking",
  "seeked",
  "ended",
  "error",
  PLAYER_ERROR,
] as const;

function waitFor(
  element: HTMLVideoElement,
  event: string | (() => Promise<void>),
  signal: AbortSignal,
) {
  return new Promise<void>((resolve, reject) => {
    const finish = (failure?: Error) => {
      clearTimeout(timer);
      if (typeof event === "string") element.removeEventListener(event, ready);
      element.removeEventListener("error", failed);
      element.removeEventListener(PLAYER_ERROR, failed);
      signal.removeEventListener("abort", cancelled);
      if (failure) reject(failure);
      else resolve();
    };
    const ready = () => finish();
    const failed = (event: Event) =>
      finish(
        event instanceof CustomEvent && event.detail instanceof Error
          ? event.detail
          : new Error("视频加载失败，可能是网络或格式不受支持，请重新播放。"),
      );
    const cancelled = () => finish(new Error("播放操作已取消"));
    const timer = setTimeout(
      () => finish(new Error("视频加载超时，请重新播放。")),
      25000,
    );
    if (typeof event === "string")
      element.addEventListener(event, ready, { once: true });
    element.addEventListener("error", failed, { once: true });
    element.addEventListener(PLAYER_ERROR, failed, { once: true });
    signal.addEventListener("abort", cancelled, { once: true });
    if (signal.aborted) cancelled();
    else if (typeof event === "function") void event().then(ready, finish);
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
  let activePointer: number | null = null;
  let abort = new AbortController();
  let disposed = false;
  let reporting = false;
  let statusQueued = false;
  let hls: Hls | undefined;

  async function loadSource(url: string, signal: AbortSignal) {
    const isHls = new URL(url).pathname.endsWith(".m3u8");
    if (isHls && !video.canPlayType("application/vnd.apple.mpegurl")) {
      const { default: Hls } = await import("hls.js");
      if (signal.aborted) return;
      if (!Hls.isSupported())
        throw new Error(
          "当前播放器不支持 HLS，请选择 [DASH] 或 [MP4] 清晰度。",
        );
      const instance = new Hls();
      hls = instance;
      instance.on(Hls.Events.ERROR, (_event, data) => {
        if (!data.fatal || hls !== instance) return;
        instance.stopLoad();
        video.dispatchEvent(
          new CustomEvent(PLAYER_ERROR, {
            detail: new Error("HLS 视频加载失败，请重新播放或切换清晰度。"),
          }),
        );
      });
      const ready = waitFor(video, "loadedmetadata", signal);
      instance.loadSource(url);
      instance.attachMedia(video);
      await ready;
    } else {
      const ready = waitFor(video, "loadedmetadata", signal);
      video.src = url;
      video.load();
      await ready;
    }
  }

  let clearingMedia: Promise<void> | undefined;
  function clearMedia(): Promise<void> {
    activePointer = null;
    hls?.destroy();
    hls = undefined;
    video.pause();
    // Stop 和会话清空可能连续到达，共用一次画中画退出操作。
    clearingMedia ??= (async () => {
      const document = video.ownerDocument;
      if (document.pictureInPictureElement === video)
        await document.exitPictureInPicture();
      video.removeAttribute("src");
      video.load();
      state.hasMedia = false;
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
      seeking: element.seeking || activePointer !== null,
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
          state.hasMedia = true;
          notifyState();
          await loadSource(action.url, signal);
          if (signal.aborted) return;
          await seekTo(element, action.position, signal);
          if (action.playing)
            await waitFor(element, () => element.play(), signal);
          if (signal.aborted) return;
          state.loading = false;
          break;
        }
        case "play":
          if (!state.hasMedia)
            throw new Error("视频尚未就绪，请点击重新播放。");
          await waitFor(element, () => element.play(), signal);
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
    if (event.type === "playing" && state.needsGesture) {
      state.needsGesture = false;
      state.error = "";
      notifyState();
    }
    if (
      ["error", PLAYER_ERROR].includes(event.type) &&
      state.hasMedia &&
      !processing
    ) {
      state.error =
        event instanceof CustomEvent && event.detail instanceof Error
          ? event.detail.message
          : "视频播放失败，请重新播放或切换清晰度。";
      notifyState();
    }
    void sendStatus();
  }
  async function startWithGesture() {
    await waitFor(video, () => video.play(), abort.signal);
    state.needsGesture = false;
    state.error = "";
    notifyState();
  }

  function pointerDown(event: PointerEvent) {
    if (event.button !== 0 || !state.hasMedia) return;
    activePointer = event.pointerId;
    void sendStatus();
  }
  function releasePointer() {
    if (activePointer === null) return;
    activePointer = null;
    void sendStatus();
  }
  function pointerUp(event: Event) {
    if ((event as PointerEvent).pointerId === activePointer) releasePointer();
  }

  // 原生滑块拖动中会反复触发 seeked；按住播放器期间仍保持定位状态。
  const document = video.ownerDocument;
  const interactionTarget = document.defaultView ?? document;
  video.addEventListener("pointerdown", pointerDown, true);
  interactionTarget.addEventListener("pointerup", pointerUp, true);
  interactionTarget.addEventListener("pointercancel", pointerUp, true);
  document.defaultView?.addEventListener("blur", releasePointer);
  for (const event of MEDIA_EVENTS) video.addEventListener(event, mediaEvent);
  const timer = setInterval(() => void sendStatus(), 1000);
  function dispose() {
    if (disposed) return;
    disposed = true;
    abort.abort();
    clearInterval(timer);
    video.removeEventListener("pointerdown", pointerDown, true);
    interactionTarget.removeEventListener("pointerup", pointerUp, true);
    interactionTarget.removeEventListener("pointercancel", pointerUp, true);
    document.defaultView?.removeEventListener("blur", releasePointer);
    for (const event of MEDIA_EVENTS)
      video.removeEventListener(event, mediaEvent);
    return clearMedia();
  }

  return { apply, startWithGesture, dispose };
}
