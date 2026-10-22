// useTheme.ts: 提供一个组合式函数，用于在应用中管理主题设置和响应系统主题变化。
import { onMounted, onScopeDispose, watch } from "vue";
import { usePreferencesStore } from "@/stores/preferences";

export function useTheme() {
  const store = usePreferencesStore();
  let colorScheme: MediaQueryList | undefined;

  function applyTheme() {
    document.documentElement.dataset.theme =
      store.preferences.theme === "system"
        ? colorScheme?.matches
          ? "dark"
          : "light"
        : store.preferences.theme;
  }

  watch(() => store.preferences.theme, applyTheme);
  onMounted(() => {
    colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
    colorScheme.addEventListener("change", applyTheme);
    applyTheme();
  });
  onScopeDispose(() => colorScheme?.removeEventListener("change", applyTheme));
}
