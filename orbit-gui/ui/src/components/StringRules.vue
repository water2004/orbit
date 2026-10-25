<script setup lang="ts">
import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-vue-next";
import type { StringRule } from "../types";
import Button from "./upstream/Button.vue";
import { t } from "../i18n";
const draft = defineModel<StringRule>({ required: true });
function add() {
  draft.value.operations.push({
    operator: "intersect",
    negated: false,
    predicate: "contains",
    value: "",
    case_sensitive: false,
  });
}
function move(index: number, delta: number) {
  const rows = draft.value.operations;
  [rows[index], rows[index + delta]] = [rows[index + delta], rows[index]];
}
</script>
<template>
  <div class="stack">
    <p>
      {{
        t(
          "Conditions run from top to bottom on the full version string. Numeric rules are separate.",
          "按从上到下的顺序匹配完整版本字符串；数字规则独立设置。",
        )
      }}
    </p>
    <label
      >{{ t("Start with", "起始集合")
      }}<select v-model="draft.initial_all">
        <option :value="true">{{ t("All versions", "全部版本") }}</option>
        <option :value="false">{{ t("No versions", "空集") }}</option>
      </select></label
    >
    <div
      v-for="(operation, index) in draft.operations"
      :key="index"
      class="card stack"
    >
      <div class="row wrap">
        <span class="badge">{{ index + 1 }}</span
        ><select v-model="operation.operator">
          <option value="intersect">
            {{ t("Keep matching", "保留匹配项（交集）") }}
          </option>
          <option value="union">
            {{ t("Also include", "加入匹配项（并集）") }}
          </option>
          <option value="complement">
            {{ t("Invert the result so far", "反转当前结果（整体取补）") }}
          </option></select
        ><span class="grow" /><Button
          :disabled="index === 0"
          type="quiet"
          :aria-label="t('Move up', '上移')"
          @click="move(index, -1)"
          ><ArrowUp /></Button
        ><Button
          :disabled="index === draft.operations.length - 1"
          type="quiet"
          :aria-label="t('Move down', '下移')"
          @click="move(index, 1)"
          ><ArrowDown /></Button
        ><Button
          type="quiet"
          :aria-label="t('Delete condition', '删除条件')"
          @click="draft.operations.splice(index, 1)"
          ><Trash2
        /></Button>
      </div>
      <div v-if="operation.operator !== 'complement'" class="row wrap">
        <label
          ><input v-model="operation.negated" type="checkbox" />{{
            t("Not", "取反")
          }}</label
        >
        <select v-model="operation.predicate">
          <option value="equals">{{ t("Equals", "精确等于") }}</option>
          <option value="contains">{{ t("Contains", "包含") }}</option>
          <option value="starts_with">{{ t("Starts with", "开头为") }}</option>
          <option value="ends_with">{{ t("Ends with", "结尾为") }}</option>
          <option value="empty">{{ t("Is empty", "为空") }}</option>
          <option value="present">{{ t("Is not empty", "非空") }}</option>
        </select>
        <input
          v-if="!['empty', 'present'].includes(operation.predicate)"
          v-model="operation.value"
          class="grow"
          :aria-label="t('Exact text', '匹配文本')"
        />
        <label v-if="!['empty', 'present'].includes(operation.predicate)"
          ><input v-model="operation.case_sensitive" type="checkbox" />{{
            t("Case sensitive", "区分大小写")
          }}</label
        >
      </div>
    </div>
    <Button @click="add"><Plus />{{ t("Add condition", "添加条件") }}</Button>
  </div>
</template>
