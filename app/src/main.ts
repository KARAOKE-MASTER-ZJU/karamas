import { createApp } from "vue";
import { createPinia, disposePinia } from "pinia";
import { ScrollRestoration } from "vue-router/experimental";
import App from "@/App.vue";
import LocalPlayer from "@/components/playback/LocalPlayer.vue";
import { router } from "@/router";
import "@/style.css";

const windowToken = window.__KARAMAS_PLAYBACK_WINDOW__;
const root = windowToken ? LocalPlayer : App;
const app = createApp(root, windowToken ? { active: true, windowToken } : {});
const pinia = createPinia();
app.use(pinia);
if (!windowToken) {
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
app.onUnmount(() => disposePinia(pinia));
if (windowToken) app.mount("#app");
else void router.isReady().then(() => app.mount("#app"));
