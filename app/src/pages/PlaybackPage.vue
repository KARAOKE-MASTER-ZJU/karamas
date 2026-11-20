<script setup lang="ts">
import { computed, ref, toRef } from "vue";
import { storeToRefs } from "pinia";
import LocalPlayer from "@/components/playback/LocalPlayer.vue";
import PlaybackSessionHeader from "@/components/playback/PlaybackSessionHeader.vue";
import NowPlaying from "@/components/playback/NowPlaying.vue";
import PlaybackProgress from "@/components/playback/PlaybackProgress.vue";
import PlaybackTransport from "@/components/playback/PlaybackTransport.vue";
import AudioDelayControl from "@/components/playback/AudioDelayControl.vue";
import VolumeControl from "@/components/playback/VolumeControl.vue";
import PlaybackOptions from "@/components/playback/PlaybackOptions.vue";
import PlaybackOutputControl from "@/components/playback/PlaybackOutputControl.vue";
import BilibiliPartsControl from "@/components/playback/BilibiliPartsControl.vue";
import { useCastingStore } from "@/stores/casting";
import { usePlaybackOutputStore } from "@/stores/playbackOutput";
import { usePlaybackControls } from "@/composables/usePlaybackControls";
import { useVideoLayout } from "@/composables/useVideoLayout";
import {
  isPlaybackController,
  setPlaybackOutput,
  focusPlaybackOutput,
} from "@/services/playbackWindow";
import { openSongRoom } from "@/services/songRoom";

const props = defineProps<{
  active: boolean;
  viewport: HTMLElement | null;
  content: HTMLElement | null;
  videoOnly: boolean;
}>();
const casting = useCastingStore();
const playbackOutput = usePlaybackOutputStore();
const { playback, volume, activeDevice, recasting, refreshingPlaylist, room } =
  storeToRefs(casting);
const controls = usePlaybackControls();
const videoContainer = ref<HTMLElement | null>(null);
const playbackBusy = computed(
  () => casting.sessionBusy || playbackOutput.state.switching,
);
const showAudioDelay = computed(
  () =>
    activeDevice.value?.kind === "local" ||
    (activeDevice.value?.kind === "dlna" && casting.dlnaProxyEnabled),
);
function handlePlaybackOutputChange(outputId: string) {
  void controls.run(async () => {
    await setPlaybackOutput(outputId);
  });
}
useVideoLayout(
  videoContainer,
  toRef(props, "viewport"),
  toRef(props, "content"),
  toRef(props, "active"),
);
</script>

<template>
  <div class="playback-page" :class="{ 'video-only': videoOnly }">
    <PlaybackSessionHeader
      v-if="!videoOnly"
      :active-device="activeDevice"
      :selected-device="casting.selectedDevice"
      :connecting="casting.connecting"
      :connected="casting.connected"
      :room="room"
      :refresh-disabled="!room || !casting.connected || playbackBusy"
      :refreshing-playlist="refreshingPlaylist"
      @open-room="controls.run(() => openSongRoom($event))"
      @refresh-playlist="controls.refreshPlaylist()"
    />

    <section
      class="content-section player-section"
      aria-labelledby="playing-title"
    >
      <div v-if="!videoOnly" class="playing-header">
        <h2 id="playing-title">正在播放</h2>
        <PlaybackOutputControl
          v-if="activeDevice?.kind === 'local'"
          :selected="playbackOutput.state.target"
          :disabled="casting.busy || playbackBusy"
          @change="handlePlaybackOutputChange"
        />
      </div>
      <div ref="videoContainer" class="video-container">
        <LocalPlayer
          v-if="!isPlaybackController"
          :active="active || videoOnly"
          :standalone="videoOnly"
        />
        <div v-else class="output-message">
          <span>{{
            playbackOutput.state.target.kind === "screen"
              ? "正在投屏到显示器"
              : "正在独立窗口中播放"
          }}</span>
          <button
            class="button secondary"
            @click="controls.run(focusPlaybackOutput)"
          >
            显示播放窗口
          </button>
        </div>
      </div>
      <template v-if="!videoOnly">
        <NowPlaying
          :local="activeDevice?.kind === 'local'"
          :title="playback?.title ?? null"
          :added-by="playback?.addedBy ?? null"
          :song-id="playback?.songId ?? null"
          :song-url="playback?.songUrl ?? null"
          :queued-count="playback?.queuedCount ?? null"
          :sung-count="playback?.sungCount ?? null"
          :disabled="!casting.connected || playbackBusy"
          :recasting="recasting"
          @recast="controls.recast"
        />
        <BilibiliPartsControl
          v-if="playback?.songId"
          :key="playback.songId"
          :song-id="playback.songId"
          :song-url="playback.songUrl"
          :disabled="playbackBusy"
        />
        <PlaybackProgress
          :key="playback?.songId ?? undefined"
          :current-secs="playback?.currentSecs ?? null"
          :total-secs="playback?.totalSecs ?? null"
          :live="playback?.isLive ?? false"
          :disabled="!playback?.capabilities.seek || playbackBusy"
          @seek="controls.seek"
        />
        <PlaybackTransport
          :disabled="!casting.connected || playbackBusy"
          :is-playing="playback?.isPlaying ?? false"
          :has-song="!!playback?.songId"
          :queued-count="playback?.queuedCount ?? 0"
          :sung-count="playback?.sungCount ?? 0"
          @previous="controls.previousSong"
          @next="controls.nextSong"
          @play="controls.play"
          @pause="controls.pause"
        />
        <VolumeControl
          :volume="volume"
          :disabled="!casting.connected || playbackBusy"
          :absolute="!!playback?.capabilities.absoluteVolume"
          @change="controls.changeVolume"
          @set-volume="controls.setVolume"
        />
        <div
          v-if="activeDevice && (showAudioDelay || !playback?.isLive)"
          class="playback-settings"
        >
          <AudioDelayControl
            v-if="showAudioDelay"
            :key="playback?.songUrl ?? activeDevice.kind"
            :value="playback?.audioDelayMs ?? null"
            :disabled="casting.busy || playbackBusy"
            :pending="casting.applyingAudioDelay"
            @set-delay="controls.setAudioDelay"
          />
          <PlaybackOptions
            v-if="!playback?.isLive"
            :mode="activeDevice.kind"
            :song-id="playback?.songId ?? null"
            :song-url="playback?.songUrl ?? null"
            :dlna-proxy-enabled="casting.dlnaProxyEnabled"
            :dlna-hls-direct-enabled="casting.dlnaHlsDirectEnabled"
            :quality-qn="playback?.qualityQn ?? null"
            :quality-format="playback?.qualityFormat ?? null"
            :media-qualities="casting.mediaQualities"
            :media-qualities-loading="casting.mediaQualitiesLoading"
            :media-qualities-error="casting.mediaQualitiesError"
            :danmaku="playback?.danmaku ?? null"
            :disabled="!casting.connected || playbackBusy"
            @quality="controls.setQuality"
            @refresh-media-qualities="casting.refreshMediaQualities"
            @danmaku="controls.setDanmaku"
          />
        </div>
      </template>
    </section>
  </div>
</template>

<style scoped>
.playback-settings {
  margin-top: 24px;
  border-top: 1px solid var(--line);
}
.playback-page.video-only .player-section {
  margin: 0;
  padding: 0;
}
.output-message {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 20px;
  margin-bottom: 16px;
  background: var(--subtle);
  border-radius: 8px;
  color: var(--muted);
}
.video-container {
  display: flow-root;
}
.playback-page .player-section {
  padding: 0 0 16px;
  border-bottom: 0;
}
.playing-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 12px;
}
.playing-header > h2 {
  flex-shrink: 0;
  margin: 0;
}

@media (max-width: 640px) {
  .playing-header {
    margin-bottom: 18px;
  }
}
</style>
