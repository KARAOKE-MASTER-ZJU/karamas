import { createApp } from "vue";
import { createPinia, disposePinia } from "pinia";
import App from "@/App.vue";
import { router } from "@/router";
import "@/style.css";

const app = createApp(App);
const pinia = createPinia();
app.use(pinia);
app.use(router);
app.onUnmount(() => disposePinia(pinia));
void router.isReady().then(() => app.mount("#app"));
