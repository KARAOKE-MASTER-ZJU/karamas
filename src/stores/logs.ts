import { acceptHMRUpdate, defineStore } from "pinia";
import { ref } from "vue";
import type { LogEntry, LogLevel } from "@/types";
import { usePreferencesStore } from "@/stores/preferences";
import { castingApi, isTauri } from "@/services/casting";

import { meetsLogLevel } from "@/utils/logs";

// 日志存储
export const useLogsStore = defineStore("logs", () => {
  const preferences = usePreferencesStore();
  const recordingLevel = ref<LogLevel>(preferences.preferences.logLevel);
  // 初始化日志条目和下一个日志 ID
  const entries = ref<LogEntry[]>([]);
  const nextId = ref(0);

  // 添加日志条目
  function add(message: string, level: LogLevel = "info") {
    if (!meetsLogLevel(level, recordingLevel.value)) return;
    entries.value.push({
      id: ++nextId.value,
      time: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
      level,
      message,
    });
    if (entries.value.length > 200) entries.value.shift();
  }

  // 清空日志条目
  function clear() {
    entries.value = [];
  }

  // 只控制后续记录，已有日志由页面单独筛选。
  async function setRecordingLevel(level: LogLevel): Promise<boolean> {
    if (isTauri()) await castingApi.setLogLevel(level);
    recordingLevel.value = level;
    return preferences.saveLogLevel(level);
  }

  return { entries, nextId, recordingLevel, setRecordingLevel, add, clear };
});

// 热更新
if (import.meta.hot) {
  import.meta.hot.accept(acceptHMRUpdate(useLogsStore, import.meta.hot));
}
