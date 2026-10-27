<script setup lang="ts">
import { Cast, Monitor, ExternalLink } from "@lucide/vue";
import { RouterLink } from "vue-router";
import {
  onActivated,
  onDeactivated,
  onMounted,
  onBeforeUnmount,
  ref,
} from "vue";
import { storeToRefs } from "pinia";
import LocalPlayer from "@/components/playback/LocalPlayer.vue";
import NowPlaying from "@/components/playback/NowPlaying.vue";
import PlaybackProgress from "@/components/playback/PlaybackProgress.vue";
import PlaybackTransport from "@/components/playback/PlaybackTransport.vue";
import VolumeControl from "@/components/playback/VolumeControl.vue";
import PlaybackOptions from "@/components/playback/PlaybackOptions.vue";
import BilibiliPartsControl from "@/components/playback/BilibiliPartsControl.vue";
import { useCastingStore } from "@/stores/casting";
import { usePlaybackControls } from "@/composables/usePlaybackControls";
import { openSongRoom } from "@/services/songRoom";

const casting = useCastingStore();
const { playback, volume, activeDevice, recasting, room } =
  storeToRefs(casting);
const controls = usePlaybackControls();
const videoContainer = ref<HTMLElement | null>(null);
let layoutObserver: ResizeObserver | undefined;
let layoutFrame = 0;

function fitVideo() {
  cancelAnimationFrame(layoutFrame);
  layoutFrame = requestAnimationFrame(() => {
    const slot = videoContainer.value;
    const page = slot?.closest<HTMLElement>(".page-content");
    const workspace = page?.closest<HTMLElement>(".workspace");
    if (!slot || !page || !workspace) return;
    // 只扣除视频上方内容；下方控件保留纵向排列，由用户滚动查看。
    // 相对页面计算偏移，滚动时不会反复改变视频尺寸。
    const videoTop =
      slot.getBoundingClientRect().top - page.getBoundingClientRect().top;
    const available = Math.max(
      1,
      Math.floor(workspace.clientHeight - videoTop - 16),
    );
    const value = `${available}px`;
    if (slot.style.getPropertyValue("--local-video-max-height") !== value)
      slot.style.setProperty("--local-video-max-height", value);
  });
}
function observeVideoLayout() {
  layoutObserver?.disconnect();
  const page = videoContainer.value?.closest<HTMLElement>(".page-content");
  const workspace = page?.closest<HTMLElement>(".workspace");
  if (page && workspace) {
    layoutObserver = new ResizeObserver(fitVideo);
    layoutObserver.observe(page);
    layoutObserver.observe(workspace);
  }
  fitVideo();
}
function stopObservingVideoLayout() {
  layoutObserver?.disconnect();
  cancelAnimationFrame(layoutFrame);
}
onMounted(observeVideoLayout);
onActivated(observeVideoLayout);
onDeactivated(stopObservingVideoLayout);
onBeforeUnmount(stopObservingVideoLayout);
</script>

<template>
  <div class="playback-page">
    <div class="session-header" aria-label="当前投屏连接">
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
      <h2 id="playing-title">正在播放</h2>
      <div ref="videoContainer" class="video-container">
        <LocalPlayer />
      </div>
      <NowPlaying
        :local="activeDevice?.kind === 'local'"
        :title="playback?.title ?? null"
        :added-by="playback?.addedBy ?? null"
        :song-id="playback?.songId ?? null"
        :song-url="playback?.songUrl ?? null"
        :queued-count="playback?.queuedCount ?? null"
        :sung-count="playback?.sungCount ?? null"
        :disabled="!casting.connected || casting.sessionBusy"
        :recasting="recasting"
        @recast="controls.recast"
      />
      <BilibiliPartsControl
        v-if="playback?.songId"
        :key="playback.songId"
        :song-id="playback.songId"
        :song-url="playback.songUrl"
        :disabled="casting.sessionBusy"
      />
      <PlaybackProgress
        :key="playback?.songId ?? undefined"
        :current-secs="playback?.currentSecs ?? null"
        :total-secs="playback?.totalSecs ?? null"
        :disabled="!playback?.capabilities.seek || casting.sessionBusy"
        @seek="controls.seek"
      />
      <PlaybackTransport
        :disabled="!casting.connected || casting.sessionBusy"
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
        :disabled="!casting.connected || casting.sessionBusy"
        :absolute="!!playback?.capabilities.absoluteVolume"
        @change="controls.changeVolume"
        @set-volume="controls.setVolume"
      />
      <PlaybackOptions
        v-if="activeDevice"
        :mode="activeDevice.kind"
        :song-id="playback?.songId ?? null"
        :song-url="playback?.songUrl ?? null"
        :dlna-proxy-enabled="casting.dlnaProxyEnabled"
        :quality-qn="playback?.qualityQn ?? null"
        :danmaku="playback?.danmaku ?? null"
        :disabled="casting.sessionBusy"
        @quality="controls.setQuality"
        @danmaku="controls.setDanmaku"
      />
    </section>
  </div>
</template>

<style scoped>
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
.player-section > h2 {
  margin-bottom: 12px;
}

@media (max-width: 640px) {
  .player-section > h2 {
    margin-bottom: 18px;
  }
}
</style>
