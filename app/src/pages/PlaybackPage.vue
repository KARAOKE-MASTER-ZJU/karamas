<script setup lang="ts">
import { Cast, Monitor, ExternalLink } from "@lucide/vue";
import { RouterLink } from "vue-router";
import { computed, ref, toRef } from "vue";
import { storeToRefs } from "pinia";
import LocalPlayer from "@/components/playback/LocalPlayer.vue";
import NowPlaying from "@/components/playback/NowPlaying.vue";
import PlaybackProgress from "@/components/playback/PlaybackProgress.vue";
import PlaybackTransport from "@/components/playback/PlaybackTransport.vue";
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
const { playback, volume, activeDevice, recasting, room } =
  storeToRefs(casting);
const controls = usePlaybackControls();
const videoContainer = ref<HTMLElement | null>(null);
const playbackBusy = computed(
  () => casting.sessionBusy || playbackOutput.state.switching,
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
    <div v-if="!videoOnly" class="session-header" aria-label="当前投屏连接">
      <div class="session-chips">
        <RouterLink
          :to="{ name: 'connection' }"
          class="session-chip device-chip"
        >
          <component
            :is="activeDevice?.kind === 'local' ? Monitor : Cast"
            :size="14"
          /><span>{{
            casting.connecting
              ? "连接中…"
              : (casting.activeDevice?.name ??
                (casting.selectedDevice
                  ? `${casting.selectedDevice.name} · ${casting.room ? "未连接" : "待加入房间"}`
                  : casting.connected
                    ? "已连接设备"
                    : "未连接设备"))
          }}</span>
        </RouterLink>
        <button
          type="button"
          class="session-chip room-chip"
          :disabled="!room"
          :title="room ? `打开点歌台：${room.serverUrl}` : '未加入房间'"
          :aria-label="room ? `打开房间 ${room.roomId} 的点歌台` : '未加入房间'"
          @click="room && controls.run(() => openSongRoom(room!))"
        >
          <span>{{ room ? `房间：${room.roomId}` : "未加入房间" }}</span>
          <ExternalLink v-if="room" :size="12" aria-hidden="true" />
        </button>
      </div>
    </div>

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
          <span>正在独立窗口中播放</span>
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
        <PlaybackOptions
          v-if="activeDevice && !playback?.isLive"
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
          :disabled="casting.busy || playbackBusy"
          @quality="controls.setQuality"
          @refresh-media-qualities="casting.refreshMediaQualities"
          @danmaku="controls.setDanmaku"
        />
      </template>
    </section>
  </div>
</template>

<style scoped>
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
.session-header {
  padding: 0 0 20px;
  text-align: center;
}
.session-chips {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 8px;
}
.session-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 32px;
  max-width: 100%;
  padding: 6px 10px;
  border: 0;
  border-radius: 6px;
  background: var(--subtle);
  color: var(--muted);
  font-size: 12px;
  line-height: 20px;
  text-align: left;
  text-decoration: none;
}
.device-chip:hover {
  background: var(--hover);
}
.session-chip > span {
  min-width: 0;
  overflow-wrap: anywhere;
}
.room-chip {
  background: var(--accent-soft);
  color: var(--accent);
}
.room-chip:disabled {
  opacity: 1;
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
