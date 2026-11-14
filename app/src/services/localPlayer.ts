import type {
  LocalCommand,
  LocalReport,
  LocalPlayerState,
  LocalPlayerCallbacks,
} from "@/types/localPlayer";
import type Hls from "hls.js";
import type Mpegts from "mpegts.js";

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
  player?: Mpegts.Player,
) {
  const target = Number.isFinite(element.duration)
    ? Math.min(seconds, element.duration)
    : seconds;
  if (Math.abs(element.currentTime - target) < 0.05) return;
  const settled = waitFor(element, "seeked", signal);
  (player ?? element).currentTime = target;
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
  let source: Extract<LocalCommand["action"], { kind: "load" }> | null = null;
  let processing = false;
  let activePointer: number | null = null;
  let abort = new AbortController();
  let disposed = false;
  let reporting = false;
  let statusQueued = false;
  let hls: Hls | undefined;
  let flv: Mpegts.Player | undefined;

  async function loadSource(
    action: Extract<LocalCommand["action"], { kind: "load" }>,
    signal: AbortSignal,
  ) {
    const { url, mediaType } = action;
    const isHls = mediaType === "hls";
    // 原生 HLS 需要通过 CORS 读取本机代理上的跨域字节范围分片。
    if (isHls) video.crossOrigin = "anonymous";
    else video.removeAttribute("crossorigin");
    if (mediaType === "flv") {
      const { default: mpegts } = await import("mpegts.js");
      if (signal.aborted) return;
      const features = mpegts.getFeatureList();
      if (!features.msePlayback || !features.networkStreamIO)
        throw new Error("当前播放器不支持 FLV，请更新系统或 WebView。");
      const live = action.live;
      const instance = mpegts.createPlayer({
        type: "flv",
        isLive: live,
        filesize: action.filesize ?? undefined,
        url,
      });
      flv = instance;
      let complete = false;
      const updateLive = () => {
        if (flv !== instance) return;
        const duration = instance.mediaInfo?.duration;
        action.live =
          !complete &&
          action.filesize == null &&
          !(
            typeof duration === "number" &&
            Number.isFinite(duration) &&
            duration > 0
          );
        void sendStatus();
      };
      instance.on(mpegts.Events.MEDIA_INFO, updateLive);
      instance.on(mpegts.Events.LOADING_COMPLETE, () => {
        if (flv !== instance) return;
        complete = true;
        updateLive();
      });
      instance.on(mpegts.Events.ERROR, (type: string, detail: string) => {
        if (flv !== instance) return;
        instance.detachMediaElement();
        instance.unload();
        video.dispatchEvent(
          new CustomEvent(PLAYER_ERROR, {
            detail: new Error(
              `FLV 加载失败（${type}：${detail}），请重新播放。`,
            ),
          }),
        );
      });
      const ready = waitFor(video, "loadedmetadata", signal);
      instance.attachMediaElement(video);
      instance.load();
      await ready;
      updateLive();
      if (!signal.aborted && !live && action.live) {
        // 元数据确认直播后重建一次，让库启用持续拉流和缓冲清理。
        flv = undefined;
        instance.detachMediaElement();
        instance.destroy();
        await loadSource(action, signal);
      }
    } else if (
      isHls &&
      (!action.live || !video.canPlayType("application/vnd.apple.mpegurl"))
    ) {
      // WebKit 原生 HLS 的大范围重复读取会拖慢点播 seek。
      const { default: Hls } = await import("hls.js");
      if (signal.aborted) return;
      if (!Hls.isSupported()) {
        if (!video.canPlayType("application/vnd.apple.mpegurl"))
          throw new Error(
            "当前播放器不支持 HLS，请选择 [DASH] 或 [MP4] 清晰度。",
          );
        const ready = waitFor(video, "loadedmetadata", signal);
        video.src = url;
        video.load();
        await ready;
        return;
      }
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
    // 先解除 SourceBuffer 回调，再销毁下载控制器，避免库的清理回调访问空对象。
    flv?.detachMediaElement();
    flv?.destroy();
    flv = undefined;
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
      // 部分原生 HLS 播放器会返回有限的回看窗口长度，直播仍按无总时长回传。
      totalSecs:
        !source?.live && Number.isFinite(element.duration)
          ? element.duration
          : 0,
      live: source?.live ?? false,
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
    if (!command || ["loading", "stop", "error"].includes(command.action.kind))
      source = null;
    else if (command.action.kind === "load") source = command.action;
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
          await loadSource(action, signal);
          if (signal.aborted) return;
          // 直播由播放器选择起播点，不能用点播的零秒位置覆盖。
          if (!action.live) await seekTo(element, action.position, signal, flv);
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
          await seekTo(element, action.seconds, signal, flv);
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
