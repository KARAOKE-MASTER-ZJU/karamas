import { acceptHMRUpdate, defineStore } from "pinia";
import { reactive, ref } from "vue";
import type { ConnectionConfig, LogLevel, Preferences, Theme } from "@/types";

import type { PlaybackQuality } from "@/types/casting";

import { isLogLevel } from "@/utils/logs";

// 偏好设置的存储键
const storageKey = "ktv-casting.preferences.v1";

// 偏好设置存储
export const usePreferencesStore = defineStore("preferences", () => {
  // 初始化偏好设置
  const preferences = reactive<Preferences>({
    serverUrl: "",
    roomId: "",
    appId: "",
    remoteControlEnabled: false,
    theme: "system",
    logLevel: "info",
    dlnaProxyEnabled: true,
    dlnaHlsDirectEnabled: false,
    proxyDeviceOnly: true,
    bilibiliBuvid: "",
    mediaQuality: { qn: 64, format: "mp4" },
    bilibiliQuality: { qn: 80, format: null },
  });
  // 初始化存储警告状态
  const storageWarning = ref(false);

  function reload() {
    try {
      // 尝试从 localStorage 中加载偏好设置
      const stored: unknown = JSON.parse(
        localStorage.getItem(storageKey) ?? "null",
      );
      // 如果存储的偏好设置是对象类型，则将其应用到当前偏好设置中
      if (stored && typeof stored === "object") {
        for (const key of ["mediaQuality", "bilibiliQuality"] as const) {
          const value =
            key in stored ? (stored as Record<string, unknown>)[key] : null;
          if (!value || typeof value !== "object") continue;
          const { qn, format } = value as Record<string, unknown>;
          if (
            typeof qn === "number" &&
            [6, 16, 32, 64, 74, 80, 112, 116, 120, 125, 126, 127].includes(
              qn,
            ) &&
            (key === "bilibiliQuality"
              ? format === null
              : format === "mp4" || format === "dash" || format === "hls")
          )
            preferences[key] = { qn, format } as PlaybackQuality;
        }
        if (
          "bilibiliBuvid" in stored &&
          typeof stored.bilibiliBuvid === "string"
        )
          preferences.bilibiliBuvid = stored.bilibiliBuvid;
        if (
          "appId" in stored &&
          typeof stored.appId === "string" &&
          stored.appId
        )
          preferences.appId = stored.appId;
        if ("serverUrl" in stored && typeof stored.serverUrl === "string")
          preferences.serverUrl = stored.serverUrl;
        if ("roomId" in stored && typeof stored.roomId === "string")
          preferences.roomId = stored.roomId;
        if ("logLevel" in stored && isLogLevel(stored.logLevel))
          preferences.logLevel = stored.logLevel;
        if (
          "dlnaProxyEnabled" in stored &&
          typeof stored.dlnaProxyEnabled === "boolean"
        )
          preferences.dlnaProxyEnabled = stored.dlnaProxyEnabled;
        if (
          "dlnaHlsDirectEnabled" in stored &&
          typeof stored.dlnaHlsDirectEnabled === "boolean"
        )
          preferences.dlnaHlsDirectEnabled = stored.dlnaHlsDirectEnabled;
        if (
          "proxyDeviceOnly" in stored &&
          typeof stored.proxyDeviceOnly === "boolean"
        )
          preferences.proxyDeviceOnly = stored.proxyDeviceOnly;
        if (
          "remoteControlEnabled" in stored &&
          typeof stored.remoteControlEnabled === "boolean"
        )
          preferences.remoteControlEnabled = stored.remoteControlEnabled;
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
  }
  reload();

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

  function ensureBilibiliBuvid(): string {
    if (preferences.bilibiliBuvid) return preferences.bilibiliBuvid;
    const bilibiliBuvid = crypto.randomUUID().replaceAll("-", "");
    if (!save({ bilibiliBuvid }))
      throw new Error("无法保存哔哩哔哩设备标识，请检查本地存储。");
    return bilibiliBuvid;
  }

  // 遥控注册标识；安装后保持不变，网页按它记忆选中的设备。
  function ensureAppId(): string {
    if (preferences.appId) return preferences.appId;
    const appId = crypto.randomUUID();
    if (!save({ appId }))
      throw new Error("无法保存遥控设备标识，请检查本地存储。");
    return appId;
  }

  // 保存连接配置
  function saveConnection({ serverUrl, roomId }: ConnectionConfig): boolean {
    return save({ serverUrl, roomId });
  }

  // 保存主题
  function saveTheme(theme: Theme): boolean {
    return save({ theme });
  }

  function saveLogLevel(logLevel: LogLevel): boolean {
    return save({ logLevel });
  }

  function saveDlnaProxyEnabled(dlnaProxyEnabled: boolean): boolean {
    return save({ dlnaProxyEnabled });
  }

  function saveDlnaHlsDirectEnabled(dlnaHlsDirectEnabled: boolean): boolean {
    return save({ dlnaHlsDirectEnabled });
  }

  function saveRemoteControlEnabled(remoteControlEnabled: boolean): boolean {
    return save({ remoteControlEnabled });
  }

  function saveProxyDeviceOnly(proxyDeviceOnly: boolean): boolean {
    return save({ proxyDeviceOnly });
  }

  function savePlaybackQuality(selection: PlaybackQuality): boolean {
    const key = selection.format === null ? "bilibiliQuality" : "mediaQuality";
    return save({ [key]: { ...selection } });
  }

  return {
    reload,
    savePlaybackQuality,
    preferences,
    storageWarning,
    ensureBilibiliBuvid,
    ensureAppId,
    saveConnection,
    saveTheme,
    saveLogLevel,
    saveDlnaProxyEnabled,
    saveDlnaHlsDirectEnabled,
    saveProxyDeviceOnly,
    saveRemoteControlEnabled,
  };
});

// 热更新
if (import.meta.hot) {
  import.meta.hot.accept(acceptHMRUpdate(usePreferencesStore, import.meta.hot));
}
