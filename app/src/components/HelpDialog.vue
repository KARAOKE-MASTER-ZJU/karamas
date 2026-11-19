<script setup lang="ts">
import { ref } from "vue";
import AppDialog from "@/components/base/AppDialog.vue";
import { isTauri, openUrl } from "@/services/platform";

const emit = defineEmits<{ close: [] }>();
const documentUrl =
  "https://idol-zju.feishu.cn/wiki/Ytt1wNh88i6E9jkEndhcxNmYnBd#share-DRDOdc1wdon3eix3QXIc9FMVnqf";
const openFailed = ref(false);

async function openDocument(event: MouseEvent) {
  if (!isTauri()) return;
  event.preventDefault();
  openFailed.value = false;
  try {
    await openUrl(documentUrl);
  } catch {
    openFailed.value = true;
  }
}
</script>

<template>
  <AppDialog title="使用帮助" close-label="关闭帮助" @close="emit('close')">
    <p class="help-summary">
      DLNA 需要本机与电视连接同一网络。小电视云投屏需要登录哔哩哔哩账号。
    </p>
    <p v-if="openFailed" class="field-error" role="alert">
      无法打开飞书文档，请重试。
    </p>
    <div class="help-actions">
      <a
        class="button secondary"
        :href="documentUrl"
        target="_blank"
        rel="noopener noreferrer"
        @click="openDocument"
      >
        查看飞书文档
      </a>
    </div>
  </AppDialog>
</template>

<style scoped>
.help-summary {
  font-size: 14px;
  line-height: 1.8;
}
.help-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 20px;
}
.help-actions a {
  text-decoration: none;
}
.field-error {
  margin-top: 8px;
}
</style>
