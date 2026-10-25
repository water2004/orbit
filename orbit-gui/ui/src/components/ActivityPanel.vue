<script setup lang="ts">
import { computed } from "vue";
import { X, Activity, LoaderCircle } from "lucide-vue-next";
import { activity, cancel, answer, type Choice } from "../services/cli";
import { act } from "../services/workspace";
import { t, changeLabel } from "../i18n";
import type { Change } from "../types";
import Button from "./upstream/Button.vue";
import Modal from "./Modal.vue";
const current = computed(
  () =>
    [...activity.tasks]
      .reverse()
      .find((x) => ["running", "waiting"].includes(x.state)) ??
    activity.tasks.at(-1),
);
const pending = computed(() =>
  activity.tasks.find((x) => x.state === "waiting" && x.interaction),
);
function changes(choice: Choice): { change: Change; different: boolean }[] {
  const values = choice.data.changes;
  if (!Array.isArray(values)) return [];
  return values
    .filter((item) => item && typeof item === "object")
    .map((item) =>
      "change" in item ? item : { change: item, different: false },
    );
}
const common = computed(() =>
  pending.value?.interaction?.interaction === "resolution" &&
  pending.value.interaction.choices.length
    ? changes(pending.value.interaction.choices[0]).filter((x) => !x.different)
    : [],
);
const confirmation = computed(() =>
  ["confirmation", "data_deletion"].includes(
    pending.value?.interaction?.interaction || "",
  ),
);
const preview = computed(
  () =>
    pending.value?.interaction?.choices.find((c) => changes(c).length)?.data
      .changes as Change[] | undefined,
);
const document = computed(
  () =>
    pending.value?.interaction?.choices.find((c) => c.data.document)?.data
      .document as
      { text: string; url: string; digest_sha256: string } | undefined,
);
const deletion = computed(
  () =>
    pending.value?.interaction?.choices.find((c) => c.id === "proceed")?.data
      .entries as
      | {
          path: string;
          kind: string;
          scope: string;
          preserved: { path: string }[];
        }[]
      | undefined,
);
</script>
<template>
  <div v-if="current" class="activity-strip">
    <button class="activity-summary" @click="activity.open = !activity.open">
      <LoaderCircle
        v-if="['running', 'waiting'].includes(current.state)"
        class="spin"
        :size="16"
      /><Activity v-else :size="16" /><strong>{{ current.label }}</strong
      ><span class="truncate">{{ current.message }}</span>
    </button>
    <Button
      v-if="['running', 'waiting'].includes(current.state)"
      size="sm"
      type="quiet"
      @click="act(() => cancel(current!))"
      >{{ t("Cancel", "取消") }}</Button
    >
    <progress
      v-if="['running', 'waiting'].includes(current.state)"
      :value="current.total ? current.completed : undefined"
      :max="current.total || 1"
    />
  </div>
  <Teleport to="body"
    ><Transition name="slide"
      ><div
        v-if="activity.open"
        class="activity-backdrop"
        @click.self="activity.open = false"
      >
        <aside class="activity-drawer">
          <header class="row">
            <h2 class="grow">{{ t("Activity", "活动") }}</h2>
            <Button type="quiet" @click="activity.open = false"><X /></Button>
          </header>
          <section class="stack">
            <article
              v-for="task in [...activity.tasks].reverse()"
              :key="task.id"
              class="card stack"
            >
              <strong>{{ task.label }}</strong>
              <p :class="{ error: task.state === 'failed' }">
                {{ task.message }}
              </p>
              <progress
                v-if="['running', 'waiting'].includes(task.state)"
                :value="task.total ? task.completed : undefined"
                :max="task.total || 1"
              />
              <small v-else>{{
                task.state === "succeeded"
                  ? t("Completed", "已完成")
                  : task.state === "cancelled"
                    ? t("Cancelled", "已取消")
                    : t("Failed", "失败")
              }}</small>
              <Button
                v-if="['running', 'waiting'].includes(task.state)"
                size="sm"
                @click="act(() => cancel(task))"
                >{{ t("Cancel", "取消") }}</Button
              >
            </article>
          </section>
        </aside>
      </div></Transition
    ></Teleport
  >
  <Modal
    v-if="pending?.interaction"
    :key="pending.interaction.interaction_id"
    :title="pending.interaction.prompt.split('\n')[0]"
    wide
    :closable="false"
  >
    <div class="stack">
      <p
        v-if="pending.interaction.prompt.includes('\n')"
        class="interaction-context"
      >
        {{ pending.interaction.prompt.split("\n").slice(1).join("\n").trim() }}
      </p>
      <template v-if="confirmation"
        ><article v-if="document" class="stack">
          <pre class="legal-document">{{ document.text }}</pre>
          <p>{{ document.url }}</p>
          <small>SHA-256: {{ document.digest_sha256 }}</small>
        </article>
        <div v-for="change in preview" :key="change.package" class="change-row">
          <span class="badge">{{ changeLabel(change.kind) }}</span
          ><strong>{{ change.package }}</strong
          ><span
            >{{ change.current_version || "—" }} →
            {{ change.selected_version || "—" }}</span
          ><small v-if="change.selected_artifact">{{
            change.selected_artifact
          }}</small>
        </div>
        <div v-for="entry in deletion" :key="entry.path" class="card">
          <strong
            >{{ entry.path }}{{ entry.kind === "tree" ? "/**" : "" }}</strong
          >
          <p v-for="keep in entry.preserved" :key="keep.path">
            {{ t("Keep", "保留") }} · {{ keep.path }}
          </p>
        </div>
      </template>
      <template v-else>
        <div v-if="common.length" class="card stack">
          <h3>{{ t("In every plan", "所有方案共有") }}</h3>
          <div
            v-for="{ change } in common"
            :key="change.package"
            class="change-row"
          >
            <span class="badge">{{ changeLabel(change.kind) }}</span
            ><strong>{{ change.package }}</strong
            ><span
              >{{ change.current_version || "—" }} →
              {{ change.selected_version || "—" }}</span
            >
          </div>
        </div>
        <button
          v-for="choice in pending.interaction.choices"
          :key="choice.id"
          class="choice card stack"
          :disabled="pending.answering"
          @click="act(() => answer(pending!, choice.id))"
        >
          <h3>{{ choice.label }}</h3>
          <p v-if="choice.description">{{ choice.description }}</p>
          <div
            v-for="{ change, different } in changes(choice).filter(
              (x) =>
                pending?.interaction?.interaction !== 'resolution' ||
                x.different,
            )"
            :key="change.package"
            class="change-row"
          >
            <span v-if="different" class="difference">◆</span
            ><span class="badge">{{ changeLabel(change.kind) }}</span
            ><strong>{{ change.package }}</strong
            ><span
              >{{ change.current_version || "—" }} →
              {{ change.selected_version || "—" }}</span
            ><small v-if="change.selected_artifact">{{
              change.selected_artifact
            }}</small>
          </div>
        </button>
      </template>
    </div>
    <template #footer>
      <template v-if="confirmation"
        ><Button
          v-for="choice in pending.interaction.choices"
          :key="choice.id"
          :disabled="pending.answering"
          :type="
            choice.id === pending.interaction.default_choice
              ? 'base'
              : 'colored'
          "
          @click="act(() => answer(pending!, choice.id))"
          >{{ choice.label }}</Button
        ></template
      >
      <Button
        v-else-if="pending.interaction.allow_cancel"
        :disabled="pending.answering"
        @click="act(() => answer(pending!, null))"
        >{{ t("Cancel operation", "取消操作") }}</Button
      >
    </template>
  </Modal>
</template>
<style scoped>
.interaction-context {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.legal-document {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font: inherit;
}
.activity-strip {
  height: 44px;
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 20px;
  border-top: 1px solid var(--surface-4);
  flex-shrink: 0;
  background: var(--surface-2);
}
.activity-summary {
  border: 0;
  background: none;
  display: flex;
  gap: 10px;
  align-items: center;
  flex: 1;
  cursor: pointer;
  overflow: hidden;
  text-align: left;
}
.activity-summary strong {
  white-space: nowrap;
  max-width: 40%;
  overflow: hidden;
  text-overflow: ellipsis;
}
progress {
  width: 100%;
  height: 4px;
  accent-color: var(--color-brand);
  border: 0;
}
.activity-strip > progress {
  position: absolute;
  bottom: 0;
  left: 0;
}
.activity-backdrop {
  position: fixed;
  inset: 0;
  z-index: 30;
  background: #0002;
}
.activity-drawer {
  width: min(440px, 90vw);
  height: 100%;
  position: absolute;
  right: 0;
  background: var(--surface-2);
  box-shadow: -10px 0 50px #0003;
  display: flex;
  flex-direction: column;
}
.activity-drawer > header {
  padding: 20px;
}
.activity-drawer > section {
  padding: 0 16px 16px;
  overflow: auto;
  overscroll-behavior: contain;
}
.activity-drawer p {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.choice {
  text-align: left;
  cursor: pointer;
  width: 100%;
  transition:
    border-color 0.15s,
    background 0.15s;
}
.choice:hover,
.choice:focus-visible {
  border-color: var(--color-brand);
  background: var(--surface-4);
}
.change-row {
  display: flex;
  gap: 9px;
  flex-wrap: wrap;
  align-items: center;
}
.change-row strong {
  flex: 1;
}
.change-row small {
  flex-basis: 100%;
  overflow-wrap: anywhere;
}
.difference {
  color: var(--color-brand);
}
</style>
