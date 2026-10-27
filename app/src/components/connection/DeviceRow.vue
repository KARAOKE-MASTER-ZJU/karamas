<script setup lang="ts">
import { Cast, Tv, Monitor, RotateCw } from "@lucide/vue";
import type { CastDevice } from "@/types/casting";

defineProps<{
  device: CastDevice;
  status: "waiting" | "connecting" | "connected" | "disconnected" | null;
  disabled: boolean;
}>();
const emit = defineEmits<{
  select: [device: CastDevice];
  retry: [device: CastDevice];
}>();
const statusLabels = {
  waiting: "待加入房间",
  connecting: "连接中…",
  connected: "已连接",
  disconnected: "未连接",
};
</script>

<template>
  <li class="device-row" :class="{ active: status === 'connected' }">
    <component
      :is="
        device.kind === 'dlna' ? Cast : device.kind === 'local' ? Monitor : Tv
      "
      :size="20"
    />
    <div class="device-info">
      <div class="device-heading">
        <h3>{{ device.name }}</h3>
        <span v-if="status" class="connection-status" role="status">{{
          statusLabels[status]
        }}</span>
      </div>
      <p>
        {{
          device.kind === "dlna"
            ? device.location
            : device.kind === "local"
              ? "在本机播放视频和声音"
              : [device.brand, device.model].filter(Boolean).join(" · ")
        }}
      </p>
    </div>
    <button
      v-if="status === 'disconnected'"
      class="icon-button"
      aria-label="重试连接"
      :disabled="disabled"
      @click="emit('retry', device)"
    >
      <RotateCw :size="18" aria-hidden="true" />
    </button>
    <button
      class="button secondary"
      :disabled="disabled"
      @click="emit('select', device)"
    >
      {{
        status === "connected"
          ? device.kind === "local"
            ? "关闭"
            : "断开"
          : status
            ? "取消"
            : device.kind === "local"
              ? "开启"
              : "连接"
      }}
    </button>
  </li>
</template>

<style scoped>
.device-row {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px 0;
  border-bottom: 1px solid var(--line);
}
.device-row:last-child {
  border-bottom: 0;
}
.device-row > svg {
  color: var(--muted);
}
.device-row.active > svg {
  color: var(--accent);
}
.device-heading {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px 8px;
}
.device-heading h3 {
  min-width: 0;
  max-width: 100%;
}
.connection-status {
  flex-shrink: 0;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--accent-soft);
  color: var(--accent);
  font-size: 12px;
  line-height: 18px;
}
.device-info {
  flex: 1;
  min-width: 0;
}
.device-info h3,
.device-info p {
  overflow-wrap: anywhere;
}
.device-info p {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--muted);
}
</style>
