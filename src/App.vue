<script setup lang="ts">
import { ArrowLeft, CircleQuestionMark } from "@lucide/vue";
import { onMounted, ref } from "vue";
import { RouterLink, RouterView, useRoute } from "vue-router";
import AppNavigation from "@/components/AppNavigation.vue";
import AppToast from "@/components/AppToast.vue";
import HelpDialog from "@/components/HelpDialog.vue";
import { usePreferencesStore } from "@/stores/preferences";
import { useLogsStore } from "@/stores/logs";
import { useNotificationsStore } from "@/stores/notifications";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { useTheme } from "@/composables/useTheme";

// 使用路由、偏好设置、日志和通知存储
const route = useRoute();
const preferences = usePreferencesStore();
const logs = useLogsStore();
const notifications = useNotificationsStore();

// 使用操作提示、应用主题，并在组件挂载时记录日志和显示存储警告（如果有）
const { notify } = useOperationNotice();
useTheme();
onMounted(() => {
  logs.add("应用界面已启动，投屏功能尚未接入。");
  if (preferences.storageWarning)
    notify("未能读取本机配置，已使用默认设置。", "warning");
});
// 控制帮助对话框的显示状态
const showHelp = ref(false);
</script>

<template>
  <!-- 应用主界面结构，包括导航、主要内容区域、页眉、路由视图、通知和帮助对话框 -->
  <div class="app-shell">
    <!-- 应用导航组件 -->
    <AppNavigation />

    <main id="main-content" class="workspace">
      <!-- 页眉部分，显示页面标题和操作按钮 -->
      <header class="page-header">
        <!-- 页面标题部分，包含返回按钮和标题文本 -->
        <div class="page-title">
          <RouterLink
            v-if="route.meta.back"
            :to="route.meta.back.to"
            class="icon-button back-button"
            :aria-label="route.meta.back.label"
          >
            <ArrowLeft />
          </RouterLink>
          <h1>{{ route.meta.title }}</h1>
        </div>

        <!-- 页眉操作部分: 帮助按钮 -->
        <div class="header-actions">
          <button
            class="icon-button"
            aria-label="使用帮助"
            @click="showHelp = true"
          >
            <CircleQuestionMark />
          </button>
        </div>
      </header>

      <!-- 路由视图部分，使用 KeepAlive 缓存组件状态 -->
      <RouterView v-slot="{ Component }">
        <KeepAlive>
          <component :is="Component" />
        </KeepAlive>
      </RouterView>
    </main>

    <!-- 应用通知组件，显示通知消息和级别 -->
    <AppToast
      :message="notifications.message"
      :level="notifications.level"
      @dismiss="notifications.dismiss"
    />

    <!-- 帮助对话框组件，根据 showHelp 状态显示 -->
    <HelpDialog v-if="showHelp" @close="showHelp = false" />
  </div>
</template>

<style scoped>
/* 应用主界面样式，包括最小高度、内边距和布局 */
.app-shell {
  min-height: 100dvh;
  padding-left: var(--navigation-rail-width);
}
/* 工作区样式，设置最大宽度、内边距和居中对齐 */
.workspace {
  max-width: 824px;
  padding: 0 48px 32px;
  margin: 0 auto;
}
/* 页眉样式，设置最小高度、弹性布局和间距 */
.page-header {
  min-height: 96px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}
/* 页面标题和页眉操作部分的样式，设置弹性布局、对齐方式和间距 */
.page-title,
.header-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.back-button {
  margin-left: -8px;
}

@media (max-width: 640px) {
  .app-shell {
    padding-bottom: calc(var(--navigation-bar-height) + 4px);
  }
  .workspace {
    padding: 0 20px 24px;
  }
  .page-header {
    min-height: 80px;
  }
  h1 {
    font-size: 22px;
  }
}
</style>
