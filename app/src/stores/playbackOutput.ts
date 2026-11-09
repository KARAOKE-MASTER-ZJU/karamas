import { acceptHMRUpdate, defineStore } from "pinia";
import { ref } from "vue";
import { isPlaybackController } from "@/services/playbackWindow";
import type { PlaybackOutputState } from "@/types/playbackOutput";

export const usePlaybackOutputStore = defineStore("playbackOutput", () => {
  const output = ref("embedded");
  const videoOnly = ref(false);
  const switching = ref(false);

  function apply(state: PlaybackOutputState) {
    output.value = state.output;
    switching.value = state.switching;
    videoOnly.value = !isPlaybackController && state.output !== "embedded";
  }

  function prepare() {
    videoOnly.value = true;
  }

  return { output, videoOnly, switching, apply, prepare };
});

if (import.meta.hot)
  import.meta.hot.accept(
    acceptHMRUpdate(usePlaybackOutputStore, import.meta.hot),
  );
