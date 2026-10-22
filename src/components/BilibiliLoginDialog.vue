<script setup lang="ts">
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
      qrError.value = "二维码生成失败，请重试。";
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
    <div class="qr-content" :aria-busy="auth.loggingIn && !auth.qrUrl">
      <canvas
        v-show="auth.qrUrl && !qrError"
        ref="canvas"
        role="img"
        aria-label="哔哩哔哩登录二维码"
      />
      <p v-if="auth.loginError || qrError" class="field-error" role="alert">
        {{ auth.loginError || qrError }}
      </p>
      <p v-else>
        {{ auth.qrUrl ? "使用哔哩哔哩 App 扫码并确认登录" : "正在获取二维码…" }}
      </p>
      <button
        v-if="auth.loginError || qrError"
        class="button secondary"
        @click="retry"
      >
        重新获取
      </button>
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
.qr-content canvas {
  max-width: 100%;
  height: auto !important;
}
</style>
