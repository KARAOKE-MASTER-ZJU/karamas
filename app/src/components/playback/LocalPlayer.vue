<script setup lang="ts">
import { Play, RotateCcw } from "@lucide/vue";
import { ref } from "vue";
import { useCastingStore } from "@/stores/casting";
import { useLocalPlayer } from "@/composables/useLocalPlayer";

const casting = useCastingStore();
const video = ref<HTMLVideoElement | null>(null);
const { hasMedia, loading, error, needsGesture, startWithGesture, retry } =
  useLocalPlayer(video);
</script>

<template>
  <div
    v-show="casting.activeDevice?.kind === 'local'"
    class="local-player"
    aria-label="本地视频播放器"
  >
    <video
      ref="video"
      :controls="hasMedia && !loading"
      playsinline
      preload="metadata"
    />
    <div
      v-if="error || loading || !hasMedia"
      class="player-message"
      :role="error ? 'alert' : 'status'"
    >
      <span>{{ error || (loading ? "正在加载视频…" : "等待播放歌曲") }}</span>
      <button
        v-if="needsGesture"
        class="button secondary"
        @click="startWithGesture"
      >
        <Play :size="16" />开始播放
      </button>
      <button
        v-else-if="error"
        class="button secondary"
        :disabled="casting.busy"
        @click="retry"
      >
        <RotateCcw :size="16" />重新播放
      </button>
    </div>
  </div>
</template>

<style scoped>
.local-player {
  position: relative;
  width: min(
    100%,
    calc(var(--local-video-max-height, max(1px, calc(100dvh - 200px))) * 16 / 9)
  );
  aspect-ratio: 16 / 9;
  margin: 0 auto 16px;
  overflow: hidden;
  border-radius: 8px;
  background: #000;
  color: #fff;
}
.local-player video {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.player-message {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 20px;
  text-align: center;
  font-size: 14px;
  background: #0009;
}
.local-player video:fullscreen {
  width: 100vw;
  height: 100vh;
  object-fit: contain;
}
</style>
