<script setup lang="ts">
import { ref, watch } from "vue";
import { useRoute } from "vue-router";
import { workspace, selectInstance } from "../services/workspace";
import { t } from "../i18n";
const route = useRoute();
const ready = ref(false);
const error = ref("");
let revision = 0;
watch(
  [
    () => route.params.id,
    () => workspace.instances.some((i) => i.id === route.params.id),
  ],
  async () => {
    const request = ++revision;
    ready.value = false;
    error.value = "";
    const instance = workspace.instances.find((i) => i.id === route.params.id);
    if (!instance) {
      if (!workspace.loading)
        error.value = t("Instance not found", "找不到此实例");
      return;
    }
    try {
      if (workspace.selected?.id !== instance.id || !workspace.detail)
        await selectInstance(instance);
      if (request === revision) ready.value = true;
    } catch (e) {
      if (request === revision) error.value = String(e);
    }
  },
  { immediate: true },
);
</script>
<template>
  <RouterView v-if="ready" :key="String(route.params.id)" />
  <div v-else class="page">
    <RouterLink to="/">← {{ t("Library", "实例库") }}</RouterLink>
    <p :class="{ error }">
      {{ error || t("Loading instance…", "正在加载实例…") }}
    </p>
  </div>
</template>
