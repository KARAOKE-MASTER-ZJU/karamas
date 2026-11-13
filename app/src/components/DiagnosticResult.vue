<script setup lang="ts">
import { Copy } from "@lucide/vue";
import type { DiagnosticEntry } from "@/types/diagnostics";
import { useOperationNotice } from "@/composables/useOperationNotice";

const props = defineProps<DiagnosticEntry>();
const { notify } = useOperationNotice();
async function copy() {
  try {
    await navigator.clipboard.writeText(
      JSON.stringify(
        { title: props.title, ...props.result, services: props.services },
        null,
        2,
      ),
    );
    notify("结果已复制");
  } catch {
    notify("无法访问剪贴板", "error");
  }
}
</script>

<template>
  <section class="test-result">
    <div class="result-heading">
      <h2>{{ title }}</h2>
      <span class="result-time">{{ result.elapsedMs }} ms</span>
      <button
        type="button"
        class="icon-button"
        aria-label="复制结果"
        @click="copy"
      >
        <Copy :size="18" aria-hidden="true" />
      </button>
    </div>
    <time
      class="result-timestamp"
      :datetime="new Date(result.time).toISOString()"
      >{{ new Date(result.time).toLocaleString("zh-CN", { hour12: false }) }}.{{
        String(new Date(result.time).getMilliseconds()).padStart(3, "0")
      }}</time
    >
    <p class="result-url">{{ result.url }}</p>
    <p
      v-if="result.status !== null"
      class="result-status"
      :class="{ error: result.status >= 400 }"
    >
      HTTP {{ result.status }}
    </p>
    <p v-if="result.error" class="field-error" role="status">
      {{ result.error }}
    </p>
    <dl v-if="Object.keys(result.fields).length">
      <div v-for="(value, name) in result.fields" :key="name">
        <dt>{{ name }}</dt>
        <dd>{{ value || "—" }}</dd>
      </div>
    </dl>
    <section v-if="services?.length" class="result-services">
      <h2>服务</h2>
      <details v-for="service in services" :key="service.serviceId">
        <summary>{{ service.serviceType }}</summary>
        <dl>
          <div v-for="(value, key) in service" :key="key">
            <dt>{{ key }}</dt>
            <dd>{{ value || "—" }}</dd>
          </div>
        </dl>
      </details>
    </section>
    <details v-if="result.request">
      <summary>请求详情</summary>
      <pre>{{ result.request }}</pre>
    </details>
    <details v-if="result.responseHeaders">
      <summary>响应头</summary>
      <p
        v-if="result.responseUrl && result.responseUrl !== result.url"
        class="result-url"
      >
        {{ result.responseUrl }}
      </p>
      <pre>{{ result.responseHeaders }}</pre>
    </details>
    <details v-if="result.raw">
      <summary>原始响应</summary>
      <pre>{{ result.raw }}</pre>
    </details>
  </section>
</template>

<style scoped>
.test-result {
  padding: 20px 0;
  border-bottom: 1px solid var(--line);
}
.result-heading {
  display: flex;
  align-items: center;
  gap: 12px;
}
.result-heading h2 {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
}
.result-time {
  color: var(--muted);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
}
.result-url {
  margin: 4px 0 12px;
  color: var(--muted);
  font-size: 12px;
  overflow-wrap: anywhere;
}
.result-timestamp {
  display: block;
  margin-top: 8px;
  color: var(--muted);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.result-services {
  margin: 20px 0;
}
.result-status {
  margin-bottom: 12px;
  font-size: 13px;
}
.error {
  color: var(--error);
}
dl {
  margin: 12px 0;
  font-size: 13px;
}
dl > div {
  display: grid;
  grid-template-columns: minmax(100px, 1fr) minmax(0, 2fr);
  gap: 16px;
  padding: 8px 0;
}
dt {
  color: var(--muted);
  overflow-wrap: anywhere;
}
dd {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
details {
  margin-top: 12px;
  font-size: 13px;
}
summary {
  cursor: pointer;
  width: fit-content;
}
pre {
  padding: 12px;
  background: var(--subtle);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 12px;
  line-height: 1.7;
  user-select: text;
}
</style>
