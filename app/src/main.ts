import { createApp } from "vue";
import { createPinia, disposePinia } from "pinia";
import App from "@/App.vue";
import LocalPlayer from "@/components/playback/LocalPlayer.vue";
import { router } from "@/router";
import "@/style.css";

const windowToken = window.__KARAMAS_PLAYBACK_WINDOW__;
const root = windowToken ? LocalPlayer : App;
const app = createApp(root, windowToken ? { active: true, windowToken } : {});
const pinia = createPinia();
app.use(pinia);
if (!windowToken) app.use(router);
app.onUnmount(() => disposePinia(pinia));
if (windowToken) app.mount("#app");
else void router.isReady().then(() => app.mount("#app"));
