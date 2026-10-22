import { acceptHMRUpdate, defineStore } from "pinia";
import { ref } from "vue";
import type { LogEntry, LogLevel } from "@/types";

// 日志存储
export const useLogsStore = defineStore("logs", () => {
  // 初始化日志条目和下一个日志 ID
  const entries = ref<LogEntry[]>([]);
  const nextId = ref(0);

  // 添加日志条目
  function add(message: string, level: LogLevel = "info") {
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

  return { entries, nextId, add, clear };
});

// 热更新
if (import.meta.hot) {
  import.meta.hot.accept(acceptHMRUpdate(useLogsStore, import.meta.hot));
}
