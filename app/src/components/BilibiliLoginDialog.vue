<script setup lang="ts">
import { RotateCcw } from "@lucide/vue";
import AppDialog from "@/components/base/AppDialog.vue";
import { ref, watch } from "vue";
import { toCanvas } from "qrcode";
import { useBilibiliStore } from "@/stores/bilibili";
const auth = useBilibiliStore();
const canvas = ref<HTMLCanvasElement | null>(null);
const qrError = ref("");
watch(
  [() => auth.qrUrl, canvas],
  async ([url]) => {
    qrError.value = "";
    if (!url || !canvas.value) return;
    try {
      await toCanvas(canvas.value, url, { width: 240, margin: 2 });
    } catch {
      qrError.value = "二维码生成失败。";
    }
  },
  { flush: "post", immediate: true },
);
function retry() {
  auth.closeLogin();
  qrError.value = "";
  void auth.login();
}
</script>

<template>
  <AppDialog
    class="login-dialog"
    title="哔哩哔哩扫码登录"
    close-label="关闭登录"
    @close="auth.closeLogin"
  >
    <template #actions>
      <button
        v-if="auth.loginError || qrError"
        type="button"
        class="icon-button"
        aria-label="重新获取二维码"
        @click="retry"
      >
        <RotateCcw :size="18" aria-hidden="true" />
      </button>
    </template>
    <div class="qr-content" :aria-busy="auth.loggingIn && !auth.qrUrl">
      <p class="credential-notice">
        登录凭证仅保存在本机，用于查询同账号的投屏设备、控制播放，以及获取账号可用的清晰度和播放地址。
      </p>
      <canvas
        v-show="auth.qrUrl && !qrError"
        ref="canvas"
        role="img"
        aria-label="哔哩哔哩登录二维码"
      />
      <p v-if="auth.loginError || qrError" class="field-error" role="alert">
        {{ auth.loginError || qrError }}
      </p>
      <p v-else-if="!auth.qrUrl" role="status">正在获取二维码…</p>
    </div>
  </AppDialog>
</template>

<style scoped>
.qr-content {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  font-size: 14px;
  line-height: 1.8;
  text-align: center;
}
.credential-notice {
  color: var(--muted);
  font-size: 13px;
  text-align: left;
}
.qr-content canvas {
  max-width: 100%;
  height: auto !important;
}
</style>
