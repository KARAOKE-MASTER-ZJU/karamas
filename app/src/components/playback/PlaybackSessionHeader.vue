<script setup lang="ts">
import { Cast, Monitor, ExternalLink, RefreshCw } from "@lucide/vue";
import { computed } from "vue";
import { RouterLink } from "vue-router";
import type { ConnectionConfig } from "@/types";
import type { CastDevice } from "@/types/casting";

const props = defineProps<{
  activeDevice: CastDevice | null;
  selectedDevice: CastDevice | null;
  connecting: boolean;
  connected: boolean;
  room: ConnectionConfig | null;
  refreshDisabled: boolean;
  refreshingPlaylist: boolean;
}>();
const emit = defineEmits<{
  openRoom: [room: ConnectionConfig];
  refreshPlaylist: [];
}>();

const deviceLabel = computed(() => {
  if (props.connecting) return "连接中…";
  if (props.activeDevice) return props.activeDevice.name;
  if (props.selectedDevice) {
    return `${props.selectedDevice.name} · ${props.room ? "未连接" : "待加入房间"}`;
  }
  return props.connected ? "已连接设备" : "未连接设备";
});
</script>

<template>
  <div class="session-header" aria-label="当前投屏连接">
    <div class="session-chips">
      <RouterLink :to="{ name: 'connection' }" class="session-chip device-chip">
        <component
          :is="activeDevice?.kind === 'local' ? Monitor : Cast"
          :size="14"
          aria-hidden="true"
        />
        <span>{{ deviceLabel }}</span>
      </RouterLink>
      <div class="room-actions">
        <button
          type="button"
          class="session-chip room-chip"
          :disabled="!room"
          :title="room ? `打开点歌台：${room.serverUrl}` : '未加入房间'"
          :aria-label="room ? `打开房间 ${room.roomId} 的点歌台` : '未加入房间'"
          @click="room && emit('openRoom', room)"
        >
          <span>{{ room ? `房间：${room.roomId}` : "未加入房间" }}</span>
          <ExternalLink v-if="room" :size="12" aria-hidden="true" />
        </button>
        <button
          type="button"
          class="session-chip refresh-chip"
          :disabled="refreshDisabled || refreshingPlaylist"
          :aria-busy="refreshingPlaylist"
          :title="refreshingPlaylist ? '正在刷新歌单' : undefined"
          @click="emit('refreshPlaylist')"
        >
          <RefreshCw
            :size="14"
            :class="{ spinning: refreshingPlaylist }"
            aria-hidden="true"
          />
          <span>刷新歌单</span>
        </button>
      </div>
    </div>
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
  align-items: center;
  justify-content: center;
  gap: 8px;
}
.session-chip {
  position: relative;
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
.device-chip:hover,
.refresh-chip:hover:not(:disabled) {
  background: var(--hover);
}
.session-chip > span {
  min-width: 0;
  overflow-wrap: anywhere;
}
.room-actions {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  max-width: 100%;
  gap: 8px;
}
.room-chip {
  min-width: 0;
  background: var(--accent-soft);
  color: var(--accent);
}
.room-chip:disabled {
  opacity: 1;
}
.refresh-chip {
  flex-shrink: 0;
  white-space: nowrap;
}
.refresh-chip[aria-busy="true"] {
  opacity: 0.72;
}
.spinning {
  animation: refresh-spin 1s linear infinite;
}
@keyframes refresh-spin {
  to {
    transform: rotate(360deg);
  }
}
@media (prefers-reduced-motion: reduce) {
  .spinning {
    animation: none;
  }
}
@media (pointer: coarse) {
  .session-chip::after {
    content: "";
    position: absolute;
    inset: -4px 0;
  }
}
</style>
