import { nextTick } from "vue";
import { isTauri } from "@/services/platform";
import {
  getPlaybackOutput,
  isPlaybackController,
  listenPlaybackOutput,
  listenPlaybackOutputPrepare,
  notifyPlaybackOutputReady,
} from "@/services/playbackWindow";
import { usePlaybackOutputStore } from "@/stores/playbackOutput";

export function usePlaybackOutput() {
  const output = usePlaybackOutputStore();

  async function initialize() {
    if (!isTauri()) return () => {};
    const listeners: Array<() => void> = [];
    const dispose = () => listeners.forEach((stop) => stop());
    let receivedUpdate = false;
    try {
      listeners.push(
        await listenPlaybackOutput((snapshot) => {
          receivedUpdate = true;
          output.apply(snapshot);
        }),
      );
      listeners.push(
        await listenPlaybackOutputPrepare(async (requestId) => {
          if (isPlaybackController) return;
          output.prepare();
          await nextTick();
          await notifyPlaybackOutputReady(requestId);
        }),
      );
      const snapshot = await getPlaybackOutput();
      // 查询返回前若已收到变更，保留事件中的新快照。
      if (!receivedUpdate) output.apply(snapshot);
      return dispose;
    } catch (error) {
      dispose();
      throw error;
    }
  }

  return { initialize };
}
