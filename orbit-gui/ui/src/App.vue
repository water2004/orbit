<script setup lang="ts">
// Shell structure adapted from Modrinth App's App.vue and title-bar/index.vue.
import { onMounted, watchEffect } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  Home,
  Compass,
  Plus,
  Settings,
  User,
  Activity,
  ChevronLeft,
  ChevronRight,
  RefreshCw,
  X,
} from "lucide-vue-next";
import {
  workspace,
  initialize,
  refresh,
  selectInstance,
  act,
} from "./services/workspace";
import { activity } from "./services/cli";
import { language, t } from "./i18n";
import NavButton from "./components/upstream/NavButton.vue";
import Button from "./components/upstream/Button.vue";
import ActivityPanel from "./components/ActivityPanel.vue";
import Avatar from "./components/Avatar.vue";
const router = useRouter();
const route = useRoute();
function changeInstance(event: Event) {
  const id = (event.target as HTMLSelectElement).value;
  const instance = workspace.instances.find((item) => item.id === id);
  if (instance) void act(() => selectInstance(instance));
}
onMounted(initialize);
const system = window.matchMedia("(prefers-color-scheme: dark)");
function applyTheme() {
  const p = workspace.preferences;
  document.documentElement.dataset.theme =
    p?.theme_mode === "system" || !p
      ? system.matches
        ? "dark"
        : "light"
      : p.theme_mode;
  document.documentElement.dataset.accent = p?.accent_theme || "indigo";
}
system.addEventListener("change", applyTheme);
watchEffect(() => {
  applyTheme();
  if (workspace.preferences) language.value = workspace.preferences.language;
});
</script>
<template>
  <div class="app-grid">
    <header class="app-title-bar">
      <img src="/orbit.svg" alt="" width="28" height="28" /><strong
        class="wordmark"
        >Orbit</strong
      ><Button
        type="quiet"
        size="sm"
        :title="t('Back', '后退')"
        @click="router.back()"
        ><ChevronLeft /></Button
      ><Button
        type="quiet"
        size="sm"
        :title="t('Forward', '前进')"
        @click="router.forward()"
        ><ChevronRight /></Button
      ><span class="grow"></span>
      <select
        v-if="route.path.startsWith('/instance') || route.path === '/browse'"
        :value="workspace.selected?.id"
        :aria-label="t('Installation', '实例')"
        @change="changeInstance"
      >
        <option
          v-for="instance in workspace.instances"
          :key="instance.id"
          :value="instance.id"
        >
          {{ instance.name }}
        </option>
      </select>
      <Button type="quiet" :title="t('Refresh', '刷新')" @click="act(refresh)"
        ><RefreshCw /></Button
      ><Button
        type="quiet"
        :title="t('Activity', '活动')"
        @click="activity.open = !activity.open"
        ><Activity
      /></Button>
    </header>
    <nav class="app-grid-navbar">
      <NavButton
        to="/"
        :is-primary="(r) => r.path === '/' || r.path.startsWith('/instance')"
        :title="t('Library', '实例库')"
        ><Home /></NavButton
      ><NavButton to="/browse" :title="t('Discover', '发现')"
        ><Compass /></NavButton
      ><NavButton to="/create" :title="t('Create instance', '创建实例')"
        ><Plus /></NavButton
      ><span class="grow"></span
      ><NavButton to="/settings" :title="t('Settings', '设置')"
        ><Settings /></NavButton
      ><NavButton to="/accounts" :title="t('Accounts', '账户')"
        ><Avatar
          v-if="workspace.accounts.find((a) => a.is_default)?.avatar_path"
          :src="workspace.accounts.find((a) => a.is_default)?.avatar_path"
          :size="28" /><User v-else
      /></NavButton>
    </nav>
    <main class="app-content">
      <RouterView v-slot="{ Component }"
        ><Transition name="page" mode="out-in"
          ><component :is="Component" :key="route.fullPath" /></Transition
      ></RouterView>
    </main>
    <div class="app-status"><ActivityPanel /></div>
    <div v-if="workspace.error" role="alert" class="error-toast">
      <p>{{ workspace.error }}</p>
      <Button
        type="quiet"
        :title="t('Dismiss', '关闭')"
        @click="workspace.error = ''"
        ><X
      /></Button>
    </div>
  </div>
</template>
<style scoped>
.app-grid {
  height: 100dvh;
  display: grid;
  grid-template-columns: 72px minmax(0, 1fr);
  grid-template-rows: 56px minmax(0, 1fr) auto;
  grid-template-areas: "title title" "nav content" "nav status";
}
.app-title-bar {
  grid-area: title;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 16px;
  background: var(--surface-2);
}
.wordmark {
  font-size: 20px;
  margin-right: 16px;
}
.app-title-bar select {
  max-width: 300px;
  min-width: 180px;
  padding: 7px 10px;
}
.app-grid-navbar {
  grid-area: nav;
  background: var(--surface-2);
  padding: 12px 8px;
  display: flex;
  align-items: center;
  flex-direction: column;
  gap: 10px;
}
.app-grid-navbar svg {
  width: 23px;
  height: 23px;
}
.app-content {
  grid-area: content;
  overflow: auto;
  overscroll-behavior: contain;
  border-radius: 18px 0 0 0;
  border-left: 1px solid var(--surface-4);
  border-top: 1px solid var(--surface-4);
}
.app-status {
  grid-area: status;
}
.error-toast {
  position: fixed;
  z-index: 50;
  bottom: 58px;
  right: 24px;
  max-width: min(540px, 80vw);
  background: var(--surface-3);
  border: 1px solid var(--color-red);
  border-radius: 14px;
  box-shadow: 0 12px 40px #0004;
  padding: 14px;
  display: flex;
  gap: 10px;
  align-items: flex-start;
}
.error-toast p {
  max-height: 130px;
  overflow: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
