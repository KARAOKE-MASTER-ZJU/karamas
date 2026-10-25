import { ref, watch, type Ref } from "vue";
import { castingApi, errorMessage } from "@/services/casting";
import type { CastMode } from "@/types";
import type { BilibiliQuality } from "@/types/casting";

export function useBilibiliQualities(
  mode: Ref<CastMode>,
  songId: Ref<string | null>,
  songUrl: Ref<string | null>,
) {
  const qualities = ref<BilibiliQuality[]>([]);
  const loading = ref(false);
  const error = ref("");

  async function refresh() {
    qualities.value = [];
    error.value = "";
    loading.value = false;
    const url = songUrl.value;
    if (mode.value !== "bilibili" || !songId.value || !url) return;
    const isCurrent = () => mode.value === "bilibili" && songUrl.value === url;
    loading.value = true;
    try {
      const result = await castingApi.getBilibiliQualities(url);
      if (isCurrent()) {
        qualities.value = result;
        error.value = "";
      }
    } catch (cause) {
      if (isCurrent() && !qualities.value.length)
        error.value = errorMessage(cause);
    } finally {
      if (isCurrent()) loading.value = false;
    }
  }

  // 分 P 修改保留歌曲 ID，通过 URL 变化触发刷新。
  watch([mode, songId, songUrl], () => void refresh(), {
    immediate: true,
    flush: "sync",
  });
  return { qualities, loading, error, refresh };
}
