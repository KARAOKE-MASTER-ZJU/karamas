<!-- 播放页面 -->

<script setup lang="ts">
import {
  Cast,
  Music2,
  SkipBack,
  Play,
  Pause,
  SkipForward,
  Volume2,
  Minus,
  Plus,
} from "@lucide/vue";
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
const progress = computed(() => {
  if (playback.value?.currentSecs == null || !playback.value.totalSecs)
    return 0;
  return Math.min(
    100,
    (100 * playback.value.currentSecs) / playback.value.totalSecs,
  );
});
function time(seconds: number | null | undefined) {
  if (seconds == null) return "--:--";
  return `${Math.floor(seconds / 60)
    .toString()
    .padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
}
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
function seek(event: Event) {
  const seconds = Number((event.target as HTMLInputElement).value);
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
      <div class="progress-track">
        <div class="progress-fill" :style="{ width: `${progress}%` }" />
        <input
          class="progress-input"
          type="range"
          min="0"
          :max="playback?.totalSecs || 1"
          :value="playback?.currentSecs ?? 0"
          :disabled="
            !playback?.capabilities.seek ||
            !playback?.totalSecs ||
            playback.currentSecs === null ||
            casting.busy
          "
          aria-label="播放进度"
          :aria-valuetext="time(playback?.currentSecs)"
          @change="seek"
        />
      </div>
      <div class="time-labels">
        <span>{{ time(playback?.currentSecs) }}</span
        ><span>{{ time(playback?.totalSecs) }}</span>
      </div>
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
      <div class="volume-heading">
        <label for="volume"><Volume2 :size="16" />音量</label>
      </div>
      <fieldset
        class="volume-control"
        :disabled="!casting.connected || casting.busy"
      >
        <legend class="sr-only">音量控制，连接设备后可用</legend>
        <button
          class="icon-button"
          aria-label="降低音量"
          @click="run(() => casting.changeVolume(false))"
        >
          <Minus :size="18" />
        </button>
        <input
          id="volume"
          type="range"
          min="0"
          max="100"
          :value="volume ?? 0"
          :disabled="!playback?.capabilities.absoluteVolume || volume === null"
          aria-label="音量"
          @change="
            run(() =>
              casting.setVolume(
                Number(($event.target as HTMLInputElement).value),
              ),
            )
          "
        />
        <button
          class="icon-button"
          aria-label="提高音量"
          @click="run(() => casting.changeVolume(true))"
        >
          <Plus :size="18" />
        </button>
      </fieldset>
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
.progress-track {
  position: relative;
  height: 4px;
  background: var(--line);
  border-radius: 2px;
}
.progress-fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent);
}
.progress-input {
  position: absolute;
  inset: -8px 0;
  width: 100%;
  height: 20px;
  margin: 0;
  opacity: 0;
  cursor: pointer;
}
.progress-track:has(input:focus-visible) {
  outline: 2px solid var(--accent);
  outline-offset: 3px;
}
.track-placeholder > div {
  min-width: 0;
  overflow-wrap: anywhere;
}
.time-labels {
  display: flex;
  justify-content: space-between;
  color: var(--muted);
  font-size: 12px;
  margin-top: 8px;
  font-variant-numeric: tabular-nums;
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
.volume-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  max-width: 320px;
  margin: 0 auto 4px;
  color: var(--muted);
  font-size: 12px;
  line-height: 20px;
}
.volume-heading label {
  display: flex;
  align-items: center;
  gap: 6px;
}
.volume-control {
  display: flex;
  align-items: center;
  gap: 12px;
  margin: 0 auto;
  max-width: 336px;
}
.volume-control input {
  flex: 1;
  width: 0;
  min-width: 0;
  margin: 0;
  accent-color: var(--accent);
}
@media (max-width: 640px) {
  .player-section > h2 {
    margin-bottom: 18px;
  }
}
</style>
