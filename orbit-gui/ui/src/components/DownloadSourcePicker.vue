<script setup lang="ts">
import { onMounted, ref } from "vue";
import { Globe2 } from "lucide-vue-next";
import { run } from "../services/cli";
import { t } from "../i18n";
import DropdownSelect from "./upstream/DropdownSelect.vue";
const emit = defineEmits<{ changed: []; pending: [value: boolean] }>();
const props = withDefaults(defineProps<{ disabled?: boolean }>(), {
  disabled: false,
});
const source = ref(""),
  allowed = ref<string[]>([]),
  busy = ref(true),
  error = ref("");
function sourceLabel(value: string) {
  return value === "official"
    ? t("Official servers", "官方源")
    : value === "bmclapi"
      ? "BMCLAPI"
      : value || t("Download source", "下载源");
}
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
    <DropdownSelect
      size="sm"
      :disabled="disabled || busy"
      :loading="busy"
      :model-value="source"
      :searchable="false"
      :label="t('Game download source', '游戏下载源')"
      :options="
        allowed.map((value) => ({
          value,
          label: sourceLabel(value),
        }))
      "
      @update:model-value="change"
    >
      <template #selected>
        <Globe2 :size="15" class="source-icon" />
        <span class="source-value">{{ sourceLabel(source) }}</span>
      </template>
      <template v-if="allowed.includes('bmclapi')" #footer>
        <div class="source-attribution">
          <span>BMCLAPI · by bangbang93</span>
          <a
            href="https://bmclapidoc.bangbang93.com/"
            target="_blank"
            rel="noreferrer"
          >
            {{ t("Service terms", "使用协议") }}
          </a>
        </div>
      </template>
    </DropdownSelect>
    <p v-if="error" class="error">{{ error }}</p>
  </div>
</template>
<style scoped>
.download-source {
  position: relative;
  flex-shrink: 0;
}
.source-icon {
  color: var(--color-base);
}
.source-value {
  font-weight: 600;
  white-space: nowrap;
}
.source-attribution {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  font-size: 11px;
  line-height: 1.5;
  color: var(--color-base);
}
.download-source p {
  position: absolute;
  right: 0;
  top: calc(100% + 8px);
  width: 280px;
  max-width: calc(100vw - 40px);
  z-index: 13;
  padding: 10px 12px;
  border: 1px solid var(--color-red);
  border-radius: 8px;
  background: var(--surface-3);
  font-size: 12px;
  overflow-wrap: anywhere;
}
</style>
