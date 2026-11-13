<script setup lang="ts">
import { computed, ref, watch } from "vue";
import FormField from "@/components/base/FormField.vue";
import SelectField from "@/components/base/SelectField.vue";
import { castingApi } from "@/services/casting";
import type {
  DeviceInfo,
  DiagnosticEntry,
  DiagnosticPanelProps,
  ServiceInfo,
} from "@/types/diagnostics";

const props = defineProps<DiagnosticPanelProps>();
const entries = defineModel<DiagnosticEntry[]>("entries", { required: true });
const description = ref<DeviceInfo | null>(null);
const serviceInfo = ref<ServiceInfo | null>(null);
const serviceId = ref("");
const actionName = ref("");
const parameters = ref<Record<string, string>>({});
const selectedAction = computed(() =>
  serviceInfo.value?.actions.find((action) => action.name === actionName.value),
);
const inputArguments = computed(
  () =>
    selectedAction.value?.arguments.filter((arg) => arg.direction === "in") ??
    [],
);

function variable(name: string) {
  return serviceInfo.value?.stateVariables.find((state) => state.name === name);
}
watch(selectedAction, (action) => {
  parameters.value = Object.fromEntries(
    (action?.arguments ?? [])
      .filter((arg) => arg.direction === "in")
      .map((arg) => {
        const state = variable(arg.relatedStateVariable);
        return [
          arg.name,
          state?.defaultValue ||
            (arg.name === "InstanceID" || arg.name === "ConnectionID"
              ? "0"
              : arg.name === "Channel"
                ? "Master"
                : (state?.allowedValues[0] ?? "")),
        ];
      }),
  );
});
watch(
  () => props.location,
  () => {
    description.value = null;
    serviceInfo.value = null;
    serviceId.value = "";
    actionName.value = "";
  },
);

function readParameters() {
  return props.run(async (current) => {
    const address = props.location;
    const info = await castingApi.diagnosticDevice(address);
    if (!current()) return;
    description.value = info;
    entries.value.unshift({
      title: "设备描述",
      result: info.response,
      services: info.services,
    });
    const id = info.services.some(
      (service) => service.serviceId === serviceId.value,
    )
      ? serviceId.value
      : info.services[0]?.serviceId;
    if (!id) return;
    serviceId.value = id;
    const detail = await castingApi.diagnosticService(address, id);
    if (!current()) return;
    serviceInfo.value = detail;
    actionName.value =
      detail.actions.find((action) => action.name.startsWith("Get"))?.name ??
      detail.actions[0]?.name ??
      "";
    entries.value.unshift({ title: "服务描述", result: detail.response });
  });
}
function query() {
  return props.run(async (current) => {
    const name = actionName.value;
    const result = await castingApi.diagnosticQuery(
      props.location,
      serviceId.value,
      name,
      { ...parameters.value },
    );
    if (current()) entries.value.unshift({ title: name, result });
  });
}
</script>

<template>
  <div class="diagnostic-form">
    <div class="diagnostic-control-row diagnostic-inline-fields">
      <SelectField
        v-if="description?.services.length"
        id="diagnostic-service"
        v-model="serviceId"
        label="服务"
        @change="readParameters"
      >
        <option
          v-for="service in description.services"
          :key="service.serviceId"
          :value="service.serviceId"
          :title="service.serviceType"
        >
          {{ service.serviceType.split(":").slice(-2).join(":") }}
        </option>
      </SelectField>
      <button
        type="button"
        class="button"
        :class="serviceInfo ? 'secondary' : 'primary'"
        @click="readParameters"
      >
        {{ description ? "刷新" : "读取服务" }}
      </button>
    </div>
    <template v-if="serviceInfo?.actions.length">
      <div class="diagnostic-control-row diagnostic-inline-fields">
        <SelectField id="diagnostic-action" v-model="actionName" label="动作"
          ><option v-for="action in serviceInfo.actions" :key="action.name">
            {{ action.name }}
          </option></SelectField
        >
        <button
          v-if="actionName.startsWith('Get')"
          type="submit"
          form="diagnostic-query"
          class="button primary"
        >
          查询
        </button>
      </div>
      <form
        v-if="actionName.startsWith('Get')"
        id="diagnostic-query"
        class="diagnostic-form parameter-fields"
        @submit.prevent="query"
      >
        <template v-for="arg in inputArguments" :key="arg.name">
          <SelectField
            v-if="variable(arg.relatedStateVariable)?.allowedValues.length"
            :id="`parameter-${arg.name}`"
            :model-value="parameters[arg.name] ?? ''"
            :label="arg.name"
            @update:model-value="parameters[arg.name] = $event"
            ><option
              v-for="value in variable(arg.relatedStateVariable)?.allowedValues"
              :key="value"
            >
              {{ value }}
            </option></SelectField
          >
          <FormField
            v-else
            :id="`parameter-${arg.name}`"
            :model-value="parameters[arg.name] ?? ''"
            :label="arg.name"
            @update:model-value="parameters[arg.name] = $event"
          />
        </template>
      </form>
      <details class="diagnostic-details">
        <summary>动作参数</summary>
        <dl class="argument-list">
          <div v-for="arg in selectedAction?.arguments" :key="arg.name">
            <dt>
              {{ arg.name }} <span>{{ arg.direction }}</span>
            </dt>
            <dd>{{ arg.relatedStateVariable }}</dd>
          </div>
        </dl>
      </details>
      <details class="diagnostic-details">
        <summary>参数范围</summary>
        <dl class="argument-list">
          <div v-for="state in serviceInfo.stateVariables" :key="state.name">
            <dt>{{ state.name }}</dt>
            <dd>
              {{
                [
                  state.dataType,
                  state.allowedValues.join(" / "),
                  state.minimum && `${state.minimum} ～ ${state.maximum}`,
                  state.step && `步长 ${state.step}`,
                  state.defaultValue && `默认 ${state.defaultValue}`,
                ]
                  .filter(Boolean)
                  .join(" · ")
              }}
            </dd>
          </div>
        </dl>
      </details>
    </template>
  </div>
</template>

<style scoped>
.parameter-fields {
  display: grid;
  grid-template-columns: fit-content(140px) minmax(0, 200px);
  max-width: 400px;
  gap: 12px;
}
.parameter-fields > .field {
  display: contents;
}
.parameter-fields :deep(label) {
  align-self: center;
  overflow-wrap: anywhere;
}
.argument-list {
  width: 100%;
  margin: 12px 0 0;
  font-size: 13px;
}
.argument-list > div {
  display: grid;
  grid-template-columns: minmax(100px, 140px) minmax(0, 1fr);
  gap: 16px;
  padding: 8px 0;
  border-bottom: 1px solid var(--line);
}
.argument-list dt {
  overflow-wrap: anywhere;
}
.argument-list dt span {
  color: var(--muted);
}
.argument-list dd {
  margin: 0;
  color: var(--muted);
  overflow-wrap: anywhere;
}
</style>
