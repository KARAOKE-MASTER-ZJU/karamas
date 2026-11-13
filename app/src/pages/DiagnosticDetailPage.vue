<script setup lang="ts">
import DiagnosticResult from "@/components/DiagnosticResult.vue";
import { useDiagnosticsStore } from "@/stores/diagnostics";

const diagnostics = useDiagnosticsStore();
</script>

<template>
  <DiagnosticResult v-if="diagnostics.selected" v-bind="diagnostics.selected" />
  <p v-else class="empty-result">暂无记录</p>
</template>

<style scoped>
.empty-result {
  padding: 24px 0;
  color: var(--muted);
  font-size: 14px;
}
</style>
