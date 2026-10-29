<script setup lang="ts" generic="T extends string | number">
import { Check, RefreshCw } from "@lucide/vue";
import AppDialog from "@/components/base/AppDialog.vue";

defineProps<{
  title: string;
  closeLabel: string;
  refreshLabel: string;
  listLabel: string;
  options: {
    id: T;
    label: string;
    description?: string;
    detail?: string;
    disabled?: boolean;
  }[];
  selectedId: T | null;
  loading: boolean;
  loadingMessage: string;
  disabled?: boolean;
  error?: string;
  emptyMessage?: string;
  message?: string;
}>();
const emit = defineEmits<{
  close: [];
  refresh: [];
  select: [id: T];
}>();
</script>

<template>
  <AppDialog :title="title" :close-label="closeLabel" @close="emit('close')">
    <template #actions>
      <button
        type="button"
        class="icon-button"
        :aria-label="refreshLabel"
        :title="refreshLabel"
        :disabled="loading || disabled"
        @click="emit('refresh')"
      >
        <RefreshCw :size="18" aria-hidden="true" />
      </button>
    </template>
    <p v-if="loading" class="selection-message" role="status">
      {{ loadingMessage }}
    </p>
    <p v-else-if="error" class="field-error" role="alert">{{ error }}</p>
    <p
      v-else-if="message || (!options.length && emptyMessage)"
      class="selection-message"
      role="status"
    >
      {{ message || emptyMessage }}
    </p>
    <ul
      v-if="!loading && options.length"
      class="selection-list"
      :aria-label="listLabel"
    >
      <li v-for="option in options" :key="option.id">
        <button
          type="button"
          class="selection-option"
          :class="{ selected: option.id === selectedId }"
          :aria-pressed="option.id === selectedId"
          :disabled="disabled || option.disabled"
          @click="emit('select', option.id)"
        >
          <span class="selection-label">
            <span>{{ option.label }}</span>
            <span v-if="option.description" class="selection-description">{{
              option.description
            }}</span>
          </span>
          <span v-if="option.detail" class="selection-detail">{{
            option.detail
          }}</span>
          <Check
            v-if="option.id === selectedId"
            :size="16"
            aria-hidden="true"
          />
        </button>
      </li>
    </ul>
  </AppDialog>
</template>

<style scoped>
.selection-message {
  font-size: 13px;
  line-height: 1.6;
  color: var(--muted);
  margin-bottom: 12px;
}
.selection-list {
  list-style: none;
  padding: 0;
  margin: 0;
  max-height: min(360px, 50dvh);
  overflow-y: auto;
}
.selection-option {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 12px 4px;
  border: 0;
  border-bottom: 1px solid var(--line);
  background: transparent;
  text-align: left;
  font-size: 14px;
}
.selection-option:hover:not(:disabled) {
  background: var(--subtle);
}
.selection-option.selected {
  color: var(--accent);
}
.selection-label {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
}
.selection-detail {
  font-size: 12px;
  color: var(--muted);
  white-space: nowrap;
}
.selection-description {
  display: block;
  margin-top: 4px;
  color: var(--muted);
  font-size: 12px;
  line-height: 1.6;
}
.selection-option > svg {
  flex-shrink: 0;
}
</style>
