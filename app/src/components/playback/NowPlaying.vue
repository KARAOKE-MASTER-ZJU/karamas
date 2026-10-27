<script setup lang="ts">
import { Music2, RotateCcw } from "@lucide/vue";
import { ref, watch } from "vue";
import { castingApi } from "@/services/casting";

const props = defineProps<{
  local?: boolean;
  title: string | null;
  addedBy: string | null;
  songId: string | null;
  songUrl: string | null;
  disabled: boolean;
  recasting: boolean;
  queuedCount: number | null;
  sungCount: number | null;
}>();
const emit = defineEmits<{ recast: [] }>();
const coverUrl = ref<string | null>(null);

watch(
  () => props.songUrl,
  async (songUrl) => {
    coverUrl.value = null;
    if (!songUrl) return;
    try {
      const url = await castingApi.getBilibiliCover(songUrl);
      if (songUrl !== props.songUrl) return;
      const image = new Image();
      image.referrerPolicy = "no-referrer";
      // 同一歌曲 URL 的封面可以复用，图片加载后再核对当前 URL。
      image.onload = () => {
        if (songUrl === props.songUrl) coverUrl.value = url;
      };
      image.src = url;
    } catch {
      // 封面获取失败时保留音符图标。
    }
  },
  { immediate: true, flush: "sync" },
);

function onCoverError(event: Event) {
  if (
    (event.currentTarget as HTMLImageElement).getAttribute("src") ===
    coverUrl.value
  )
    coverUrl.value = null;
}
</script>

<template>
  <div class="track-summary">
    <img
      v-if="coverUrl"
      :key="coverUrl"
      :src="coverUrl"
      class="track-cover"
      alt=""
      referrerpolicy="no-referrer"
      @error="onCoverError"
    />
    <Music2 v-else :size="24" aria-hidden="true" />
    <div class="track-info">
      <h3>{{ title || "暂无播放内容" }}</h3>
      <div class="track-meta">
        <span v-if="songId" class="user-badge">{{ addedBy || "SYSTEM" }}</span>
        <p v-if="queuedCount !== null && sungCount !== null">
          待唱 {{ queuedCount }} 首 · 已唱 {{ sungCount }} 首
        </p>
      </div>
    </div>
    <button
      class="icon-button recast-button"
      :aria-label="
        local
          ? recasting
            ? '重新播放中'
            : '重新播放'
          : recasting
            ? '重新投屏中'
            : '重新投屏'
      "
      :title="
        local
          ? recasting
            ? '重新播放中'
            : '重新播放'
          : recasting
            ? '重新投屏中'
            : '重新投屏'
      "
      :aria-busy="recasting"
      :disabled="disabled || !songId"
      @click="emit('recast')"
    >
      <RotateCcw :size="19" :class="{ spinning: recasting }" />
    </button>
  </div>
</template>

<style scoped>
.track-summary {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 4px 0 12px;
}
.track-summary > svg {
  flex-shrink: 0;
  box-sizing: content-box;
  padding: 16px;
  border-radius: 6px;
  background: var(--subtle);
  color: var(--muted);
}
.track-cover {
  width: 56px;
  height: 56px;
  flex-shrink: 0;
  border-radius: 6px;
  object-fit: cover;
}
.track-info {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
}
.track-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px 8px;
  margin-top: 6px;
}
.user-badge {
  max-width: 100%;
  padding: 2px 6px;
  border-radius: 4px;
  background: var(--subtle);
  color: var(--muted);
  font-size: 10px;
  line-height: 1.4;
}
.track-meta p {
  font-size: 12px;
  line-height: 1.6;
  color: var(--muted);
}
.recast-button,
.recast-button:hover:not(:disabled) {
  color: var(--accent);
}
.spinning {
  animation: spin 1s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(-360deg);
  }
}
@media (prefers-reduced-motion: reduce) {
  .spinning {
    animation: none;
  }
}
</style>
