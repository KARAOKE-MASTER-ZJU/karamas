import {
  computed,
  onActivated,
  onDeactivated,
  onScopeDispose,
  ref,
  watch,
} from "vue";
import type { LogEntry, LogLevel } from "@/types";
import { castingApi, errorMessage, isTauri } from "@/services/casting";
import { useOperationNotice } from "@/composables/useOperationNotice";

const PAGE_SIZE = 200;

// 查看状态只控制读取范围，不持有日志留存规则。
export function useLogViewer() {
  const { notify } = useOperationNotice();
  const entries = ref<LogEntry[]>([]);
  const filter = ref<LogLevel>("info");
  const query = ref("");
  const total = ref(0);
  const retained = ref(0);
  const loading = ref(false);
  const clearing = ref(false);
  const limit = ref(PAGE_SIZE);
  const hasMore = computed(() => entries.value.length < total.value);
  let active = false;
  let generation = 0;
  let request = 0;
  let unlisten: (() => void) | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let lastReadError = "";

  async function refresh() {
    if (!active || !isTauri()) return;
    const currentRequest = ++request;
    loading.value = true;
    try {
      const snapshot = await castingApi.getLogs(
        filter.value,
        query.value,
        limit.value,
      );
      if (!active || currentRequest !== request) return;
      entries.value = snapshot.entries;
      total.value = snapshot.total;
      retained.value = snapshot.retained;
      lastReadError = "";
    } catch (error) {
      if (active && currentRequest === request) {
        const message = errorMessage(error);
        if (message !== lastReadError)
          notify(`读取日志失败：${message}`, "error");
        lastReadError = message;
      }
    } finally {
      if (currentRequest === request) loading.value = false;
    }
  }

  function scheduleRefresh() {
    if (!active || timer !== undefined) return;
    // 合并同一批日志通知，避免每条记录都发起查询。
    timer = setTimeout(() => {
      timer = undefined;
      void refresh();
    }, 100);
  }

  async function activate() {
    active = true;
    const currentGeneration = ++generation;
    if (!isTauri()) return;
    try {
      const stop = await castingApi.listenLogs(scheduleRefresh);
      if (!active || currentGeneration !== generation) {
        stop();
        return;
      }
      unlisten = stop;
    } catch (error) {
      if (!active || currentGeneration !== generation) return;
      notify(`订阅日志失败：${errorMessage(error)}`, "error");
    }
    // 先订阅再读取，读取期间产生的新记录会触发后续刷新。
    if (active && currentGeneration === generation) await refresh();
  }

  function deactivate() {
    active = false;
    ++generation;
    ++request;
    clearTimeout(timer);
    timer = undefined;
    unlisten?.();
    unlisten = undefined;
    loading.value = false;
  }

  async function clear() {
    if (clearing.value) return;
    clearing.value = true;
    try {
      await castingApi.clearLogs();
      await refresh();
    } catch (error) {
      notify(`清空日志失败：${errorMessage(error)}`, "error");
    } finally {
      clearing.value = false;
    }
  }

  function loadMore() {
    if (loading.value || !hasMore.value) return;
    limit.value += PAGE_SIZE;
    void refresh();
  }

  async function readAll() {
    const snapshot = await castingApi.getLogs(filter.value, query.value);
    return snapshot.entries;
  }

  watch([filter, query], () => {
    limit.value = PAGE_SIZE;
    void refresh();
  });
  onActivated(() => void activate());
  onDeactivated(deactivate);
  onScopeDispose(deactivate);

  return {
    entries,
    filter,
    query,
    total,
    retained,
    loading,
    clearing,
    hasMore,
    clear,
    loadMore,
    readAll,
  };
}
