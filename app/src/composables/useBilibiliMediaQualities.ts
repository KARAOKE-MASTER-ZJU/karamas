import { ref, watch, type Ref } from "vue";
import { castingApi, errorMessage } from "@/services/casting";
import type { BilibiliMediaQuality } from "@/types/casting";

// 调用方传入 DLNA / 本机播放的当前视频 URL，其他模式传 null。
export function useBilibiliMediaQualities(songUrl: Ref<string | null>) {
  const qualities = ref<BilibiliMediaQuality[]>([]);
  const loading = ref(false);
  const error = ref("");
  let revision = 0;

  async function refresh() {
    const current = ++revision;
    qualities.value = [];
    error.value = "";
    loading.value = false;
    const url = songUrl.value;
    // 忽略 YouTube 和外部媒体；完整的视频和分 P 校验仍由核心库负责。
    if (
      !url ||
      !/^(BV[0-9A-Za-z]{10}(?:-page\d+)?$|bilibili:\/\/video\/BV|https?:\/\/(?:www\.|m\.)?bilibili\.com\/video\/BV)/.test(
        url,
      )
    )
      return;
    loading.value = true;
    try {
      const result = await castingApi.getBilibiliMediaQualities(url);
      if (current === revision) qualities.value = result;
    } catch (cause) {
      if (current === revision) error.value = errorMessage(cause);
    } finally {
      if (current === revision) loading.value = false;
    }
  }

  // URL 包含分 P；轮询播放进度、seek 和音量变化不会重复查询。
  watch(songUrl, () => void refresh(), { immediate: true, flush: "sync" });
  return { qualities, loading, error, refresh };
}
