import { acceptHMRUpdate, defineStore } from "pinia";
import { computed, ref, shallowRef } from "vue";
import { isPlaybackController } from "@/services/playbackWindow";
import type { PlaybackOutputState } from "@/types/playbackOutput";

export const usePlaybackOutputStore = defineStore("playbackOutput", () => {
  const state = shallowRef<PlaybackOutputState>({
    target: { kind: "embedded" },
    switching: false,
  });
  const preparing = ref(false);
  const videoOnly = computed(
    () =>
      !isPlaybackController &&
      (preparing.value || state.value.target.kind !== "embedded"),
  );

  function apply(snapshot: PlaybackOutputState) {
    state.value = snapshot;
    preparing.value = false;
  }

  function prepare() {
    preparing.value = true;
  }

  return { state, videoOnly, apply, prepare };
});

if (import.meta.hot)
  import.meta.hot.accept(
    acceptHMRUpdate(usePlaybackOutputStore, import.meta.hot),
  );
