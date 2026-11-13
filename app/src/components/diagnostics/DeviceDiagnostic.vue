<script setup lang="ts">
import { castingApi } from "@/services/casting";
import type {
  DiagnosticEntry,
  DiagnosticPanelProps,
} from "@/types/diagnostics";

const props = defineProps<DiagnosticPanelProps>();
const entries = defineModel<DiagnosticEntry[]>("entries", { required: true });

function readDevice() {
  return props.run(async (current) => {
    const info = await castingApi.diagnosticDevice(props.location);
    if (current())
      entries.value.unshift({
        title: "设备描述",
        result: info.response,
        services: info.services,
      });
  });
}
</script>

<template>
  <button
    type="button"
    class="button primary"
    aria-label="读取设备信息"
    @click="readDevice"
  >
    读取
  </button>
</template>
