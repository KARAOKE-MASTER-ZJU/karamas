<!-- 设置页面 -->

<script setup lang="ts">
import { ChevronRight, ChevronDown } from "@lucide/vue";
import { RouterLink } from "vue-router";
import { storeToRefs } from "pinia";
import { usePreferencesStore } from "@/stores/preferences";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { useBilibiliStore } from "@/stores/bilibili";
import { useCastingStore } from "@/stores/casting";
import { errorMessage } from "@/services/casting";
import { ref } from "vue";
import type { Theme } from "@/types";
import { version } from "../../package.json";
const store = usePreferencesStore();
const auth = useBilibiliStore();
const casting = useCastingStore();
const loggingOut = ref(false);
const { preferences } = storeToRefs(store);
const { notify } = useOperationNotice();
const themes: { value: Theme; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
];

function updateTheme(theme: Theme) {
  if (!store.saveTheme(theme))
    notify("无法保存主题设置，请检查本机存储是否可用。", "error");
}
async function logout() {
  loggingOut.value = true;
  try {
    if (casting.activeDevice?.kind === "bilibili") await casting.disconnect();
    await auth.logout();
  } catch (error) {
    notify(errorMessage(error), "error");
  } finally {
    loggingOut.value = false;
  }
}
</script>

<template>
  <div class="settings-page">
    <section class="content-section" aria-labelledby="general-title">
      <h2 id="general-title">通用</h2>
      <div class="settings-row">
        <label for="theme">外观</label>
        <div class="select-control theme-select">
          <select
            id="theme"
            :value="preferences.theme"
            @change="
              updateTheme(($event.target as HTMLSelectElement).value as Theme)
            "
          >
            <option
              v-for="option in themes"
              :key="option.value"
              :value="option.value"
            >
              {{ option.label }}
            </option>
          </select>
          <ChevronDown :size="16" aria-hidden="true" />
        </div>
      </div>
      <RouterLink class="settings-row" :to="{ name: 'logs' }">
        <span>运行日志</span><ChevronRight :size="18" />
      </RouterLink>
    </section>
    <section class="content-section" aria-labelledby="account-title">
      <h2 id="account-title">账号</h2>
      <div class="settings-row">
        <span
          >哔哩哔哩<span v-if="auth.session" class="muted-text">
            · UID {{ auth.session.mid }}</span
          ></span
        >
        <button
          v-if="auth.session"
          class="button text-button"
          :disabled="casting.busy || loggingOut || auth.loggingIn"
          @click="logout"
        >
          退出登录
        </button>
        <button
          v-else
          class="button text-button"
          :disabled="auth.loggingIn"
          @click="auth.login"
        >
          扫码登录
        </button>
      </div>
    </section>
    <section class="content-section" aria-labelledby="about-title">
      <h2 id="about-title">关于</h2>
      <div class="about-row">
        <img src="/app-icon.svg" alt="" width="44" height="44" />
        <div>
          <strong>KTV Casting</strong>
          <p>版本 {{ version }}</p>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.settings-page .content-section > h2 {
  margin-bottom: 8px;
  color: var(--muted);
  font-size: 13px;
  font-weight: 500;
}
.settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  min-height: 62px;
  padding: 12px 0;
  font-size: 14px;
  background: none;
  border: 0;
  width: 100%;
  text-align: left;
  color: inherit;
  text-decoration: none;
}
.theme-select {
  width: 140px;
}
.about-row {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 0;
  font-size: 14px;
}
.about-row strong {
  font-weight: 500;
}
.about-row p {
  font-size: 13px;
  color: var(--muted);
  margin-top: 6px;
}
.settings-row[href]:hover {
  color: var(--accent);
}
</style>
