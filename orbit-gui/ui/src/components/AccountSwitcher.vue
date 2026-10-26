<script setup lang="ts">
import { accountCommands } from "../services/commands";
import { computed, ref } from "vue";
import { Check, ChevronDown, UserRound, Users } from "lucide-vue-next";
import { workspace, act, launcher, refresh } from "../services/workspace";
import { accountProviderLabel } from "../services/account-label";
import type { Account } from "../types";
import { t } from "../i18n";
import ActionMenu from "./ActionMenu.vue";
import Avatar from "./Avatar.vue";
const busy = ref(false);
const current = computed(() => workspace.accounts.find((a) => a.is_default));
async function choose(account: Account) {
  busy.value = true;
  try {
    await launcher(
      accountCommands.default(account.id),
      t("Switching account", "正在切换账户"),
      null,
    );
    await refresh();
  } finally {
    busy.value = false;
  }
}
</script>
<template>
  <ActionMenu :label="t('Switch default account', '切换默认账户')">
    <template #trigger
      ><Avatar
        skin
        v-if="current"
        :src="current.avatar_path"
        :alt="current.profile_name"
        :size="28" /><UserRound v-else :size="22" /><span>{{
        current?.profile_name || t("Accounts", "账户")
      }}</span
      ><small v-if="current" class="default-label">{{
        t("Default", "默认")
      }}</small
      ><ChevronDown :size="14"
    /></template>
    <small class="scope">{{
      t(
        "Default account for instances without their own selection",
        "未单独指定账户的实例使用此默认账户",
      )
    }}</small>
    <button
      v-for="account in workspace.accounts"
      :key="account.id"
      :disabled="busy"
      @click="act(() => choose(account))"
    >
      <Avatar
        skin
        :src="account.avatar_path"
        :alt="account.profile_name"
        :size="32"
      />
      <span class="grow"
        >{{ account.profile_name
        }}<small>{{
          accountProviderLabel(account.provider, account.provider_id)
        }}</small></span
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
.default-label {
  color: var(--color-secondary);
  font-size: 11px;
}
</style>
