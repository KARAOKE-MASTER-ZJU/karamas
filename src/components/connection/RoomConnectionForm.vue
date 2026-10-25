<script setup lang="ts">
import { computed, reactive } from "vue";
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
const isJoined = computed(
  () =>
    casting.room !== null &&
    casting.room.serverUrl === form.serverUrl.trim().replace(/\/$/, "") &&
    casting.room.roomId === form.roomId.trim(),
);

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
  } catch (error) {
    notify(errorMessage(error), "error");
  }
  // 投屏启动失败不影响已经确认的房间及其本机配置。
  if (
    casting.room?.serverUrl === config.serverUrl &&
    casting.room.roomId === config.roomId
  ) {
    Object.assign(form, config);
    if (!store.saveConnection(config))
      notify("已加入房间，但无法保存本机配置。", "warning");
  }
}
</script>

<template>
  <section class="content-section" aria-labelledby="room-title">
    <div class="room-heading">
      <h2 id="room-title">点歌房间</h2>
      <p
        class="room-status"
        :class="{ joined: casting.room !== null }"
        :title="casting.room?.serverUrl"
        role="status"
      >
        {{
          casting.enteringRoom
            ? "正在加入…"
            : casting.room
              ? `已加入：${casting.room.roomId}`
              : "未加入"
        }}
      </p>
    </div>
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
        autocomplete="off"
      />
      <div class="room-actions">
        <button
          class="button primary"
          type="submit"
          :disabled="casting.busy || isJoined"
          aria-label="创建并加入"
        >
          <span class="room-action-label">创建并加入</span>
          <span class="room-action-compact" aria-hidden="true">创建</span>
        </button>
        <button
          class="button secondary"
          type="button"
          :aria-label="isJoined ? '已加入房间' : '加入房间'"
          :disabled="casting.busy || isJoined"
          @click="enterRoom('join')"
        >
          <span class="room-action-label">{{
            isJoined ? "已加入" : "加入房间"
          }}</span>
          <span class="room-action-compact" aria-hidden="true">{{
            isJoined ? "已加入" : "加入"
          }}</span>
        </button>
      </div>
    </form>
  </section>
</template>

<style scoped>
.room-heading {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px 16px;
  margin-bottom: 20px;
}
.room-status {
  min-width: 0;
  max-width: 100%;
  overflow-wrap: anywhere;
  font-size: 12px;
  line-height: 1.5;
  color: var(--muted);
}
.room-status.joined {
  color: var(--accent);
}
.room-form {
  container-type: inline-size;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: start;
  gap: 18px 12px;
}
.room-form > .field:first-child {
  grid-column: 1 / -1;
}
.room-actions {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 8px;
  margin-top: calc(var(--control-line-height) + 8px);
}
.room-actions .button {
  min-width: 104px;
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
}
@media (max-width: 899px) {
  .room-form .button {
    min-width: 50px;
  }
  .room-form :deep(.field > input) {
    padding: 0 8px;
  }
}
@container (max-width: 360px) {
  .room-action-label {
    display: none;
  }
  .room-action-compact {
    display: inline;
  }
}
@media (max-width: 640px) {
  .room-heading {
    margin-bottom: 18px;
  }
}
</style>
