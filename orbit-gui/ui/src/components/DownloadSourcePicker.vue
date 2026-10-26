<script setup lang="ts">
import { onMounted, ref } from "vue";
import { run } from "../services/cli";
import { t } from "../i18n";
import DropdownSelect from "./upstream/DropdownSelect.vue";
import ProgressSpinner from "./upstream/ProgressSpinner.vue";
const emit = defineEmits<{ changed: []; pending: [value: boolean] }>();
const props = withDefaults(defineProps<{ disabled?: boolean }>(), {
  disabled: false,
});
const source = ref(""),
  allowed = ref<string[]>([]),
  busy = ref(true),
  error = ref("");
onMounted(async () => {
  emit("pending", true);
  try {
    const entry = await run<{ value: string; allowed_values: string[] }>({
      kind: "launcher",
      args: ["config", "get", "network.download-source"],
      label: t("Loading download source", "正在读取下载源"),
    });
    source.value = entry.value;
    allowed.value = entry.allowed_values;
  } catch (reason) {
    error.value = String(reason);
  } finally {
    busy.value = false;
    emit("pending", false);
  }
});
async function change(value: string) {
  if (busy.value || props.disabled || value === source.value) return;
  busy.value = true;
  emit("pending", true);
  error.value = "";
  try {
    await run({
      kind: "launcher",
      args: ["config", "set", "network.download-source", value],
      label: t("Updating download source", "正在更新下载源"),
    });
    source.value = value;
    emit("changed");
  } catch (reason) {
    error.value = String(reason);
  } finally {
    busy.value = false;
    emit("pending", false);
  }
}
</script>
<template>
  <div class="download-source">
    <div class="source-control">
      <span>{{ t("Download source", "下载源") }}</span
      ><ProgressSpinner v-if="busy" :size="14" />
    </div>
    <DropdownSelect
      :disabled="disabled || busy"
      :loading="busy"
      :model-value="source"
      :searchable="false"
      :label="t('Game download source', '游戏下载源')"
      :options="
        allowed.map((value) => ({
          value,
          label:
            value === 'official'
              ? t('Official servers', '官方源')
              : value === 'bmclapi'
                ? 'BMCLAPI'
                : value,
        }))
      "
      @update:model-value="change"
    />
    <small v-if="source === 'bmclapi'"
      >by bangbang93 ·
      <a
        href="https://bmclapidoc.bangbang93.com/"
        target="_blank"
        rel="noreferrer"
        >{{ t("Service terms", "使用协议") }}</a
      ></small
    >
    <p v-if="error" class="error">{{ error }}</p>
  </div>
</template>
<style scoped>
.download-source {
  display: flex;
  flex-direction: column;
  width: 174px;
  gap: 5px;
  flex-shrink: 0;
}
.source-control {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--color-secondary);
}
.download-source small {
  font-size: 11px;
  color: var(--color-secondary);
}
.download-source p {
  font-size: 12px;
  overflow-wrap: anywhere;
}
</style>
