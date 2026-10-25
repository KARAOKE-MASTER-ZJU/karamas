import { onScopeDispose, ref, watch, type Ref } from "vue";
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
  let revision = 0;

  async function refresh() {
    const currentRevision = ++revision;
    qualities.value = [];
    error.value = "";
    loading.value = false;
    const url = songUrl.value;
    if (mode.value !== "bilibili" || !songId.value || !url) return;
    loading.value = true;
    try {
      const result = await castingApi.getBilibiliQualities(url);
      if (currentRevision === revision) qualities.value = result;
    } catch (cause) {
      if (currentRevision === revision) error.value = errorMessage(cause);
    } finally {
      if (currentRevision === revision) loading.value = false;
    }
  }

  // 分 P 修改保留歌曲 ID，通过 URL 变化触发刷新。
  watch([mode, songId, songUrl], () => void refresh(), {
    immediate: true,
    flush: "sync",
  });
  onScopeDispose(() => {
    ++revision;
  });
  return { qualities, loading, error, refresh };
}
