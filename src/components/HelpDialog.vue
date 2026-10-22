<script setup lang="ts">
import AppDialog from "@/components/base/AppDialog.vue";
const emit = defineEmits<{ close: [] }>();
</script>

<template>
  <AppDialog
    v-slot="{ close }"
    class="help-dialog"
    title="使用帮助"
    close-label="关闭帮助"
    @close="emit('close')"
  >
    <h3>投屏设备</h3>
    <p>DLNA 需要本机与电视连接同一网络。小电视云投屏需要登录哔哩哔哩账号。</p>
    <div class="dialog-actions">
      <button class="button primary" @click="close">知道了</button>
    </div>
  </AppDialog>
</template>

<style scoped>
.help-dialog h3 {
  margin: 20px 0 8px;
}
.help-dialog p {
  font-size: 14px;
  line-height: 1.8;
  margin-bottom: 16px;
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 24px;
}
</style>
