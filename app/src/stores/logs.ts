import { acceptHMRUpdate, defineStore } from "pinia";
import { ref } from "vue";
import type { LogLevel } from "@/types";
import { usePreferencesStore } from "@/stores/preferences";
import { castingApi, isTauri } from "@/services/casting";

// 应用日志记录配置，日志留存在 Rust 内存中。
export const useLogsStore = defineStore("logs", () => {
  const preferences = usePreferencesStore();
  const recordingLevel = ref<LogLevel>(preferences.preferences.logLevel);

  function record(message: string, level: LogLevel = "info") {
    return castingApi.writeLog(message, level);
  }

  async function setRecordingLevel(level: LogLevel): Promise<boolean> {
    if (isTauri()) await castingApi.setLogLevel(level);
    recordingLevel.value = level;
    return preferences.saveLogLevel(level);
  }

  return { recordingLevel, setRecordingLevel, record };
});

if (import.meta.hot) {
  import.meta.hot.accept(acceptHMRUpdate(useLogsStore, import.meta.hot));
}
