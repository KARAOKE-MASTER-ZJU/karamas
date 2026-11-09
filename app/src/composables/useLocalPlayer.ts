import {
  onMounted,
  onScopeDispose,
  reactive,
  toRefs,
  watch,
  type Ref,
} from "vue";
import { useCastingStore } from "@/stores/casting";
import { castingApi, errorMessage } from "@/services/casting";
import { createLocalPlayer } from "@/services/localPlayer";

// 只有常驻播放文档创建播放器，窗口切换不参与媒体生命周期。
export function useLocalPlayer(video: Ref<HTMLVideoElement | null>) {
  const casting = useCastingStore();
  const state = reactive({
    hasMedia: false,
    loading: false,
    error: "",
    needsGesture: false,
  });
  let player: ReturnType<typeof createLocalPlayer> | undefined;
  let disposed = false;
  onMounted(() => {
    if (!video.value) return;
    player = createLocalPlayer(video.value, {
      onState: (value) => Object.assign(state, value),
      onPlayback: casting.applyLocalReport,
      onReport: async (report) => {
        try {
          await castingApi.reportLocalPlayback(report);
        } catch {
          /* 会话切换时忽略旧报告，后续事件继续回传。 */
        }
      },
    });
    void player.apply(casting.localCommand);
  });
  watch(
    () => casting.localCommand,
    (command) => {
      if (!disposed) void player?.apply(command);
    },
  );
  onScopeDispose(() => {
    disposed = true;
    void player
      ?.dispose()
      ?.catch((cause: unknown) => console.warn("清理本地播放器失败：", cause));
  });
  async function startWithGesture() {
    try {
      await player?.startWithGesture();
      await casting.play();
    } catch (cause) {
      state.error = errorMessage(cause);
    }
  }
  async function retry() {
    try {
      await casting.recast();
    } catch (cause) {
      state.error = errorMessage(cause);
    }
  }
  return { ...toRefs(state), startWithGesture, retry };
}
