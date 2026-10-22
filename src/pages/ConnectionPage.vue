<!-- ConnectionPage.vue: 连接页面组件 -->

<script setup lang="ts">
import { Cast, Tv, ChevronDown, RefreshCw, ScanQrCode } from "@lucide/vue";
import { reactive, ref } from "vue";
import type { CastMode, RoomEntryMode } from "@/types";
import { usePreferencesStore } from "@/stores/preferences";
import { useOperationNotice } from "@/composables/useOperationNotice";

const store = usePreferencesStore();
const { notify } = useOperationNotice();
const form = reactive({
  serverUrl: store.preferences.serverUrl,
  roomId: store.preferences.roomId,
});
const mode = ref<CastMode>("dlna");
const errors = reactive({ serverUrl: "", roomId: "", deviceUrl: "" });
const deviceUrl = ref("");
const showManualConnect = ref(false);

// 处理房间创建或加入的逻辑，包括输入验证和配置保存
function enterRoom(entryMode: RoomEntryMode) {
  errors.serverUrl = "";
  errors.roomId = "";
  let server: URL | undefined;
  try {
    server = new URL(form.serverUrl.trim());
    if (
      !["http:", "https:"].includes(server.protocol) ||
      server.username ||
      server.password ||
      server.search ||
      server.hash
    ) {
      throw new Error();
    }
  } catch {
    errors.serverUrl = "请输入有效的 HTTP 或 HTTPS 服务器网址。";
  }
  if (!form.roomId.trim()) errors.roomId = "请输入房间号。";
  if (errors.serverUrl || errors.roomId || !server) return;
  form.serverUrl = server.toString().replace(/\/$/, "");
  form.roomId = form.roomId.trim();
  if (!store.saveConnection({ ...form })) {
    notify("无法保存配置，请检查本机存储是否可用。", "error");
    return;
  }
  unavailable(entryMode === "create" ? "创建房间" : "加入房间");
}

// 显示功能尚未接入的提示消息
function unavailable(feature: string) {
  notify(`${feature}尚未接入。`, "warning");
}

// 处理手动输入设备地址的连接逻辑，包括输入验证
function connectByUrl() {
  errors.deviceUrl = "";
  try {
    const address = new URL(deviceUrl.value.trim());
    if (
      !["http:", "https:"].includes(address.protocol) ||
      address.username ||
      address.password
    )
      throw new Error();
  } catch {
    errors.deviceUrl = "请输入设备描述文件的完整 HTTP 或 HTTPS 地址。";
    return;
  }
  unavailable("手动连接");
}
</script>

<template>
  <div class="connection-page">
    <!-- 点歌房间创建/加入表单部分 -->
    <section class="content-section" aria-labelledby="room-title">
      <h2 id="room-title">点歌房间</h2>
      <form class="room-form" novalidate @submit.prevent="enterRoom('create')">
        <!-- 服务器网址输入字段 -->
        <div class="field">
          <label for="server-url">服务器网址</label>
          <input
            id="server-url"
            v-model="form.serverUrl"
            type="url"
            inputmode="url"
            placeholder="https://你的点歌台网址"
            autocomplete="url"
            :aria-invalid="!!errors.serverUrl"
            :aria-describedby="errors.serverUrl ? 'server-error' : undefined"
          />
          <p v-if="errors.serverUrl" id="server-error" class="field-error">
            {{ errors.serverUrl }}
          </p>
        </div>
        <!-- 房间号输入字段 -->
        <div class="field">
          <label for="room-id">房间号</label>
          <input
            id="room-id"
            v-model="form.roomId"
            placeholder="输入房间号"
            autocomplete="off"
            :aria-invalid="!!errors.roomId"
            :aria-describedby="errors.roomId ? 'room-error' : undefined"
          />
          <p v-if="errors.roomId" id="room-error" class="field-error">
            {{ errors.roomId }}
          </p>
        </div>
        <!-- 创建和加入房间的操作按钮 -->
        <div class="room-actions">
          <button class="button primary" type="submit" aria-label="创建并加入">
            <span class="room-action-label">创建并加入</span>
            <span class="room-action-compact" aria-hidden="true">创建</span>
          </button>
          <button
            class="button secondary"
            type="button"
            aria-label="加入房间"
            @click="enterRoom('join')"
          >
            <span class="room-action-label">加入房间</span>
            <span class="room-action-compact" aria-hidden="true">加入</span>
          </button>
        </div>
      </form>
    </section>

    <!-- 投屏设备选择和手动连接部分 -->
    <section
      class="content-section devices-section"
      aria-labelledby="device-title"
    >
      <!-- 投屏方式选择 -->
      <fieldset class="mode-options">
        <legend class="sr-only">投屏方式</legend>
        <label :class="['mode-option', { selected: mode === 'dlna' }]">
          <input v-model="mode" type="radio" name="cast-mode" value="dlna" />
          <Cast :size="18" /><span>DLNA</span>
        </label>
        <label :class="['mode-option', { selected: mode === 'bilibili' }]">
          <input
            v-model="mode"
            type="radio"
            name="cast-mode"
            value="bilibili"
          />
          <Tv :size="18" /><span>小电视</span>
        </label>
      </fieldset>

      <!-- 投屏设备列表和手动连接表单 -->
      <div class="section-toolbar">
        <h2 id="device-title">
          {{ mode === "dlna" ? "可用设备" : "小电视设备" }}
        </h2>
        <div class="device-actions">
          <button
            class="button text-button"
            @click="unavailable(mode === 'dlna' ? '设备搜索' : '扫码登录')"
          >
            <component
              :is="mode === 'dlna' ? RefreshCw : ScanQrCode"
              :size="17"
            />{{ mode === "dlna" ? "搜索设备" : "扫码登录" }}
          </button>
          <button
            v-if="mode === 'dlna'"
            class="button text-button manual-toggle"
            type="button"
            aria-label="手动输入设备地址"
            :aria-expanded="showManualConnect"
            aria-controls="manual-connect-form"
            @click="showManualConnect = !showManualConnect"
          >
            手动输入<ChevronDown :size="16" />
          </button>
        </div>
      </div>

      <!-- 手动连接表单 -->
      <form
        v-if="mode === 'dlna'"
        v-show="showManualConnect"
        id="manual-connect-form"
        class="manual-form"
        novalidate
        @submit.prevent="connectByUrl"
      >
        <div class="field">
          <label for="device-url">设备描述文件地址</label>
          <input
            id="device-url"
            v-model="deviceUrl"
            type="url"
            inputmode="url"
            placeholder="http://设备 IP:端口/description.xml"
            :aria-invalid="!!errors.deviceUrl"
            :aria-describedby="errors.deviceUrl ? 'device-error' : undefined"
          />
          <p v-if="errors.deviceUrl" id="device-error" class="field-error">
            {{ errors.deviceUrl }}
          </p>
        </div>
        <button type="submit" class="button secondary">连接</button>
      </form>

      <!-- 显示设备为空的提示信息，根据投屏方式显示不同的图标和文本 -->
      <div class="device-empty">
        <component :is="mode === 'dlna' ? Cast : Tv" :size="32" />
        <p>{{ mode === "dlna" ? "暂无设备" : "尚未登录哔哩哔哩" }}</p>
        <span v-if="mode === 'dlna'">将电视与本机连接到同一网络后搜索</span>
      </div>
    </section>

    <p class="page-note">房间创建、加入和投屏功能尚未接入。</p>
  </div>
</template>

<style scoped>
.room-form {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: start;
  gap: 18px 8px;
}
.room-form > .field:first-child {
  grid-column: 1 / -1;
}
.room-actions {
  display: flex;
  flex-wrap: nowrap;
  justify-content: flex-end;
  gap: 6px;
  margin-top: calc(var(--control-line-height) + 8px);
}
.room-action-compact {
  display: none;
}
.mode-options {
  display: flex;
  gap: 4px;
  background: var(--subtle);
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 4px;
  margin: 0 0 24px;
  width: 100%;
}
.mode-option {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  flex: 1;
  position: relative;
  min-height: 36px;
  padding: 6px 12px;
  font-size: 14px;
  border-radius: 5px;
  color: var(--muted);
  cursor: pointer;
}
.mode-option input {
  position: absolute;
  opacity: 0;
  width: 1px;
  height: 1px;
}
.mode-option:has(input:focus-visible) {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.mode-option.selected {
  background: var(--surface);
  color: var(--text);
  box-shadow: 0 1px 3px #00000014;
}
.section-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}
.device-empty {
  min-height: 144px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  text-align: center;
  color: var(--muted);
  padding: 24px 8px;
}
.device-empty > svg {
  opacity: 0.65;
  margin-bottom: 4px;
}
.device-empty p {
  font-size: 14px;
}
.device-empty span {
  font-size: 13px;
  line-height: 1.6;
}
.device-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}
.manual-toggle[aria-expanded="true"] > svg {
  transform: rotate(180deg);
}
.manual-form {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  margin-top: 20px;
}
.manual-form .field {
  flex: 1;
}
.manual-form .button {
  margin-top: calc(var(--control-line-height) + 8px);
}
.page-note {
  font-size: 12px;
  line-height: 1.7;
  color: var(--muted);
}
.page-note {
  padding-top: 16px;
}
@media (min-width: 900px) {
  .room-form {
    grid-template-columns: minmax(0, 1fr) 140px auto;
    align-items: start;
    gap: 12px;
  }
  .room-form > .field:first-child {
    grid-column: auto;
  }
  .room-actions {
    gap: 8px;
  }
}
@media (max-width: 899px) {
  .room-form .button {
    padding: 0 8px;
    font-size: 13px;
  }
  .room-form .field > input {
    padding: 0 8px;
  }
  .room-action-label {
    display: none;
  }
  .room-action-compact {
    display: inline;
  }
}
</style>
