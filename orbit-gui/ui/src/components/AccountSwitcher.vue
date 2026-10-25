<script setup lang="ts">
import { computed, ref } from "vue";
import { Check, ChevronDown, UserRound, Users } from "lucide-vue-next";
import { workspace, act, launcher } from "../services/workspace";
import type { Account, Detail } from "../types";
import { t } from "../i18n";
import ActionMenu from "./ActionMenu.vue";
import Avatar from "./Avatar.vue";
const props = defineProps<{ instanceId?: string }>();
const busy = ref(false);
const current = computed(() =>
  workspace.accounts.find((a) =>
    props.instanceId && workspace.detail?.selected_account_id
      ? a.id === workspace.detail.selected_account_id
      : a.is_default,
  ),
);
async function choose(account: Account) {
  const instance =
    workspace.instances.find((i) => i.id === props.instanceId) ?? null;
  busy.value = true;
  try {
    await launcher(
      ["account", "select", account.id, ...(instance ? [] : ["--global"])],
      t("Switching account", "正在切换账户"),
      instance,
    );
    if (instance) {
      const detail = await launcher<Detail>(
        ["instance", "show"],
        t("Loading installation", "正在加载实例"),
        instance,
      );
      if (workspace.selected?.id === instance.id) workspace.detail = detail;
    } else {
      const result = await launcher<{ accounts: Account[] }>(
        ["account", "list"],
        t("Loading accounts", "正在加载账户"),
        null,
      );
      workspace.accounts = result.accounts;
    }
  } finally {
    busy.value = false;
  }
}
</script>
<template>
  <ActionMenu :label="t('Switch account', '切换账户')">
    <template #trigger
      ><Avatar
        v-if="current"
        :src="current.avatar_path"
        :alt="current.profile_name"
        :size="28" /><UserRound v-else :size="22" /><span>{{
        current?.profile_name || t("Accounts", "账户")
      }}</span
      ><ChevronDown :size="14"
    /></template>
    <small class="scope">{{
      instanceId
        ? t("For this instance", "此实例使用的账户")
        : t("Default account", "默认账户")
    }}</small>
    <button
      v-for="account in workspace.accounts"
      :key="account.id"
      :disabled="busy"
      @click="act(() => choose(account))"
    >
      <Avatar
        :src="account.avatar_path"
        :alt="account.profile_name"
        :size="32"
      />
      <span class="grow"
        >{{ account.profile_name }}<small>{{ account.provider }}</small></span
      >
      <Check v-if="current?.id === account.id" :size="16" />
    </button>
    <hr />
    <RouterLink to="/accounts"
      ><Users :size="18" />{{ t("Manage accounts", "管理账户") }}</RouterLink
    >
  </ActionMenu>
</template>
<style scoped>
.scope {
  padding: 8px 10px 4px;
}
</style>
