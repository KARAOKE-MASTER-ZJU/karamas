<!-- ConnectionPage.vue: 连接页面组件 -->

<script setup lang="ts">
import { Cast, Tv, ChevronDown, RefreshCw, ScanQrCode } from "@lucide/vue";
import { computed, reactive, ref, watch } from "vue";
import { useRouter } from "vue-router";
import type { CastMode, RoomEntryMode } from "@/types";
import type { CastDevice } from "@/types/casting";
import { usePreferencesStore } from "@/stores/preferences";
import { useCastingStore } from "@/stores/casting";
import { useBilibiliStore } from "@/stores/bilibili";
import { castingApi, errorMessage } from "@/services/casting";
import { useOperationNotice } from "@/composables/useOperationNotice";

const store = usePreferencesStore();
const casting = useCastingStore();
const auth = useBilibiliStore();
const router = useRouter();
const { notify } = useOperationNotice();
const form = reactive({
  serverUrl: store.preferences.serverUrl,
  roomId: store.preferences.roomId,
});
const mode = ref<CastMode>("dlna");
const errors = reactive({ serverUrl: "", roomId: "", deviceUrl: "" });
const deviceUrl = ref("");
const showManualConnect = ref(false);
const dlnaDevices = ref<CastDevice[]>([]);
const bilibiliDevices = ref<CastDevice[]>([]);
const searching = reactive({ dlna: false, bilibili: false });
const resolvingDevice = ref(false);
const devices = computed(() => {
  const found =
    mode.value === "dlna" ? dlnaDevices.value : bilibiliDevices.value;
  const active = casting.activeDevice;
  // 刷新搜索时仍保留当前设备，保证用户随时可以断开。
  return active?.kind === mode.value &&
    !found.some((device) => deviceKey(device) === deviceKey(active))
    ? [active, ...found]
    : found;
});

// 校验表单后等待服务器确认，再保存房间配置。
async function enterRoom(entryMode: RoomEntryMode) {
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
    )
      throw new Error();
  } catch {
    errors.serverUrl = "请输入有效的 HTTP 或 HTTPS 服务器网址。";
  }
  if (!form.roomId.trim()) errors.roomId = "请输入房间号。";
  if (errors.serverUrl || errors.roomId || !server) return;
  const config = {
    serverUrl: server.toString().replace(/\/$/, ""),
    roomId: form.roomId.trim(),
  };
  try {
    await casting.enterRoom(config, entryMode);
    Object.assign(form, config);
    if (store.saveConnection(config))
      notify(entryMode === "create" ? "房间已创建并加入" : "已加入房间");
    else notify("已加入房间，但无法保存本机配置。", "warning");
  } catch (error) {
    notify(errorMessage(error), "error");
  }
}
async function searchDevices(kind: CastMode = mode.value) {
  if (searching[kind]) return;
  const session = auth.session;
  if (kind === "bilibili" && !session) {
    void auth.login();
    return;
  }
  searching[kind] = true;
  try {
    if (kind === "dlna")
      dlnaDevices.value = (await castingApi.discoverDlnaDevices()).map(
        (device) => ({ ...device, kind: "dlna" }),
      );
    else if (session) {
      const result = await castingApi.listBilibiliDevices(session);
      if (session === auth.session)
        bilibiliDevices.value = result.map((device) => ({
          ...device,
          kind: "bilibili",
        }));
    }
  } catch (error) {
    notify(errorMessage(error), "error");
  } finally {
    searching[kind] = false;
    if (
      kind === "bilibili" &&
      mode.value === kind &&
      auth.session &&
      session !== auth.session
    )
      void searchDevices(kind);
  }
}
watch(
  () => auth.session,
  (session) => {
    bilibiliDevices.value = [];
    if (session && mode.value === "bilibili") void searchDevices("bilibili");
  },
);
watch(mode, (kind) => {
  if (kind === "bilibili" && auth.session) void searchDevices(kind);
});
function deviceKey(device: CastDevice) {
  return device.kind === "dlna" ? device.location : device.buvid;
}
function isActive(device: CastDevice) {
  return (
    casting.activeDevice?.kind === device.kind &&
    deviceKey(casting.activeDevice) === deviceKey(device)
  );
}
async function connect(device: CastDevice) {
  try {
    if (isActive(device)) {
      await casting.disconnect();
      notify("已断开设备");
    } else {
      await casting.connect(device);
      await router.push({ name: "playback" });
    }
  } catch (error) {
    notify(errorMessage(error), "error");
  }
}
// 设备地址先解析为真实设备，再走与搜索列表相同的连接流程。
async function connectByUrl() {
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
  resolvingDevice.value = true;
  try {
    const device: CastDevice = {
      ...(await castingApi.discoverDlnaDevice(deviceUrl.value.trim())),
      kind: "dlna",
    };
    if (
      !dlnaDevices.value.some((item) => deviceKey(item) === deviceKey(device))
    )
      dlnaDevices.value.push(device);
    await connect(device);
  } catch (error) {
    notify(errorMessage(error), "error");
  } finally {
    resolvingDevice.value = false;
  }
}
</script>

<template>
  <div class="connection-page">
    <!-- 点歌房间创建/加入表单部分 -->
    <section class="content-section" aria-labelledby="room-title">
      <h2 id="room-title">点歌房间</h2>
      <form
        class="room-form"
        :aria-busy="casting.enteringRoom"
        novalidate
        @submit.prevent="enterRoom('create')"
      >
        <!-- 服务器网址输入字段 -->
        <div class="field">
          <label for="server-url">服务器网址</label>
          <input
            id="server-url"
            v-model="form.serverUrl"
            :disabled="casting.enteringRoom || casting.connecting"
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
            :disabled="casting.enteringRoom || casting.connecting"
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
          <button
            class="button primary"
            type="submit"
            :disabled="casting.busy"
            aria-label="创建并加入"
          >
            <span class="room-action-label">创建并加入</span>
            <span class="room-action-compact" aria-hidden="true">创建</span>
          </button>
          <button
            class="button secondary"
            type="button"
            aria-label="加入房间"
            :disabled="casting.busy"
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
            :disabled="searching[mode] || auth.loggingIn"
            @click="searchDevices()"
          >
            <component
              :is="mode === 'dlna' || auth.session ? RefreshCw : ScanQrCode"
              :size="17"
            />{{
              searching[mode]
                ? "搜索中…"
                : mode === "dlna"
                  ? "搜索设备"
                  : auth.session
                    ? "刷新设备"
                    : "扫码登录"
            }}
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
        <button
          type="submit"
          class="button secondary"
          :disabled="resolvingDevice || casting.busy"
        >
          {{ resolvingDevice ? "连接中…" : "连接" }}
        </button>
      </form>

      <!-- 设备列表与空状态 -->
      <ul v-if="devices.length" class="device-list">
        <li
          v-for="device in devices"
          :key="deviceKey(device)"
          class="device-row"
        >
          <component :is="device.kind === 'dlna' ? Cast : Tv" :size="20" />
          <div class="device-info">
            <h3>{{ device.name }}</h3>
            <p>
              {{
                device.kind === "dlna"
                  ? device.location
                  : [device.brand, device.model].filter(Boolean).join(" · ")
              }}
            </p>
          </div>
          <button
            class="button secondary"
            :disabled="casting.busy || resolvingDevice"
            @click="connect(device)"
          >
            {{ isActive(device) ? "断开" : "连接" }}
          </button>
        </li>
      </ul>
      <div v-else class="device-empty" :aria-busy="searching[mode]">
        <component :is="mode === 'dlna' ? Cast : Tv" :size="32" />
        <p>
          {{
            searching[mode]
              ? "正在搜索设备…"
              : mode === "bilibili" && !auth.session
                ? "尚未登录哔哩哔哩"
                : "暂无设备"
          }}
        </p>
        <span v-if="mode === 'dlna'">将电视与本机连接到同一网络后搜索</span>
      </div>
    </section>
  </div>
</template>

<style scoped>
.device-list {
  list-style: none;
  margin: 8px 0 0;
  padding: 0;
}
.device-row {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px 0;
  border-bottom: 1px solid var(--line);
}
.device-row:last-child {
  border-bottom: 0;
}
.device-row > svg {
  color: var(--muted);
}
.device-info {
  flex: 1;
  min-width: 0;
}
.device-info h3,
.device-info p {
  overflow-wrap: anywhere;
}
.device-info p {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--muted);
}

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
