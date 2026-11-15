<script setup lang="ts">
import AppDialog from "@/components/base/AppDialog.vue";
import { useUpdaterStore } from "@/stores/updater";
import { useCastingStore } from "@/stores/casting";
import { openUrl, isTauri } from "@/services/platform";
import { errorMessage } from "@/services/casting";
import { useOperationNotice } from "@/composables/useOperationNotice";

const updater = useUpdaterStore();
const casting = useCastingStore();
const { notify } = useOperationNotice();
const releaseUrl =
  "https://github.com/KARAOKE-MASTER-ZJU/karamas/releases/latest";

async function openRelease(event: MouseEvent) {
  if (!isTauri()) return;
  event.preventDefault();
  try {
    await openUrl(releaseUrl);
  } catch (error) {
    notify(`打开 GitHub 失败：${errorMessage(error)}`, "error");
  }
}
</script>

<template>
  <AppDialog
    title="应用更新"
    close-label="关闭更新窗口"
    @close="updater.showDialog = false"
  >
    <div class="update-content" aria-live="polite">
      <p v-if="updater.status === 'checking'">正在检查更新…</p>
      <template v-if="updater.update">
        <p>新版本 {{ updater.update.version }}</p>
        <p v-if="updater.update.notes" class="release-notes">
          {{ updater.update.notes }}
        </p>
      </template>
      <div v-if="updater.status === 'downloading'">
        <p>
          正在下载{{
            updater.progress === undefined ? "…" : ` ${updater.progress}%`
          }}
        </p>
        <progress
          :value="updater.progress"
          max="100"
          aria-label="更新下载进度"
        />
      </div>
      <p v-if="updater.status === 'installing'">正在安装更新…</p>
      <p v-if="updater.error" role="alert" class="update-error">
        {{ updater.error }}
      </p>
    </div>
    <div class="update-actions">
      <a
        class="button secondary"
        :href="releaseUrl"
        target="_blank"
        rel="noopener noreferrer"
        @click="openRelease"
      >
        在 GitHub 查看
      </a>
      <button
        v-if="updater.status === 'available'"
        class="button primary"
        @click="updater.download"
      >
        下载更新
      </button>
      <button
        v-else-if="updater.status === 'ready'"
        class="button primary"
        :disabled="casting.busy"
        @click="updater.install"
      >
        安装并重启
      </button>
      <button
        v-else-if="updater.status === 'downloading'"
        class="button"
        :disabled="!updater.canCancel || updater.cancelling"
        @click="updater.cancelDownload"
      >
        {{ updater.cancelling ? "正在取消…" : "取消下载" }}
      </button>
      <button
        v-else-if="!updater.busy && updater.error"
        class="button primary"
        @click="updater.check()"
      >
        重新检查
      </button>
    </div>
  </AppDialog>
</template>

<style scoped>
.update-content {
  display: grid;
  gap: 12px;
  font-size: 14px;
}
.release-notes {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 35dvh;
  overflow-y: auto;
}
progress {
  width: 100%;
  margin-top: 12px;
  accent-color: var(--accent);
}
.update-error {
  color: var(--danger, #c33);
}
.update-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  justify-content: flex-end;
  margin-top: 20px;
}
.update-actions a {
  text-decoration: none;
}
</style>
