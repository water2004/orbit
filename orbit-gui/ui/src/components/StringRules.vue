<script setup lang="ts">
import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-vue-next";
import type { StringRule } from "../types";
import Button from "./upstream/Button.vue";
import { t } from "../i18n";
import DropdownSelect from "./upstream/DropdownSelect.vue";
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
      }}<DropdownSelect
        :model-value="draft.initial_all ? 'all' : 'none'"
        :label="t('Start with', '起始集合')"
        :searchable="false"
        :options="[
          { value: 'all', label: t('All versions', '全部版本') },
          { value: 'none', label: t('No versions', '空集') },
        ]"
        @update:model-value="(value) => (draft.initial_all = value === 'all')"
    /></label>
    <div
      v-for="(operation, index) in draft.operations"
      :key="index"
      class="card stack"
    >
      <div class="row wrap">
        <span class="badge">{{ index + 1 }}</span
        ><DropdownSelect
          :model-value="operation.operator"
          :label="t('Set operation', '集合运算')"
          :searchable="false"
          :options="[
            {
              value: 'intersect',
              label: t('Keep matching', '保留匹配项（交集）'),
            },
            { value: 'union', label: t('Also include', '加入匹配项（并集）') },
            {
              value: 'complement',
              label: t('Invert the result so far', '反转当前结果（整体取补）'),
            },
          ]"
          @update:model-value="
            (value) => (operation.operator = value as typeof operation.operator)
          "
        />
        <span class="grow" /><Button
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
        <DropdownSelect
          :model-value="operation.predicate"
          :label="t('String comparison', '字符串比较')"
          :searchable="false"
          :options="[
            { value: 'equals', label: t('Equals', '精确等于') },
            { value: 'contains', label: t('Contains', '包含') },
            { value: 'starts_with', label: t('Starts with', '开头为') },
            { value: 'ends_with', label: t('Ends with', '结尾为') },
            { value: 'empty', label: t('Is empty', '为空') },
            { value: 'present', label: t('Is not empty', '非空') },
          ]"
          @update:model-value="
            (value) =>
              (operation.predicate = value as typeof operation.predicate)
          "
        />
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
