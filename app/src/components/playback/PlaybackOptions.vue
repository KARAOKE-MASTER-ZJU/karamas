<script setup lang="ts">
import { computed, toRef, useId } from "vue";
import { ChevronDown, RefreshCw } from "@lucide/vue";
import type { CastMode } from "@/types";
import { useBilibiliQualities } from "@/composables/useBilibiliQualities";
import type {
  BilibiliMediaQuality,
  MediaFormat,
  PlaybackQuality,
} from "@/types/casting";
const props = defineProps<{
  mode: CastMode;
  songId: string | null;
  songUrl: string | null;
  dlnaProxyEnabled: boolean;
  dlnaHlsDirectEnabled: boolean;
  qualityQn: number | null;
  qualityFormat: MediaFormat | null;
  mediaQualities: BilibiliMediaQuality[];
  mediaQualitiesLoading: boolean;
  mediaQualitiesError: string;
  danmaku: boolean | null;
  disabled: boolean;
}>();
const emit = defineEmits<{
  quality: [selection: PlaybackQuality];
  refreshMediaQualities: [];
  danmaku: [enabled: boolean];
}>();
const qualityId = useId();
const danmakuId = useId();
const {
  qualities,
  loading: bilibiliLoading,
  error: bilibiliError,
  refresh: refreshBilibili,
} = useBilibiliQualities(
  toRef(props, "mode"),
  toRef(props, "songId"),
  toRef(props, "songUrl"),
);
interface QualityOption extends PlaybackQuality {
  id: string;
  label: string;
}
const canMux = computed(
  () =>
    props.mode === "local" ||
    props.dlnaProxyEnabled ||
    props.dlnaHlsDirectEnabled,
);
const options = computed<QualityOption[]>(() => {
  if (props.mode === "bilibili")
    return qualities.value.map((quality) => ({
      ...quality,
      format: null,
      id: `bilibili:${quality.qn}`,
    }));
  const result: QualityOption[] = [];
  for (const quality of [...props.mediaQualities].sort((a, b) => a.qn - b.qn)) {
    const label = quality.label.replace(/\s+/g, "");
    if (quality.directMp4 === true)
      result.push({
        qn: quality.qn,
        format: "mp4",
        id: `mp4:${quality.qn}`,
        label,
      });
    if (quality.dash && canMux.value)
      result.push({
        qn: quality.qn,
        format: "dash",
        id: `dash:${quality.qn}`,
        label: `*${label}`,
      });
  }
  // YouTube 继续使用已有的双轨取源能力。
  if (
    !result.length &&
    canMux.value &&
    /^https?:\/\/(?:www\.|m\.)?(?:youtube\.com|youtu\.be)\//.test(
      props.songUrl ?? "",
    )
  )
    return [
      { qn: 64, format: "dash", id: "dash:64", label: "*720P准高清" },
      { qn: 80, format: "dash", id: "dash:80", label: "*1080P高清" },
    ];
  return result;
});
const loading = computed(() =>
  props.mode === "bilibili"
    ? bilibiliLoading.value
    : props.mediaQualitiesLoading,
);
const error = computed(() =>
  props.mode === "bilibili" ? bilibiliError.value : props.mediaQualitiesError,
);
const selectedQuality = computed(
  () =>
    options.value.find(
      ({ qn, format }) =>
        qn === props.qualityQn && format === props.qualityFormat,
    )?.id ?? "",
);
const placeholder = computed(() => {
  if (loading.value) return "正在加载…";
  if (error.value) return "加载失败";
  if (!options.value.length) return "暂无清晰度";
  return props.qualityQn === null ? "—" : "请选择清晰度";
});
function setQuality(event: Event) {
  const option = options.value.find(
    ({ id }) => id === (event.target as HTMLSelectElement).value,
  );
  if (option) emit("quality", { qn: option.qn, format: option.format });
}
function refresh() {
  if (props.mode === "bilibili") void refreshBilibili();
  else emit("refreshMediaQualities");
}
</script>

<template>
  <div class="playback-options">
    <div class="option-row">
      <label :for="qualityId">清晰度</label>
      <div class="select-control quality-select">
        <select
          :id="qualityId"
          :value="selectedQuality"
          :disabled="
            disabled || qualityQn === null || loading || !options.length
          "
          @change="setQuality"
        >
          <option v-if="selectedQuality === ''" value="" disabled>
            {{ placeholder }}
          </option>
          <option v-for="option in options" :key="option.id" :value="option.id">
            {{ option.label }}
          </option>
        </select>
        <ChevronDown :size="16" aria-hidden="true" />
      </div>
    </div>
    <div v-if="error" class="quality-error">
      <p class="field-error" role="alert">{{ error }}</p>
      <button
        type="button"
        class="button text-button"
        :disabled="disabled || loading"
        @click="refresh"
      >
        <RefreshCw :size="14" aria-hidden="true" />重试
      </button>
    </div>
    <div v-if="mode === 'bilibili'" class="option-row">
      <span :id="danmakuId">弹幕</span>
      <button
        type="button"
        class="danmaku-switch"
        role="switch"
        :aria-labelledby="danmakuId"
        :aria-checked="danmaku ?? false"
        :disabled="disabled || danmaku === null"
        @click="emit('danmaku', !danmaku)"
      >
        <span />
      </button>
    </div>
  </div>
</template>

<style scoped>
.playback-options {
  margin-top: 24px;
  border-top: 1px solid var(--line);
}
.option-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-top: 16px;
  font-size: 14px;
}
.quality-select {
  width: 168px;
}
.quality-error {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}
.quality-error p {
  flex: 1;
  margin: 0;
}
.danmaku-switch {
  width: 38px;
  height: 22px;
  border: 0;
  border-radius: 11px;
  padding: 3px;
  background: var(--input-line);
}
.danmaku-switch[aria-checked="true"] {
  background: var(--accent);
}
.danmaku-switch span {
  display: block;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #fff;
}
.danmaku-switch[aria-checked="true"] span {
  transform: translateX(16px);
}
</style>
