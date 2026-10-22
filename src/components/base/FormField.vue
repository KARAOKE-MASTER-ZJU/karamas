<script setup lang="ts">
defineOptions({ inheritAttrs: false });
defineProps<{ id: string; label: string; error?: string }>();
const model = defineModel<string>({ required: true });
</script>

<template>
  <div class="field">
    <label :for="id">{{ label }}</label>
    <input
      v-bind="$attrs"
      :id="id"
      v-model="model"
      :aria-invalid="!!error"
      :aria-describedby="error ? `${id}-error` : undefined"
    />
    <p v-if="error" :id="`${id}-error`" class="field-error">{{ error }}</p>
  </div>
</template>
