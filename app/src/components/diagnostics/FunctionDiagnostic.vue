<script setup lang="ts">
import { ref, watch } from "vue";
import FormField from "@/components/base/FormField.vue";
import { castingApi } from "@/services/casting";
import type {
  DiagnosticEntry,
  DiagnosticPanelProps,
  FunctionCheck,
} from "@/types/diagnostics";

const props = defineProps<DiagnosticPanelProps>();
const entries = defineModel<DiagnosticEntry[]>("entries", { required: true });
const songUrl = ref("https://www.bilibili.com/video/BV1GJ411x7h7/");
const checks = ref<FunctionCheck[]>([]);
const states: Record<FunctionCheck["state"], string> = {
  running: "检查中",
  passed: "成功",
  failed: "失败",
  undeclared: "未声明",
  skipped: "未执行",
};
watch(
  () => props.location,
  () => {
    checks.value = [];
  },
);

function start() {
  return props.run(async (current) => {
    checks.value = [];
    const results = checks.value;
    await castingApi.diagnosticFunctions(
      props.location,
      songUrl.value.trim(),
      (check) => {
        if (!current()) return;
        const index = results.findIndex(
          (item) =>
            item.title === check.title && item.service === check.service,
        );
        if (index === -1) results.push(check);
        else results[index] = check;
        if (check.response)
          entries.value.unshift({ title: check.title, result: check.response });
      },
    );
  });
}
</script>

<template>
  <div class="diagnostic-form">
    <form
      class="diagnostic-control-row function-source"
      @submit.prevent="start"
    >
      <FormField
        id="function-video"
        v-model="songUrl"
        label="测试视频"
        type="url"
        required
      />
      <button class="button primary">自动测试</button>
    </form>
    <ul v-if="checks.length" class="function-checks" aria-label="功能检查结果">
      <li
        v-for="check in checks"
        :key="`${check.service}:${check.title}`"
        class="check-row"
      >
        <div class="check-main">
          <span class="check-title">{{ check.title }}</span>
          <span class="check-action"
            >{{ check.service }} · {{ check.action }}</span
          >
          <span v-if="check.detail" class="check-detail">{{
            check.detail
          }}</span>
        </div>
        <span
          class="check-state"
          :class="{
            failed: check.state === 'failed',
            muted: check.state === 'undeclared' || check.state === 'skipped',
          }"
          >{{ states[check.state] }}</span
        >
      </li>
    </ul>
  </div>
</template>

<style scoped>
.function-source {
  width: 100%;
  align-items: flex-end;
}
.function-checks {
  width: 100%;
  padding: 0;
  margin: 0;
  list-style: none;
}
.check-row {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 12px 0;
  border-bottom: 1px solid var(--line);
}
.check-main {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
  gap: 4px;
}
.check-title,
.check-state {
  font-size: 14px;
  line-height: 20px;
}
.check-title {
  font-weight: 500;
}
.check-action,
.check-detail {
  color: var(--muted);
  font-size: 12px;
  line-height: 18px;
  overflow-wrap: anywhere;
}
.check-state {
  flex-shrink: 0;
}
.failed {
  color: var(--error);
}
.muted {
  color: var(--muted);
}
</style>
