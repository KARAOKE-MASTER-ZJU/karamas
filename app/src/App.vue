<script setup lang="ts">
import { ArrowLeft, CircleQuestionMark } from "@lucide/vue";
import { onMounted, ref } from "vue";
import { RouterLink, RouterView, useRoute } from "vue-router";
import PlaybackPage from "@/pages/PlaybackPage.vue";
import AppNavigation from "@/components/AppNavigation.vue";
import AppToast from "@/components/AppToast.vue";
import HelpDialog from "@/components/HelpDialog.vue";
import { usePreferencesStore } from "@/stores/preferences";
import BilibiliLoginDialog from "@/components/BilibiliLoginDialog.vue";
import { useBilibiliStore } from "@/stores/bilibili";
import { useCastingStore } from "@/stores/casting";
import { useCastingRuntime } from "@/composables/useCastingRuntime";
import { useNotificationsStore } from "@/stores/notifications";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { useTheme } from "@/composables/useTheme";
import { useAndroidBack } from "@/composables/useAndroidBack";
import { useUpdaterStore } from "@/stores/updater";
import UpdateDialog from "@/components/UpdateDialog.vue";
import UpdateStatusButton from "@/components/UpdateStatusButton.vue";

// 使用路由、偏好设置、日志和通知存储
const route = useRoute();
const preferences = usePreferencesStore();
const auth = useBilibiliStore();
const casting = useCastingStore();
const notifications = useNotificationsStore();
const updater = useUpdaterStore();

// 初始化应用主题和原生交互，读取配置失败时提示。
const { notify } = useOperationNotice();
useTheme();
useCastingRuntime();
useAndroidBack();
onMounted(() => {
  void updater.initialize();
  if (preferences.storageWarning)
    notify("未能读取本机配置，已使用默认设置。", "warning");
});
// 控制帮助对话框的显示状态
const showHelp = ref(false);
const workspace = ref<HTMLElement | null>(null);
const pageContent = ref<HTMLElement | null>(null);
</script>

<template>
  <!-- 应用主界面结构，包括导航、主要内容区域、页眉、路由视图、通知和帮助对话框 -->
  <div class="app-shell">
    <!-- 应用导航组件 -->
    <AppNavigation />

    <main id="main-content" ref="workspace" class="workspace">
      <div
        ref="pageContent"
        class="page-content"
        :class="{
          'local-playback-layout':
            route.name === 'playback' && casting.activeDevice?.kind === 'local',
        }"
      >
        <!-- 页眉部分，显示页面标题和操作按钮 -->
        <header class="page-header">
          <!-- 页面标题部分，包含返回按钮和标题文本 -->
          <div class="page-title">
            <RouterLink
              v-if="route.meta.back"
              :to="route.meta.back.to"
              replace
              class="icon-button back-button"
              :aria-label="route.meta.back.label"
            >
              <ArrowLeft />
            </RouterLink>
            <h1>{{ route.meta.title }}</h1>
          </div>

          <!-- 页眉操作部分: 帮助按钮 -->
          <div class="header-actions">
            <UpdateStatusButton />
            <button
              class="icon-button"
              aria-label="使用帮助"
              @click="showHelp = true"
            >
              <CircleQuestionMark />
            </button>
          </div>
        </header>

        <!-- 播放页只切换可见性，视频元素保持在文档中；其他页面沿用缓存。 -->
        <PlaybackPage
          v-show="route.name === 'playback'"
          :active="route.name === 'playback'"
          :viewport="workspace"
          :content="pageContent"
        />
        <RouterView v-slot="{ Component }">
          <KeepAlive>
            <component
              :is="Component"
              v-if="route.name !== 'playback'"
              :key="route.name"
            />
          </KeepAlive>
        </RouterView>
      </div>
    </main>

    <!-- 应用通知组件，显示通知消息和级别 -->
    <AppToast
      :message="notifications.message"
      :level="notifications.level"
      @dismiss="notifications.dismiss"
    />

    <!-- 帮助对话框组件，根据 showHelp 状态显示 -->
    <HelpDialog v-if="showHelp" @close="showHelp = false" />
    <BilibiliLoginDialog v-if="auth.showLogin" />
    <UpdateDialog v-if="updater.showDialog" />
  </div>
</template>

<style scoped>
.app-shell {
  height: 100dvh;
  overflow: hidden;
  padding-left: var(--navigation-rail-width);
}
.workspace {
  height: 100%;
  overflow-y: auto;
  scrollbar-gutter: stable both-edges;
}
.workspace::-webkit-scrollbar {
  width: 4px;
}
.workspace::-webkit-scrollbar-thumb {
  background: color-mix(in srgb, var(--muted) 50%, transparent);
  border-radius: 2px;
}
.page-content {
  max-width: 824px;
  padding: 0 48px 32px;
  margin: 0 auto;
}
.page-content.local-playback-layout {
  max-width: none;
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
    padding-bottom: var(--navigation-bar-height);
  }
  .page-content {
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
