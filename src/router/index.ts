import {
  createRouter,
  createWebHashHistory,
  type RouteLocationRaw,
} from "vue-router";
import ConnectionPage from "@/pages/ConnectionPage.vue";
import PlaybackPage from "@/pages/PlaybackPage.vue";
import SettingsPage from "@/pages/SettingsPage.vue";
import LogsPage from "@/pages/LogsPage.vue";

declare module "vue-router" {
  interface RouteMeta {
    title?: string;
    back?: { to: RouteLocationRaw; label: string };
  }
}

// 创建路由实例，使用哈希模式，并定义应用的路由配置
export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    // 默认路由，重定向到连接页面
    { path: "/", redirect: { name: "connection" } },
    // 连接页面路由，显示设备连接界面
    {
      path: "/connection",
      name: "connection",
      component: ConnectionPage,
      meta: { title: "连接设备" },
    },
    // 播放页面路由，显示播放控制界面
    {
      path: "/playback",
      name: "playback",
      component: PlaybackPage,
      meta: { title: "播放控制" },
    },
    // 设置页面路由，显示设置选项和日志查看界面
    {
      path: "/settings",
      // 设置页面路由，显示设置选项和日志查看界面
      children: [
        {
          path: "",
          name: "settings",
          component: SettingsPage,
          meta: { title: "设置" },
        },
        {
          path: "logs",
          name: "logs",
          component: LogsPage,
          meta: {
            title: "运行日志",
            back: { to: { name: "settings" }, label: "返回设置" },
          },
        },
      ],
    },
    // 404路由，重定向到连接页面
    { path: "/:pathMatch(.*)*", redirect: { name: "connection", params: {} } },
  ],
});
