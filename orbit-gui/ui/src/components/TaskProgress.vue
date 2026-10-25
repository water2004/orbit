<script setup lang="ts">
import { computed } from "vue";
import { cancel, type Task } from "../services/cli";
import { act } from "../services/workspace";
import { t } from "../i18n";
import Button from "./upstream/Button.vue";
import ProgressSpinner from "./upstream/ProgressSpinner.vue";
const props = defineProps<{ task: Task }>();
const active = computed(() =>
  ["running", "waiting"].includes(props.task.state),
);
</script>
<template>
  <div
    class="task-progress"
    role="status"
    :aria-busy="active"
    :class="{ error: task.state === 'failed' }"
  >
    <div class="row">
      <ProgressSpinner v-if="active" /><strong class="grow">{{
        task.label
      }}</strong
      ><Button
        v-if="active"
        size="sm"
        type="quiet"
        @click="act(() => cancel(task))"
        >{{ t("Cancel", "取消") }}</Button
      >
    </div>
    <p>
      {{
        task.message ||
        (task.state === "succeeded"
          ? t("Completed", "已完成")
          : task.state === "cancelled"
            ? t("Cancelled", "已取消")
            : t("Working…", "正在处理…"))
      }}
    </p>
    <progress
      v-if="active"
      :value="task.total ? task.completed : undefined"
      :max="task.total || 1"
      :aria-label="task.label"
    />
  </div>
</template>
<style scoped>
.task-progress {
  padding: 12px 20px;
  background: var(--surface-2);
  border-top: 1px solid var(--surface-4);
  min-width: 0;
  flex-shrink: 0;
}
.task-progress p {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 96px;
  overflow: auto;
  margin: 4px 0 0;
  font-size: 13px;
}
progress {
  width: 100%;
  height: 4px;
  accent-color: var(--color-brand);
  display: block;
  margin-top: 9px;
}
</style>
