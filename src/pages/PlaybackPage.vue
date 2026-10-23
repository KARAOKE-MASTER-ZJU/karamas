<script setup lang="ts">
import { Cast, ExternalLink } from "@lucide/vue";
import { computed } from "vue";
import { storeToRefs } from "pinia";
import NowPlaying from "@/components/playback/NowPlaying.vue";
import PlaybackProgress from "@/components/playback/PlaybackProgress.vue";
import PlaybackTransport from "@/components/playback/PlaybackTransport.vue";
import VolumeControl from "@/components/playback/VolumeControl.vue";
import PlaybackOptions from "@/components/playback/PlaybackOptions.vue";
import BilibiliPartsControl from "@/components/playback/BilibiliPartsControl.vue";
import { usePreferencesStore } from "@/stores/preferences";
import { useCastingStore } from "@/stores/casting";
import { usePlaybackControls } from "@/composables/usePlaybackControls";
import { openSongRoom } from "@/services/songRoom";

const { preferences } = storeToRefs(usePreferencesStore());
const casting = useCastingStore();
const { playback, volume, activeDevice, recasting } = storeToRefs(casting);
const controls = usePlaybackControls();
const room = computed(() => casting.room ?? preferences.value);
</script>

<template>
  <div class="playback-page">
    <div class="session-header" aria-label="当前投屏连接">
      <div class="session-chips">
        <span class="session-chip">
          <Cast :size="14" /><span>{{
            casting.connecting
              ? "连接中…"
              : (casting.activeDevice?.name ??
                (casting.connected ? "已连接设备" : "未连接设备"))
          }}</span>
        </span>
        <button
          type="button"
          class="session-chip room-chip"
          :disabled="!room.serverUrl || !room.roomId"
          :title="
            room.serverUrl ? `打开点歌台：${room.serverUrl}` : '服务器未设置'
          "
          :aria-label="
            room.serverUrl && room.roomId
              ? `打开房间 ${room.roomId} 的点歌台`
              : '房间未设置'
          "
          @click="controls.run(() => openSongRoom(room))"
        >
          <span>房间：{{ room.roomId || "未设置" }}</span>
          <ExternalLink :size="12" aria-hidden="true" />
        </button>
      </div>
    </div>

    <section
      class="content-section player-section"
      aria-labelledby="playing-title"
    >
      <h2 id="playing-title">正在播放</h2>
      <NowPlaying
        :title="playback?.title ?? null"
        :song-id="playback?.songId ?? null"
        :queued-count="playback?.queuedCount ?? null"
        :sung-count="playback?.sungCount ?? null"
        :disabled="!casting.connected || casting.busy"
        :recasting="recasting"
        @recast="controls.recast"
      />
      <BilibiliPartsControl
        v-if="playback?.songId"
        :key="playback.songId"
        :song-id="playback.songId"
        :disabled="casting.busy"
      />
      <PlaybackProgress
        :key="playback?.songId ?? undefined"
        :current-secs="playback?.currentSecs ?? null"
        :total-secs="playback?.totalSecs ?? null"
        :disabled="!playback?.capabilities.seek || casting.busy"
        @seek="controls.seek"
      />
      <PlaybackTransport
        :disabled="!casting.connected || casting.busy"
        :is-playing="playback?.isPlaying ?? false"
        :has-song="!!playback?.songId"
        :queued-count="playback?.queuedCount ?? 0"
        :sung-count="playback?.sungCount ?? 0"
        @previous="controls.previousSong"
        @next="controls.nextSong"
        @toggle-pause="controls.togglePause"
      />
      <VolumeControl
        :volume="volume"
        :disabled="!casting.connected || casting.busy"
        :absolute="!!playback?.capabilities.absoluteVolume"
        @change="controls.changeVolume"
        @set-volume="controls.setVolume"
      />
      <PlaybackOptions
        v-if="activeDevice"
        :mode="activeDevice.kind"
        :quality-qn="playback?.qualityQn ?? null"
        :danmaku="playback?.danmaku ?? null"
        :disabled="casting.busy"
        @quality="controls.setQuality"
        @danmaku="controls.setDanmaku"
      />
    </section>
  </div>
</template>

<style scoped>
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
.player-section > h2 {
  margin-bottom: 12px;
}

@media (max-width: 640px) {
  .player-section > h2 {
    margin-bottom: 18px;
  }
}
</style>
