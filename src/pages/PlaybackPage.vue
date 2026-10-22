<!-- 播放页面 -->

<script setup lang="ts">
import { Cast, Music2, SkipBack, Play, Pause, SkipForward } from "@lucide/vue";
import PlaybackProgress from "@/components/playback/PlaybackProgress.vue";
import VolumeControl from "@/components/playback/VolumeControl.vue";
import { computed } from "vue";
import { storeToRefs } from "pinia";
import { usePreferencesStore } from "@/stores/preferences";
import { useCastingStore } from "@/stores/casting";
import { castingApi, errorMessage } from "@/services/casting";
import { useOperationNotice } from "@/composables/useOperationNotice";
const { preferences } = storeToRefs(usePreferencesStore());
const casting = useCastingStore();
const { notify } = useOperationNotice();
const { playback, volume } = storeToRefs(casting);
const room = computed(() => casting.room ?? preferences.value);
async function run(action: () => Promise<unknown>) {
  try {
    await action();
  } catch (error) {
    notify(errorMessage(error), "error");
  }
}
function previousSong() {
  void run(() => casting.control(castingApi.previousSong));
}
function nextSong() {
  void run(() => casting.control(castingApi.nextSong));
}
function seek(seconds: number) {
  void run(() => casting.control(() => castingApi.seek(seconds)));
}
</script>

<template>
  <div class="playback-page">
    <!-- 当前投屏连接信息 -->
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
        <span
          class="session-chip room-chip"
          :title="room.serverUrl || '服务器未设置'"
        >
          <span>房间：{{ room.roomId || "未设置" }}</span>
        </span>
      </div>
    </div>

    <!-- 播放器控制部分 -->
    <section
      class="content-section player-section"
      aria-labelledby="playing-title"
    >
      <h2 id="playing-title">正在播放</h2>
      <div class="track-placeholder">
        <Music2 :size="24" />
        <div>
          <h3>{{ playback?.title || "暂无播放内容" }}</h3>
        </div>
      </div>
      <PlaybackProgress
        :current-secs="playback?.currentSecs ?? null"
        :total-secs="playback?.totalSecs ?? null"
        :disabled="!playback?.capabilities.seek || casting.busy"
        @seek="seek"
      />
      <fieldset
        class="player-controls"
        :disabled="!casting.connected || casting.busy"
      >
        <legend class="sr-only">播放控制，连接设备后可用</legend>
        <button class="icon-button" aria-label="上一首" @click="previousSong()">
          <SkipBack :size="24" />
        </button>
        <button
          class="play-button"
          :aria-label="playback?.isPlaying ? '暂停' : '播放'"
          :disabled="!playback?.songId"
          @click="run(casting.togglePause)"
        >
          <component :is="playback?.isPlaying ? Pause : Play" :size="25" />
        </button>
        <button class="icon-button" aria-label="下一首" @click="nextSong()">
          <SkipForward :size="24" />
        </button>
      </fieldset>
      <VolumeControl
        :volume="volume"
        :disabled="!casting.connected || casting.busy"
        :absolute="!!playback?.capabilities.absoluteVolume"
        @change="(increase) => run(() => casting.changeVolume(increase))"
        @set-volume="(value) => run(() => casting.setVolume(value))"
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
.playback-page .player-section {
  padding: 0 0 16px;
  border-bottom: 0;
}
.player-section > h2 {
  margin-bottom: 12px;
}
.track-placeholder {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 4px 0 20px;
}
.track-placeholder > svg {
  box-sizing: content-box;
  padding: 16px;
  border-radius: 6px;
  background: var(--subtle);
  color: var(--muted);
}
.track-placeholder > div {
  min-width: 0;
  overflow-wrap: anywhere;
}
.player-controls {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 32px;
  padding: 12px 0;
}
.play-button {
  width: 52px;
  height: 52px;
  border: 0;
  border-radius: 50%;
  display: grid;
  place-items: center;
  background: var(--accent-soft);
  color: var(--accent);
}
@media (max-width: 640px) {
  .player-section > h2 {
    margin-bottom: 18px;
  }
}
</style>
