<script setup lang="ts">
import { ref, watch } from "vue";
import FormField from "@/components/base/FormField.vue";
import SelectField from "@/components/base/SelectField.vue";
import { castingApi } from "@/services/casting";
import type { MediaFormat } from "@/types/casting";
import type {
  DiagnosticEntry,
  DiagnosticPanelProps,
  PlaybackTest,
} from "@/types/diagnostics";

const props = defineProps<DiagnosticPanelProps>();
const entries = defineModel<DiagnosticEntry[]>("entries", { required: true });
const songUrl = ref("https://www.bilibili.com/video/BV1GJ411x7h7/");
const format = ref<MediaFormat>("mp4");
const direct = ref("proxy");
const seconds = ref("30");
const volume = ref("20");

watch(format, (value) => {
  if (value === "dash") direct.value = "proxy";
});

function playback(test: PlaybackTest, title: string) {
  return props.run(async (current) => {
    const responses = await castingApi.diagnosticPlayback(props.location, test);
    if (current())
      entries.value.unshift(
        ...responses.map((result, index) => ({
          title:
            test.action === "load"
              ? ["Stop", "SetAVTransportURI", "Play"][index]!
              : title,
          result,
        })),
      );
  });
}
function loadMedia() {
  return playback(
    {
      action: "load",
      songUrl: songUrl.value.trim(),
      format: format.value,
      direct: direct.value === "direct",
    },
    "播放测试",
  );
}
function integer(value: string, max: number) {
  const number = Number(value);
  if (!value.trim() || !Number.isInteger(number) || number < 0 || number > max)
    throw new Error(`请输入 0 到 ${max} 之间的整数`);
  return number;
}
function setNumber(action: "seek" | "volume") {
  return props.run(async (current) => {
    const test: PlaybackTest =
      action === "seek"
        ? { action, seconds: integer(seconds.value, 86400) }
        : { action, volume: integer(volume.value, 100) };
    const responses = await castingApi.diagnosticPlayback(props.location, test);
    if (current())
      entries.value.unshift(
        ...responses.map((result) => ({
          title: action === "seek" ? "定位" : "音量",
          result,
        })),
      );
  });
}
</script>

<template>
  <div class="diagnostic-form">
    <form class="diagnostic-form" @submit.prevent="loadMedia">
      <FormField
        id="diagnostic-video"
        v-model="songUrl"
        label="测试视频"
        type="url"
        required
      />
      <div class="mode-fields diagnostic-inline-fields">
        <SelectField id="diagnostic-format" v-model="format" label="格式"
          ><option value="mp4">MP4</option>
          <option value="dash">DASH</option>
          <option value="hls">HLS</option></SelectField
        >
        <SelectField id="diagnostic-route" v-model="direct" label="传输"
          ><option value="proxy">反向代理</option>
          <option value="direct" :disabled="format === 'dash'">
            直连
          </option></SelectField
        >
        <button class="button primary">播放</button>
      </div>
    </form>
    <div
      class="diagnostic-actions transport-actions"
      role="group"
      aria-label="播放控制"
    >
      <button
        type="button"
        class="button secondary"
        @click="playback({ action: 'pause' }, '暂停')"
      >
        暂停
      </button>
      <button
        type="button"
        class="button secondary"
        @click="playback({ action: 'play' }, '继续')"
      >
        继续
      </button>
      <button
        type="button"
        class="button secondary"
        @click="playback({ action: 'stop' }, '停止')"
      >
        停止
      </button>
    </div>
    <div class="number-controls">
      <form class="number-action" @submit.prevent="setNumber('seek')">
        <FormField
          id="diagnostic-seconds"
          v-model="seconds"
          label="位置"
          aria-label="位置（秒）"
          inputmode="numeric"
          pattern="[0-9]*"
          required
        />
        <span class="number-unit" aria-hidden="true">秒</span>
        <button class="button secondary">定位</button>
        <button
          type="button"
          class="button secondary"
          aria-label="读取播放位置"
          @click="playback({ action: 'read' }, '播放位置')"
        >
          读取
        </button>
      </form>
      <form
        class="number-action volume-action"
        @submit.prevent="setNumber('volume')"
      >
        <FormField
          id="diagnostic-volume"
          v-model="volume"
          label="音量"
          inputmode="numeric"
          pattern="[0-9]*"
          required
        />
        <button class="button secondary" aria-label="设置音量">设置</button>
        <button
          type="button"
          class="button secondary"
          aria-label="读取音量"
          @click="playback({ action: 'readVolume' }, '音量')"
        >
          读取
        </button>
      </form>
    </div>
  </div>
</template>

<style scoped>
.mode-fields {
  display: flex;
  align-items: center;
  gap: 20px;
  max-width: 100%;
}
.mode-fields :deep(.select-control) {
  width: 104px;
}
.mode-fields > .field:nth-child(2) :deep(.select-control) {
  width: 144px;
}
.transport-actions {
  gap: 0;
}
.transport-actions .button {
  border-radius: 0;
}
.transport-actions .button + .button {
  margin-left: -1px;
}
.transport-actions .button:first-child {
  border-radius: 6px 0 0 6px;
}
.transport-actions .button:last-child {
  border-radius: 0 6px 6px 0;
}
.number-controls {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.number-action {
  display: grid;
  grid-template-columns: 28px 72px 14px repeat(2, max-content);
  align-items: center;
  gap: 8px;
}
.number-action :deep(.field) {
  display: contents;
}
.number-unit {
  color: var(--muted);
  font-size: 13px;
}
.volume-action > button:first-of-type {
  grid-column: 4;
}
@media (max-width: 640px) {
  .mode-fields {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.3fr) auto;
    align-items: end;
    gap: 8px;
    width: 100%;
  }
  .mode-fields > .field {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
  .mode-fields :deep(.select-control),
  .mode-fields > .field:nth-child(2) :deep(.select-control) {
    width: 100%;
  }
}
</style>
