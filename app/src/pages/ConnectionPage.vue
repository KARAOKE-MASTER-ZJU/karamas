<script setup lang="ts">
import { watch } from "vue";
import { useRouter } from "vue-router";
import { useCastingStore } from "@/stores/casting";
import RoomConnectionForm from "@/components/connection/RoomConnectionForm.vue";
import DeviceConnectionPanel from "@/components/connection/DeviceConnectionPanel.vue";

const casting = useCastingStore();
const router = useRouter();
// 本地连接成功后打开播放页，页面挂载时接收已到达的播放命令。
watch(
  () => casting.activeDevice,
  (device) => {
    if (device?.kind === "local") void router.push({ name: "playback" });
  },
);
</script>

<template>
  <div class="connection-page">
    <RoomConnectionForm />
    <DeviceConnectionPanel />
  </div>
</template>
