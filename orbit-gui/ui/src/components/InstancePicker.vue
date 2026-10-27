<script setup lang="ts">
import { computed } from "vue";
import type { Instance } from "../types";
import { imageUrl } from "../services/cli";
import { t } from "../i18n";
import DropdownSelect from "./upstream/DropdownSelect.vue";
const props = defineProps<{
  modelValue: string;
  instances: Instance[];
  label: string;
  disabled?: boolean;
  loading?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [id: string] }>();
const options = computed(() =>
  props.instances.map((instance) => ({
    value: instance.id,
    label: instance.name,
    description: [
      instance.game_version,
      instance.loader,
      instance.kind === "server"
        ? t("Server", "服务端")
        : t("Client", "客户端"),
    ]
      .filter(Boolean)
      .join(" · "),
    icon:
      imageUrl(instance.icon_path || undefined) ||
      (instance.kind === "server"
        ? "/upstream/terminal.png"
        : "/upstream/grass-block.png"),
  })),
);
</script>
<template>
  <DropdownSelect
    class="instance-picker"
    :model-value="modelValue"
    :options="options"
    :label="label"
    :placeholder="t('Choose an instance', '选择实例')"
    :disabled="disabled || !instances.length"
    :loading="loading"
    @update:model-value="(id) => emit('update:modelValue', id)"
  >
    <template #selected="{ option }">
      <img
        v-if="option?.icon"
        :src="option.icon"
        alt=""
        class="instance-picker-icon"
      />
      <span class="instance-picker-name" :title="option?.label">{{
        option?.label || t("Choose an instance", "选择实例")
      }}</span>
    </template>
  </DropdownSelect>
</template>
<style scoped>
.instance-picker {
  width: 100%;
}
.instance-picker :deep(.select-options .select-copy strong),
.instance-picker :deep(.select-options .select-copy small) {
  white-space: normal;
  overflow-wrap: anywhere;
}
.instance-picker-icon {
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  object-fit: contain;
}
.instance-picker-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}
</style>
