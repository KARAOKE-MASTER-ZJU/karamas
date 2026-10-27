import { onMounted, onScopeDispose, reactive, toRefs, watch, type Ref } from "vue";
import { useCastingStore } from "@/stores/casting";
import { castingApi, errorMessage } from "@/services/casting";
import { createLocalPlayer } from "@/services/localPlayer";
import type { LocalPlayerState } from "@/types/localPlayer";

export function useLocalPlayer(video: Ref<HTMLVideoElement | null>) {
  const casting = useCastingStore();
  const state = reactive<LocalPlayerState>({
    hasMedia: false,
    loading: false,
    error: "",
    needsGesture: false,
  });
  let player: ReturnType<typeof createLocalPlayer> | undefined;

  onMounted(() => {
    if (!video.value) return;
    player = createLocalPlayer(video.value, {
      onState: (value) => Object.assign(state, value),
      onPlayback: casting.applyLocalReport,
      onReport: async (report) => {
        try {
          await castingApi.reportLocalPlayback(report);
        } catch {
          // 回传失败由后续状态事件重试，不作为媒体播放错误显示。
        }
      },
    });
    void player.apply(casting.localCommand);
  });
  watch(
    () => casting.localCommand,
    (command) => void player?.apply(command),
    { flush: "post" },
  );
  onScopeDispose(() => {
    void player?.dispose()?.catch((cause: unknown) => {
      console.warn("清理本地播放器失败：", cause);
    });
  });

  async function startWithGesture() {
    if (!player) return;
    try {
      // 在点击事件内解锁媒体播放，再通过核心确认恢复状态。
      await player.startWithGesture();
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
