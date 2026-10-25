import { onScopeDispose, ref, watch, type Ref } from "vue";
import { castingApi, errorMessage } from "@/services/casting";
import { useCastingStore } from "@/stores/casting";
import type { CurrentBilibiliParts } from "@/types/casting";

// 获取当前歌曲的分 P 信息，并提供刷新和选择分 P 的方法
export function useBilibiliParts(songId: Ref<string>) {
  const casting = useCastingStore();
  const parts = ref<CurrentBilibiliParts | null>(null);
  const loading = ref(false);
  const error = ref("");
  const showDialog = ref(false);
  let revision = 0;

  // 刷新当前歌曲的分 P 信息
  async function refresh() {
    const currentRevision = ++revision;
    const id = songId.value;
    loading.value = true;
    error.value = "";
    try {
      const result = await castingApi.getBilibiliParts();
      if (currentRevision !== revision) return;
      if (result.songId !== id) throw new Error("当前歌曲已切换，请刷新分 P");
      parts.value = result;
    } catch (cause) {
      if (currentRevision === revision) {
        parts.value = null;
        error.value = errorMessage(cause);
      }
    } finally {
      if (currentRevision === revision) loading.value = false;
    }
  }

  // 选择指定的分 P
  async function select(pageIndex: number) {
    const current = parts.value;
    if (!current || loading.value || casting.busy) return;
    const currentRevision = revision;
    error.value = "";
    try {
      await casting.control(async () => {
        await castingApi.selectBilibiliPart(current.songId, pageIndex);
        if (currentRevision !== revision) return;
        parts.value = { ...current, currentPageIndex: pageIndex };
        showDialog.value = false;
      });
    } catch (cause) {
      if (currentRevision === revision) error.value = errorMessage(cause);
    }
  }
  watch(
    songId,
    () => {
      parts.value = null;
      showDialog.value = false;
      void refresh();
    },
    { immediate: true },
  );
  onScopeDispose(() => {
    ++revision;
  });
  return { parts, loading, error, showDialog, refresh, select };
}
