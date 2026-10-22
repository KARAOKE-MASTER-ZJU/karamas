<script setup lang="ts">
import { Cast, Tv } from "@lucide/vue";
import type { CastDevice } from "@/types/casting";

defineProps<{ device: CastDevice; active: boolean; disabled: boolean }>();
const emit = defineEmits<{ select: [device: CastDevice] }>();
</script>

<template>
  <li class="device-row">
    <component :is="device.kind === 'dlna' ? Cast : Tv" :size="20" />
    <div class="device-info">
      <h3>{{ device.name }}</h3>
      <p>
        {{
          device.kind === "dlna"
            ? device.location
            : [device.brand, device.model].filter(Boolean).join(" · ")
        }}
      </p>
    </div>
    <button
      class="button secondary"
      :disabled="disabled"
      @click="emit('select', device)"
    >
      {{ active ? "断开" : "连接" }}
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
