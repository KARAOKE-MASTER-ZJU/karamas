<script setup lang="ts">
import { X } from "@lucide/vue";
import { onMounted, ref, watch } from "vue";
import { toCanvas } from "qrcode";
import { useBilibiliStore } from "@/stores/bilibili";
const auth = useBilibiliStore();
const dialog = ref<HTMLDialogElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const qrError = ref("");
onMounted(() => dialog.value?.showModal());
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
  <dialog
    ref="dialog"
    class="login-dialog"
    aria-labelledby="login-title"
    @click="$event.target === dialog && dialog?.close()"
    @close="auth.closeLogin"
  >
    <div class="dialog-heading">
      <h2 id="login-title" tabindex="-1" autofocus>哔哩哔哩扫码登录</h2>
      <button
        class="icon-button"
        aria-label="关闭登录"
        @click="dialog?.close()"
      >
        <X />
      </button>
    </div>
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
  </dialog>
</template>

<style scoped>
.login-dialog {
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface);
  color: var(--text);
  width: min(460px, calc(100% - 40px));
  padding: 24px;
  box-shadow: 0 16px 64px #0003;
}
.login-dialog::backdrop {
  background: #0006;
}
.dialog-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 20px;
  gap: 12px;
}
.dialog-heading h2:focus {
  outline: none;
}
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
