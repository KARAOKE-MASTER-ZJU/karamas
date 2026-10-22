<!-- 播放页面 -->

<script setup lang="ts">
import {
  Cast,
  Music2,
  SkipBack,
  Play,
  SkipForward,
  Volume2,
  Minus,
  Plus,
} from "@lucide/vue";
import { storeToRefs } from "pinia";
import { usePreferencesStore } from "@/stores/preferences";
const { preferences } = storeToRefs(usePreferencesStore());
</script>

<template>
  <div class="playback-page">
    <!-- 当前投屏连接信息 -->
    <div class="session-header" aria-label="当前投屏连接">
      <div class="session-chips">
        <span class="session-chip">
          <Cast :size="14" /><span>未连接设备</span>
        </span>
        <span
          class="session-chip room-chip"
          :title="preferences.serverUrl || '服务器未设置'"
        >
          <span>房间：{{ preferences.roomId || "未设置" }}</span>
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
          <h3>暂无播放内容</h3>
        </div>
      </div>
      <div
        class="progress-track"
        role="progressbar"
        aria-label="播放进度"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow="0"
      />
      <div class="time-labels"><span>00:00</span><span>00:00</span></div>
      <fieldset class="player-controls" disabled>
        <legend class="sr-only">播放控制，连接设备后可用</legend>
        <button class="icon-button" aria-label="上一首">
          <SkipBack :size="24" />
        </button>
        <button class="play-button" aria-label="播放">
          <Play :size="25" />
        </button>
        <button class="icon-button" aria-label="下一首">
          <SkipForward :size="24" />
        </button>
      </fieldset>
      <div class="volume-heading">
        <label for="volume"><Volume2 :size="16" />音量</label>
      </div>
      <fieldset class="volume-control" disabled>
        <legend class="sr-only">音量控制，连接设备后可用</legend>
        <button class="icon-button" aria-label="降低音量">
          <Minus :size="18" />
        </button>
        <input
          id="volume"
          type="range"
          min="0"
          max="100"
          value="0"
          aria-label="音量"
        />
        <button class="icon-button" aria-label="提高音量">
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
  height: 4px;
  background: var(--line);
  border-radius: 2px;
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
