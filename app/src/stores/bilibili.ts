import { acceptHMRUpdate, defineStore } from "pinia";
import { ref } from "vue";
import { castingApi, errorMessage } from "@/services/casting";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { usePreferencesStore } from "@/stores/preferences";
import type { BilibiliSession } from "@/types/casting";

export const useBilibiliStore = defineStore("bilibili", () => {
  const preferences = usePreferencesStore();
  const session = ref<BilibiliSession | null>(null);
  const showLogin = ref(false);
  const loggingIn = ref(false);
  const qrUrl = ref("");
  const loginError = ref("");
  const { notify } = useOperationNotice();
  // 取消后忽略旧调用的结果和二维码，防止覆盖下一次登录。
  let attempt = 0;
  let cancelling: Promise<void> | undefined;

  async function restore() {
    const currentAttempt = attempt;
    const saved = await castingApi.loadSession();
    if (currentAttempt !== attempt) return;
    if (
      saved &&
      (saved.expires_at === null || saved.expires_at > Date.now() / 1000)
    )
      session.value = saved;
  }
  async function login() {
    if (loggingIn.value) return;
    const currentAttempt = ++attempt;
    showLogin.value = true;
    loggingIn.value = true;
    qrUrl.value = "";
    loginError.value = "";
    try {
      // 关闭后立即重开时，先等取消命令结束，避免取消新任务。
      await cancelling;
      if (currentAttempt !== attempt) return;
      const result = await castingApi.login(
        preferences.ensureBilibiliBuvid(),
        (url) => {
          if (currentAttempt === attempt) qrUrl.value = url;
        },
      );
      if (currentAttempt !== attempt) return;
      session.value = result;
      // 登录与保存是两个接口，由应用明确决定保存。
      try {
        await castingApi.saveSession(result);
      } catch (error) {
        notify(`已登录，但未能保存会话：${errorMessage(error)}`, "warning");
      }
      if (currentAttempt === attempt) showLogin.value = false;
    } catch (error) {
      if (currentAttempt === attempt) loginError.value = errorMessage(error);
    } finally {
      if (currentAttempt === attempt) loggingIn.value = false;
    }
  }
  function closeLogin() {
    ++attempt;
    showLogin.value = false;
    qrUrl.value = "";
    if (loggingIn.value) {
      cancelling = castingApi.cancelLogin().catch((error: unknown) => {
        notify(`取消登录失败：${errorMessage(error)}`, "error");
      });
    }
    loggingIn.value = false;
  }
  async function logout() {
    await castingApi.clearSession();
    ++attempt;
    session.value = null;
  }
  return {
    session,
    showLogin,
    loggingIn,
    qrUrl,
    loginError,
    restore,
    login,
    closeLogin,
    logout,
  };
});
if (import.meta.hot)
  import.meta.hot.accept(acceptHMRUpdate(useBilibiliStore, import.meta.hot));
