<script setup lang="ts">
import AppDialog from "@/components/base/AppDialog.vue";
const emit = defineEmits<{ close: [] }>();
</script>

<template>
  <AppDialog
    class="help-dialog"
    title="使用帮助"
    close-label="关闭帮助"
    @close="emit('close')"
  >
    <p>DLNA 需要本机与电视连接同一网络。小电视云投屏需要登录哔哩哔哩账号。</p>
  </AppDialog>
</template>

<style scoped>
.help-dialog p {
  font-size: 14px;
  line-height: 1.8;
}
</style>
