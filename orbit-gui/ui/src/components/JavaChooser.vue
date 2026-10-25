<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, RefreshCw } from "lucide-vue-next";
import { launcher, act } from "../services/workspace";
import { t } from "../i18n";
import Button from "./upstream/Button.vue";
import DropdownSelect from "./upstream/DropdownSelect.vue";
interface Java {
  executable: string;
  provider: string;
  major: number;
  version: string;
}
const props = defineProps<{
  modelValue?: string | null;
  inherit?: boolean;
  disabled?: boolean;
  candidates?: Java[];
}>();
const emit = defineEmits<{ "update:modelValue": [value: string | null] }>();
const runtimes = ref<Java[]>([]);
const scanned = ref(false);
const loading = ref(false),
  error = ref("");
const system = computed(() =>
  (scanned.value
    ? runtimes.value
    : (props.candidates ?? runtimes.value)
  ).filter((java) => java.provider === "system"),
);
const chosen = computed(
  () => props.modelValue ?? (props.inherit ? "inherit" : "auto"),
);
const custom = computed(
  () =>
    !["auto", "managed", "inherit"].includes(chosen.value) &&
    !system.value.some((java) => java.executable === chosen.value),
);
const options = computed(() => [
  ...(props.inherit
    ? [{ value: "inherit", label: t("Use global setting", "使用全局设置") }]
    : []),
  {
    value: "auto",
    label: t("Automatic — local Java first", "自动选择 — 优先本地 Java"),
  },
  { value: "managed", label: t("Mojang managed Java", "Mojang 托管 Java") },
  ...system.value.map((java) => ({
    value: java.executable,
    label: `Java ${java.major} · ${java.version}`,
    description: java.executable,
  })),
  ...(custom.value ? [{ value: chosen.value, label: chosen.value }] : []),
]);
async function discover() {
  loading.value = true;
  error.value = "";
  try {
    runtimes.value = (
      await launcher<{ runtimes: typeof runtimes.value }>(
        ["java", "list"],
        t("Discovering Java", "正在检测 Java"),
        null,
      )
    ).runtimes;
    scanned.value = true;
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}
async function chooseFile() {
  const path = await open({ multiple: false });
  if (typeof path === "string") emit("update:modelValue", path);
}
onMounted(() => {
  if (!props.candidates) void discover();
});
</script>
<template>
  <div class="stack">
    <div class="row wrap">
      <DropdownSelect
        class="grow"
        :model-value="chosen"
        :options="options"
        :disabled="disabled"
        :loading="loading"
        :label="t('Java selection', 'Java 选择')"
        @update:model-value="
          (value) =>
            emit('update:modelValue', value === 'inherit' ? null : value)
        "
      />
      <Button :disabled="disabled" @click="act(chooseFile)"
        ><FolderOpen />{{ t("Choose Java…", "选择 Java…") }}</Button
      ><Button
        :disabled="loading"
        :title="t('Rescan', '重新检测')"
        @click="discover"
        ><RefreshCw
      /></Button>
    </div>
    <small>{{
      t(
        "Automatic uses a matching local Java version and architecture. Only if none is available will installation download Mojang Java. Explicit choices never silently fall back.",
        "自动选择匹配版本与架构的本地 Java；仅在没有合适版本时，安装流程才下载 Mojang Java。明确指定的 Java 不会被静默替换。",
      )
    }}</small>
    <p v-if="error" class="error">{{ error }}</p>
  </div>
</template>
