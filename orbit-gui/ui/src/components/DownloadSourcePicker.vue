<script setup lang="ts">
import { onMounted, ref } from "vue";
import { run } from "../services/cli";
import { t } from "../i18n";
import DropdownSelect from "./upstream/DropdownSelect.vue";
import ProgressSpinner from "./upstream/ProgressSpinner.vue";
const emit = defineEmits<{ changed: [] }>();
const source = ref(""),
  allowed = ref<string[]>([]),
  busy = ref(true),
  error = ref("");
onMounted(async () => {
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
  }
});
async function change(value: string) {
  if (busy.value || value === source.value) return;
  busy.value = true;
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
  }
}
</script>
<template>
  <div class="download-source card">
    <label>{{ t("Game download source", "游戏下载源") }}</label>
    <ProgressSpinner v-if="busy" />
    <DropdownSelect
      v-else
      :model-value="source"
      :searchable="false"
      :label="t('Game download source', '游戏下载源')"
      :options="
        allowed.map((value) => ({
          value,
          label:
            value === 'official'
              ? t('Official servers', '官方源')
              : 'BMCLAPI · bangbang93',
        }))
      "
      @update:model-value="change"
    />
    <small v-if="source === 'bmclapi'"
      >BMCLAPI · by bangbang93 ·
      <a
        href="https://bmclapidoc.bangbang93.com/"
        target="_blank"
        rel="noreferrer"
        >{{ t("Service terms", "使用协议") }}</a
      ><br />{{
        t(
          "Public game downloads only. Authentication, pinned metadata and checksums stay upstream.",
          "仅加速公开游戏资源。账户认证、带摘要的元数据和校验文件仍使用原站。",
        )
      }}</small
    >
    <small v-else>{{
      t(
        "Saved through Launcher for future game, loader and Java downloads.",
        "通过 Launcher 保存，用于后续游戏、加载器和 Java 下载。",
      )
    }}</small>
    <p v-if="error" class="error">{{ error }}</p>
  </div>
</template>
<style scoped>
.download-source {
  display: grid;
  grid-template-columns: auto minmax(12rem, 20rem) 1fr;
  gap: 1rem;
  align-items: center;
}
@media (max-width: 900px) {
  .download-source {
    grid-template-columns: 1fr;
  }
}
</style>
