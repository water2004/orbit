<script setup lang="ts">
import { activity } from "../services/cli";
import { t } from "../i18n";
import Modal from "./Modal.vue";
import Button from "./upstream/Button.vue";
</script>
<template>
  <Modal
    v-if="activity.diagnostics"
    :title="t('Game exited unexpectedly', '游戏异常退出')"
    wide
    @close="activity.diagnostics = null"
  >
    <div class="stack">
      <p v-if="!activity.diagnostics.reports.length">
        {{
          t(
            "No new crash report was generated. The available log tail is shown below.",
            "没有生成新的崩溃报告。下方显示可用的日志末尾。",
          )
        }}
      </p>
      <article
        v-for="report in activity.diagnostics.reports"
        :key="report.path"
        class="stack"
      >
        <h3>{{ t("Crash report", "崩溃报告") }}</h3>
        <small class="report-path">{{ report.path }}</small>
        <pre class="report-content">{{ report.text }}</pre>
        <small v-if="report.truncated">{{
          t(
            "Preview truncated; the original file contains the full report.",
            "预览已截断，完整报告保存在原文件中。",
          )
        }}</small>
      </article>
      <details
        v-if="activity.diagnostics.log_tail"
        :open="!activity.diagnostics.reports.length"
      >
        <summary>{{ t("Latest game log", "本次游戏日志") }}</summary>
        <small class="report-path">{{
          activity.diagnostics.log_tail.path
        }}</small>
        <pre class="report-content">{{
          activity.diagnostics.log_tail.text
        }}</pre>
      </details>
      <p v-for="warning in activity.diagnostics.warnings" :key="warning">
        {{ warning }}
      </p>
    </div>
    <template #footer
      ><Button @click="activity.diagnostics = null">{{
        t("Close", "关闭")
      }}</Button></template
    >
  </Modal>
</template>
<style scoped>
.report-path {
  overflow-wrap: anywhere;
}
.report-content {
  max-height: 24rem;
  overflow: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 0.8rem;
  line-height: 1.5;
  padding: 1rem;
  background: var(--color-bg);
  border-radius: 0.75rem;
  user-select: text;
}
</style>
