<!-- 设置页面 -->

<script setup lang="ts">
import { ChevronRight, ChevronDown, Info, ScanQrCode } from "@lucide/vue";
import { RouterLink } from "vue-router";
import { storeToRefs } from "pinia";
import { usePreferencesStore } from "@/stores/preferences";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { useBilibiliStore } from "@/stores/bilibili";
import { useCastingStore } from "@/stores/casting";
import { errorMessage } from "@/services/casting";
import { onDeactivated, onMounted, onScopeDispose, ref } from "vue";
import type { LogLevel, Theme } from "@/types";
import { useLogsStore } from "@/stores/logs";
import { logLevelLabels } from "@/utils/logs";
import { version } from "../../package.json";
const store = usePreferencesStore();
const auth = useBilibiliStore();
const casting = useCastingStore();
const loggingOut = ref(false);
const logs = useLogsStore();
const changingLogLevel = ref(false);
const showProxyInfo = ref(false);
const proxyInfo = ref<HTMLElement | null>(null);
const proxyInfoButton = ref<HTMLButtonElement | null>(null);
const { preferences } = storeToRefs(store);
const { notify } = useOperationNotice();
const themes: { value: Theme; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
];

function closeProxyInfoOutside(event: PointerEvent) {
  if (event.target instanceof Node && !proxyInfo.value?.contains(event.target))
    showProxyInfo.value = false;
}
function closeProxyInfoOnEscape(event: KeyboardEvent) {
  if (event.key === "Escape" && showProxyInfo.value) {
    showProxyInfo.value = false;
    proxyInfoButton.value?.focus();
  }
}
onMounted(() => {
  document.addEventListener("pointerdown", closeProxyInfoOutside);
  document.addEventListener("keydown", closeProxyInfoOnEscape);
});
onDeactivated(() => {
  showProxyInfo.value = false;
});
onScopeDispose(() => {
  document.removeEventListener("pointerdown", closeProxyInfoOutside);
  document.removeEventListener("keydown", closeProxyInfoOnEscape);
});

function updateTheme(theme: Theme) {
  if (!store.saveTheme(theme))
    notify("无法保存主题设置，请检查本机存储是否可用。", "error");
}
async function updateLogLevel(level: LogLevel) {
  changingLogLevel.value = true;
  try {
    if (!(await logs.setRecordingLevel(level)))
      notify("日志记录级别已生效，但无法保存本机配置。", "warning");
  } catch (error) {
    notify(errorMessage(error), "error");
  } finally {
    changingLogLevel.value = false;
  }
}
async function updateDlnaProxy() {
  try {
    if (
      (await casting.setDlnaProxyEnabled(!casting.dlnaProxyEnabled)) === false
    )
      notify("反向代理设置已生效，但无法保存本机配置。", "warning");
  } catch (error) {
    notify(`设置反向代理失败：${errorMessage(error)}`, "error");
  }
}
async function logout() {
  loggingOut.value = true;
  try {
    if (casting.selectedDevice?.kind === "bilibili") await casting.disconnect();
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
        <div class="select-control settings-select">
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
      <div class="settings-row">
        <label for="recording-level">日志记录级别</label>
        <div class="select-control settings-select">
          <select
            id="recording-level"
            :value="logs.recordingLevel"
            :disabled="changingLogLevel"
            @change="
              updateLogLevel(
                ($event.target as HTMLSelectElement).value as LogLevel,
              )
            "
          >
            <option
              v-for="(label, level) in logLevelLabels"
              :key="level"
              :value="level"
            >
              {{ label }}
            </option>
          </select>
          <ChevronDown :size="16" aria-hidden="true" />
        </div>
      </div>
      <RouterLink class="settings-row" :to="{ name: 'logs' }">
        <span>运行日志</span><ChevronRight :size="18" />
      </RouterLink>
    </section>
    <section class="content-section" aria-labelledby="dlna-title">
      <h2 id="dlna-title">DLNA 投屏</h2>
      <div class="settings-row">
        <div ref="proxyInfo" class="setting-label">
          <span id="dlna-proxy-label">反向代理</span>
          <button
            ref="proxyInfoButton"
            type="button"
            class="icon-button info-button"
            aria-label="反向代理说明"
            aria-controls="dlna-proxy-info"
            :aria-expanded="showProxyInfo"
            @click="showProxyInfo = !showProxyInfo"
          >
            <Info :size="15" aria-hidden="true" />
          </button>
          <div
            v-if="showProxyInfo"
            id="dlna-proxy-info"
            class="info-popover"
            role="note"
            aria-label="反向代理说明"
          >
            <p>
              开启后由本机代理媒体；关闭后设备直接访问媒体源。下次切歌或点击“重新投屏”生效。
            </p>
          </div>
        </div>
        <button
          type="button"
          class="proxy-switch"
          role="switch"
          aria-labelledby="dlna-proxy-label"
          :aria-checked="casting.dlnaProxyEnabled"
          :aria-busy="casting.changingDlnaProxy"
          :disabled="casting.busy"
          @click="updateDlnaProxy"
        >
          <span />
        </button>
      </div>
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
          <ScanQrCode :size="18" aria-hidden="true" />
          <span>扫码登录</span>
        </button>
      </div>
    </section>
    <section class="content-section" aria-labelledby="about-title">
      <h2 id="about-title">关于</h2>
      <div class="about-row">
        <img src="/app-icon.svg" alt="" width="44" height="44" />
        <div>
          <strong>KARAM@S</strong>
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
.settings-select {
  width: 140px;
}
.setting-label {
  position: relative;
  display: flex;
  align-items: center;
  gap: 4px;
}
.info-button {
  width: 28px;
  height: 28px;
  color: var(--muted);
}
.info-popover {
  position: absolute;
  top: calc(100% + 8px);
  left: 0;
  z-index: 10;
  width: min(300px, calc(100vw - 40px));
  padding: 14px 16px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--surface);
  box-shadow: 0 6px 24px #0002;
  color: var(--muted);
  font-size: 13px;
  line-height: 1.7;
}
.info-popover p + p {
  margin-top: 8px;
}
.proxy-switch {
  flex-shrink: 0;
  width: 38px;
  height: 22px;
  border: 0;
  border-radius: 11px;
  padding: 3px;
  background: var(--input-line);
}
.proxy-switch[aria-checked="true"] {
  background: var(--accent);
}
.proxy-switch span {
  display: block;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #fff;
}
.proxy-switch[aria-checked="true"] span {
  transform: translateX(16px);
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
