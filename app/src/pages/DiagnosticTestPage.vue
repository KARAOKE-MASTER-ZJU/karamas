<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import DiagnosticResultList from "@/components/DiagnosticResultList.vue";
import DeviceDiagnostic from "@/components/diagnostics/DeviceDiagnostic.vue";
import ParameterDiagnostic from "@/components/diagnostics/ParameterDiagnostic.vue";
import PlaybackDiagnostic from "@/components/diagnostics/PlaybackDiagnostic.vue";
import NetworkDiagnostic from "@/components/diagnostics/NetworkDiagnostic.vue";
import { useDiagnosticsStore } from "@/stores/diagnostics";
import { useCastingStore } from "@/stores/casting";
import { errorMessage } from "@/services/casting";
import type {
  DiagnosticEntry,
  DiagnosticKind,
  DiagnosticPanelProps,
} from "@/types/diagnostics";
import "@/components/diagnostics/diagnostics.css";

const props = defineProps<{ kind: DiagnosticKind }>();
const casting = useCastingStore();
const diagnostics = useDiagnosticsStore();
const router = useRouter();
const route = useRoute();
const device = computed(() =>
  casting.selectedDevice?.kind === "dlna" ? casting.selectedDevice : null,
);
const panels = {
  device: DeviceDiagnostic,
  parameters: ParameterDiagnostic,
  playback: PlaybackDiagnostic,
  latency: NetworkDiagnostic,
};
const error = ref("");
const results = ref<DiagnosticEntry[]>([]);
let revision = 0;

watch(
  () => device.value?.location,
  () => {
    ++revision;
    results.value = [];
    error.value = "";
  },
);
watch(
  () => route.name,
  (name) => {
    if (
      name === `diagnostic-${props.kind}` ||
      name === `diagnostic-${props.kind}-detail`
    )
      return;
    ++revision;
  },
);

const run: DiagnosticPanelProps["run"] = async (work) => {
  const id = revision;
  error.value = "";
  try {
    await work(() => id === revision);
  } catch (reason) {
    if (id === revision) error.value = errorMessage(reason);
  }
};
function openResult(entry: DiagnosticEntry) {
  diagnostics.selected = entry;
  void router.push({ name: `diagnostic-${props.kind}-detail` });
}
function clearResults() {
  results.value = [];
  diagnostics.selected = null;
}
</script>

<template>
  <div class="diagnostic-test">
    <section v-if="kind !== 'latency' && !device" class="diagnostic-empty">
      <p>
        {{ casting.selectedDevice ? "请先连接 DLNA 设备" : "请先连接设备" }}
      </p>
      <RouterLink class="button primary" :to="{ name: 'connection' }"
        >去连接</RouterLink
      >
    </section>
    <template v-else>
      <section class="content-section test-controls">
        <p v-if="kind !== 'latency' && device" class="device-name">
          {{ device.name }}
        </p>
        <component
          :is="panels[kind]"
          v-model:entries="results"
          :location="device?.location ?? ''"
          :run="run"
        />
        <p v-if="error" class="field-error" role="alert">{{ error }}</p>
      </section>
      <DiagnosticResultList
        :entries="results"
        @open="openResult"
        @clear="clearResults"
      />
    </template>
  </div>
</template>

<style scoped>
.test-controls {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 16px;
  padding-top: 0;
  padding-bottom: 28px;
  border-bottom: 0;
}
.device-name {
  color: var(--muted);
  font-size: 13px;
}
</style>
