<script setup lang="ts">
import { ChevronRight } from "@lucide/vue";
import { RouterLink } from "vue-router";
import { diagnosticSections } from "@/types/diagnostics";
</script>

<template>
  <section class="content-section diagnostic-menu">
    <RouterLink
      v-for="item in diagnosticSections"
      :key="item.key"
      :to="{ name: `diagnostic-${item.key}` }"
    >
      <span>{{ item.title }}</span>
      <ChevronRight :size="18" aria-hidden="true" />
    </RouterLink>
  </section>
</template>

<style scoped>
.diagnostic-menu a {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  min-height: 62px;
  padding: 12px 0;
  color: inherit;
  font-size: 14px;
  text-decoration: none;
}
.diagnostic-menu a:hover {
  color: var(--accent);
}
</style>
