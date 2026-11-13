import { defineStore } from "pinia";
import { shallowRef } from "vue";
import type { DiagnosticEntry } from "@/types/diagnostics";

export const useDiagnosticsStore = defineStore("diagnostics", () => {
  const selected = shallowRef<DiagnosticEntry | null>(null);
  return { selected };
});
