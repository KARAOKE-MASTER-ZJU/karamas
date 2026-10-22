import { acceptHMRUpdate, defineStore } from "pinia";
import { reactive, ref } from "vue";
import type { ConnectionConfig, Preferences, Theme } from "@/types";

// 偏好设置的存储键
const storageKey = "ktv-casting.preferences.v1";

// 偏好设置存储
export const usePreferencesStore = defineStore("preferences", () => {
  // 初始化偏好设置
  const preferences = reactive<Preferences>({
    serverUrl: "",
    roomId: "",
    theme: "system",
  });
  // 初始化存储警告状态
  const storageWarning = ref(false);

  try {
    // 尝试从 localStorage 中加载偏好设置
    const stored: unknown = JSON.parse(
      localStorage.getItem(storageKey) ?? "null",
    );
    // 如果存储的偏好设置是对象类型，则将其应用到当前偏好设置中
    if (stored && typeof stored === "object") {
      if ("serverUrl" in stored && typeof stored.serverUrl === "string")
        preferences.serverUrl = stored.serverUrl;
      if ("roomId" in stored && typeof stored.roomId === "string")
        preferences.roomId = stored.roomId;
      if (
        "theme" in stored &&
        (stored.theme === "system" ||
          stored.theme === "light" ||
          stored.theme === "dark")
      ) {
        preferences.theme = stored.theme;
      }
    }
  } catch {
    // 如果从 localStorage 中加载偏好设置失败，则设置存储警告状态为 true
    storageWarning.value = true;
  }

  // 保存偏好设置
  function save(patch: Partial<Preferences>): boolean {
    const next = { ...preferences, ...patch };
    try {
      localStorage.setItem(storageKey, JSON.stringify(next));
      Object.assign(preferences, next);
      return true;
    } catch {
      return false;
    }
  }

  // 保存连接配置
  function saveConnection({ serverUrl, roomId }: ConnectionConfig): boolean {
    return save({ serverUrl, roomId });
  }

  // 保存主题
  function saveTheme(theme: Theme): boolean {
    return save({ theme });
  }

  return { preferences, storageWarning, saveConnection, saveTheme };
});

// 热更新
if (import.meta.hot) {
  import.meta.hot.accept(acceptHMRUpdate(usePreferencesStore, import.meta.hot));
}
