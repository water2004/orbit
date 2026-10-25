<script setup lang="ts">
import { ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Plus,
  RefreshCw,
  LogOut,
  Check,
  ExternalLink,
  Trash2,
} from "lucide-vue-next";
import { workspace, act } from "../services/workspace";
import { useCommandScope } from "../services/task-scope";
import type { Account } from "../types";
import { t } from "../i18n";
import Avatar from "../components/Avatar.vue";
import Button from "../components/upstream/Button.vue";
import Modal from "../components/Modal.vue";
import DropdownSelect from "../components/upstream/DropdownSelect.vue";
import { openAuthEndpoint } from "../services/auth-endpoint";
const route = useRoute();
const router = useRouter();
const { task, launcher, run } = useCommandScope();
const flow = ref(""),
  name = ref(""),
  username = ref(""),
  password = ref(""),
  profile = ref(""),
  provider = ref(""),
  busy = ref(false);
const providers = ref<{ id: string; api_root: string }[]>([]),
  remove = ref<Account>();
const session = ref<{
  login_session_id: string;
  verification_uri: string;
  user_code: string;
}>();
watch(provider, () => {
  password.value = "";
});
async function reload() {
  const result = await launcher<{ accounts: Account[] }>(
    ["account", "list"],
    t("Loading accounts", "正在加载账户"),
    null,
  );
  workspace.accounts = result.accounts;
  workspace.accountsError = "";
}
async function loadProviders() {
  providers.value = (
    await launcher<{ providers: typeof providers.value }>(
      ["config", "yggdrasil", "list"],
      t("Loading authentication endpoints", "正在加载认证端点"),
      null,
    )
  ).providers;
  provider.value = provider.value || providers.value[0]?.id || "";
}
async function change(args: string[]) {
  await launcher(args, t("Updating account", "正在更新账户"), null);
  await reload();
}
async function signIn() {
  busy.value = true;
  try {
    if (flow.value === "offline")
      await change(["account", "login", "offline", name.value]);
    else {
      const secret = password.value;
      password.value = "";
      await run({
        kind: "launcher",
        args: [
          "account",
          "login",
          "yggdrasil",
          "--provider",
          provider.value,
          "--username",
          username.value,
          "--password-stdin",
          ...(profile.value ? ["--profile", profile.value] : []),
        ],
        secret,
        label: t("Signing in", "正在登录"),
      });
      await reload();
    }
    flow.value = "";
  } finally {
    busy.value = false;
  }
}
async function microsoft() {
  busy.value = true;
  try {
    session.value = await launcher(
      ["account", "login", "microsoft", "begin"],
      t("Starting Microsoft sign-in", "正在开始微软登录"),
      null,
    );
    flow.value = "";
    const uri = new URL(session.value!.verification_uri);
    if (uri.protocol !== "https:")
      throw new Error(t("Login URL must use HTTPS", "登录地址必须使用 HTTPS"));
    await openUrl(uri.toString());
  } finally {
    busy.value = false;
  }
}
async function complete() {
  busy.value = true;
  try {
    await change([
      "account",
      "login",
      "microsoft",
      "complete",
      session.value!.login_session_id,
    ]);
    session.value = undefined;
  } finally {
    busy.value = false;
  }
}
async function endpointAdded(id: string) {
  await loadProviders();
  provider.value = id;
}
watch(
  () => route.query.provider,
  (id, previous) => {
    if (typeof id !== "string" && previous !== undefined) return;
    return act(async () => {
      await loadProviders();
      if (
        typeof id === "string" &&
        providers.value.some((item) => item.id === id)
      ) {
        provider.value = id;
        username.value = "";
        password.value = "";
        profile.value = "";
        flow.value = "yggdrasil";
        const query = { ...route.query };
        delete query.provider;
        await router.replace({ path: route.path, query });
      }
    });
  },
  { immediate: true },
);
</script>
<template>
  <div class="page">
    <header class="page-heading">
      <div>
        <h1>{{ t("Accounts", "账户") }}</h1>
        <p>
          {{
            t(
              "Choose your identity for each installation. Credentials stay with Launcher.",
              "为每个实例选择身份，登录凭据由 Launcher 管理。",
            )
          }}
        </p>
      </div>
      <Button type="colored" @click="flow = 'choose'"
        ><Plus />{{ t("Add account", "添加账户") }}</Button
      >
    </header>
    <div v-if="workspace.accountsError" class="card stack">
      <p class="error">{{ workspace.accountsError }}</p>
      <Button @click="act(reload)">{{ t("Retry", "重试") }}</Button>
    </div>
    <TransitionGroup name="list" tag="div" class="accounts-grid">
      <article
        v-for="account in workspace.accounts"
        :key="account.id"
        class="card stack"
      >
        <div class="row">
          <Avatar
            skin
            :src="account.avatar_path"
            :alt="account.profile_name"
            :size="64"
          />
          <div class="grow">
            <h2>{{ account.profile_name }}</h2>
            <p>{{ account.provider_id || account.provider }}</p>
          </div>
          <span v-if="account.is_default" class="badge"
            ><Check :size="14" />{{ t("Default", "默认") }}</span
          >
        </div>
        <p
          v-if="account.authentication_state === 'reauthentication-required'"
          class="error"
        >
          {{
            t(
              "Your session expired. Sign in again.",
              "登录已失效，请重新登录。",
            )
          }}
        </p>
        <div class="toolbar">
          <Button
            size="sm"
            @click="
              act(() => change(['account', 'select', account.id, '--global']))
            "
            >{{ t("Make default", "设为默认") }}</Button
          ><Button
            v-if="workspace.selected"
            size="sm"
            @click="
              act(() =>
                launcher(
                  ['account', 'select', account.id],
                  t('Selecting instance account', '正在选择实例账户'),
                ),
              )
            "
            >{{ t("Use for this instance", "用于当前实例") }}</Button
          >
        </div>
        <div class="toolbar">
          <Button
            v-if="account.authentication_state === 'reauthentication-required'"
            size="sm"
            @click="
              account.provider === 'microsoft'
                ? act(microsoft)
                : ((flow = 'yggdrasil'),
                  (username = account.login_name || ''),
                  (provider = account.provider_id || ''))
            "
            >{{ t("Sign in again", "重新登录") }}</Button
          ><Button
            type="quiet"
            size="sm"
            @click="act(() => change(['account', 'refresh', account.id]))"
            ><RefreshCw />{{ t("Refresh profile", "刷新资料") }}</Button
          ><span class="grow" /><Button
            type="quiet"
            size="sm"
            @click="remove = account"
            ><LogOut />{{ t("Sign out", "退出登录") }}</Button
          >
        </div>
      </article>
    </TransitionGroup>
    <div
      v-if="!workspace.accounts.length && !workspace.accountsError"
      class="empty"
    >
      <h2>{{ t("Who is playing?", "今天用哪个身份？") }}</h2>
      <p>
        {{
          t(
            "Add a Microsoft, Yggdrasil or offline account to get started.",
            "添加微软、Yggdrasil 或离线账户即可开始。",
          )
        }}
      </p>
    </div>
    <details class="card">
      <summary>{{ t("Authentication endpoints", "认证端点") }}</summary>
      <div class="stack" style="margin-top: 16px">
        <div v-for="item in providers" :key="item.id" class="row">
          <div class="grow">
            <strong>{{ item.id }}</strong>
            <p>{{ item.api_root }}</p>
          </div>
          <Button
            type="quiet"
            :title="t('Remove endpoint', '移除端点')"
            @click="
              act(async () => {
                await launcher(
                  ['config', 'yggdrasil', 'remove', item.id],
                  t('Removing endpoint', '正在移除端点'),
                  null,
                );
                await loadProviders();
              })
            "
            ><Trash2
          /></Button>
        </div>
        <Button @click="openAuthEndpoint('', endpointAdded)"
          ><Plus />{{ t("Add endpoint", "添加端点") }}</Button
        >
      </div>
    </details>
    <Modal
      v-if="flow"
      :task="task"
      :title="t('Add account', '添加账户')"
      @close="
        flow = '';
        password = '';
      "
    >
      <div v-if="flow === 'choose'" class="stack">
        <button
          class="signin-option card"
          :disabled="busy"
          @click="act(microsoft)"
        >
          <h3>Microsoft</h3>
          <p>
            {{
              t(
                "Sign in securely in your browser using a device code.",
                "通过设备码在浏览器中安全登录。",
              )
            }}
          </p></button
        ><button class="signin-option card" @click="flow = 'yggdrasil'">
          <h3>Yggdrasil</h3>
          <p>
            {{
              t(
                "Use an external authentication service.",
                "使用第三方认证服务。",
              )
            }}
          </p></button
        ><button class="signin-option card" @click="flow = 'offline'">
          <h3>{{ t("Offline", "离线") }}</h3>
          <p>
            {{
              t(
                "A local profile without online authentication.",
                "不进行在线认证的本地身份。",
              )
            }}
          </p>
        </button>
      </div>
      <form v-else class="stack" @submit.prevent="act(signIn)">
        <label v-if="flow === 'offline'" class="field"
          >{{ t("Player name", "玩家名称")
          }}<input v-model="name" autocomplete="off"
        /></label>
        <template v-else
          ><label class="field"
            >{{ t("Authentication service", "认证服务")
            }}<DropdownSelect
              v-model="provider"
              :label="t('Authentication service', '认证服务')"
              :options="
                providers.map((item) => ({
                  value: item.id,
                  label: item.id,
                  description: item.api_root,
                }))
              " /></label
          ><Button type="quiet" @click="openAuthEndpoint('', endpointAdded)"
            ><Plus />{{ t("Add a service first", "添加认证端点") }}</Button
          ><label class="field"
            >{{ t("Email or username", "邮箱或用户名")
            }}<input v-model="username" autocomplete="username" /></label
          ><label class="field"
            >{{ t("Password", "密码")
            }}<input
              v-model="password"
              type="password"
              autocomplete="current-password" /></label
          ><label class="field"
            >{{ t("Profile (optional)", "角色（可选）")
            }}<input v-model="profile" /></label
        ></template>
      </form>
      <template v-if="flow !== 'choose'" #footer
        ><Button type="quiet" @click="flow = 'choose'">{{
          t("Back", "返回")
        }}</Button
        ><Button
          type="colored"
          :disabled="
            busy ||
            (flow === 'offline'
              ? !name.trim()
              : !provider || !username || !password)
          "
          @click="act(signIn)"
          >{{ t("Sign in", "登录") }}</Button
        ></template
      >
    </Modal>
    <Modal
      v-if="session"
      :task="task"
      :title="t('Sign in to Microsoft', '登录微软账户')"
      @close="session = undefined"
      ><div class="stack">
        <p>
          {{
            t(
              "Enter this code in the browser, then finish sign-in below.",
              "在浏览器中输入此代码，授权后点击下方完成登录。",
            )
          }}
        </p>
        <strong class="device-code">{{ session.user_code }}</strong>
        <p>{{ session.verification_uri }}</p>
        <Button @click="act(() => openUrl(session!.verification_uri))"
          ><ExternalLink />{{ t("Open sign-in page", "打开登录网页") }}</Button
        >
      </div>
      <template #footer
        ><Button type="colored" :disabled="busy" @click="act(complete)">{{
          t("Finish sign-in", "完成登录")
        }}</Button></template
      ></Modal
    >
    <Modal
      v-if="remove"
      :task="task"
      :title="t('Sign out of this account?', '退出此账户？')"
      @close="remove = undefined"
      ><p>
        {{ remove.profile_name }} ·
        {{
          t(
            "Only this account will be signed out.",
            "仅退出这个账户，不影响其他账户。",
          )
        }}
      </p>
      <template #footer
        ><Button @click="remove = undefined">{{ t("Cancel", "取消") }}</Button
        ><Button
          type="colored"
          @click="
            act(async () => {
              await change(['account', 'logout', remove!.id]);
              remove = undefined;
            })
          "
          >{{ t("Sign out", "退出登录") }}</Button
        ></template
      ></Modal
    >
  </div>
</template>
<style scoped>
.accounts-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(330px, 1fr));
  gap: 16px;
}
.signin-option {
  text-align: left;
  cursor: pointer;
}
.signin-option:hover {
  border-color: var(--color-brand);
}
.signin-option p {
  margin-top: 6px;
}
.device-code {
  font-size: 32px;
  letter-spacing: 5px;
  padding: 18px;
  text-align: center;
  background: var(--surface-4);
  border-radius: 12px;
  user-select: all;
}
</style>
