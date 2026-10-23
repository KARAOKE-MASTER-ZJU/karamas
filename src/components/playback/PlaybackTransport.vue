<script setup lang="ts">
import { SkipBack, Play, Pause, SkipForward } from "@lucide/vue";
defineProps<{
  disabled: boolean;
  isPlaying: boolean;
  hasSong: boolean;
  queuedCount: number;
  sungCount: number;
}>();
const emit = defineEmits<{
  previous: [];
  next: [];
  play: [];
  pause: [];
}>();
</script>

<template>
  <fieldset class="player-controls" :disabled="disabled">
    <legend class="sr-only">播放控制</legend>
    <button
      class="icon-button"
      aria-label="上一首"
      :disabled="sungCount === 0"
      @click="emit('previous')"
    >
      <SkipBack :size="24" />
    </button>
    <button
      class="play-button"
      :aria-label="isPlaying ? '暂停' : '播放'"
      :disabled="!hasSong"
      @click="isPlaying ? emit('pause') : emit('play')"
    >
      <component :is="isPlaying ? Pause : Play" :size="25" />
    </button>
    <button
      class="icon-button"
      aria-label="下一首"
      :disabled="queuedCount === 0"
      @click="emit('next')"
    >
      <SkipForward :size="24" />
    </button>
  </fieldset>
</template>

<style scoped>
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
</style>
