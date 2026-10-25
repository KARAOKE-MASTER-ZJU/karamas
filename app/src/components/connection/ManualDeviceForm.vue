<script setup lang="ts">
import FormField from "@/components/base/FormField.vue";

defineProps<{ error: string; resolving: boolean; disabled: boolean }>();
const deviceUrl = defineModel<string>({ required: true });
const emit = defineEmits<{ submit: [] }>();
</script>

<template>
  <form
    id="manual-connect-form"
    class="manual-form"
    novalidate
    @submit.prevent="emit('submit')"
  >
    <FormField
      id="device-url"
      v-model="deviceUrl"
      label="设备描述文件地址"
      :error="error"
      type="url"
      inputmode="url"
      placeholder="http://设备 IP:端口/description.xml"
    />
    <button
      type="submit"
      class="button secondary"
      :disabled="resolving || disabled"
    >
      {{ resolving ? "连接中…" : "连接" }}
    </button>
  </form>
</template>

<style scoped>
.manual-form {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  margin-top: 20px;
}
.manual-form .field {
  flex: 1;
}
.manual-form .button {
  margin-top: calc(var(--control-line-height) + 8px);
}
</style>
