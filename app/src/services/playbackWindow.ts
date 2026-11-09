import { nextTick, reactive } from "vue";
import {
  invoke,
  listen,
  supportsMultipleWindows,
  isTauri,
} from "@/services/platform";

export const isPlaybackController = new URLSearchParams(location.search).has(
  "playback-controller",
);
export const playbackOutput = reactive({
  output: "embedded",
  videoOnly: false,
  switching: false,
});

export async function supportsPlaybackWindow() {
  return isTauri() && (await supportsMultipleWindows());
}

export async function initializePlaybackOutput() {
  if (!isTauri()) return () => {};
  const unlisten = await listen<{ output: string; switching: boolean }>(
    "playback-output-changed",
    ({ payload }) => {
      playbackOutput.output = payload.output;
      playbackOutput.switching = payload.switching;
      if (!isPlaybackController)
        playbackOutput.videoOnly = payload.output !== "embedded";
    },
  );
  const unprepare = await listen<number>(
    "playback-output-prepare",
    async ({ payload }) => {
      if (isPlaybackController) return;
      playbackOutput.videoOnly = true;
      await nextTick();
      await invoke("playback_output_ready", { requestId: payload });
    },
  );
  const state = await invoke<{ output: string; switching: boolean }>(
    "get_playback_output",
  );
  playbackOutput.output = state.output;
  playbackOutput.switching = state.switching;
  if (!isPlaybackController)
    playbackOutput.videoOnly = state.output !== "embedded";
  return () => {
    unlisten();
    unprepare();
  };
}

export function setPlaybackOutput(output: string) {
  return invoke<void>("set_playback_output", { output });
}
export function focusPlaybackOutput() {
  return invoke<void>("focus_playback_output");
}
