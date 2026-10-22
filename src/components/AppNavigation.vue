<!-- AppNavigation.vue: 应用侧边栏/底边栏导航组件 -->
<script setup lang="ts">
import { Cast, Play, Settings, type LucideIcon } from "@lucide/vue";
import { RouterLink } from "vue-router";
const navigation: { name: string; label: string; icon: LucideIcon }[] = [
  { name: "connection", label: "连接", icon: Cast },
  { name: "playback", label: "控制", icon: Play },
  { name: "settings", label: "设置", icon: Settings },
];
</script>

<template>
  <nav class="navigation" aria-label="主导航">
    <img
      class="app-logo"
      src="/app-icon.svg"
      alt="KTV Casting"
      width="36"
      height="36"
    />
    <RouterLink
      v-for="item in navigation"
      :key="item.name"
      :to="{ name: item.name }"
      class="nav-item"
    >
      <span class="nav-icon"><component :is="item.icon" :size="22" /></span>
      <span>{{ item.label }}</span>
    </RouterLink>
  </nav>
</template>

<style scoped>
.navigation {
  position: fixed;
  inset: 0 auto 0 0;
  width: var(--navigation-rail-width);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  background: var(--subtle);
  border-right: 1px solid var(--line);
  padding: 24px 8px;
  z-index: 5;
}
.app-logo {
  margin: 0 0 28px;
}
.nav-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  width: 72px;
  border: 0;
  background: none;
  color: var(--muted);
  padding: 6px 0;
  font-size: 12px;
  line-height: 18px;
  font-weight: 500;
  text-decoration: none;
}
.nav-icon {
  display: grid;
  place-items: center;
  width: 56px;
  height: 34px;
  border-radius: 18px;
}
.nav-item:hover .nav-icon {
  background: var(--hover);
}
.nav-item.router-link-active {
  color: var(--accent);
}
.nav-item.router-link-active .nav-icon {
  background: var(--accent-soft);
}
@media (max-width: 640px) {
  .navigation {
    inset: auto 0 0;
    width: auto;
    height: var(--navigation-bar-height);
    padding: 8px 12px env(safe-area-inset-bottom);
    flex-direction: row;
    justify-content: space-around;
    border-right: 0;
    border-top: 1px solid var(--line);
    background: var(--surface);
    gap: 0;
  }
  .app-logo {
    display: none;
  }
  .nav-item {
    flex: 1;
    gap: 3px;
    padding: 0;
  }
  .nav-icon {
    height: 32px;
  }
}
</style>
