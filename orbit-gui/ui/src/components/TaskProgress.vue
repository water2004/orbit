<script setup lang="ts">
import { computed } from "vue";
import { cancel, type Task } from "../services/cli";
import { CheckCircle2 } from "lucide-vue-next";
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
      <ProgressSpinner v-if="active" /><CheckCircle2
        v-else-if="task.state === 'succeeded'"
        :size="18"
        class="success"
      /><strong class="grow">{{ task.outcome?.title || task.label }}</strong
      ><Button
        v-if="active"
        size="sm"
        type="quiet"
        @click="act(() => cancel(task))"
        >{{ t("Cancel", "取消") }}</Button
      >
    </div>
    <p v-if="task.state !== 'succeeded' || !task.outcome">
      {{
        task.state === "succeeded"
          ? t("Completed", "已完成")
          : task.message ||
            (task.state === "cancelled"
              ? t("Cancelled", "已取消")
              : t("Working…", "正在处理…"))
      }}
    </p>
    <details
      v-if="task.outcome?.notices.length"
      class="outcome-details"
      :open="task.outcome.detailsExpanded"
    >
      <summary>
        {{ t("Result details", "查看结果与原因") }} ·
        {{ task.outcome.notices.length + task.outcome.omitted }}
      </summary>
      <article v-for="(notice, index) in task.outcome.notices" :key="index">
        <strong>{{ notice.title }}</strong>
        <ul>
          <li v-for="(line, i) in notice.lines" :key="i">{{ line }}</li>
        </ul>
      </article>
      <small v-if="task.outcome.omitted">{{
        t(
          `${task.outcome.omitted} more notices in CLI output`,
          `另有 ${task.outcome.omitted} 条提示，可通过命令行查看完整输出`,
        )
      }}</small>
    </details>
    <p v-if="task.observationNotice" class="observation-notice">
      {{
        t(
          "Started without runtime data observation. New file ownership is not recorded.",
          "此次启动未启用运行时文件观测，不会记录新的文件归属。",
        )
      }}
      {{ task.observationNotice }}
    </p>
    <progress
      v-if="active"
      :value="task.total ? task.completed : undefined"
      :max="task.total || 1"
      :aria-label="task.label"
    />
    <small v-if="task.downloadSource"
      ><a
        :href="task.downloadSource.documentation"
        target="_blank"
        rel="noreferrer"
        >{{ task.downloadSource.name }}</a
      ></small
    >
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
.success {
  color: var(--color-green);
  flex-shrink: 0;
}
.outcome-details {
  margin-top: 8px;
  max-height: 180px;
  overflow: auto;
  overscroll-behavior: contain;
  font-size: 13px;
}
.outcome-details article {
  margin-top: 10px;
}
.outcome-details ul {
  margin: 4px 0 0;
  padding-left: 18px;
}
.outcome-details li {
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}
</style>
