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
    try {
      listeners.push(await listenPlaybackOutput(output.apply));
      listeners.push(
        await listenPlaybackOutputPrepare(async (requestId) => {
          if (isPlaybackController) return;
          output.prepare();
          await nextTick();
          await notifyPlaybackOutputReady(requestId);
        }),
      );
      output.apply(await getPlaybackOutput());
      return dispose;
    } catch (error) {
      dispose();
      throw error;
    }
  }

  return { initialize };
}
