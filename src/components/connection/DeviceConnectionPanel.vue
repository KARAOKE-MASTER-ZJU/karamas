<script setup lang="ts">
import {
  Cast,
  Tv,
  ChevronDown,
  RefreshCw,
  ScanQrCode,
  LoaderCircle,
} from "@lucide/vue";
import { computed, ref } from "vue";
import { storeToRefs } from "pinia";
import DeviceRow from "@/components/connection/DeviceRow.vue";
import ManualDeviceForm from "@/components/connection/ManualDeviceForm.vue";
import { useDeviceDiscovery } from "@/composables/useDeviceDiscovery";
import { useCastingStore } from "@/stores/casting";
import { useBilibiliStore } from "@/stores/bilibili";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { errorMessage } from "@/services/casting";
import type { CastDevice } from "@/types/casting";

const emit = defineEmits<{ connected: [] }>();
const casting = useCastingStore();
const auth = useBilibiliStore();
const { activeDevice } = storeToRefs(casting);
const { notify } = useOperationNotice();
const showManualConnect = ref(false);
const {
  mode,
  devices,
  searching,
  deviceUrl,
  deviceError,
  resolvingDevice,
  searchDevices,
  resolveDevice,
  deviceKey,
  isActive,
} = useDeviceDiscovery(activeDevice);
const searchLabel = computed(() =>
  searching[mode.value]
    ? "正在搜索设备"
    : mode.value === "dlna"
      ? "搜索设备"
      : auth.session
        ? "刷新设备"
        : "扫码登录",
);

async function selectDevice(device: CastDevice) {
  try {
    if (isActive(device)) {
      await casting.disconnect();
    } else {
      await casting.connect(device);
      emit("connected");
    }
  } catch (error) {
    notify(errorMessage(error), "error");
  }
}
</script>

<template>
  <section
    class="content-section devices-section"
    aria-labelledby="device-title"
  >
    <fieldset class="mode-options">
      <legend class="sr-only">投屏方式</legend>
      <label :class="['mode-option', { selected: mode === 'dlna' }]">
        <input v-model="mode" type="radio" name="cast-mode" value="dlna" />
        <Cast :size="18" /><span>DLNA</span>
      </label>
      <label :class="['mode-option', { selected: mode === 'bilibili' }]">
        <input v-model="mode" type="radio" name="cast-mode" value="bilibili" />
        <Tv :size="18" /><span>小电视</span>
      </label>
    </fieldset>
    <div class="section-toolbar">
      <h2 id="device-title">
        {{ mode === "dlna" ? "可用设备" : "小电视设备" }}
      </h2>
      <div class="device-actions">
        <button
          :class="
            mode === 'bilibili' && !auth.session
              ? 'button text-button'
              : 'icon-button'
          "
          :disabled="searching[mode] || auth.loggingIn"
          :aria-label="searchLabel"
          :aria-busy="searching[mode]"
          @click="searchDevices()"
        >
          <component
            :is="
              searching[mode]
                ? LoaderCircle
                : mode === 'dlna' || auth.session
                  ? RefreshCw
                  : ScanQrCode
            "
            :size="18"
            :class="{ searching: searching[mode] }"
            aria-hidden="true"
          />
          <span v-if="mode === 'bilibili' && !auth.session">扫码登录</span>
        </button>
        <button
          v-if="mode === 'dlna'"
          class="button text-button manual-toggle"
          type="button"
          aria-label="手动输入设备地址"
          :aria-expanded="showManualConnect"
          aria-controls="manual-connect-form"
          @click="showManualConnect = !showManualConnect"
        >
          手动输入<ChevronDown :size="16" />
        </button>
      </div>
    </div>

    <ManualDeviceForm
      v-if="mode === 'dlna'"
      v-show="showManualConnect"
      v-model="deviceUrl"
      :error="deviceError"
      :resolving="resolvingDevice"
      :disabled="casting.busy"
      @submit="resolveDevice(selectDevice)"
    />
    <ul v-if="devices.length" class="device-list">
      <DeviceRow
        v-for="device in devices"
        :key="deviceKey(device)"
        :device="device"
        :active="isActive(device)"
        :disabled="casting.busy || resolvingDevice"
        @select="selectDevice"
      />
    </ul>
    <div v-else class="device-empty" :aria-busy="searching[mode]">
      <component :is="mode === 'dlna' ? Cast : Tv" :size="32" />
      <p>
        {{
          searching[mode]
            ? "正在搜索设备…"
            : mode === "bilibili" && !auth.session
              ? "尚未登录哔哩哔哩"
              : "暂无设备"
        }}
      </p>
      <span v-if="mode === 'dlna'">将电视与本机连接到同一网络后搜索</span>
    </div>
  </section>
</template>

<style scoped>
.device-list {
  list-style: none;
  margin: 8px 0 0;
  padding: 0;
}
.mode-options {
  display: flex;
  gap: 4px;
  background: var(--subtle);
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 4px;
  margin: 0 0 24px;
  width: 100%;
}
.mode-option {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  flex: 1;
  position: relative;
  min-height: 36px;
  padding: 6px 12px;
  font-size: 14px;
  border-radius: 5px;
  color: var(--muted);
  cursor: pointer;
}
.mode-option input {
  position: absolute;
  opacity: 0;
  width: 1px;
  height: 1px;
}
.mode-option:has(input:focus-visible) {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.mode-option.selected {
  background: var(--surface);
  color: var(--text);
  box-shadow: 0 1px 3px #00000014;
}
.section-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}
.device-empty {
  min-height: 144px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  text-align: center;
  color: var(--muted);
  padding: 24px 8px;
}
.device-empty > svg {
  opacity: 0.65;
  margin-bottom: 4px;
}
.device-empty p {
  font-size: 14px;
}
.device-empty span {
  font-size: 13px;
  line-height: 1.6;
}
.device-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}
.manual-toggle[aria-expanded="true"] > svg {
  transform: rotate(180deg);
}
.searching {
  animation: search-spin 1s linear infinite;
}
@keyframes search-spin {
  to {
    transform: rotate(360deg);
  }
}
@media (prefers-reduced-motion: reduce) {
  .searching {
    animation: none;
  }
}
</style>
