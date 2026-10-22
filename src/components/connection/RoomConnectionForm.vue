<script setup lang="ts">
import { reactive } from "vue";
import FormField from "@/components/base/FormField.vue";
import { usePreferencesStore } from "@/stores/preferences";
import { useCastingStore } from "@/stores/casting";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { errorMessage } from "@/services/casting";
import type { RoomEntryMode } from "@/types";

const store = usePreferencesStore();
const casting = useCastingStore();
const { notify } = useOperationNotice();
const form = reactive({
  serverUrl: store.preferences.serverUrl,
  roomId: store.preferences.roomId,
});
const errors = reactive({ serverUrl: "", roomId: "" });

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
</script>

<template>
  <section class="content-section" aria-labelledby="room-title">
    <h2 id="room-title">点歌房间</h2>
    <form
      class="room-form"
      :aria-busy="casting.enteringRoom"
      novalidate
      @submit.prevent="enterRoom('create')"
    >
      <FormField
        id="server-url"
        v-model="form.serverUrl"
        label="服务器网址"
        :error="errors.serverUrl"
        :disabled="casting.enteringRoom || casting.connecting"
        type="url"
        inputmode="url"
        placeholder="https://你的点歌台网址"
        autocomplete="url"
      />
      <FormField
        id="room-id"
        v-model="form.roomId"
        label="房间号"
        :error="errors.roomId"
        :disabled="casting.enteringRoom || casting.connecting"
        placeholder="输入房间号"
        autocomplete="off"
      />
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
  .room-form :deep(.field > input) {
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
