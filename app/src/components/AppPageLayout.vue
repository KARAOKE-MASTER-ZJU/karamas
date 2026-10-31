<script setup lang="ts">
import { RouterView, useRoute } from "vue-router";
import { useScrollRestoration } from "vue-router/experimental";

const route = useRoute();
useScrollRestoration();
</script>

<template>
  <RouterView v-slot="{ Component }">
    <KeepAlive>
      <component
        :is="Component"
        v-if="route.name !== 'playback'"
        :key="route.name"
      />
    </KeepAlive>
  </RouterView>
</template>
