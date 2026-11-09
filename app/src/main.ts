import { invoke } from "@/services/platform";
import { createApp, nextTick } from "vue";
import { createPinia, disposePinia } from "pinia";
import { ScrollRestoration } from "vue-router/experimental";
import App from "@/App.vue";
import {
  initializePlaybackOutput,
  isPlaybackController,
} from "@/services/playbackWindow";
import { router } from "@/router";
import "@/style.css";

const app = createApp(App);
const pinia = createPinia();
app.use(pinia);
{
  const scrollStoragePrefix = "karamas:scroll:";
  app.use(ScrollRestoration, {
    router,
    storageKeyPrefix: scrollStoragePrefix,
    key: (to) => to.path,
    capture: () => {
      const viewport = document.getElementById("main-content");
      return viewport
        ? { default: { top: viewport.scrollTop, left: viewport.scrollLeft } }
        : null;
    },
    restore: (entry, to) => {
      // 只保留当前路由分支上的位置，退出的子页和其他分支重新进入时回到顶部。
      const activePaths = new Set(to.matched.map((record) => record.path));
      try {
        for (const key of Object.keys(sessionStorage)) {
          if (
            key.startsWith(scrollStoragePrefix) &&
            !activePaths.has(key.slice(scrollStoragePrefix.length))
          ) {
            sessionStorage.removeItem(key);
          }
        }
      } catch {
        // 存储不可用时，Vue Router 同样无法保存位置，按首次进入处理。
      }
      document.getElementById("main-content")?.scrollTo({
        top: entry?.default?.top ?? 0,
        left: entry?.default?.left ?? 0,
        behavior: "instant",
      });
    },
  });
  app.use(router);
}
let disposed = false;
let stopPlaybackOutput: (() => void) | undefined;
function disposePlaybackOutput() {
  disposed = true;
  stopPlaybackOutput?.();
  stopPlaybackOutput = undefined;
}
app.onUnmount(() => {
  disposePlaybackOutput();
  disposePinia(pinia);
});
if (import.meta.hot) import.meta.hot.dispose(disposePlaybackOutput);
void initializePlaybackOutput().then(async (stop) => {
  if (disposed) {
    stop();
    return;
  }
  stopPlaybackOutput = stop;
  if (isPlaybackController) await router.replace({ name: "playback" });
  await router.isReady();
  if (disposed) return;
  app.mount("#app");
  await nextTick();
  if (!disposed && isPlaybackController)
    await invoke("playback_controller_ready");
});
