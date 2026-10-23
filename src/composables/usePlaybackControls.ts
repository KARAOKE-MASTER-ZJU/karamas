import { useCastingStore } from "@/stores/casting";
import { castingApi, errorMessage } from "@/services/casting";
import { useOperationNotice } from "@/composables/useOperationNotice";

export function usePlaybackControls() {
  const casting = useCastingStore();
  const { notify } = useOperationNotice();
  async function run(action: () => Promise<unknown>) {
    try {
      await action();
    } catch (error) {
      notify(errorMessage(error), "error");
    }
  }
  async function recast() {
    await run(async () => {
      const result = await casting.recast();
      if (result === "noCurrentSong")
        notify("当前没有可重新投屏的歌曲", "warning");
      else if (result === "busy") notify("正在投屏，请稍后重试", "warning");
    });
  }
  return {
    run,
    recast,
    previousSong: () => run(() => casting.control(castingApi.previousSong)),
    nextSong: () => run(() => casting.control(castingApi.nextSong)),
    togglePause: () => run(casting.togglePause),
    seek: (seconds: number) => run(() => casting.seek(seconds)),
    changeVolume: (increase: boolean) =>
      run(() => casting.changeVolume(increase)),
    setVolume: (volume: number) => run(() => casting.setVolume(volume)),
    setQuality: (qn: number) => run(() => casting.setQuality(qn)),
    setDanmaku: (enabled: boolean) => run(() => casting.setDanmaku(enabled)),
  };
}
