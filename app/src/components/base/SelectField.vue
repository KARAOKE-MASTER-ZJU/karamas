<script setup lang="ts" generic="T extends string">
import { ChevronDown } from "@lucide/vue";

defineOptions({ inheritAttrs: false });
defineProps<{ id: string; label: string }>();
const model = defineModel<T>({ required: true });
</script>

<template>
  <div class="field">
    <label :for="id">{{ label }}</label>
    <div class="select-control">
      <select v-bind="$attrs" :id="id" v-model="model">
        <slot />
      </select>
      <ChevronDown :size="16" aria-hidden="true" />
    </div>
  </div>
</template>
