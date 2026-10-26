<script setup lang="ts">
import { computed } from "vue";
import { AlertCircle, UserRound } from "lucide-vue-next";
import type { LaunchAccount } from "../types";
import { t } from "../i18n";
import { accountProviderLabel } from "../services/account-label";
import Avatar from "./Avatar.vue";
const props = defineProps<{ selection?: LaunchAccount | null }>();
const account = computed(() =>
  props.selection?.state === "selected" ? props.selection.account : undefined,
);
const label = computed(() => {
  if (account.value) return account.value.profile_name;
  return props.selection?.state === "unavailable"
    ? t("Account unavailable", "账户不可用")
    : t("No launch account", "未选择启动账户");
});
const description = computed(() =>
  account.value
    ? accountProviderLabel(
        account.value.provider.kind,
        account.value.provider.provider_id,
      )
    : "",
);
const scope = computed(() =>
  props.selection?.state === "selected" ||
  props.selection?.state === "unavailable"
    ? props.selection.source === "global"
      ? t("Global default", "全局默认账户")
      : t("Instance account", "实例指定账户")
    : "",
);
</script>
<template>
  <div
    class="launch-account"
    :title="[label, description, scope].filter(Boolean).join(' · ')"
  >
    <Avatar
      v-if="account"
      :src="account.avatar_path || undefined"
      :alt="account.profile_name"
      :size="22"
      skin
    /><UserRound v-else :size="20" />
    <span class="launch-account-text"
      >{{ label }}<small v-if="description"> · {{ description }}</small></span
    >
    <AlertCircle
      v-if="account?.authentication_state === 'reauthentication-required'"
      :size="15"
      :aria-label="t('Sign in again', '需要重新登录')"
    />
  </div>
</template>
<style scoped>
.launch-account {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  width: 100%;
  color: var(--color-primary);
  font-size: 12px;
  line-height: 22px;
}
.launch-account > :not(.launch-account-text) {
  flex-shrink: 0;
}
.launch-account-text {
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
small {
  font-size: inherit;
}
</style>
