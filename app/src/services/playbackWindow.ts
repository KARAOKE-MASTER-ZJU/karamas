import { invoke, listen, isTauri } from "@/services/platform";
import type {
  PlaybackOutputCapabilities,
  PlaybackOutputState,
} from "@/types/playbackOutput";

export const isPlaybackController = new URLSearchParams(location.search).has(
  "playback-controller",
);
export function getPlaybackOutputCapabilities(): Promise<PlaybackOutputCapabilities> {
  return isTauri()
    ? invoke<PlaybackOutputCapabilities>("get_playback_output_capabilities")
    : Promise.resolve({ window: false, screen: false });
}

export function getPlaybackOutput() {
  return invoke<PlaybackOutputState>("get_playback_output");
}

export function listenPlaybackOutput(
  callback: (state: PlaybackOutputState) => void,
) {
  return listen<PlaybackOutputState>("playback-output-changed", ({ payload }) =>
    callback(payload),
  );
}

export function listenPlaybackOutputPrepare(
  callback: (requestId: number) => void | Promise<void>,
) {
  return listen<number>("playback-output-prepare", ({ payload }) =>
    callback(payload),
  );
}

export function notifyPlaybackOutputReady(requestId: number) {
  return invoke<void>("playback_output_ready", { requestId });
}

export function setPlaybackOutput(output: string) {
  return invoke<void>("set_playback_output", { output });
}
export function focusPlaybackOutput() {
  return invoke<void>("focus_playback_output");
}
