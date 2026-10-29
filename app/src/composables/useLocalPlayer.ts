import {
  onMounted,
  onScopeDispose,
  reactive,
  ref,
  toRefs,
  watch,
  type Ref,
} from "vue";
import { useCastingStore } from "@/stores/casting";
import { castingApi, errorMessage } from "@/services/casting";
import {
  createLocalPlayer,
  updatePlaybackSnapshot,
} from "@/services/localPlayer";
import {
  connectPlaybackWindow,
  createPlaybackWindow,
} from "@/services/playbackWindow";
import { usePlaybackControls } from "@/composables/usePlaybackControls";
import type {
  LocalCommand,
  LocalPlayerState,
  LocalReport,
} from "@/types/localPlayer";

export function useLocalPlayer(
  video: Ref<HTMLVideoElement | null>,
  windowToken?: string,
) {
  const casting = useCastingStore();
  const { run } = usePlaybackControls();
  const output = ref<"embedded" | "window">("embedded");
  const switching = ref(false);
  const state = reactive<LocalPlayerState>({
    hasMedia: false,
    loading: false,
    error: "",
    needsGesture: false,
  });
  let player: ReturnType<typeof createLocalPlayer> | undefined;
  let remote: ReturnType<typeof createPlaybackWindow> | undefined;
  let connection: Awaited<ReturnType<typeof connectPlaybackWindow>> | undefined;
  let snapshot: LocalCommand | null = null;
  let routingPaused = false;
  let closeRequested = false;
  let disposed = false;
  function acceptPlayback(report: LocalReport) {
    const current = casting.localCommand;
    if (
      disposed ||
      current?.sessionId !== report.sessionId ||
      current.commandId !== report.commandId
    )
      return;
    if (
      snapshot?.action.kind === "load" &&
      snapshot.sessionId === report.sessionId &&
      snapshot.commandId === report.commandId
    ) {
      snapshot = {
        ...snapshot,
        action: {
          ...snapshot.action,
          position: report.currentSecs,
          playing: !report.paused && !report.ended,
          volume: report.volume,
          muted: report.muted,
        },
      };
    }
    casting.applyLocalReport(report);
  }

  function restoreAfterClose() {
    remote = undefined;
    if (disposed || output.value !== "window") return;
    output.value = "embedded";
    routingPaused = false;
    void player?.apply(snapshot);
  }

  async function setOutput(target: string) {
    if (
      windowToken ||
      !player ||
      disposed ||
      switching.value ||
      !["embedded", "window"].includes(target)
    )
      return;
    if (target === output.value) {
      if (target === "window") await remote?.focus();
      return;
    }
    switching.value = true;
    let suspendedEmbedded = false;
    try {
      if (target === "window") {
        const opened = createPlaybackWindow({
          onReady: () => {
            if (output.value === "window") {
              void run(async () => remote?.apply(snapshot));
            }
          },
          onPlayback: (report) => {
            if (output.value === "window" && !routingPaused)
              acceptPlayback(report);
          },
          onState: (value) => {
            if (output.value === "window") Object.assign(state, value);
          },
          onClosing: () => {
            if (switching.value) closeRequested = true;
            else void run(() => setOutput("embedded"));
          },
          onClosed: restoreAfterClose,
          onRetry: () => {
            void retry();
          },
        });
        remote = opened;
        await opened.open();
        if (
          disposed ||
          closeRequested ||
          casting.activeDevice?.kind !== "local"
        ) {
          await opened.close();
          remote = undefined;
          return;
        }
        routingPaused = true;
        suspendedEmbedded = true;
        const current = casting.localCommand;
        const captured = await player.suspend();
        if (
          current &&
          casting.localCommand === current &&
          captured?.sessionId === current.sessionId &&
          captured.commandId === current.commandId
        )
          snapshot = captured;
        output.value = "window";
        routingPaused = false;
        await opened.apply(snapshot);
      } else {
        routingPaused = true;
        const current = casting.localCommand;
        const opened = remote;
        try {
          const captured = await opened?.suspend();
          if (
            current &&
            casting.localCommand === current &&
            captured?.sessionId === current.sessionId &&
            captured.commandId === current.commandId
          )
            snapshot = captured;
        } catch {
          // 窗口提前销毁时，使用最近回传的播放状态恢复。
        }
        await opened?.close();
        remote = undefined;
        output.value = "embedded";
        routingPaused = false;
        await player.apply(snapshot);
      }
    } catch (cause) {
      try {
        await remote?.close();
      } catch (closeCause) {
        routingPaused = false;
        if (output.value === "window") await remote?.apply(snapshot);
        throw Object.assign(
          new Error(`${errorMessage(cause)}；${errorMessage(closeCause)}`),
          { cause: closeCause },
        );
      }
      remote = undefined;
      const restore = suspendedEmbedded || output.value === "window";
      output.value = "embedded";
      routingPaused = false;
      if (restore && !disposed) await player.apply(snapshot);
      throw cause;
    } finally {
      switching.value = false;
      if (closeRequested) {
        closeRequested = false;
        void run(() => setOutput("embedded"));
      }
    }
  }

  onMounted(async () => {
    if (!video.value) return;
    player = createLocalPlayer(video.value, {
      onState: (value) => {
        Object.assign(state, value);
        void connection?.state(value).catch(() => {});
      },
      onPlayback: (report) => {
        if (windowToken) void connection?.playback(report).catch(() => {});
        else if (output.value === "embedded" && !routingPaused)
          acceptPlayback(report);
      },
      onReport: async (report) => {
        if (
          disposed ||
          (!windowToken && (output.value !== "embedded" || routingPaused))
        )
          return;
        try {
          await castingApi.reportLocalPlayback(report);
        } catch {
          // 后续媒体事件和心跳继续回传。
        }
      },
    });
    if (windowToken) {
      try {
        connection = await connectPlaybackWindow(windowToken, player);
        if (disposed) {
          connection.dispose();
          return;
        }
        await connection.ready();
      } catch (cause) {
        state.error = errorMessage(cause);
      }
    } else {
      snapshot = updatePlaybackSnapshot(snapshot, casting.localCommand);
      void player.apply(casting.localCommand);
    }
  });
  watch(
    () => casting.localCommand,
    (command) => {
      if (windowToken || disposed) return;
      snapshot = updatePlaybackSnapshot(snapshot, command);
      if (routingPaused) return;
      if (output.value === "window") {
        void run(async () => remote?.apply(command));
        if (!command) void run(() => setOutput("embedded"));
      } else void player?.apply(command);
    },
    { flush: "post" },
  );
  onScopeDispose(() => {
    disposed = true;
    connection?.dispose();
    void remote?.close().catch(() => {});
    void player?.dispose()?.catch((cause: unknown) => {
      console.warn("清理本地播放器失败：", cause);
    });
  });

  async function startWithGesture() {
    if (!player) return;
    try {
      // 在点击事件内解锁媒体播放，再通过核心确认恢复状态。
      await player.startWithGesture();
      if (windowToken) await castingApi.play();
      else await casting.play();
    } catch (cause) {
      state.error = errorMessage(cause);
    }
  }
  async function retry() {
    try {
      if (windowToken) await connection?.retry();
      else await casting.recast();
    } catch (cause) {
      state.error = errorMessage(cause);
    }
  }

  return {
    ...toRefs(state),
    output,
    switching,
    setOutput,
    focusOutput: () => run(async () => remote?.focus()),
    startWithGesture,
    retry,
  };
}
