<script setup lang="ts">
import { accountCommands } from "../services/commands";
import { computed, ref } from "vue";
import type { InstanceContext } from "../services/instance-context";
import { workspace, refresh } from "../services/workspace";
import { accountProviderLabel } from "../services/account-label";
import { useCommandScope } from "../services/task-scope";
import { t } from "../i18n";
import DropdownSelect from "./upstream/DropdownSelect.vue";
import TaskProgress from "./TaskProgress.vue";
const props = defineProps<{ context: InstanceContext }>();
const { launcher, task } = useCommandScope(props.context.state.instance);
const pending = ref(false);
const error = ref("");
const value = computed(
  () => props.context.state.detail?.selected_account_id || "inherit",
);
const options = computed(() => {
  const account = workspace.accounts.find((account) => account.is_default);
  const choices = [
    {
      value: "inherit",
      label: t("Use global default", "使用全局默认账户"),
      description:
        account?.profile_name || t("No default account", "未设置默认账户"),
    },
    ...workspace.accounts.map((account) => ({
      value: account.id,
      label: account.profile_name,
      description: accountProviderLabel(account.provider, account.provider_id),
    })),
  ];
  if (
    value.value !== "inherit" &&
    !workspace.accounts.some((account) => account.id === value.value)
  )
    choices.push({
      value: value.value,
      label: t("Account unavailable", "账户不可用"),
      description: value.value,
    });
  return choices;
});
async function change(id: string) {
  if (pending.value || id === value.value) return;
  pending.value = true;
  error.value = "";
  try {
    await launcher(
      accountCommands.instance(id === "inherit" ? undefined : id),
      t("Updating instance account", "正在更新实例账户"),
    );
    await props.context.refresh();
    await refresh();
  } catch (cause) {
    error.value = String(cause);
  } finally {
    pending.value = false;
  }
}
</script>
<template>
  <div class="card stack">
    <h2>{{ t("Launch account", "启动账户") }}</h2>
    <p>
      {{
        t(
          `Account used only by ${context.state.instance.name}.`,
          `仅设置 ${context.state.instance.name} 使用的账户。`,
        )
      }}
    </p>
    <DropdownSelect
      :model-value="value"
      :options="options"
      :disabled="pending"
      :label="t('Launch account', '启动账户')"
      @update:model-value="change"
    />
    <p v-if="workspace.accountsError" class="error">
      {{ workspace.accountsError }}
    </p>
    <p v-if="error" class="error" role="alert">{{ error }}</p>
    <TaskProgress v-if="pending && task" :task="task" />
  </div>
</template>
