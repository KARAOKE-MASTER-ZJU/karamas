<script setup lang="ts">
import { computed, ref } from "vue";
import { RouterLink } from "vue-router";
import FormField from "@/components/base/FormField.vue";
import SelectField from "@/components/base/SelectField.vue";
import { usePreferencesStore } from "@/stores/preferences";
import { castingApi } from "@/services/casting";
import type {
  DiagnosticEntry,
  DiagnosticPanelProps,
} from "@/types/diagnostics";

const props = defineProps<DiagnosticPanelProps>();
const entries = defineModel<DiagnosticEntry[]>("entries", { required: true });
const preferences = usePreferencesStore();
const targets = {
  device: "设备",
  "bilibili-web": "B站网页",
  "bilibili-api": "B站 API",
  website: "点歌网站",
  custom: "自定义网址",
};
const target = ref<keyof typeof targets>("bilibili-api");
const customUrl = ref("");
const website = ref(preferences.preferences.serverUrl);
const url = computed(() => {
  switch (target.value) {
    case "device":
      return props.location;
    case "bilibili-web":
      return "https://www.bilibili.com/";
    case "bilibili-api":
      return "https://api.bilibili.com/x/web-interface/view?bvid=BV1GJ411x7h7";
    case "website":
      return website.value;
    default:
      return customUrl.value;
  }
});

function measure() {
  return props.run(async (current) => {
    const address = url.value.trim();
    if (!address)
      throw new Error(
        target.value === "device" ? "请先连接设备" : "请输入网址",
      );
    const lan = target.value === "device";
    const title = `${targets[target.value]}延迟`;
    for (let index = 1; index <= 5 && current(); index++) {
      const result = await castingApi.diagnosticLatency(address, lan);
      if (!current()) return;
      entries.value.push({ title: `${title} #${index}`, result });
    }
  });
}
</script>

<template>
  <form class="diagnostic-form" @submit.prevent="measure">
    <div class="diagnostic-control-row diagnostic-inline-fields network-target">
      <SelectField id="diagnostic-target" v-model="target" label="目标">
        <option v-for="(label, value) in targets" :key="value" :value="value">
          {{ label }}
        </option>
      </SelectField>
      <button
        v-if="target !== 'device' || location"
        class="button primary"
        aria-label="开始测试"
      >
        测试
      </button>
    </div>
    <FormField
      v-if="target === 'website'"
      id="diagnostic-website"
      v-model="website"
      label="网址"
      type="url"
      required
    />
    <FormField
      v-if="target === 'custom'"
      id="diagnostic-url"
      v-model="customUrl"
      label="网址"
      type="url"
      placeholder="https://"
      required
    />
    <div v-if="target === 'device' && !location" class="diagnostic-empty">
      <p>请先连接设备</p>
      <RouterLink class="button primary" :to="{ name: 'connection' }"
        >去连接</RouterLink
      >
    </div>
  </form>
</template>

<style scoped>
.network-target {
  max-width: 320px;
}
</style>
