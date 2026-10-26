<script setup lang="ts">
import { onUnmounted, ref, watch } from "vue";
import { orbit } from "../services/workspace";
import { Cancelled } from "../services/cli";
import type { Instance } from "../types";
import { t } from "../i18n";
import ProgressSpinner from "./upstream/ProgressSpinner.vue";

interface CatalogEntry {
  key: string;
  value: string[];
  choices: { id: string; name: string; available: boolean }[];
}
const props = defineProps<{ instance: Instance; ready: boolean }>();
const emit = defineEmits<{
  changed: [];
  busy: [instanceId: string, value: boolean];
}>();
const settings = ref<CatalogEntry>();
const selected = ref<string[]>([]);
const loading = ref(false),
  saving = ref(false),
  error = ref("");
let revision = 0;
watch(
  () => [props.instance.id, props.ready] as const,
  async ([, ready]) => {
    const current = ++revision;
    settings.value = undefined;
    selected.value = [];
    error.value = "";
    loading.value = false;
    if (!ready) return;
    loading.value = true;
    try {
      const data = await orbit<{ entries: CatalogEntry[] }>(
        ["config", "--scope", "instance", "list"],
        t("Loading search sources", "正在读取搜索来源"),
        props.instance,
      );
      if (current === revision) {
        const entry = data.entries.find(
          (entry) => entry.key === "resolver.catalogs",
        );
        if (!entry)
          throw new Error(
            t(
              "The CLI did not return search source settings.",
              "CLI 未返回搜索来源设置。",
            ),
          );
        settings.value = entry;
        selected.value = [...entry.value];
      }
    } catch (e) {
      if (current === revision && !(e instanceof Cancelled))
        error.value = String(e);
    } finally {
      if (current === revision) loading.value = false;
    }
  },
  { immediate: true },
);
onUnmounted(() => {
  revision++;
});
async function save() {
  if (saving.value || !selected.value.length || !settings.value) return;
  const current = revision;
  const target = props.instance;
  // Retain the CLI-provided display order; no provider policy is implemented here.
  const enabled = settings.value.choices
    .filter((option) => selected.value.includes(option.id))
    .map((option) => option.id);
  saving.value = true;
  error.value = "";
  emit("busy", target.id, true);
  try {
    const data = await orbit<{ entry: CatalogEntry }>(
      [
        "config",
        "--scope",
        "instance",
        "set",
        "resolver.catalogs",
        enabled.join(","),
      ],
      t("Saving search sources", "正在保存搜索来源"),
      target,
    );
    if (current === revision) {
      settings.value = data.entry;
      selected.value = [...data.entry.value];
      emit("changed");
    }
  } catch (e) {
    if (current === revision) {
      selected.value = [...(settings.value?.value || [])];
      if (!(e instanceof Cancelled)) error.value = String(e);
    }
  } finally {
    saving.value = false;
    emit("busy", target.id, false);
  }
}
</script>
<template>
  <div class="catalog-settings">
    <div class="row wrap">
      <strong>{{ t("Search sources", "搜索来源") }}</strong>
      <ProgressSpinner v-if="loading" :size="16" />
      <label v-for="option in settings?.choices" :key="option.id">
        <input
          v-model="selected"
          type="checkbox"
          :value="option.id"
          :disabled="
            loading ||
            saving ||
            (selected.length === 1 && selected.includes(option.id)) ||
            (!option.available && !settings?.value.includes(option.id))
          "
          @change="save"
        />
        {{ option.name }}
        <small v-if="!option.available">{{
          t(
            "API key required in Orbit settings",
            "请先在 Orbit 设置中配置 API key",
          )
        }}</small>
      </label>
      <span v-if="saving" class="row" role="status"
        ><ProgressSpinner :size="16" />{{ t("Saving…", "正在保存…") }}</span
      >
    </div>
    <small>{{
      t(
        "Changes are saved automatically for this instance. Existing package remotes are unchanged.",
        "修改后自动按实例保存，不改变已有模组的远端。",
      )
    }}</small>
    <p v-if="error" class="error" role="alert">{{ error }}</p>
  </div>
</template>
<style scoped>
.catalog-settings {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13px;
}
.catalog-settings .row {
  gap: 10px 16px;
}
.catalog-settings label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.catalog-settings small {
  color: var(--color-base);
}
</style>
