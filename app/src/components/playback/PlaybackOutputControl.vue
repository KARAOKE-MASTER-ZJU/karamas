<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ChevronDown } from "@lucide/vue";
import SelectionDialog from "@/components/base/SelectionDialog.vue";
import { castingApi, errorMessage } from "@/services/casting";
import type {
  PlaybackDisplay,
  PlaybackOutputCapabilities,
} from "@/types/playbackOutput";
import { getPlaybackOutputCapabilities } from "@/services/playbackWindow";

const props = defineProps<{ selected: string; disabled: boolean }>();
const emit = defineEmits<{ change: [outputId: string] }>();
const showDialog = ref(false);
const capabilities = ref<PlaybackOutputCapabilities>({
  window: false,
  screen: false,
});
onMounted(() => {
  void getPlaybackOutputCapabilities()
    .then((value) => {
      capabilities.value = value;
    })
    .catch(() => {});
});
const displays = ref<PlaybackDisplay[]>([]);
const loading = ref(false);
const error = ref("");
const refreshed = ref(false);
const options = computed(() => [
  { id: "embedded", label: "本地内嵌播放", detail: "" },
  {
    id: "window",
    label: "独立窗口",
    detail: "",
    disabled: !capabilities.value.window,
  },
  ...displays.value.map((display, index) => ({
    id: display.id,
    disabled: !capabilities.value.screen,
    label: `屏幕 ${index + 1}${display.name ? ` · ${display.name}` : ""}`,
    description: [
      `${display.width} × ${display.height}`,
      display.builtIn ? "内置" : "",
    ]
      .filter(Boolean)
      .join(" · "),
  })),
]);
const selectedLabel = computed(
  () => options.value.find((option) => option.id === props.selected)?.label,
);

function selectOutput(outputId: string) {
  emit("change", outputId);
  showDialog.value = false;
}

function open() {
  showDialog.value = true;
  void refresh();
}

async function refresh() {
  if (loading.value) return;
  loading.value = true;
  error.value = "";
  try {
    displays.value = await castingApi.listPlaybackDisplays();
    refreshed.value = true;
    if (
      props.selected !== "embedded" &&
      props.selected !== "window" &&
      !displays.value.some((display) => display.id === props.selected)
    ) {
      emit("change", "embedded");
    }
  } catch (cause) {
    error.value = `检测屏幕失败：${errorMessage(cause)}`;
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <div class="playback-output">
    <button
      type="button"
      class="button text-button output-button"
      :aria-label="`选择播放位置，当前：${selectedLabel}`"
      aria-haspopup="dialog"
      :aria-expanded="showDialog"
      :disabled="disabled"
      @click="open"
    >
      <span>{{ selectedLabel }}</span>
      <ChevronDown :size="16" aria-hidden="true" />
    </button>
    <SelectionDialog
      v-if="showDialog"
      title="选择播放位置"
      close-label="关闭播放位置选择"
      refresh-label="刷新屏幕列表"
      list-label="播放位置列表"
      loading-message="正在检测屏幕…"
      :options="options"
      :selected-id="selected"
      :loading="loading"
      :disabled="disabled"
      :error="error"
      :message="refreshed && !displays.length ? '未检测到可用屏幕' : ''"
      @close="showDialog = false"
      @refresh="refresh"
      @select="selectOutput"
    />
  </div>
</template>

<style scoped>
.playback-output {
  min-width: 0;
  max-width: 100%;
}
.output-button {
  max-width: 100%;
}
.output-button > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.output-button > svg {
  flex-shrink: 0;
}
</style>
