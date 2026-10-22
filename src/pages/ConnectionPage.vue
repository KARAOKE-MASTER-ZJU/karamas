<script setup lang="ts">
import { useRouter } from "vue-router";
import RoomConnectionForm from "@/components/connection/RoomConnectionForm.vue";
import DeviceConnectionPanel from "@/components/connection/DeviceConnectionPanel.vue";
import { useOperationNotice } from "@/composables/useOperationNotice";
import { errorMessage } from "@/services/casting";

const router = useRouter();
const { notify } = useOperationNotice();
async function showPlayback() {
  try {
    await router.push({ name: "playback" });
  } catch (error) {
    notify(errorMessage(error), "error");
  }
}
</script>

<template>
  <div class="connection-page">
    <RoomConnectionForm />
    <DeviceConnectionPanel @connected="showPlayback" />
  </div>
</template>
