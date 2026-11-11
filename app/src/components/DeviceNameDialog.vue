<script setup lang="ts">
import { ref } from "vue";
import AppDialog from "@/components/base/AppDialog.vue";
import FormField from "@/components/base/FormField.vue";
import { usePreferencesStore } from "@/stores/preferences";
import { useOperationNotice } from "@/composables/useOperationNotice";

const props = defineProps<{ defaultName: string }>();
const emit = defineEmits<{ close: [] }>();
const store = usePreferencesStore();
const { notify } = useOperationNotice();
// 预填实际生效的名字，用户才知道默认是哪个。
const name = ref(store.preferences.deviceName || props.defaultName);

function save(close: () => void) {
  const value = name.value.trim();
  if (!store.saveDeviceName(value === props.defaultName ? "" : value)) {
    notify("无法保存设备名称，请检查本机存储是否可用。", "error");
    return;
  }
  close();
}
</script>

<template>
  <AppDialog title="设备名称" close-label="关闭设备名称" @close="emit('close')">
    <template #default="{ close }">
      <form class="device-name-form" @submit.prevent="save(close)">
        <FormField
          id="device-name"
          v-model="name"
          aria-label="设备名称"
          :placeholder="defaultName"
          maxlength="80"
          autocomplete="off"
        />
        <div class="device-name-actions">
          <button type="button" class="button secondary" @click="close">
            取消
          </button>
          <button type="submit" class="button primary">保存</button>
        </div>
      </form>
    </template>
  </AppDialog>
</template>

<style scoped>
.device-name-form {
  display: flex;
  flex-direction: column;
  gap: 20px;
}
.device-name-actions {
  display: flex;
  justify-content: flex-end;
  gap: 12px;
}
</style>
