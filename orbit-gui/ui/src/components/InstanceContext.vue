<script setup lang="ts">
import { onBeforeUnmount, provide, ref, shallowRef, watch } from "vue";
import { useRoute } from "vue-router";
import { workspace } from "../services/workspace";
import {
  createInstanceContext,
  instanceContextKey,
  type InstanceContext,
} from "../services/instance-context";
import { t } from "../i18n";
import ProgressSpinner from "./upstream/ProgressSpinner.vue";
import Button from "./upstream/Button.vue";
const route = useRoute();
const context = shallowRef<InstanceContext>();
const generation = ref(0);
provide(instanceContextKey, context);
let revision = 0;
watch(
  [
    () => route.params.id,
    () => workspace.instances.find((i) => i.id === route.params.id),
  ],
  async () => {
    const request = ++revision;
    const instance = workspace.instances.find((i) => i.id === route.params.id);
    if (
      instance &&
      context.value?.state.instance.id === instance.id &&
      context.value.state.instance.directory === instance.directory
    ) {
      context.value.state.instance = { ...instance };
      await retry();
      return;
    }
    context.value?.dispose();
    context.value = undefined;
    ++generation.value;
    if (!instance) return;
    const next = createInstanceContext(instance);
    context.value = next;
    try {
      await next.refresh();
    } catch (e) {
      if (request === revision) next.state.error = String(e);
    }
  },
  { immediate: true },
);
async function retry() {
  try {
    await context.value?.refresh();
  } catch {
    /* The context exposes the error and the explicit retry action. */
  }
}
onBeforeUnmount(() => {
  ++revision;
  context.value?.dispose();
});
</script>
<template>
  <RouterView
    v-if="context?.state.detail && !context.state.error"
    :key="generation"
  />
  <div v-else class="page">
    <RouterLink to="/">← {{ t("Library", "实例库") }}</RouterLink>
    <p v-if="context?.state.error" class="error">{{ context.state.error }}</p>
    <p
      v-else-if="!context && workspace.initialized && !workspace.loading"
      class="error"
    >
      {{
        t(
          "Instance not found. Choose an instance from the library.",
          "找不到此实例，请从实例库选择。",
        )
      }}
    </p>
    <p v-else class="row">
      <ProgressSpinner />{{ t("Loading instance…", "正在加载实例…") }}
    </p>
    <Button v-if="context?.state.error" @click="retry">{{
      t("Retry", "重试")
    }}</Button>
  </div>
</template>
