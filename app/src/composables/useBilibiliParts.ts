import { ref, watch, type Ref } from "vue";
import { castingApi, errorMessage } from "@/services/casting";
import { useCastingStore } from "@/stores/casting";
import type { CurrentBilibiliParts } from "@/types/casting";

// 获取当前歌曲的分 P 信息，并提供刷新和选择分 P 的方法
export function useBilibiliParts(
  songId: Ref<string>,
  songUrl: Ref<string | null>,
) {
  const casting = useCastingStore();
  const parts = ref<CurrentBilibiliParts | null>(null);
  const loading = ref(false);
  const error = ref("");
  const showDialog = ref(false);

  // 刷新当前歌曲的分 P 信息
  async function refresh() {
    const url = songUrl.value;
    loading.value = false;
    error.value = "";
    if (!url) return;
    loading.value = true;
    try {
      const result = await castingApi.getBilibiliParts();
      if (songUrl.value !== url || result.songUrl !== url) return;
      parts.value = result;
      error.value = "";
    } catch (cause) {
      if (songUrl.value === url && !parts.value) {
        error.value = errorMessage(cause);
      }
    } finally {
      if (songUrl.value === url) loading.value = false;
    }
  }

  // 选择指定的分 P
  async function select(pageIndex: number) {
    const current = parts.value;
    const url = songUrl.value;
    if (!current || current.songUrl !== url || loading.value || casting.busy)
      return;
    const id = songId.value;
    error.value = "";
    try {
      await casting.control(async () => {
        await castingApi.selectBilibiliPart(id, pageIndex);
        if (songUrl.value !== url || songId.value !== id) return;
        parts.value = { ...current, currentPageIndex: pageIndex };
        showDialog.value = false;
      });
    } catch (cause) {
      if (songUrl.value === url && songId.value === id)
        error.value = errorMessage(cause);
    }
  }
  watch(
    [songId, songUrl],
    () => {
      parts.value = null;
      showDialog.value = false;
      void refresh();
    },
    { immediate: true, flush: "sync" },
  );
  return { parts, loading, error, showDialog, refresh, select };
}
